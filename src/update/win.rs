//! the update on windows: the request through winhttp, sha-256 through cng, signatures through wintrust, and the
//! files put in place by renames. a running program and the libraries it loaded cannot be written over, but they can
//! be renamed: the installed file is set aside as `<name>.old-<version>` and the new one copied in beside it. nothing
//! is deleted here but those set-aside files, by `tidy`, one at a time by their exact names.

use super::{aside, set_aside, split, Asset, Release, Version, AGENT, PROGRAM, WAIT};
use crate::preview::{home, safe_root};
use std::ffi::{c_void, OsString};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
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
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};

/// how much is read or written at a time
const PIECE: usize = 1 << 16;
const STOPPED: &str = "stopped";
const UNSIGNED: &str = "this build is not signed: install updates by hand";
const NOT_SIGNED: &str = "it is not signed";
/// win32's codes, as an io error carries them
const SHARING_VIOLATION: i32 = 32;
const ACCESS_DENIED: i32 = 5;

/// asks for `url` and gives its body, refusing one longer than `limit`
pub(super) fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    get(url, &mut |piece| {
        body.extend_from_slice(piece);
        if body.len() > limit { Err("the answer is too long for a release manifest".to_string()) } else { Ok(()) }
    })?;
    Ok(body)
}

/// downloads the files of `release` that `install` does not already hold as they are, each on its own, into the
/// update's folder under the os cache folder. each file's size and sha-256 are counted as it lands, and a file that
/// does not match, or stops coming, is set aside there as `<name>.failed`, where only that file's next failure
/// replaces it. `progress` hears the bytes
/// come so far and the bytes to come in all; `stop`, once set, ends it at the next piece. a file already downloaded
/// whole is not asked for again.
pub fn download(release: &Release, install: &Path, progress: &mut dyn FnMut(u64, u64), stop: &AtomicBool) -> Result<(), String> {
    let staged = staging(release.version)?;
    fs::create_dir_all(&staged).map_err(|e| format!("eyepiece cannot write to {}: {e}", staged.display()))?;
    let mut wanted = Vec::new();
    for asset in changed(release, install)? {
        if !holds(&staged.join(&asset.name), asset)? {
            wanted.push(asset);
        }
    }
    let total = wanted.iter().map(|a| a.size).sum();
    let mut done = 0;
    progress(done, total);
    for asset in wanted {
        let mut landing = Landing::begin(&staged, asset)?;
        let got = get(&release.url_of(asset), &mut |piece| {
            if stop.load(Ordering::Relaxed) {
                return Err(STOPPED.to_string());
            }
            landing.take(piece)?;
            done += piece.len() as u64;
            progress(done, total);
            Ok(())
        });
        landing.end(got)?;
    }
    Ok(())
}

/// puts the downloaded `release` in place of the running build in `install`, the folder the program runs from, and
/// says how many files changed. first every file to change is checked again from the disk, its size and sha-256, and
/// each program and library must be signed as the running program is; a build that is not signed itself updates
/// nothing. then, file by file, the installed one is renamed `<name>.old-<current>` and the new one copied in. a
/// failure puts back every step taken, newest first, and says why. only files the release names are touched, and
/// only those whose bytes differ.
pub fn apply(release: &Release, install: &Path, current: &str) -> Result<usize, String> {
    let from = Version::parse(current).ok_or_else(|| format!("{current} is not a version"))?;
    let running = std::env::current_exe().map_err(|e| format!("eyepiece cannot find itself: {e}"))?;
    apply_from(release, &staging(release.version)?, install, from, &running, &signer_of)
}

/// starts the program in `install` with `args`, for the caller to end once it has. the new one looks for a player by
/// the one-window name and hands its files to it, so the caller lets that name go first.
pub fn relaunch(install: &Path, args: &[OsString]) -> Result<(), String> {
    let program = install.join(PROGRAM);
    Command::new(&program).args(args).spawn().map(drop).map_err(|e| format!("{} did not start: {e}", program.display()))
}

/// removes from `install` what an update set aside there, each `<name>.old-<version>` file, one at a time by its
/// exact name, and nothing else: no folder, no link, no other name. it says which it could not remove, and why: a
/// player still running holds its own until it ends.
pub fn tidy(install: &Path) -> Vec<String> {
    if !safe_root(install, home().as_deref(), &[]) {
        return vec![format!("not tidying {}: not a folder eyepiece is installed in", install.display())];
    }
    let listing = match fs::read_dir(install) {
        Ok(listing) => listing,
        Err(e) => return vec![format!("{} cannot be listed: {e}", install.display())],
    };
    let mut left = Vec::new();
    for item in listing {
        let item = match item {
            Ok(item) => item,
            Err(e) => {
                left.push(format!("{} cannot be listed: {e}", install.display()));
                continue;
            }
        };
        // the listing's own type, which does not follow links
        let name = item.file_name();
        let Some(name) = name.to_str().filter(|n| set_aside(n) && item.file_type().is_ok_and(|t| t.is_file())) else { continue };
        if let Err(e) = fs::remove_file(install.join(name)) {
            left.push(format!("{name}: {e}"));
        }
    }
    left
}

/// `apply`, with its folders and the signer check handed in, so the tests can give their own
fn apply_from(release: &Release, staged: &Path, install: &Path, from: Version, running: &Path, signer_of: &dyn Fn(&Path) -> Result<Signer, String>) -> Result<usize, String> {
    if !safe_root(install, home().as_deref(), &[]) {
        return Err(format!("{} is not a folder eyepiece updates", install.display()));
    }
    if release.version <= from {
        return Err(format!("{} is not newer than {from}", release.version));
    }
    // what a failed try set aside goes first, so the next try finds the names free. what is still held stays, and
    // is refused below
    tidy(install);
    let files = changed(release, install)?;
    verify(staged, &files)?;
    signed_alike(&files, staged, running, signer_of)?;
    // set-aside names are never written over, so one left from before would stop the update halfway: refused now,
    // while nothing is changed. tidy at the next start takes it
    for asset in &files {
        for earlier in [aside(&asset.name, from), aside(&asset.name, release.version)] {
            if fs::symlink_metadata(install.join(&earlier)).is_ok() {
                return Err(format!("{earlier} is left from an earlier update: start eyepiece again, then update"));
            }
        }
    }
    put_in_place(&files, staged, install, from, release.version)
}

/// the files of `release` that `install` does not hold as they are, in the manifest's order. a file it does not have
/// at all is one of them
fn changed<'a>(release: &'a Release, install: &Path) -> Result<Vec<&'a Asset>, String> {
    let mut out = Vec::new();
    for asset in &release.files {
        if !holds(&install.join(&asset.name), asset)? {
            out.push(asset);
        }
    }
    Ok(out)
}

/// every file to be put in place is in the update's folder, as the release names it
fn verify(staged: &Path, files: &[&Asset]) -> Result<(), String> {
    for asset in files {
        let path = staged.join(&asset.name);
        if fs::symlink_metadata(&path).is_err() {
            return Err(format!("{} has not been downloaded", asset.name));
        }
        if !holds(&path, asset)? {
            return Err(format!("{} does not match the release's size and checksum", asset.name));
        }
    }
    Ok(())
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
        Ok(_) => Ok(sha256_of(path).map_err(cannot)? == (asset.size, asset.sha256)),
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

/// one step taken in the install folder, to be undone if a later one fails
enum Step {
    /// the installed file renamed `<name>.old-<from>`
    SetAside(String),
    /// the new file made at `<name>`, whole or not
    CopiedIn(String),
}

fn put_in_place(files: &[&Asset], staged: &Path, install: &Path, from: Version, to: Version) -> Result<usize, String> {
    let mut done = Vec::new();
    for asset in files {
        if let Err(why) = put(asset, staged, install, from, &mut done) {
            let left = undo(&done, install, from, to);
            return Err(if left.is_empty() { format!("{why}. everything was put back.") } else { format!("{why}. not everything could be put back: {}", left.join("; ")) });
        }
    }
    Ok(files.len())
}

fn put(asset: &Asset, staged: &Path, install: &Path, from: Version, done: &mut Vec<Step>) -> Result<(), String> {
    let name = &asset.name;
    let installed = install.join(name);
    if fs::symlink_metadata(&installed).is_ok() {
        rename_new(&installed, &install.join(aside(name, from))).map_err(|e| why_not(name, install, e))?;
        done.push(Step::SetAside(name.clone()));
    }
    let mut source = File::open(staged.join(name)).map_err(|e| format!("{name} cannot be read from {}: {e}", staged.display()))?;
    let mut out = File::create_new(&installed).map_err(|e| why_not(name, install, e))?;
    done.push(Step::CopiedIn(name.clone()));
    // what goes in place is counted again on the way, so it is the bytes that were checked
    let (mut hash, mut size, mut piece) = (Sha256::new().map_err(|e| e.to_string())?, 0, vec![0; PIECE]);
    let copied = (|| loop {
        let n = source.read(&mut piece)?;
        if n == 0 {
            return out.sync_all();
        }
        hash.update(&piece[..n])?;
        out.write_all(&piece[..n])?;
        size += n as u64;
    })();
    copied.map_err(|e| why_not(name, install, e))?;
    if size != asset.size || hash.finish().map_err(|e| e.to_string())? != asset.sha256 {
        return Err(format!("{name} changed after it was checked"));
    }
    Ok(())
}

/// takes back `done`, newest first: a new file is set aside as `<name>.old-<to>` for `tidy`, an old one renamed back.
/// what could not be taken back is said, with where it is
fn undo(done: &[Step], install: &Path, from: Version, to: Version) -> Vec<String> {
    let mut left = Vec::new();
    for step in done.iter().rev() {
        let (now, back) = match step {
            Step::CopiedIn(name) => (name.clone(), aside(name, to)),
            Step::SetAside(name) => (aside(name, from), name.clone()),
        };
        if let Err(e) = rename_new(&install.join(&now), &install.join(&back)) {
            left.push(format!("{now} in {} could not be renamed {back}: {e}", install.display()));
        }
    }
    left
}

/// why a file could not go in or out of the install folder, in the words the update's modal uses
fn why_not(name: &str, install: &Path, e: io::Error) -> String {
    match e.raw_os_error() {
        Some(SHARING_VIOLATION) => format!("{name} is in use by another program"),
        Some(ACCESS_DENIED) => format!("eyepiece cannot write to its folder: {}", install.display()),
        _ => format!("{name}: {e}"),
    }
}

/// renames, and never over a file already there: std's rename would replace it
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    unsafe { MoveFileExW(&HSTRING::from(from), &HSTRING::from(to), MOVE_FILE_FLAGS(0)) }
        // a win32 error in an hresult: its code is the low half
        .map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xffff))
}

/// who signed a file, as far as the update compares: the name its certificate is issued to, as encoded, and the root
/// its chain ends in. not the certificate itself: artifact signing issues a new one, good for three days, to each build
struct Signer {
    /// the name in words, for what is said
    name: String,
    subject: Vec<u8>,
    root: Vec<u8>,
}

/// every program and library to be put in place is signed as the running program is: the same subject and the
/// same root. a running program that is not signed (a developer build) cannot say whose a new one must be, so it
/// takes none
fn signed_alike(files: &[&Asset], staged: &Path, running: &Path, signer_of: &dyn Fn(&Path) -> Result<Signer, String>) -> Result<(), String> {
    let ours = signer_of(running).map_err(|why| if why == NOT_SIGNED { UNSIGNED.to_string() } else { format!("this build's own signature cannot be checked, {why}: install updates by hand") })?;
    let code = |name: &str| name.rsplit_once('.').is_some_and(|(_, kind)| kind.eq_ignore_ascii_case("exe") || kind.eq_ignore_ascii_case("dll"));
    for asset in files.iter().filter(|a| code(&a.name)) {
        let not_ours = |why: &str| format!("{} is not signed by {}: {why}", asset.name, ours.name);
        let theirs = signer_of(&staged.join(&asset.name)).map_err(|why| not_ours(&why))?;
        if theirs.subject != ours.subject {
            return Err(not_ours(&format!("its signer is {}", theirs.name)));
        }
        if theirs.root != ours.root {
            return Err(not_ours("its certificate comes from another authority"));
        }
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
fn sha256_of(path: &Path) -> io::Result<(u64, [u8; 32])> {
    let mut file = File::open(path)?;
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
    use std::os::windows::fs::OpenOptionsExt;

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
    fn an_update_goes_in_place_and_keeps_the_old() {
        let (install, staged) = (Scratch::new("in-place"), Scratch::new("in-place-staged"));
        install.put("eyepiece.exe", b"program 0.2.1");
        install.put("avcodec-63.dll", b"codecs 0.2.1");
        install.put("THIRD-PARTY-NOTICES.txt", b"the same notices");
        install.put("readme.txt", b"not the release's");
        let new: [(&str, &[u8]); 4] =
            [("eyepiece.exe", b"program 0.3.0"), ("avcodec-63.dll", b"codecs 0.3.0, longer"), ("THIRD-PARTY-NOTICES.txt", b"the same notices"), ("fonts-new-OFL.txt", b"new in 0.3.0")];
        let release = release("0.3.0", &new);
        // what download leaves: the files that differ
        for (name, bytes) in [new[0], new[1], new[3]] {
            staged.put(name, bytes);
        }
        assert_eq!(apply_from(&release, &staged.0, &install.0, OLD, Path::new("running"), &tridant), Ok(3));
        for (name, bytes) in new {
            assert_eq!(install.read(name), bytes, "{name}");
        }
        assert_eq!(install.read("eyepiece.exe.old-0.2.1"), b"program 0.2.1");
        assert_eq!(install.read("avcodec-63.dll.old-0.2.1"), b"codecs 0.2.1");
        assert_eq!(install.read("readme.txt"), b"not the release's");
        // the notices were the same and the new file replaced nothing, so neither left an old one
        assert_eq!(install.names().len(), 7, "{:?}", install.names());
        assert_eq!(tidy(&install.0), Vec::<String>::new());
        install.clear(&["eyepiece.exe", "avcodec-63.dll", "THIRD-PARTY-NOTICES.txt", "readme.txt", "fonts-new-OFL.txt"]);
        staged.clear(&["eyepiece.exe", "avcodec-63.dll", "fonts-new-OFL.txt"]);
    }

    #[test]
    fn a_file_in_use_puts_everything_back() {
        let (install, staged) = (Scratch::new("in-use"), Scratch::new("in-use-staged"));
        install.put("eyepiece.exe", b"program 0.2.1");
        install.put("avcodec-63.dll", b"codecs 0.2.1");
        // not the same size, so the update knows it differs without reading it
        let new: [(&str, &[u8]); 2] = [("eyepiece.exe", b"program 0.3.0"), ("avcodec-63.dll", b"codecs 0.3.0, longer")];
        let release = release("0.3.0", &new);
        for (name, bytes) in new {
            staged.put(name, bytes);
        }
        // held as another program holds a file: shared with no one
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(install.0.join("avcodec-63.dll")).unwrap();
        let why = apply_from(&release, &staged.0, &install.0, OLD, Path::new("running"), &tridant);
        drop(held);
        assert_eq!(why, Err("avcodec-63.dll is in use by another program. everything was put back.".to_string()));
        assert_eq!((install.read("eyepiece.exe"), install.read("avcodec-63.dll")), (b"program 0.2.1".to_vec(), b"codecs 0.2.1".to_vec()));
        // the new program that had gone in is set aside
        assert_eq!(install.read("eyepiece.exe.old-0.3.0"), b"program 0.3.0");
        // and try again, now nothing holds it, takes that away first
        assert_eq!(apply_from(&release, &staged.0, &install.0, OLD, Path::new("running"), &tridant), Ok(2));
        assert_eq!(install.names(), ["avcodec-63.dll", "avcodec-63.dll.old-0.2.1", "eyepiece.exe", "eyepiece.exe.old-0.2.1"]);
        assert_eq!((install.read("eyepiece.exe"), install.read("avcodec-63.dll")), (b"program 0.3.0".to_vec(), b"codecs 0.3.0, longer".to_vec()));
        assert_eq!(tidy(&install.0), Vec::<String>::new());
        install.clear(&["eyepiece.exe", "avcodec-63.dll"]);
        staged.clear(&["eyepiece.exe", "avcodec-63.dll"]);
    }

    #[test]
    fn a_file_changed_after_its_check_is_taken_back_out() {
        let (install, staged) = (Scratch::new("changed"), Scratch::new("changed-staged"));
        install.put("eyepiece.exe", b"program 0.2.1");
        let release = release("0.3.0", &[("eyepiece.exe", b"program 0.3.0")]);
        staged.put("eyepiece.exe", b"program 0.6.6");
        let files: Vec<&Asset> = release.files.iter().collect();
        let why = put_in_place(&files, &staged.0, &install.0, OLD, release.version);
        assert_eq!(why, Err("eyepiece.exe changed after it was checked. everything was put back.".to_string()));
        assert_eq!(install.read("eyepiece.exe"), b"program 0.2.1");
        assert_eq!(install.read("eyepiece.exe.old-0.3.0"), b"program 0.6.6");
        install.clear(&["eyepiece.exe", "eyepiece.exe.old-0.3.0"]);
        staged.clear(&["eyepiece.exe"]);
    }

    #[test]
    fn nothing_is_touched_until_everything_checks() {
        let (install, staged) = (Scratch::new("refused"), Scratch::new("refused-staged"));
        install.put("eyepiece.exe", b"program 0.2.1");
        install.put("avcodec-63.dll", b"codecs 0.2.1");
        let release = release("0.3.0", &[("eyepiece.exe", b"program 0.3.0"), ("avcodec-63.dll", b"codecs 0.3.0, longer")]);
        let try_it = |signer_of: &dyn Fn(&Path) -> Result<Signer, String>| {
            let why = apply_from(&release, &staged.0, &install.0, OLD, Path::new("running"), signer_of).unwrap_err();
            assert_eq!((install.read("eyepiece.exe"), install.read("avcodec-63.dll")), (b"program 0.2.1".to_vec(), b"codecs 0.2.1".to_vec()));
            why
        };
        staged.put("eyepiece.exe", b"program 0.3.0");
        assert_eq!(try_it(&tridant), "avcodec-63.dll has not been downloaded");
        staged.put("avcodec-63.dll", b"codecs 0.3.0");
        assert_eq!(try_it(&tridant), "avcodec-63.dll does not match the release's size and checksum");
        staged.put("avcodec-63.dll", b"codecs 0.3.0, LONGER");
        assert_eq!(try_it(&tridant), "avcodec-63.dll does not match the release's size and checksum");
        staged.put("avcodec-63.dll", b"codecs 0.3.0, longer");
        assert_eq!(try_it(&|_| Err("it is not signed".into())), UNSIGNED);
        // left by the update before, and still held by the player it replaced
        install.put("eyepiece.exe.old-0.2.1", b"from an update before");
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(install.0.join("eyepiece.exe.old-0.2.1")).unwrap();
        assert_eq!(try_it(&tridant), "eyepiece.exe.old-0.2.1 is left from an earlier update: start eyepiece again, then update");
        drop(held);
        let same = super::super::read("version = 0.2.1\ndate = 2026-11-24\nfile = eyepiece.exe · 1 · e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "https://example.invalid/").unwrap();
        assert_eq!(apply_from(&same, &staged.0, &install.0, OLD, Path::new("running"), &tridant), Err("0.2.1 is not newer than 0.2.1".to_string()));
        assert_eq!(apply_from(&release, &staged.0, Path::new(r"C:\"), OLD, Path::new("running"), &tridant), Err(r"C:\ is not a folder eyepiece updates".to_string()));
        install.clear(&["eyepiece.exe", "avcodec-63.dll", "eyepiece.exe.old-0.2.1"]);
        staged.clear(&["eyepiece.exe", "avcodec-63.dll"]);
    }

    #[test]
    fn only_files_signed_as_the_running_program_go_in() {
        let release = release(
            "0.3.0",
            &[("eyepiece.exe", b"program"), ("avcodec-63.dll", b"codecs"), ("swresample-7.DLL", b"resampler"), ("THIRD-PARTY-NOTICES.txt", b"notices")],
        );
        let files: Vec<&Asset> = release.files.iter().collect();
        let staged = Path::new(r"C:\staged");
        let running = Path::new(r"C:\installed\eyepiece.exe");
        let as_ = |dll: Result<Signer, String>| {
            let dll = std::cell::RefCell::new(Some(dll));
            signed_alike(&files, staged, running, &|path| match path.file_name().unwrap().to_str().unwrap() {
                "avcodec-63.dll" => dll.borrow_mut().take().unwrap(),
                "eyepiece.exe" | "swresample-7.DLL" => tridant(path),
                // text is never asked about: it cannot carry a signature
                other => panic!("asked for the signer of {other}"),
            })
        };
        assert_eq!(as_(tridant(running)), Ok(()));
        assert_eq!(as_(Err("it is not signed".into())), Err("avcodec-63.dll is not signed by Tridant, Inc.: it is not signed".to_string()));
        assert_eq!(as_(Ok(signer("Someone Else", "root 2020"))), Err("avcodec-63.dll is not signed by Tridant, Inc.: its signer is Someone Else".to_string()));
        assert_eq!(as_(Ok(signer("Tridant, Inc.", "another root"))), Err("avcodec-63.dll is not signed by Tridant, Inc.: its certificate comes from another authority".to_string()));
        let ours = |why: &'static str| signed_alike(&files, staged, running, &|path| if path == running { Err(why.into()) } else { tridant(path) });
        assert_eq!(ours(NOT_SIGNED), Err(UNSIGNED.to_string()));
        assert_eq!(
            ours("its signature does not hold (0x800b010a)"),
            Err("this build's own signature cannot be checked, its signature does not hold (0x800b010a): install updates by hand".to_string())
        );
    }

    #[test]
    fn a_program_built_here_is_not_signed() {
        // the test program itself, as cargo built it, through wintrust
        assert_eq!(signer_of(&std::env::current_exe().unwrap()).err(), Some("it is not signed".to_string()));
    }

    /// the signers of signed files, through wintrust, as the update reads them, and that they are one signer to it:
    /// the files of EYEPIECE_SIGNED, separated by `;`, such as dist\eyepiece\eyepiece.exe after scripts\package.ps1
    /// and a build signed on another day, whose certificate is another:
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
    fn tidy_takes_only_what_an_update_set_aside() {
        let install = Scratch::new("tidy");
        for name in ["eyepiece.exe.old-0.2.1", "avcodec-63.dll.old-0.10.0", "THIRD-PARTY-NOTICES.txt.old-1.0.0"] {
            install.put(name, b"set aside");
        }
        let kept = [
            "eyepiece.exe",
            "notes.txt",
            "eyepiece.exe.old-latest",
            "eyepiece.exe.old-0.2",
            "eyepiece.exe.old-01.2.3",
            "eyepiece.exe.old-0.2.1.bak",
            "eyepiece.exe.old-0.2.1.old-0.2.2",
            "run.bat.old-0.1.0",
            ".old-0.2.1",
        ];
        for name in kept {
            install.put(name, b"not the update's");
        }
        // a folder with an update's name is still not one of its files
        fs::create_dir(install.0.join("avformat-63.dll.old-0.1.0")).unwrap();
        assert_eq!(tidy(&install.0), Vec::<String>::new());
        let mut left: Vec<&str> = kept.to_vec();
        left.push("avformat-63.dll.old-0.1.0");
        for refused in ["", r"C:\", r"eyepiece\old"] {
            assert_eq!(tidy(Path::new(refused)).len(), 1, "{refused}");
        }
        install.clear(&left);
    }

    #[test]
    fn a_download_lands_only_whole() {
        let staged = Scratch::new("landing");
        let release = release("0.3.0", &[("eyepiece.exe", b"program 0.3.0")]);
        let land = |pieces: &[&[u8]], net: Result<(), String>| {
            let mut landing = Landing::begin(&staged.0, &release.files[0]).unwrap();
            let got = pieces.iter().try_for_each(|piece| landing.take(piece)).and(net);
            landing.end(got)
        };
        assert_eq!(land(&[b"program ", b"0.3.0"], Ok(())), Ok(()));
        assert_eq!(staged.read("eyepiece.exe"), b"program 0.3.0");
        assert_eq!(land(&[b"program 0.3.1"], Ok(())), Err("eyepiece.exe does not match the release's checksum".to_string()));
        assert_eq!(staged.read("eyepiece.exe.failed"), b"program 0.3.1");
        assert_eq!(land(&[b"program 0.3.0", b"!"], Ok(())), Err("eyepiece.exe is longer than the release says".to_string()));
        assert_eq!(land(&[b"program"], Ok(())), Err("eyepiece.exe came short: 7 of 13 bytes".to_string()));
        assert_eq!(land(&[b"program "], Err("github did not answer: it timed out".into())), Err("github did not answer: it timed out".to_string()));
        assert_eq!(staged.read("eyepiece.exe.failed"), b"program ");
        // the whole one stayed as it was through the failures after it
        assert_eq!(staged.read("eyepiece.exe"), b"program 0.3.0");
        staged.clear(&["eyepiece.exe", "eyepiece.exe.failed"]);
    }

    /// a request for real, to a folder served by hand on this machine: not run with the others, which ask nothing of
    /// any server. EYEPIECE_UPDATE_FROM names it, as `http://127.0.0.1:8123/`:
    /// `cargo test a_release_is_read_over_http -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn a_release_is_read_over_http() {
        let release = super::super::check("0.0.1").unwrap().expect("a release newer than 0.0.1");
        println!("{} of {}: {} files, {}", release.version, release.date, release.files.len(), release.page);
    }
}
