//! gate 6 on macos: does browsing a share open nothing?
//! usage: gate6_browse <mode> <folder>      mode: names | bulk | stat | open
//! each mode learns about a folder's files a different way. run one mode at a time against a scratch share that
//! logs opens: the share's log is the verdict, not this program's output.
//!   names  the folder listing alone: names and kinds, no sizes or dates.
//!   bulk   getattrlistbulk: names, sizes and modified times in the listing call itself. what finder uses.
//!   stat   rust's standard walk: read_dir, then one stat call per file for size and date.
//!   open   opens each file and reads one byte. the control: this must show in the log.
//! never point this at the owner's nas, only at a scratch share.

use std::fs;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;

#[repr(C)]
struct AttrList {
    bitmapcount: u16,
    reserved: u16,
    commonattr: u32,
    volattr: u32,
    dirattr: u32,
    fileattr: u32,
    forkattr: u32,
}

extern "C" {
    fn getattrlistbulk(dirfd: i32, alist: *const AttrList, buf: *mut u8, size: usize, options: u64) -> i32;
}

// from <sys/attr.h>
const CMN_NAME: u32 = 0x1;
const CMN_OBJTYPE: u32 = 0x8;
const CMN_MODTIME: u32 = 0x400;
const CMN_RETURNED_ATTRS: u32 = 0x8000_0000;
const FILE_DATALENGTH: u32 = 0x200;
const VREG: u32 = 1;

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes(b[at..at + 4].try_into().unwrap())
}
fn i64_at(b: &[u8], at: usize) -> i64 {
    i64::from_ne_bytes(b[at..at + 8].try_into().unwrap())
}

/// files, bytes and newest modified time (unix seconds), all from the listing call
fn bulk(dir: &str) -> (u64, u64, i64) {
    let d = fs::File::open(dir).expect("open the folder");
    let want = AttrList { bitmapcount: 5, reserved: 0, commonattr: CMN_RETURNED_ATTRS | CMN_NAME | CMN_OBJTYPE | CMN_MODTIME, volattr: 0, dirattr: 0, fileattr: FILE_DATALENGTH, forkattr: 0 };
    let mut buf = vec![0u8; 64 << 10];
    let (mut count, mut bytes, mut newest) = (0u64, 0u64, 0i64);
    loop {
        let n = unsafe { getattrlistbulk(d.as_raw_fd(), &want, buf.as_mut_ptr(), buf.len(), 0) };
        assert!(n >= 0, "getattrlistbulk failed: {}", std::io::Error::last_os_error());
        if n == 0 {
            break;
        }
        let mut p = 0usize;
        for _ in 0..n {
            let len = u32_at(&buf, p) as usize;
            // what came back for this entry, one word per group: common, volume, dir, file, fork
            let (common, file) = (u32_at(&buf, p + 4), u32_at(&buf, p + 16));
            let mut q = p + 24;
            if common & CMN_NAME != 0 {
                q += 8;
            }
            let kind = if common & CMN_OBJTYPE != 0 {
                q += 4;
                u32_at(&buf, q - 4)
            } else {
                0
            };
            let mtime = if common & CMN_MODTIME != 0 {
                q += 16;
                i64_at(&buf, q - 16)
            } else {
                0
            };
            if kind == VREG {
                if file & FILE_DATALENGTH != 0 {
                    bytes += i64_at(&buf, q) as u64;
                }
                newest = newest.max(mtime);
                count += 1;
            }
            p += len;
        }
    }
    (count, bytes, newest)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 || !["names", "bulk", "stat", "open"].contains(&args[0].as_str()) {
        eprintln!("usage: gate6_browse <names|bulk|stat|open> <folder>");
        std::process::exit(2);
    }
    let (mode, dir) = (args[0].as_str(), &args[1]);
    let (mut count, mut bytes, mut newest) = (0u64, 0u64, 0i64);
    if mode == "bulk" {
        (count, bytes, newest) = bulk(dir);
    } else {
        for entry in fs::read_dir(dir).expect("list the folder") {
            let entry = entry.expect("entry");
            // the kind rides along with the name: no call per file
            if !entry.file_type().expect("kind").is_file() {
                continue;
            }
            match mode {
                "stat" => {
                    let m = entry.metadata().expect("stat");
                    bytes += m.len();
                    newest = newest.max(m.mtime());
                }
                "open" => {
                    let mut b = [0u8; 1];
                    let mut f = fs::File::open(entry.path()).expect("open");
                    let _ = f.read(&mut b);
                    bytes += f.metadata().map(|m| m.len()).unwrap_or(0);
                }
                _ => {}
            }
            count += 1;
        }
    }
    println!("{mode}: {count} files, {bytes} bytes, newest modified at {newest}");
}
