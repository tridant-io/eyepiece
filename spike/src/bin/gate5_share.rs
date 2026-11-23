//! gate 5: can a render overwrite a file we have open?
//! usage: gate5_share [small-media-file]
//! holds a file open three ways, lets a second process overwrite it three ways, and reports what the os allowed
//! and whether the change shows in the file's size and modified time. works only in its own scratch folder.
//! passes when our own way of opening (read, write and delete sharing) lets every kind of overwrite through, noticed.

use spike::ff::Input;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

const SHARE_READ: u32 = 1;
const SHARE_WRITE: u32 = 2;
const SHARE_DELETE: u32 = 4;
const N: usize = 4 << 20;

fn writer(mode: &str, path: &Path) -> std::io::Result<()> {
    match mode {
        // a renderer that rewrites the same file
        "overwrite" => OpenOptions::new().write(true).truncate(true).open(path)?.write_all(&vec![b'B'; N + 1]),
        // a renderer that writes beside it and renames over it, the careful kind
        "rename" => {
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, vec![b'C'; N + 2])?;
            fs::rename(&tmp, path)
        }
        // a renderer that deletes the old file first
        "recreate" => {
            fs::remove_file(path)?;
            fs::write(path, vec![b'D'; N + 3])
        }
        _ => Err(std::io::Error::other("unknown mode")),
    }
}

enum Holder {
    Rust(File),
    Ffmpeg(#[allow(dead_code)] Input),
}

fn stat(p: &Path) -> Option<(u64, SystemTime)> {
    fs::metadata(p).ok().map(|m| (m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("writer") {
        let r = writer(&args[1], Path::new(&args[2]));
        if let Err(e) = &r {
            print!("{e}");
        }
        std::process::exit(if r.is_ok() { 0 } else { 1 });
    }
    let media = args.first().map(PathBuf::from);
    let work = std::env::temp_dir().join("eyepiece-gate5");
    fs::create_dir_all(&work).expect("scratch folder");
    let exe = std::env::current_exe().unwrap();
    let ways: [(&str, Option<u32>); 3] = [
        ("ours: read + write + delete sharing", Some(SHARE_READ | SHARE_WRITE | SHARE_DELETE)),
        ("read + write sharing, no delete", Some(SHARE_READ | SHARE_WRITE)),
        ("ffmpeg's own open (avformat_open_input)", None),
    ];
    let mut ours_ok = true;
    println!("{:<42} {:<10} {:<34} {:<9} we then read", "file held open by", "writer", "os said", "noticed");
    for (label, share) in ways {
        for mode in ["overwrite", "rename", "recreate"] {
            let path = work.join(if share.is_some() { "clip.bin" } else { "clip.mov" });
            // a fresh subject for every case
            let _ = fs::remove_file(&path);
            match (share, &media) {
                (Some(_), _) => fs::write(&path, vec![b'A'; N]).unwrap(),
                (None, Some(m)) => {
                    fs::copy(m, &path).unwrap();
                }
                (None, None) => {
                    println!("{label:<42} {mode:<10} skipped: give a small media file as the argument");
                    continue;
                }
            }
            let mut holder = match share {
                Some(s) => Holder::Rust(OpenOptions::new().read(true).share_mode(s).open(&path).unwrap()),
                None => Holder::Ffmpeg(Input::open(path.to_str().unwrap(), false).unwrap()),
            };
            let before = stat(&path);
            std::thread::sleep(std::time::Duration::from_millis(30));
            let out = Command::new(&exe).args(["writer", mode, path.to_str().unwrap()]).output().unwrap();
            let allowed = out.status.success();
            let said = if allowed { "allowed".to_string() } else { String::from_utf8_lossy(&out.stdout).trim().chars().take(32).collect() };
            let after = stat(&path);
            let noticed = allowed && after.is_some() && after != before;
            // what our still-open handle sees afterwards
            let saw = match &mut holder {
                Holder::Rust(f) => {
                    let mut b = [0u8; 1];
                    match f.seek(SeekFrom::Start(0)).and_then(|_| f.read(&mut b)) {
                        Ok(1) => match b[0] {
                            b'A' => "the old file".to_string(),
                            other => format!("the new file ({})", other as char),
                        },
                        Ok(_) => "nothing (empty)".to_string(),
                        Err(e) => format!("error: {e}"),
                    }
                }
                Holder::Ffmpeg(_) => "-".to_string(),
            };
            println!("{label:<42} {mode:<10} {said:<34} {:<9} {saw}", if noticed { "yes" } else { "no" });
            if share == Some(SHARE_READ | SHARE_WRITE | SHARE_DELETE) {
                ours_ok &= allowed && noticed;
            }
            drop(holder);
        }
    }
    std::process::exit(spike::verdict(ours_ok, "with read, write and delete sharing, every kind of overwrite goes through and shows in size or modified time"));
}
