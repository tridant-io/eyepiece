//! test-media helper: copies a .mov and rewrites the four-character code of its first video sample description.
//! ffmpeg's muxer refuses to write Hap7 or HapH, so this is how a clip tagged as hap r or hap hdr is made for the demuxer check.
//! the payload stays what it was; only the tag changes.
//! usage: retag <in.mov> <out.mov> <from fourcc> <to fourcc>

use std::fs;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() != 4 || a[2].len() != 4 || a[3].len() != 4 {
        eprintln!("usage: retag <in.mov> <out.mov> <from fourcc> <to fourcc>");
        std::process::exit(2);
    }
    let mut d = fs::read(&a[0]).expect("read input");
    let (from, to) = (a[2].as_bytes(), a[3].as_bytes());
    // stsd: size, 'stsd', version+flags, entry count, then the first entry: size, format
    let mut done = 0;
    let mut i = 0;
    while i + 20 <= d.len() {
        if &d[i..i + 4] == b"stsd" && &d[i + 16..i + 20] == from {
            d[i + 16..i + 20].copy_from_slice(to);
            done += 1;
        }
        i += 1;
    }
    if done == 0 {
        eprintln!("no sample description tagged {} found", a[2]);
        std::process::exit(1);
    }
    fs::write(&a[1], &d).expect("write output");
    println!("retagged {done} sample description(s) {} -> {} in {}", a[2], a[3], a[1]);
}
