//! hap frames, parsed and decompressed to the texture blocks exactly as stored (adr 0003).
//! follows the vidvox spec: https://github.com/Vidvox/hap/blob/master/documentation/HapVideoDRAFT.md

use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tex {
    Bc1,
    Bc3,
    /// scaled ycocg in bc3: hap q
    YCoCgBc3,
    Bc7,
    /// a lone alpha plane: hap alpha-only, or the alpha half of hap q alpha
    Bc4,
    Bc6u,
    Bc6s,
}

impl Tex {
    fn from_nibble(n: u8) -> Result<Tex, String> {
        Ok(match n {
            0x0b => Tex::Bc1,
            0x0e => Tex::Bc3,
            0x0f => Tex::YCoCgBc3,
            0x0c => Tex::Bc7,
            0x01 => Tex::Bc4,
            0x02 => Tex::Bc6u,
            0x03 => Tex::Bc6s,
            other => return Err(format!("unknown hap texture format 0x{other:x}")),
        })
    }
    /// bytes per 4x4 block.
    pub fn block_bytes(self) -> usize {
        match self {
            Tex::Bc1 | Tex::Bc4 => 8,
            _ => 16,
        }
    }
}

pub struct Chunk<'a> {
    pub data: &'a [u8],
    pub snappy: bool,
    pub out_len: usize,
}

/// one texture of a frame. hap q alpha has two; every other variant has one.
pub struct Layer<'a> {
    pub tex: Tex,
    pub chunks: Vec<Chunk<'a>>,
    /// the size of the texture data once decompressed
    pub len: usize,
}

/// a section: its type, its body, and how many bytes the whole section took.
fn section(d: &[u8]) -> Result<(u8, &[u8], usize), String> {
    if d.len() < 4 {
        return Err("hap section cut short".into());
    }
    let mut size = d[0] as usize | (d[1] as usize) << 8 | (d[2] as usize) << 16;
    let mut header = 4;
    if size == 0 {
        if d.len() < 8 {
            return Err("hap section cut short".into());
        }
        size = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
        header = 8;
    }
    if header + size > d.len() {
        return Err("hap section longer than its packet".into());
    }
    Ok((d[3], &d[header..header + size], header + size))
}

fn snappy_len(d: &[u8]) -> Result<usize, String> {
    snap::raw::decompress_len(d).map_err(|e| e.to_string())
}

fn layer(kind: u8, body: &[u8]) -> Result<Layer<'_>, String> {
    let tex = Tex::from_nibble(kind & 0x0f)?;
    let chunks = match kind >> 4 {
        0xa => vec![Chunk { data: body, snappy: false, out_len: body.len() }],
        0xb => vec![Chunk { data: body, snappy: true, out_len: snappy_len(body)? }],
        0xc => {
            // the decode instructions come first, then the chunks themselves
            let (t, mut di, used) = section(body)?;
            if t != 0x01 {
                return Err("hap frame without decode instructions".into());
            }
            let data = &body[used..];
            let (mut comp, mut sizes, mut offsets): (&[u8], Vec<usize>, Vec<usize>) = (&[], Vec::new(), Vec::new());
            let le = |b: &[u8]| b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c) as usize).collect::<Vec<_>>();
            while !di.is_empty() {
                let (t, b, used) = section(di)?;
                match t {
                    0x02 => comp = b,
                    0x03 => sizes = le(b),
                    0x04 => offsets = le(b),
                    _ => {}
                }
                di = &di[used..];
            }
            // the offset table is optional, but one that is there has to cover every chunk
            if comp.len() != sizes.len() || comp.is_empty() || (!offsets.is_empty() && offsets.len() != sizes.len()) {
                return Err("hap chunk tables disagree".into());
            }
            let mut at = 0;
            let mut out = Vec::with_capacity(sizes.len());
            for i in 0..sizes.len() {
                let start = if offsets.is_empty() { at } else { offsets[i] };
                let end = start.checked_add(sizes[i]).ok_or("hap chunk outside its frame")?;
                let d = data.get(start..end).ok_or("hap chunk outside its frame")?;
                let snappy = comp[i] == 0x0b;
                out.push(Chunk { data: d, snappy, out_len: if snappy { snappy_len(d)? } else { d.len() } });
                at = end;
            }
            out
        }
        other => return Err(format!("unknown hap compressor 0x{other:x}")),
    };
    let len = chunks.iter().map(|c| c.out_len).sum();
    Ok(Layer { tex, chunks, len })
}

/// reads the structure of one hap frame. nothing is decompressed yet.
pub fn parse(frame: &[u8]) -> Result<Vec<Layer<'_>>, String> {
    let (kind, body, _) = section(frame)?;
    if kind == 0x0d {
        let (mut rest, mut out) = (body, Vec::new());
        while !rest.is_empty() {
            let (k, b, used) = section(rest)?;
            out.push(layer(k, b)?);
            rest = &rest[used..];
        }
        return Ok(out);
    }
    Ok(vec![layer(kind, body)?])
}

/// decompresses a layer into `out`, which must hold `layer.len` bytes, on up to `threads` threads.
/// each chunk goes to its own part of `out`, so `out` can be the gpu's own mapped memory.
pub fn decompress(layer: &Layer, out: &mut [u8], threads: usize) -> Result<(), String> {
    let mut jobs = Vec::with_capacity(layer.chunks.len());
    let mut rest = &mut out[..layer.len];
    for c in &layer.chunks {
        let (mine, tail) = rest.split_at_mut(c.out_len);
        jobs.push((c, mine));
        rest = tail;
    }
    let run = |(c, dst): (&Chunk, &mut [u8])| -> Result<(), String> {
        if c.snappy {
            snap::raw::Decoder::new().decompress(c.data, dst).map(|_| ()).map_err(|e| e.to_string())
        } else {
            dst.copy_from_slice(c.data);
            Ok(())
        }
    };
    let n = threads.max(1).min(jobs.len());
    if n == 1 {
        return jobs.into_iter().try_for_each(run);
    }
    let queue = Mutex::new(jobs.into_iter());
    let failed = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..n {
            s.spawn(|| loop {
                let job = queue.lock().unwrap().next();
                let Some(job) = job else { break };
                if let Err(e) = run(job) {
                    *failed.lock().unwrap() = Some(e);
                }
            });
        }
    });
    match failed.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// the size a hap texture really has: the picture rounded up to whole 4x4 blocks.
pub fn padded(w: usize, h: usize) -> (usize, usize) {
    (w.div_ceil(4) * 4, h.div_ceil(4) * 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::ff::{self, Input};

    /// frame `n` of a clip in tests/media, as stored in the file.
    fn stored(name: &str, n: usize) -> Vec<u8> {
        let path = format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"));
        let input = Input::open(&path).expect("open");
        let s = input.samples(input.first_video().expect("video").index)[n];
        let mut raw = vec![0u8; s.size];
        ff::read_at(&ff::open_shared(&path).unwrap(), &mut raw, s.pos).expect("read");
        raw
    }

    /// bc1 blocks to rgb, written out plainly: two 5:6:5 colors a block, two more mixed from them, two bits a pixel.
    fn bc1_to_rgb(blocks: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut out = vec![0u8; w * h * 3];
        for (i, b) in blocks.chunks_exact(8).enumerate() {
            let (c0, c1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
            let rgb = |c: u16| {
                let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
                [((r << 3) | (r >> 2)) as u32, ((g << 2) | (g >> 4)) as u32, ((b << 3) | (b >> 2)) as u32]
            };
            let (a, c) = (rgb(c0), rgb(c1));
            let mix = |wa: u32, wc: u32| [0, 1, 2].map(|k| (a[k] * wa + c[k] * wc + (wa + wc) / 2) / (wa + wc));
            let palette = if c0 > c1 { [a, c, mix(2, 1), mix(1, 2)] } else { [a, c, mix(1, 1), [0, 0, 0]] };
            let picks = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            for p in 0..16 {
                let (x, y) = (i % (w / 4) * 4 + p % 4, i / (w / 4) * 4 + p / 4);
                let color = palette[((picks >> (2 * p)) & 3) as usize];
                for k in 0..3 {
                    out[(y * w + x) * 3 + k] = color[k] as u8;
                }
            }
        }
        out
    }

    #[test]
    fn a_hap_frame_decodes_to_ffmpegs_picture() {
        let raw = stored("hap_64.mov", 0);
        let layers = parse(&raw).expect("parse");
        assert_eq!(layers.len(), 1);
        assert_eq!((layers[0].tex, layers[0].chunks.len(), layers[0].len), (Tex::Bc1, 2, 64 / 4 * (64 / 4) * 8));
        let mut blocks = vec![0u8; layers[0].len];
        decompress(&layers[0], &mut blocks, 1).expect("decompress");
        // the referee: ffmpeg's own decode of the same frame, written by tests/media/make.ps1.
        // the two decoders round the mixed colors differently, by a few code values at most
        let referee = std::fs::read(format!("{}/tests/media/hap_64.frame0.rgb", env!("CARGO_MANIFEST_DIR"))).expect("referee");
        let ours = bc1_to_rgb(&blocks, 64, 64);
        let worst = ours.iter().zip(&referee).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 3, "worst difference from ffmpeg's decode: {worst} of 255");
    }

    #[test]
    fn threads_do_not_change_the_result() {
        let raw = stored("hapq_64.mov", 1);
        let layers = parse(&raw).expect("parse");
        assert_eq!((layers[0].tex, layers[0].chunks.len(), layers[0].len), (Tex::YCoCgBc3, 4, 64 / 4 * (64 / 4) * 16));
        let (mut one, mut four) = (vec![0u8; layers[0].len], vec![0u8; layers[0].len]);
        decompress(&layers[0], &mut one, 1).expect("decompress");
        decompress(&layers[0], &mut four, 4).expect("decompress");
        assert!(one == four && one.iter().any(|&b| b != 0));
    }

    #[test]
    fn a_damaged_frame_is_refused_not_trusted() {
        let raw = stored("hapq_64.mov", 0);
        assert!(parse(&[]).is_err());
        assert!(parse(&raw[..3]).is_err());
        assert!(parse(&raw[..raw.len() - 9]).is_err());
        // a section that claims more than the packet holds
        let mut lying = raw.clone();
        lying[0] = 0xff;
        lying[1] = 0xff;
        lying[2] = 0xff;
        assert!(parse(&lying).is_err());
    }

    /// a hap section: three bytes of length, one of type, then the body.
    fn section(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut s = vec![body.len() as u8, (body.len() >> 8) as u8, (body.len() >> 16) as u8, kind];
        s.extend_from_slice(body);
        s
    }

    /// a frame of two stored chunks, with whatever offset table the test asks for.
    fn two_chunks(offsets: &[u32]) -> Vec<u8> {
        let le = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let mut tables = section(0x02, &[0x0a, 0x0a]);
        tables.extend(section(0x03, &le(&[4, 4])));
        if !offsets.is_empty() {
            tables.extend(section(0x04, &le(offsets)));
        }
        let mut body = section(0x01, &tables);
        body.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        section(0xcb, &body)
    }

    #[test]
    fn chunk_tables_that_disagree_are_refused() {
        // the same frame, well formed, parses: the refusals below are about the tables alone
        assert_eq!(parse(&two_chunks(&[])).unwrap()[0].len, 8);
        assert_eq!(parse(&two_chunks(&[0, 4])).unwrap()[0].len, 8);
        // an offset table shorter than the size table
        assert!(parse(&two_chunks(&[0])).is_err());
        // offsets that point outside the frame, or wrap around
        assert!(parse(&two_chunks(&[0, 6])).is_err());
        assert!(parse(&two_chunks(&[0, u32::MAX])).is_err());
    }

    #[test]
    fn sizes_round_up_to_whole_blocks() {
        assert_eq!(padded(64, 64), (64, 64));
        assert_eq!(padded(1921, 1079), (1924, 1080));
    }
}
