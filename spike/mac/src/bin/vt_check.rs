//! measured alongside on macos: what does videotoolbox hand back, and is it the same picture?
//! usage: vt_check <file>
//! decodes frame 0 through videotoolbox and in software and compares them sample for sample at the software decoder's
//! bit depth. reports the buffer's pixel format and layout, how many bits the hardware samples really carry, how far
//! the two decodes are apart, and whether alpha came through. speed means nothing if depth or alpha is lost on the way.
//! prints what it finds. passes when every component the software decoder has is there and within one code value.

use spike::dec::Decoder;
use spike::ff::{AVPixelFormat, Input, Packet};
use spike_mac::PixelBuffer;

/// one component's comparison: how many samples differ, by how much at worst, and the total
#[derive(Default, Clone, Copy)]
struct Diff {
    differ: u64,
    worst: u32,
    of: u64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: vt_check <file>");
        std::process::exit(2);
    };
    let (hw_in, sw_in) = (Input::open(path, false).expect("open"), Input::open(path, false).expect("open"));
    let v = hw_in.first_video().expect("video");
    let (w, h) = (v.width as usize, v.height as usize);
    let mut hw = Decoder::open_videotoolbox(&hw_in, v.index).expect("hardware decoder");
    let mut sw = Decoder::open_software(&sw_in, v.index, 0).expect("software decoder");
    let (mut hp, mut sp) = (Packet::new(), Packet::new());
    assert!(hw.next(&hw_in, &mut hp, v.index).expect("hw decode") && sw.next(&sw_in, &mut sp, v.index).expect("sw decode"), "no first frame");
    let file = path.rsplit('/').next().unwrap_or(path);
    if hw.format() != AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
        println!("{file}: videotoolbox is not offered for this codec");
        std::process::exit(1);
    }
    // what the software decoder gives: bits per sample, chroma layout, alpha
    let (bits, cw, ch, alpha) = match sw.format() {
        AVPixelFormat::AV_PIX_FMT_YUV420P10LE => (10u32, w / 2, h / 2, false),
        AVPixelFormat::AV_PIX_FMT_YUV422P10LE => (10, w / 2, h, false),
        AVPixelFormat::AV_PIX_FMT_YUV444P10LE => (10, w, h, false),
        AVPixelFormat::AV_PIX_FMT_YUV444P12LE => (12, w, h, false),
        AVPixelFormat::AV_PIX_FMT_YUVA444P10LE => (10, w, h, true),
        AVPixelFormat::AV_PIX_FMT_YUVA444P12LE => (12, w, h, true),
        other => {
            println!("{file}: software decodes as {other:?}, which this check does not know");
            std::process::exit(1);
        }
    };
    let pb = PixelBuffer(hw.pixel_buffer());
    let sizes: Vec<String> = (0..pb.planes()).map(|p| format!("{}x{}", pb.plane_size(p).0, pb.plane_size(p).1)).collect();
    let layout = if pb.planes() == 0 { "one packed plane".to_string() } else { format!("{} planes: {}", pb.planes(), sizes.join(", ")) };
    println!("{file}: {w}x{h}");
    println!("  software decodes as {:?}: {bits} bits, chroma {cw}x{ch}, alpha {}", sw.format(), if alpha { "yes" } else { "no" });
    println!("  videotoolbox hands back '{}', {layout}", pb.format());

    let s16 = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let mut low_bits = 0u16;
    // components in the software decoder's order: luma, cb, cr, alpha
    let mut d = [Diff::default(); 4];
    let mut compare = |c: usize, got: u16, want: u16| {
        low_bits |= got;
        // the hardware sample holds the value in its top bits: round it to the software decoder's depth
        let got = ((got as u32 + (1 << (15 - bits))) >> (16 - bits)).min((1 << bits) - 1);
        let apart = got.abs_diff(want as u32);
        d[c].differ += (apart != 0) as u64;
        d[c].worst = d[c].worst.max(apart);
        d[c].of += 1;
    };
    let (sy, ys) = sw.plane16(0, h);
    let (su, us) = sw.plane16(1, ch);
    let (sv, vs) = sw.plane16(2, ch);
    let sa = alpha.then(|| sw.plane16(3, h));
    let mut alpha_there = false;
    if pb.planes() == 0 && pb.format() == "y416" {
        // packed 16-bit 4:4:4:4, in the order alpha, luma, cb, cr
        let (px, stride) = pb.copy_packed();
        alpha_there = true;
        for y in 0..h {
            for x in 0..w {
                let at = y * stride + x * 8;
                compare(0, s16(&px, at + 2), sy[y * ys + x]);
                compare(1, s16(&px, at + 4), su[y * us + x]);
                compare(2, s16(&px, at + 6), sv[y * vs + x]);
                if let Some((sa, sas)) = sa {
                    compare(3, s16(&px, at), sa[y * sas + x]);
                }
            }
        }
    } else if pb.planes() >= 2 && pb.plane_size(0) == (w, h) && pb.plane_size(1) == (cw, ch) {
        let (luma, ls) = pb.copy_plane(0);
        let (chroma, cs) = pb.copy_plane(1);
        for y in 0..h {
            for x in 0..w {
                compare(0, s16(&luma, y * ls + x * 2), sy[y * ys + x]);
            }
        }
        for y in 0..ch {
            for x in 0..cw {
                compare(1, s16(&chroma, y * cs + x * 4), su[y * us + x]);
                compare(2, s16(&chroma, y * cs + x * 4 + 2), sv[y * vs + x]);
            }
        }
        if let (Some((sa, sas)), true) = (sa, pb.planes() >= 3 && pb.plane_size(2) == (w, h)) {
            let (a, astride) = pb.copy_plane(2);
            alpha_there = true;
            for y in 0..h {
                for x in 0..w {
                    compare(3, s16(&a, y * astride + x * 2), sa[y * sas + x]);
                }
            }
        }
    } else {
        std::process::exit(spike::verdict(false, "the hardware buffer has a layout this check does not know"));
    }
    // how many of the 16 bits ever carry anything: 10-bit content in the top bits leaves the low 6 at zero
    println!("  bits the hardware samples carry: {}", 16 - low_bits.trailing_zeros());
    for (c, name) in ["luma", "cb", "cr", "alpha"].iter().enumerate().filter(|(c, _)| d[*c].of > 0) {
        println!("  {name:<5} against software at {bits} bits: {:.2}% of samples differ, by at most {} code value(s)", d[c].differ as f64 * 100.0 / d[c].of as f64, d[c].worst);
    }
    if alpha && !alpha_there {
        println!("  alpha: NOT in the hardware buffer. the file has it and the software decoder returns it");
    }
    let worst = d.iter().map(|x| x.worst).max().unwrap();
    let pass = worst <= 1 && (alpha_there || !alpha);
    std::process::exit(spike::verdict(pass, "videotoolbox returns the software decoder's picture to within one code value, alpha included"));
}
