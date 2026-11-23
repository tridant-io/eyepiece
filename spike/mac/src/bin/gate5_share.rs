//! gate 5 on macos: can a render overwrite a file we have open?
//! usage: gate5_share [small-media-file]
//! there are no share modes here, so nothing is refused. the question becomes: do we notice each kind of overwrite
//! from the path (size, modified time, or which file the name now points at), and what does our open handle read?
//! works only in its own scratch folder. passes when every overwrite goes through and is noticed from the path.

use spike::ff::Input;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// size, modified time, and the file's identity
fn stat(p: &Path) -> Option<(u64, i64, i64, u64)> {
    fs::metadata(p).ok().map(|m| (m.len(), m.mtime(), m.mtime_nsec(), m.ino()))
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
    let mut ok = true;
    println!("{:<42} {:<10} {:<9} {:<26} our open handle then reads", "file held open by", "writer", "os said", "noticed from the path by");
    for ours in [true, false] {
        let label = if ours { "ours: a plain read-only open" } else { "ffmpeg's own open (avformat_open_input)" };
        for mode in ["overwrite", "rename", "recreate"] {
            let path = work.join(if ours { "clip.bin" } else { "clip.mov" });
            // a fresh subject for every case
            let _ = fs::remove_file(&path);
            match (ours, &media) {
                (true, _) => fs::write(&path, vec![b'A'; N]).unwrap(),
                (false, Some(m)) => {
                    fs::copy(m, &path).unwrap();
                }
                (false, None) => {
                    println!("{label:<42} {mode:<10} skipped: give a small media file as the argument");
                    continue;
                }
            }
            let mut holder = if ours { Holder::Rust(File::open(&path).unwrap()) } else { Holder::Ffmpeg(Input::open(path.to_str().unwrap(), false).unwrap()) };
            let before = stat(&path).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            let out = Command::new(&exe).args(["writer", mode, path.to_str().unwrap()]).output().unwrap();
            let allowed = out.status.success();
            let said = if allowed { "allowed".to_string() } else { String::from_utf8_lossy(&out.stdout).trim().chars().take(8).collect() };
            let after = stat(&path);
            let mut by = Vec::new();
            if let Some(a) = after {
                if a.0 != before.0 {
                    by.push("size");
                }
                if (a.1, a.2) != (before.1, before.2) {
                    by.push("time");
                }
                if a.3 != before.3 {
                    by.push("identity");
                }
            }
            let noticed = allowed && !by.is_empty();
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
            println!("{label:<42} {mode:<10} {said:<9} {:<26} {saw}", if noticed { by.join(", ") } else { "not noticed".to_string() });
            ok &= allowed && noticed;
            drop(holder);
        }
    }
    std::process::exit(spike::verdict(ok, "every kind of overwrite goes through with the file open, and shows from the path"));
}
