//! gate 6: does browsing a share open nothing?
//! usage: gate6_browse <mode> <folder>      mode: list | stat | attrs | open
//! each mode learns the names, sizes and modified times of a folder's files a different way. run one mode at a time
//! against a scratch share that logs opens: the share's log is the verdict, not this program's output.
//!   list   the folder listing alone. the only way the player may use on a network path (adr 0004).
//!   stat   std::fs::metadata on each path. on windows this opens a handle to the file.
//!   attrs  GetFileAttributesExW on each path.
//!   open   opens each file and reads one byte. the control: this must show in the log.
//! never point this at the owner's nas, only at a scratch share.

use std::fs;
use std::io::Read;
use std::time::SystemTime;
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{GetFileAttributesExW, GetFileExInfoStandard, WIN32_FILE_ATTRIBUTE_DATA};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 || !["list", "stat", "attrs", "open"].contains(&args[0].as_str()) {
        eprintln!("usage: gate6_browse <list|stat|attrs|open> <folder>");
        std::process::exit(2);
    }
    let (mode, dir) = (args[0].as_str(), &args[1]);
    let (mut count, mut bytes, mut newest) = (0u64, 0u64, SystemTime::UNIX_EPOCH);
    for entry in fs::read_dir(dir).expect("list the folder") {
        let entry = entry.expect("entry");
        let path = entry.path();
        match mode {
            "list" => {
                // on windows the size and times ride along with the listing: no call per file
                let m = entry.metadata().expect("listing data");
                if m.is_dir() {
                    continue;
                }
                bytes += m.len();
                newest = newest.max(m.modified().unwrap_or(SystemTime::UNIX_EPOCH));
            }
            "stat" => {
                let m = fs::metadata(&path).expect("metadata");
                if m.is_dir() {
                    continue;
                }
                bytes += m.len();
                newest = newest.max(m.modified().unwrap_or(SystemTime::UNIX_EPOCH));
            }
            "attrs" => {
                let mut d = WIN32_FILE_ATTRIBUTE_DATA::default();
                unsafe { GetFileAttributesExW(&HSTRING::from(path.as_os_str()), GetFileExInfoStandard, &mut d as *mut _ as *mut _).expect("attributes") };
                if d.dwFileAttributes & 0x10 != 0 {
                    continue;
                }
                bytes += (d.nFileSizeHigh as u64) << 32 | d.nFileSizeLow as u64;
            }
            _ => {
                if path.is_dir() {
                    continue;
                }
                let mut b = [0u8; 1];
                let mut f = fs::File::open(&path).expect("open");
                let _ = f.read(&mut b);
                bytes += f.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        count += 1;
    }
    let age = newest.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    println!("{mode}: {count} files, {bytes} bytes{}", if newest > SystemTime::UNIX_EPOCH { format!(", newest modified {age} s ago") } else { String::new() });
}
