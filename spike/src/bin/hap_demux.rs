//! measured alongside: does ffmpeg's demuxer hand over the packets of a hap stream whose tag it has no decoder for?
//! usage: hap_demux <file> [<file> ...]
//! passes when every file yields as many video packets as its sample table says, each starting with a valid hap section header.

use spike::ff::{self, AVMediaType, Input, Packet};

/// the top-level hap section header: 3-byte little-endian size and a type byte; a zero size means an 8-byte header.
fn hap_section(d: &[u8]) -> Option<(usize, u8, usize)> {
    if d.len() < 4 {
        return None;
    }
    let mut size = d[0] as usize | (d[1] as usize) << 8 | (d[2] as usize) << 16;
    let kind = d[3];
    let mut header = 4;
    if size == 0 {
        if d.len() < 8 {
            return None;
        }
        size = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
        header = 8;
    }
    Some((size, kind, header))
}

fn texture_name(kind: u8) -> &'static str {
    match kind & 0x0f {
        0x0b => "bc1 (hap)",
        0x0e => "bc3 (hap alpha)",
        0x0f => "bc3 scaled ycocg (hap q)",
        0x0c => "bc7 (hap r)",
        0x01 => "bc4 alpha (hap alpha-only)",
        0x02 => "bc6h unsigned (hap hdr)",
        0x03 => "bc6h signed (hap hdr)",
        0x0d => "several textures (hap q alpha)",
        _ => "unknown",
    }
}

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: hap_demux <file> [<file> ...]");
        std::process::exit(2);
    }
    println!("ffmpeg {}", ff::version());
    let mut all_ok = true;
    for path in &files {
        // no avformat_find_stream_info: the player wants facts from the header alone, and an unknown codec cannot be probed anyway
        let input = match Input::open(path, false) {
            Ok(i) => i,
            Err(e) => {
                println!("{path}: {e}");
                all_ok = false;
                continue;
            }
        };
        let Some(v) = input.streams().into_iter().find(|s| s.kind == AVMediaType::AVMEDIA_TYPE_VIDEO) else {
            println!("{path}: no video stream");
            all_ok = false;
            continue;
        };
        let mut pkt = Packet::new();
        let (mut count, mut bad, mut bytes, mut first) = (0i64, 0i64, 0u64, None);
        loop {
            match pkt.read(&input) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    println!("{path}: read error {e}");
                    all_ok = false;
                    break;
                }
            }
            if pkt.stream() != v.index {
                continue;
            }
            count += 1;
            bytes += pkt.data().len() as u64;
            match hap_section(pkt.data()) {
                Some((size, kind, header)) if size + header == pkt.data().len() => {
                    if first.is_none() {
                        first = Some(kind);
                    }
                }
                _ => bad += 1,
            }
        }
        let ok = count > 0 && count == v.nb_frames && bad == 0;
        all_ok &= ok;
        println!(
            "{path}\n  tag {} · ffmpeg codec id: {} · {}x{} · header says {} frames\n  packets delivered {} · with a valid hap section {} · {:.1} MB · first section type 0x{:02x} = {}\n  {}",
            ff::fourcc(v.tag),
            v.codec_name,
            v.width,
            v.height,
            v.nb_frames,
            count,
            count - bad,
            bytes as f64 / 1048576.0,
            first.unwrap_or(0),
            first.map(texture_name).unwrap_or("none"),
            if ok { "ok" } else { "NOT ok" }
        );
    }
    std::process::exit(spike::verdict(all_ok, "the demuxer delivers every hap packet, whatever the tag"));
}
