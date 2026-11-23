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
            let le = |b: &[u8]| b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as usize).collect::<Vec<_>>();
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
            if comp.len() != sizes.len() || comp.is_empty() {
                return Err("hap chunk tables disagree".into());
            }
            let mut at = 0;
            let mut out = Vec::with_capacity(sizes.len());
            for i in 0..sizes.len() {
                let start = if offsets.is_empty() { at } else { offsets[i] };
                let d = data.get(start..start + sizes[i]).ok_or("hap chunk outside its frame")?;
                let snappy = comp[i] == 0x0b;
                out.push(Chunk { data: d, snappy, out_len: if snappy { snappy_len(d)? } else { d.len() } });
                at = start + sizes[i];
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
