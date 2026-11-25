//! updates: a check for a newer release and, on windows, its download, its checks, and putting it in place of the
//! running build. a release is read from one plain file, its release manifest `eyepiece-release.txt`, which every
//! release carries as an asset in the settings file's `key = value` form (adr 0007):
//!
//! ```text
//! version = 0.3.0
//! date = 2026-12-08
//! notes = added · settings, with a page for each part of the player
//! file = eyepiece.exe · 6495000 · <the file's sha-256, 64 hex digits>
//! ```
//!
//! the manifest is read from `<releases>/latest/download/eyepiece-release.txt`, which github sends on to the newest
//! release's, and each file from `<releases>/download/v<version>/<name>`. `EYEPIECE_UPDATE_FROM` puts another folder
//! in place of `<releases>`, laid out the same way, so the flow can be tried on files served by hand. nothing is sent
//! but the requests themselves, as `eyepiece/<version>`.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod win;
#[cfg(target_os = "macos")]
pub use mac::{apply, download, relaunch, tidy};
#[cfg(target_os = "macos")]
use mac as sys;
#[cfg(windows)]
pub use win::{apply, download, relaunch, tidy};
#[cfg(windows)]
use win as sys;

use std::fmt;
use std::time::Duration;

/// the release manifest's name among a release's assets
const MANIFEST: &str = "eyepiece-release.txt";
/// github's releases of the repo: `latest/download/<asset>` is the newest release's, `download/<tag>/<asset>` one's
const RELEASES: &str = "https://github.com/tridant-io/eyepiece/releases/";
/// the program among a release's files. a release without it is not one
const PROGRAM: &str = "eyepiece.exe";
/// the user agent, and all the player says of itself
const AGENT: &str = concat!("eyepiece/", env!("CARGO_PKG_VERSION"));
/// how long a request may go without an answer
const WAIT: Duration = Duration::from_secs(10);
/// a manifest is a few hundred bytes: an answer this long is not one
const MANIFEST_MAX: usize = 64 << 10;
/// a file of a release larger than this is not one: avcodec, the largest, was 94 MB in dist\eyepiece on 2026-11-24
const FILE_MAX: u64 = 1 << 30;

/// a release as its manifest names it
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// the day it was released, yyyy-mm-dd
    pub date: String,
    /// what changed, a line each, with its kind as the notes head it (added, changed, fixed), or none
    pub notes: Vec<(String, String)>,
    pub files: Vec<Asset>,
    /// the release's page on github, with the full notes
    pub page: String,
    /// where its files are downloaded from
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    from: String,
}

/// one file of a release
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

/// x.y.z, compared number by number
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u32, u32, u32);

impl Version {
    /// three whole numbers with dots between, each without a leading zero. anything else is not a version
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.').map(|p| number(p).filter(|_| p == "0" || !p.starts_with('0')).and_then(|n| u32::try_from(n).ok()));
        let version = Version(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

impl Release {
    #[cfg(windows)]
    fn url_of(&self, asset: &Asset) -> String {
        format!("{}download/v{}/{}", self.from, self.version, asset.name)
    }
}

/// asks for the newest release and gives it when its version is greater than `current`. it blocks for as long as
/// the request takes (up to about 10 s without an answer), so the caller runs it on a thread of its own.
pub fn check(current: &str) -> Result<Option<Release>, String> {
    let current = Version::parse(current).ok_or_else(|| format!("{current} is not a version"))?;
    let from = base()?;
    offered(&sys::fetch(&format!("{from}latest/download/{MANIFEST}"), MANIFEST_MAX)?, &from, current)
}

/// the release a manifest names, when it is newer than `current`: an equal or older one is never offered
fn offered(manifest: &[u8], from: &str, current: Version) -> Result<Option<Release>, String> {
    let text = std::str::from_utf8(manifest).map_err(|_| "the release's manifest is not text".to_string())?;
    let release = read(text, from)?;
    Ok((release.version > current).then_some(release))
}

/// a release manifest. a version, a date, or a file it cannot read refuses the whole, since the files are taken by
/// what it says; keys it does not know are passed over, for a later build's.
fn read(text: &str, from: &str) -> Result<Release, String> {
    let bad = |line: &str| format!("the release's manifest has a line eyepiece cannot read: {}", line.trim());
    let (mut version, mut date, mut notes, mut files) = (None, None, Vec::new(), Vec::<Asset>::new());
    // notepad may start a file with a byte order mark
    for line in text.trim_start_matches('\u{feff}').lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "version" if version.is_none() => version = Some(Version::parse(value).ok_or_else(|| bad(line))?),
            "date" if date.is_none() && day(value) => date = Some(value.to_string()),
            // a second version or date, or a date not written yyyy-mm-dd
            "version" | "date" => return Err(bad(line)),
            "notes" => notes.push(match value.split_once('·') {
                Some((kind, change)) => (kind.trim().to_string(), change.trim().to_string()),
                None => (String::new(), value.to_string()),
            }),
            "file" => {
                let asset = asset(value).ok_or_else(|| bad(line))?;
                // names on windows are not case sensitive: two such would be one file
                if files.iter().any(|f| f.name.eq_ignore_ascii_case(&asset.name)) {
                    return Err(format!("the release's manifest names {} twice", asset.name));
                }
                files.push(asset);
            }
            _ => {}
        }
    }
    let version = version.ok_or("the release's manifest names no version")?;
    let date = date.ok_or("the release's manifest has no date")?;
    if !files.iter().any(|f| f.name == PROGRAM) {
        return Err(format!("the release's manifest has no {PROGRAM}"));
    }
    Ok(Release { version, date, notes, files, page: format!("{from}tag/v{version}"), from: from.to_string() })
}

/// `name · size · sha-256`
fn asset(value: &str) -> Option<Asset> {
    let mut parts = value.split('·').map(str::trim);
    let (name, size, sum) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || !plain(name) {
        return None;
    }
    Some(Asset { name: name.to_string(), size: number(size).filter(|&s| s <= FILE_MAX)?, sha256: hex(sum)? })
}

/// a name a release may give a file: letters, digits, `.`, `-` and `_`, ending .exe, .dll or .txt. never a name
/// windows keeps for a device, nor one the update's own set-aside names (`<name>.old-<version>`) could be taken for.
/// it is joined to the install folder as it is, so nothing in it may climb out
fn plain(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().to_ascii_lowercase();
    let numbered = stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && stem.as_bytes()[3].is_ascii_digit();
    let device = numbered || ["con", "prn", "aux", "nul"].contains(&stem.as_str());
    let kind = name.rsplit_once('.').map(|(_, kind)| kind.to_ascii_lowercase());
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        && !name.starts_with(['.', '-'])
        && !name.contains(".old-")
        && !device
        && matches!(kind.as_deref(), Some("exe" | "dll" | "txt"))
}

/// what the update set a file aside as, beside it: `<name>.old-<version>`
#[cfg(windows)]
fn aside(name: &str, version: Version) -> String {
    format!("{name}.old-{version}")
}

/// a name `aside` made, and no other
#[cfg(windows)]
fn set_aside(name: &str) -> bool {
    name.rsplit_once(".old-").is_some_and(|(file, version)| plain(file) && Version::parse(version).is_some())
}

/// digits alone: no sign, no space
fn number(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok()).flatten()
}

/// 64 hex digits
fn hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// yyyy-mm-dd
fn day(text: &str) -> bool {
    text.len() == 10 && text.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() })
}

/// where releases are read from: github's, or `EYEPIECE_UPDATE_FROM`, the seam for trying the flow on a folder served
/// by hand. it changes where the files come from and none of the checks on them
fn base() -> Result<String, String> {
    match std::env::var("EYEPIECE_UPDATE_FROM") {
        Ok(from) => seam(&from),
        Err(std::env::VarError::NotPresent) => Ok(RELEASES.to_string()),
        Err(std::env::VarError::NotUnicode(_)) => Err("EYEPIECE_UPDATE_FROM is not text".to_string()),
    }
}

/// the seam's folder, ending in a slash: the files' paths are put after it
fn seam(from: &str) -> Result<String, String> {
    let from = if from.ends_with('/') { from.to_string() } else { format!("{from}/") };
    split(&from)?;
    if from.contains(['?', '#']) {
        return Err(format!("not a folder to read releases from: {from}"));
    }
    Ok(from)
}

/// a url the player may ask, as (secure, host, port, path): https to any host, or plain http to 127.0.0.1 alone,
/// which only the seam names. anything else is refused before a request is made
fn split(url: &str) -> Result<(bool, &str, u16, &str), String> {
    let refused = || format!("not a url eyepiece asks: {url}");
    let (secure, rest) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(rest), _) => (true, rest),
        (None, Some(rest)) => (false, rest),
        _ => return Err(refused()),
    };
    if !url.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(refused());
    }
    let (authority, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    if !path.starts_with('/') {
        return Err(refused());
    }
    let (host, port) = match authority.split_once(':') {
        None => (authority, if secure { 443 } else { 80 }),
        Some((host, port)) => (host, number(port).and_then(|p| u16::try_from(p).ok()).filter(|&p| p > 0).ok_or_else(refused)?),
    };
    // a name or a dotted address and nothing more: no user before it, no ipv6 brackets, nothing read two ways
    if host.is_empty() || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') {
        return Err(refused());
    }
    // a name could be pointed elsewhere; the address cannot
    if !secure && host != "127.0.0.1" {
        return Err(format!("plain http is only for 127.0.0.1: {url}"));
    }
    Ok((secure, host, port, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUM: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const FROM: &str = "https://github.com/tridant-io/eyepiece/releases/";

    fn manifest(lines: &[String]) -> String {
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    fn good() -> Vec<String> {
        vec![
            "version = 0.3.0".into(),
            "date = 2026-12-08".into(),
            "notes = added · settings, with a page for each part of the player".into(),
            "notes = fixed · the timebar's picture lagged on long h.264 clips".into(),
            "notes = and 4 smaller fixes".into(),
            format!("file = eyepiece.exe · 6495000 · {}", SUM.to_uppercase()),
            format!("file = avformat-63.dll · 19979264 · {SUM}"),
            "from_a_later_build = 42".into(),
            "a line with no equals sign".into(),
        ]
    }

    fn reads(lines: &[String]) -> Result<Release, String> {
        read(&manifest(lines), FROM)
    }

    /// the good manifest with one line put in place of the one starting `key =`, or left out for none
    fn with(key: &str, line: Option<&str>) -> Result<Release, String> {
        let mut lines = good();
        let at = lines.iter().position(|l| l.starts_with(&format!("{key} ="))).unwrap();
        match line {
            Some(line) => lines[at] = line.to_string(),
            None => drop(lines.remove(at)),
        }
        reads(&lines)
    }

    #[test]
    fn a_manifest_reads() {
        // as notepad might save it: a byte order mark, and \r\n
        let release = read(&format!("\u{feff}{}", manifest(&good())).replace('\n', "\r\n"), FROM).unwrap();
        assert_eq!((release.version, release.version.to_string(), release.date.as_str()), (Version(0, 3, 0), "0.3.0".into(), "2026-12-08"));
        assert_eq!(release.notes[0], ("added".into(), "settings, with a page for each part of the player".into()));
        assert_eq!(release.notes[1].0, "fixed");
        assert_eq!(release.notes[2], ("".into(), "and 4 smaller fixes".into()));
        let sum = hex(SUM).unwrap();
        assert_eq!(sum[..3], [0xe3, 0xb0, 0xc4]);
        assert_eq!(
            release.files,
            [Asset { name: "eyepiece.exe".into(), size: 6495000, sha256: sum }, Asset { name: "avformat-63.dll".into(), size: 19979264, sha256: sum }]
        );
        assert_eq!(release.page, "https://github.com/tridant-io/eyepiece/releases/tag/v0.3.0");
        #[cfg(windows)]
        assert_eq!(release.url_of(&release.files[1]), "https://github.com/tridant-io/eyepiece/releases/download/v0.3.0/avformat-63.dll");
    }

    #[test]
    fn a_manifest_without_what_it_needs_is_refused() {
        assert!(with("version", None).unwrap_err().contains("no version"));
        assert!(with("date", None).unwrap_err().contains("no date"));
        assert!(with("file", None).unwrap_err().contains("no eyepiece.exe"));
        assert!(read("", FROM).is_err());
        for line in ["version = 0.3", "version = v0.3.0", "date = 2026-12-8", "date = 08.12.2026", "date ="] {
            let key = line.split(' ').next().unwrap();
            assert!(with(key, Some(line)).is_err(), "{line}");
        }
        let mut twice = good();
        twice.push("version = 0.4.0".into());
        assert!(reads(&twice).is_err());
        let mut twice = good();
        twice.push(format!("file = EYEPIECE.EXE · 1 · {SUM}"));
        assert!(reads(&twice).unwrap_err().contains("twice"));
        assert!(offered(b"\xff\xfe", FROM, Version(0, 1, 0)).unwrap_err().contains("not text"));
    }

    #[test]
    fn bad_hashes_and_sizes_are_refused() {
        let short = &SUM[1..];
        let odd = format!("{}g", &SUM[1..]);
        let long = format!("{SUM}0");
        for (size, sum) in [
            ("6495000", short),
            ("6495000", odd.as_str()),
            ("6495000", ""),
            ("6495000", long.as_str()),
            ("-1", SUM),
            ("+1", SUM),
            ("1e6", SUM),
            ("6 495 000", SUM),
            ("", SUM),
            ("18446744073709551616", SUM),
            ("1073741825", SUM),
        ] {
            let line = format!("file = eyepiece.exe · {size} · {sum}");
            assert!(with("file", Some(&line)).is_err(), "{line}");
        }
        for line in ["file = eyepiece.exe · 6495000".into(), format!("file = eyepiece.exe · 1 · {SUM} · more"), format!("file = eyepiece.exe 1 {SUM}")] {
            assert!(with("file", Some(&line)).is_err(), "{line}");
        }
        assert!(with("file", Some(&format!("file = eyepiece.exe · 0 · {SUM}"))).is_ok(), "an empty file has a size too");
        assert!(with("file", Some(&format!("file = eyepiece.exe · 1073741824 · {SUM}"))).is_ok());
    }

    #[test]
    fn names_are_plain() {
        for name in ["eyepiece.exe", "avcodec-63.dll", "THIRD-PARTY-NOTICES.txt", "fonts-IBM-Plex-LICENSE.txt", "swresample_7.DLL"] {
            assert!(plain(name), "{name}");
        }
        let long = format!("{}.dll", "a".repeat(61));
        for name in [
            long.as_str(),
            "",
            "../eyepiece.exe",
            "..\\eyepiece.exe",
            "bin/eyepiece.exe",
            "C:eyepiece.exe",
            "eyepiece.exe:stream",
            ".eyepiece.exe",
            "-x.dll",
            "eyepiece",
            "eyepiece.bat",
            "eyepiece.exe.",
            "eyepiece .exe",
            "eyepiecé.exe",
            "nul.txt",
            "NUL",
            "com1.dll",
            "Lpt9.txt",
            "avcodec-63.dll.old-0.2.1.dll",
        ] {
            assert!(!plain(name), "{name}");
        }
        assert!(plain("com.dll") && plain("console.txt"), "only the devices themselves are kept");
    }

    #[test]
    fn versions_compare_number_by_number() {
        let v = |text| Version::parse(text).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.2.10") > v("0.2.9"));
        assert_eq!(v("0.3.0"), v("0.3.0"));
        assert_eq!(v("4294967295.0.0").to_string(), "4294967295.0.0");
        for refused in ["", "0", "0.3", "0.3.0.1", "v0.3.0", "0.3.0-beta", "0.3.0+1", "01.2.3", "1.02.3", "+1.2.3", "1..2", " 1.2.3", "1.2.3 ", "1.2.-3", "4294967296.0.0", "a.b.c"] {
            assert_eq!(Version::parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn only_a_newer_version_is_offered() {
        let text = manifest(&good());
        let offer = |current| offered(text.as_bytes(), FROM, Version::parse(current).unwrap()).unwrap().map(|r| r.version);
        assert_eq!(offer("0.2.1"), Some(Version(0, 3, 0)));
        assert_eq!(offer("0.3.0"), None, "the same version");
        assert_eq!(offer("0.10.0"), None, "an older one released later");
        assert!(check("0.3").unwrap_err().contains("not a version"));
    }

    #[test]
    fn the_seam_takes_local_http_alone() {
        assert_eq!(seam("http://127.0.0.1:8123").unwrap(), "http://127.0.0.1:8123/");
        assert_eq!(seam("http://127.0.0.1/releases/").unwrap(), "http://127.0.0.1/releases/");
        assert_eq!(seam("https://example.com/eyepiece/releases").unwrap(), "https://example.com/eyepiece/releases/");
        for refused in [
            "http://localhost:8123/",
            "http://example.com/",
            "http://127.0.0.2/",
            "http://127.0.0.1.example.com/",
            "http://127.0.0.1@example.com/",
            "http://example.com@127.0.0.1/",
            "http://user:pass@127.0.0.1/",
            "http://[::1]:8123/",
            "http://0x7f000001/",
            "http://127.0.0.1:/",
            "http://127.0.0.1:0/",
            "http://127.0.0.1:65536/",
            "http://127.0.0.1:+80/",
            "http://127.0.0.1:80:80/",
            "http://127.0.0.1\\@example.com/",
            "http://127.0.0.1 /",
            "HTTP://127.0.0.1/",
            "ftp://127.0.0.1/",
            "file:///C:/releases/",
            "//127.0.0.1/",
            "127.0.0.1:8123",
            "https:///",
            "https://example.com/?x=1",
            "https://example.com/#x",
        ] {
            assert!(seam(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn urls_split_as_they_are_asked() {
        assert_eq!(split("https://github.com/a/b").unwrap(), (true, "github.com", 443, "/a/b"));
        assert_eq!(split("http://127.0.0.1:8123/latest").unwrap(), (false, "127.0.0.1", 8123, "/latest"));
        // where a redirect may end: a signed link with its query
        assert_eq!(split("https://release-assets.githubusercontent.com/x?sig=a%2Fb&se=1").unwrap().3, "/x?sig=a%2Fb&se=1");
        assert!(split("https://github.com?x=/y").is_err());
        assert!(split("https://example.com@127.0.0.1/").is_err());
    }
}
