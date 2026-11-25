//! the update on windows: the request through winhttp, sha-256 through cng, signatures through wintrust, and the
//! release's installer, which puts the new build in place once this one has ended and starts it again (adr 0008).
//! nothing is written to the install folder here, and nothing is deleted.

use super::{setup_name, split, Asset, Release, Version, AGENT, WAIT};
use crate::preview::{home, safe_root};
use std::ffi::{c_void, OsString};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use windows::core::{w, Error, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, INVALID_HANDLE_VALUE, TRUST_E_NOSIGNATURE};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH, WINHTTP_DISABLE_COOKIES, WINHTTP_FLAG_SECURE, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_AUTOLOGON_POLICY,
    WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP, WINHTTP_OPTION_REDIRECT_POLICY_NEVER, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE,
};
use windows::Win32::Security::Cryptography::{
    BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptHashData, CertGetNameStringW, BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, CERT_NAME_SIMPLE_DISPLAY_TYPE,
};
use windows::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_NONE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
};
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;

/// how much is read or written at a time
const PIECE: usize = 1 << 16;
const STOPPED: &str = "stopped";
const UNSIGNED: &str = "this build is not signed: install updates by hand";
const NOT_SIGNED: &str = "it is not signed";
/// what the installer hands the new build as its command line: the files open now (scripts\installer.iss)
const REOPEN: &str = "EYEPIECE_REOPEN";

/// the installer `apply` checked, held open until `relaunch` starts it
static CHECKED: Mutex<Option<(PathBuf, File)>> = Mutex::new(None);

/// asks for `url` and gives its body, refusing one longer than `limit`
pub(super) fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    get(url, &mut |piece| {
        body.extend_from_slice(piece);
        if body.len() > limit { Err("the answer is too long for a release manifest".to_string()) } else { Ok(()) }
    })?;
    Ok(body)
}

/// downloads the release's installer into the update's folder under the os cache folder, its size and sha-256
/// counted as it lands; one that does not match, or stops coming, is set aside there as `<name>.failed`, where only
/// its next failure replaces it. `progress` hears the bytes come so far and the bytes to come in all; `stop`, once
/// set, ends it at the next piece. an installer already downloaded whole is not asked for again.
pub fn download(release: &Release, _: &Path, progress: &mut dyn FnMut(u64, u64), stop: &AtomicBool) -> Result<(), String> {
    let staged = staging(release.version)?;
    fs::create_dir_all(&staged).map_err(|e| format!("eyepiece cannot write to {}: {e}", staged.display()))?;
    let setup = installer(release)?;
    if holds(&staged.join(&setup.name), setup)? {
        progress(0, 0);
        return Ok(());
    }
    let mut done = 0;
    progress(done, setup.size);
    let mut landing = Landing::begin(&staged, setup)?;
    let got = get(&release.url_of(setup), &mut |piece| {
        if stop.load(Ordering::Relaxed) {
            return Err(STOPPED.to_string());
        }
        landing.take(piece)?;
        done += piece.len() as u64;
        progress(done, setup.size);
        Ok(())
    });
    landing.end(got)
}

/// checks the downloaded installer from the disk once more, its size and sha-256, and that it is signed as the
/// running program is; a build that is not signed itself updates nothing. the installer is then held, so nothing can
/// change it, until `relaunch` starts it. says how many files it checked: the one
pub fn apply(release: &Release, _: &Path, current: &str) -> Result<usize, String> {
    let from = Version::parse(current).ok_or_else(|| format!("{current} is not a version"))?;
    let running = std::env::current_exe().map_err(|e| format!("eyepiece cannot find itself: {e}"))?;
    let checked = check(release, &staging(release.version)?, from, &running, &signer_of)?;
    *CHECKED.lock().unwrap_or_else(PoisonError::into_inner) = Some(checked);
    Ok(1)
}

/// starts the installer `apply` checked, for the caller to end once it has. the installer waits for this process to
/// end, which holds the installed files and the one-window name, installs, and starts the new build on `args`.
pub fn relaunch(_: &Path, args: &[OsString]) -> Result<(), String> {
    let (setup, _held) = CHECKED.lock().unwrap_or_else(PoisonError::into_inner).take().ok_or("the update's installer has not been checked")?;
    Command::new(&setup).args(setup_args(std::process::id())).env(REOPEN, reopen(args)).spawn().map(drop).map_err(|e| format!("{} did not start: {e}", setup.display()))
}

/// the installer's switches for an update: no wizard, only its progress (and a message, if it fails), no restart of
/// the system, and the player it waits for (scripts\installer.iss)
fn setup_args(pid: u32) -> [String; 3] {
    ["/SILENT".into(), "/NORESTART".into(), format!("/update={pid}")]
}

/// `files` as a command line, each in quotes as windows splits one. a path holds no quote, so only the backslashes
/// before the closing quote are doubled
fn reopen(files: &[OsString]) -> OsString {
    let mut line = OsString::new();
    for (i, file) in files.iter().enumerate() {
        line.push(if i == 0 { "\"" } else { " \"" });
        line.push(file);
        line.push("\\".repeat(file.as_encoded_bytes().iter().rev().take_while(|&&b| b == b'\\').count()));
        line.push("\"");
    }
    line
}

/// the release's installer as the manifest names it
fn installer(release: &Release) -> Result<&Asset, String> {
    let name = setup_name(release.version);
    release.files.iter().find(|f| f.name == name).ok_or_else(|| format!("the release's manifest has no {name}"))
}

/// `apply`, with its folder and the signer check handed in, so the tests can give their own: the installer, open,
/// once it is the release's and signed as `running` is
fn check(release: &Release, staged: &Path, from: Version, running: &Path, signer_of: &dyn Fn(&Path) -> Result<Signer, String>) -> Result<(PathBuf, File), String> {
    if release.version <= from {
        return Err(format!("{} is not newer than {from}", release.version));
    }
    let setup = installer(release)?;
    let path = staged.join(&setup.name);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(format!("{} has not been downloaded", setup.name)),
        Err(e) => return Err(format!("{} cannot be read: {e}", path.display())),
        // a folder or a link there is never taken for it
        Ok(meta) if !meta.is_file() => return Err(format!("{} is not a file", path.display())),
        Ok(_) => {}
    }
    // shared for reading alone: from here until it has started, no program can write, rename or remove it, so what
    // runs is what was checked
    let mut file = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ.0).open(&path).map_err(|e| format!("{} cannot be read: {e}", path.display()))?;
    if sha256_of(&mut file).map_err(|e| format!("{} cannot be read: {e}", path.display()))? != (setup.size, setup.sha256) {
        return Err(format!("{} does not match the release's size and checksum", setup.name));
    }
    signed_alike(&path, running, signer_of)?;
    Ok((path, file))
}

/// whether `path` is a file of the size and sha-256 `asset` names: false when nothing is there. a folder or a link
/// there is an error, never taken for the file
fn holds(path: &Path, asset: &Asset) -> Result<bool, String> {
    let cannot = |e: io::Error| format!("{} cannot be read: {e}", path.display());
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(cannot(e)),
        Ok(meta) if !meta.is_file() => Err(format!("{} is not a file", path.display())),
        Ok(meta) if meta.len() != asset.size => Ok(false),
        Ok(_) => Ok(File::open(path).and_then(|mut file| sha256_of(&mut file)).map_err(cannot)? == (asset.size, asset.sha256)),
    }
}

/// the update's own folder for one release, under the os cache folder: `%LOCALAPPDATA%\eyepiece\update\<version>`,
/// or under `EYEPIECE_CACHE` when that is set, as the cache's is. never the install folder
fn staging(version: Version) -> Result<PathBuf, String> {
    let system = std::env::var_os("EYEPIECE_CACHE").or_else(|| std::env::var_os("LOCALAPPDATA")).ok_or("the system names no cache folder")?;
    let dir = PathBuf::from(system).join("eyepiece").join("update").join(version.to_string());
    if !safe_root(&dir, home().as_deref(), &[]) {
        return Err(format!("{} is not a safe folder for the update", dir.display()));
    }
    Ok(dir)
}

/// one file of a release as it comes, into `<name>.part` in the update's folder, its size and sha-256 counted on the
/// way. it ends as `<name>` when both are the release's, else it is set aside as `<name>.failed`
struct Landing<'a> {
    asset: &'a Asset,
    part: PathBuf,
    out: File,
    hash: Sha256,
    size: u64,
}

impl<'a> Landing<'a> {
    fn begin(staged: &Path, asset: &'a Asset) -> Result<Landing<'a>, String> {
        let part = staged.join(format!("{}.part", asset.name));
        let out = File::create(&part).map_err(|e| format!("eyepiece cannot write to {}: {e}", staged.display()))?;
        Ok(Landing { asset, part, out, hash: Sha256::new().map_err(|e| e.to_string())?, size: 0 })
    }

    fn take(&mut self, piece: &[u8]) -> Result<(), String> {
        self.size += piece.len() as u64;
        if self.size > self.asset.size {
            return Err(format!("{} is longer than the release says", self.asset.name));
        }
        self.hash.update(piece).and_then(|()| self.out.write_all(piece)).map_err(|e| format!("{} could not be written: {e}", self.asset.name))
    }

    /// `got` is how the request ended
    fn end(self, got: Result<(), String>) -> Result<(), String> {
        let Landing { asset, part, out, hash, size } = self;
        let whole = got.and_then(|()| {
            if size != asset.size {
                return Err(format!("{} came short: {size} of {} bytes", asset.name, asset.size));
            }
            if hash.finish().map_err(|e| e.to_string())? != asset.sha256 {
                return Err(format!("{} does not match the release's checksum", asset.name));
            }
            out.sync_all().map_err(|e| format!("{} could not be written: {e}", asset.name))
        });
        drop(out);
        let to = part.with_file_name(if whole.is_ok() { asset.name.clone() } else { format!("{}.failed", asset.name) });
        let moved = fs::rename(&part, &to).map_err(|e| format!("{} could not be renamed {}: {e}", part.display(), to.display()));
        whole.and(moved)
    }
}

/// who signed a file, as far as the update compares: the name its certificate is issued to, as encoded, and the root
/// its chain ends in. not the certificate itself: artifact signing issues a new one, good for three days, to each build
struct Signer {
    /// the name in words, for what is said
    name: String,
    subject: Vec<u8>,
    root: Vec<u8>,
}

/// the installer at `setup` is signed as the running program is: the same subject and the same root. a running
/// program that is not signed (a developer build) cannot say whose a new one must be, so it takes none
fn signed_alike(setup: &Path, running: &Path, signer_of: &dyn Fn(&Path) -> Result<Signer, String>) -> Result<(), String> {
    let ours = signer_of(running).map_err(|why| if why == NOT_SIGNED { UNSIGNED.to_string() } else { format!("this build's own signature cannot be checked, {why}: install updates by hand") })?;
    let name = setup.file_name().unwrap_or_default().to_string_lossy();
    let not_ours = |why: &str| format!("{name} is not signed by {}: {why}", ours.name);
    let theirs = signer_of(setup).map_err(|why| not_ours(&why))?;
    if theirs.subject != ours.subject {
        return Err(not_ours(&format!("its signer is {}", theirs.name)));
    }
    if theirs.root != ours.root {
        return Err(not_ours("its certificate comes from another authority"));
    }
    Ok(())
}

/// the signer of a file whose authenticode signature windows trusts. nothing is fetched to decide it, no revocation
/// list and no certificate the system lacks, so the player asks nothing of any server but github's
fn signer_of(path: &Path) -> Result<Signer, String> {
    let path = HSTRING::from(path);
    let mut file = WINTRUST_FILE_INFO { cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: PCWSTR(path.as_ptr()), ..Default::default() };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // no window to ask anyone from
    let alone = HWND(INVALID_HANDLE_VALUE.0);
    let verdict = unsafe { WinVerifyTrust(alone, &mut action, &mut data as *mut WINTRUST_DATA as *mut c_void) };
    let signer = match verdict {
        0 => unsafe { read_signer(data.hWVTStateData) },
        code if code == TRUST_E_NOSIGNATURE.0 => Err(NOT_SIGNED.to_string()),
        code => Err(format!("its signature does not hold (0x{:08x})", code as u32)),
    };
    // the verdict's state is held until it is closed, whatever the verdict
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe { WinVerifyTrust(alone, &mut action, &mut data as *mut WINTRUST_DATA as *mut c_void) };
    signer
}

/// the signer's subject and root, from a trusted verdict's state
///
/// safety: `state` is a verdict's, not yet closed
unsafe fn read_signer(state: HANDLE) -> Result<Signer, String> {
    let none = || "its signature names no signer".to_string();
    let provider = WTHelperProvDataFromStateData(state);
    let signer = if provider.is_null() { std::ptr::null_mut() } else { WTHelperGetProvSignerFromChain(provider, 0, false, 0) };
    if signer.is_null() || (*signer).csCertChain == 0 || (*signer).pasCertChain.is_null() {
        return Err(none());
    }
    // from the signer's own certificate to the root
    let chain = std::slice::from_raw_parts((*signer).pasCertChain, (*signer).csCertChain as usize);
    let (leaf, root) = (chain[0].pCert, chain[chain.len() - 1].pCert);
    if leaf.is_null() || root.is_null() || (*leaf).pCertInfo.is_null() {
        return Err(none());
    }
    let bytes = |at: *const u8, len: u32| if at.is_null() { Vec::new() } else { std::slice::from_raw_parts(at, len as usize).to_vec() };
    let subject = &(*(*leaf).pCertInfo).Subject;
    let mut name = [0u16; 256];
    // the count takes in the closing nul
    let len = CertGetNameStringW(leaf, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut name)) as usize;
    Ok(Signer {
        name: String::from_utf16_lossy(&name[..len.saturating_sub(1).min(name.len())]),
        subject: bytes(subject.pbData, subject.cbData),
        root: bytes((*root).pbCertEncoded, (*root).cbCertEncoded),
    })
}

/// sha-256, through the system's cng
struct Sha256(BCRYPT_HASH_HANDLE);

impl Sha256 {
    fn new() -> io::Result<Sha256> {
        let mut hash = BCRYPT_HASH_HANDLE(std::ptr::null_mut());
        // the algorithm's own pseudo handle: no provider to open or close
        unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut hash, None, None, 0) }.ok().map_err(io::Error::other)?;
        Ok(Sha256(hash))
    }

    fn update(&mut self, bytes: &[u8]) -> io::Result<()> {
        unsafe { BCryptHashData(self.0, bytes, 0) }.ok().map_err(io::Error::other)
    }

    fn finish(self) -> io::Result<[u8; 32]> {
        let mut sum = [0; 32];
        unsafe { BCryptFinishHash(self.0, &mut sum, 0) }.ok().map_err(io::Error::other)?;
        Ok(sum)
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        let _ = unsafe { BCryptDestroyHash(self.0) };
    }
}

/// a file's size and sha-256, read through
fn sha256_of(file: &mut File) -> io::Result<(u64, [u8; 32])> {
    let (mut hash, mut size, mut piece) = (Sha256::new()?, 0, vec![0; PIECE]);
    loop {
        let n = file.read(&mut piece)?;
        if n == 0 {
            return Ok((size, hash.finish()?));
        }
        hash.update(&piece[..n])?;
        size += n as u64;
    }
}

/// a winhttp handle, closed with it
struct Handle(*mut c_void);

impl Handle {
    fn new(handle: *mut c_void) -> Result<Handle, String> {
        if handle.is_null() { Err(lost(&Error::from_thread())) } else { Ok(Handle(handle)) }
    }

    fn set(&self, option: u32, value: u32) -> Result<(), String> {
        unsafe { WinHttpSetOption(Some(self.0 as *const c_void), option, Some(&value.to_ne_bytes())) }.map_err(|e| lost(&e))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// asks for `url` and hands its body to `each` a piece at a time, as it comes. redirects are followed, but never
/// from https down to http; plain http, which only the seam asks and only of 127.0.0.1, follows none. no cookie and
/// no logon are ever sent, and the system's proxy settings are used
fn get(url: &str, each: &mut dyn FnMut(&[u8]) -> Result<(), String>) -> Result<(), String> {
    let (secure, host, port, path) = split(url)?;
    let wait = WAIT.as_millis() as i32;
    let session = Handle::new(unsafe { WinHttpOpen(&HSTRING::from(AGENT), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0) })?;
    unsafe { WinHttpSetTimeouts(session.0, wait, wait, wait, wait) }.map_err(|e| lost(&e))?;
    let connection = Handle::new(unsafe { WinHttpConnect(session.0, &HSTRING::from(host), port, 0) })?;
    let secure_flag = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
    let request = Handle::new(unsafe { WinHttpOpenRequest(connection.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), secure_flag) })?;
    request.set(WINHTTP_OPTION_REDIRECT_POLICY, if secure { WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP } else { WINHTTP_OPTION_REDIRECT_POLICY_NEVER })?;
    request.set(WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_DISABLE_COOKIES)?;
    request.set(WINHTTP_OPTION_AUTOLOGON_POLICY, WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH)?;
    unsafe { WinHttpSendRequest(request.0, None, None, 0, 0, 0) }.map_err(|e| lost(&e))?;
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }.map_err(|e| lost(&e))?;
    let (mut status, mut len) = (0u32, size_of::<u32>() as u32);
    unsafe { WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut u32 as *mut c_void), &mut len, std::ptr::null_mut()) }
        .map_err(|e| lost(&e))?;
    if status != 200 {
        return Err(format!("github answered {status}"));
    }
    let mut piece = vec![0u8; PIECE];
    loop {
        let mut read = 0;
        unsafe { WinHttpReadData(request.0, piece.as_mut_ptr() as *mut c_void, PIECE as u32, &mut read) }.map_err(|e| lost(&e))?;
        if read == 0 {
            return Ok(());
        }
        each(&piece[..read as usize])?;
    }
}

/// a request that failed, in words: winhttp's codes are not among the system's messages
fn lost(e: &Error) -> String {
    let why = match e.code().0 & 0xffff {
        12002 => "it timed out".to_string(),
        12007 => "its name did not resolve".to_string(),
        12029 => "no connection could be made".to_string(),
        12030 => "the connection was cut".to_string(),
        12175 => "its certificate was refused".to_string(),
        code => format!("winhttp error {code}"),
    };
    format!("github did not answer: {why}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a folder of its own for one test, in the temp folder
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("eyepiece-update-{test}-{}", std::process::id()));
            fs::create_dir(&dir).unwrap();
            Scratch(dir)
        }

        fn put(&self, name: &str, bytes: &[u8]) {
            fs::write(self.0.join(name), bytes).unwrap();
        }

        fn read(&self, name: &str) -> Vec<u8> {
            fs::read(self.0.join(name)).unwrap()
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
            names.sort();
            names
        }

        /// takes away what the test made, one name at a time, then the folder once it is empty. the folder must hold
        /// those names and no more; whatever could not be removed fails the test, named. a test that failed before
        /// this leaves its folder for a look
        fn clear(self, names: &[&str]) {
            let mut want: Vec<String> = names.iter().map(|n| n.to_string()).collect();
            want.sort();
            assert_eq!(self.names(), want, "in {}", self.0.display());
            let mut left = Vec::new();
            for name in names {
                let path = self.0.join(name);
                let gone = if path.is_dir() { fs::remove_dir(&path) } else { fs::remove_file(&path) };
                if let Err(e) = gone {
                    left.push(format!("{}: {e}", path.display()));
                }
            }
            if let Err(e) = fs::remove_dir(&self.0) {
                left.push(format!("{}: {e}", self.0.display()));
            }
            assert!(left.is_empty(), "left behind: {left:?}");
        }
    }

    fn sum(bytes: &[u8]) -> [u8; 32] {
        let mut hash = Sha256::new().unwrap();
        hash.update(bytes).unwrap();
        hash.finish().unwrap()
    }

    fn hex(sum: &[u8]) -> String {
        sum.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// a release of these files, as its manifest would name them
    fn release(version: &str, files: &[(&str, &[u8])]) -> Release {
        let mut text = format!("version = {version}\ndate = 2026-12-08\n");
        for (name, bytes) in files {
            text += &format!("file = {name} · {} · {}\n", bytes.len(), hex(&sum(bytes)));
        }
        super::super::read(&text, "https://example.invalid/").unwrap()
    }

    fn signer(subject: &str, root: &str) -> Signer {
        Signer { name: subject.to_string(), subject: subject.as_bytes().to_vec(), root: root.as_bytes().to_vec() }
    }

    fn tridant(_: &Path) -> Result<Signer, String> {
        Ok(signer("Tridant, Inc.", "root 2020"))
    }

    const OLD: Version = Version(0, 2, 1);
    const SETUP: &str = "eyepiece-0.3.0-windows-setup.exe";

    #[test]
    fn sha256_gives_the_known_sums() {
        assert_eq!(hex(&sum(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let mut abc = Sha256::new().unwrap();
        // in two pieces, as a download comes
        abc.update(b"a").unwrap();
        abc.update(b"bc").unwrap();
        assert_eq!(hex(&abc.finish().unwrap()), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sum(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    }

    #[test]
    fn the_installer_runs_only_once_it_checks() {
        let staged = Scratch::new("checked");
        // a notices file beside it in the manifest is not the update's
        let release = release("0.3.0", &[(SETUP, b"installer 0.3.0"), ("THIRD-PARTY-NOTICES.txt", b"notices")]);
        let try_it = |signer_of: &dyn Fn(&Path) -> Result<Signer, String>| check(&release, &staged.0, OLD, Path::new("running"), signer_of).map(|(path, _)| path);
        assert_eq!(try_it(&tridant), Err(format!("{SETUP} has not been downloaded")));
        staged.put(SETUP, b"installer 0.3.1");
        assert_eq!(try_it(&tridant), Err(format!("{SETUP} does not match the release's size and checksum")));
        staged.put(SETUP, b"installer 0.3.0, longer");
        assert_eq!(try_it(&tridant), Err(format!("{SETUP} does not match the release's size and checksum")));
        staged.put(SETUP, b"installer 0.3.0");
        assert_eq!(try_it(&|_| Err(NOT_SIGNED.into())), Err(UNSIGNED.to_string()));
        assert_eq!(try_it(&tridant), Ok(staged.0.join(SETUP)));
        // held from its check until it is started: nothing can write it, move it or take it away meanwhile
        let (_, held) = check(&release, &staged.0, OLD, Path::new("running"), &tridant).unwrap();
        assert!(fs::OpenOptions::new().write(true).open(staged.0.join(SETUP)).is_err());
        assert!(fs::rename(staged.0.join(SETUP), staged.0.join("moved.exe")).is_err());
        drop(held);
        assert_eq!(check(&release, &staged.0, Version(0, 3, 0), Path::new("running"), &tridant).err(), Some("0.3.0 is not newer than 0.3.0".to_string()));
        assert_eq!(staged.read(SETUP), b"installer 0.3.0");
        staged.clear(&[SETUP]);
        // a folder where the installer should be is not taken for it
        let folder = Scratch::new("checked-folder");
        fs::create_dir(folder.0.join(SETUP)).unwrap();
        assert!(check(&release, &folder.0, OLD, Path::new("running"), &tridant).unwrap_err().ends_with("is not a file"));
        folder.clear(&[SETUP]);
    }

    #[test]
    fn only_an_installer_signed_as_the_running_program_runs() {
        let setup = Path::new(r"C:\staged\eyepiece-0.3.0-windows-setup.exe");
        let running = Path::new(r"C:\installed\eyepiece.exe");
        let as_ = |theirs: Result<Signer, String>| {
            let theirs = std::cell::RefCell::new(Some(theirs));
            signed_alike(setup, running, &|path| if path == running { tridant(path) } else { theirs.borrow_mut().take().unwrap() })
        };
        assert_eq!(as_(tridant(setup)), Ok(()));
        assert_eq!(as_(Err("it is not signed".into())), Err(format!("{SETUP} is not signed by Tridant, Inc.: it is not signed")));
        assert_eq!(as_(Ok(signer("Someone Else", "root 2020"))), Err(format!("{SETUP} is not signed by Tridant, Inc.: its signer is Someone Else")));
        assert_eq!(as_(Ok(signer("Tridant, Inc.", "another root"))), Err(format!("{SETUP} is not signed by Tridant, Inc.: its certificate comes from another authority")));
        let ours = |why: &'static str| signed_alike(setup, running, &|path| if path == running { Err(why.into()) } else { tridant(path) });
        assert_eq!(ours(NOT_SIGNED), Err(UNSIGNED.to_string()));
        assert_eq!(
            ours("its signature does not hold (0x800b010a)"),
            Err("this build's own signature cannot be checked, its signature does not hold (0x800b010a): install updates by hand".to_string())
        );
    }

    #[test]
    fn the_installer_waits_for_this_player_and_reopens_its_files() {
        assert_eq!(setup_args(4242), ["/SILENT", "/NORESTART", "/update=4242"]);
        let files = [r"C:\clips\a b é.mov", r"\\nas\share\x.mov", r"C:\a folder\"].map(OsString::from);
        // as windows splits it again: a backslash is itself, except in a run before a quote, where two are one
        assert_eq!(reopen(&files), OsString::from(r#""C:\clips\a b é.mov" "\\nas\share\x.mov" "C:\a folder\\""#));
        assert_eq!(reopen(&[]), OsString::new());
    }

    #[test]
    fn nothing_starts_unless_it_was_checked() {
        assert_eq!(relaunch(Path::new("unused"), &[]), Err("the update's installer has not been checked".to_string()));
    }

    #[test]
    fn a_program_built_here_is_not_signed() {
        // the test program itself, as cargo built it, through wintrust
        assert_eq!(signer_of(&std::env::current_exe().unwrap()).err(), Some("it is not signed".to_string()));
    }

    /// the signers of signed files, through wintrust, as the update reads them, and that they are one signer to it:
    /// the files of EYEPIECE_SIGNED, separated by `;`, such as dist\eyepiece\eyepiece.exe and the installer after
    /// scripts\package.ps1, and a build signed on another day, whose certificate is another:
    /// `cargo test signed_files_name_their_signer -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn signed_files_name_their_signer() {
        let paths = std::env::var("EYEPIECE_SIGNED").expect("EYEPIECE_SIGNED names signed files");
        let signers: Vec<Signer> = paths.split(';').map(|path| signer_of(Path::new(path)).unwrap()).collect();
        for (path, signer) in paths.split(';').zip(&signers) {
            println!("{path}: {}, a subject of {} bytes, a root of {} bytes", signer.name, signer.subject.len(), signer.root.len());
        }
        assert!(!signers[0].name.is_empty());
        assert!(signers.iter().all(|s| s.subject == signers[0].subject && s.root == signers[0].root), "not one signer");
    }

    #[test]
    fn a_download_lands_only_whole() {
        let staged = Scratch::new("landing");
        let release = release("0.3.0", &[(SETUP, b"installer 0.3.0")]);
        let land = |pieces: &[&[u8]], net: Result<(), String>| {
            let mut landing = Landing::begin(&staged.0, &release.files[0]).unwrap();
            let got = pieces.iter().try_for_each(|piece| landing.take(piece)).and(net);
            landing.end(got)
        };
        assert_eq!(land(&[b"installer ", b"0.3.0"], Ok(())), Ok(()));
        assert_eq!(staged.read(SETUP), b"installer 0.3.0");
        assert_eq!(land(&[b"installer 0.3.1"], Ok(())), Err(format!("{SETUP} does not match the release's checksum")));
        assert_eq!(staged.read(&format!("{SETUP}.failed")), b"installer 0.3.1");
        assert_eq!(land(&[b"installer 0.3.0", b"!"], Ok(())), Err(format!("{SETUP} is longer than the release says")));
        assert_eq!(land(&[b"installer"], Ok(())), Err(format!("{SETUP} came short: 9 of 15 bytes")));
        assert_eq!(land(&[b"installer "], Err("github did not answer: it timed out".into())), Err("github did not answer: it timed out".to_string()));
        assert_eq!(staged.read(&format!("{SETUP}.failed")), b"installer ");
        // the whole one stayed as it was through the failures after it
        assert_eq!(staged.read(SETUP), b"installer 0.3.0");
        // and is known as downloaded, so it is not asked for again
        assert_eq!(holds(&staged.0.join(SETUP), &release.files[0]), Ok(true));
        staged.clear(&[SETUP, &format!("{SETUP}.failed")]);
    }

    /// a request for real, to a folder served by hand on this machine: not run with the others, which ask nothing of
    /// any server. EYEPIECE_UPDATE_FROM names it, as `http://127.0.0.1:39417/`:
    /// `cargo test a_release_is_read_over_http -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn a_release_is_read_over_http() {
        let release = super::super::check("0.0.1").unwrap().expect("a release newer than 0.0.1");
        println!("{} of {}: {} files, {}", release.version, release.date, release.files.len(), release.page);
    }
}
