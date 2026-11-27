//! a copy of a clip into a new file of the same kind, its packets as they are: a trim, or the clip without its sound
//! (adr 0009). libavformat alone, reading through the player's own shared handle (`ff::Input`), so a render can still
//! replace the source while a copy runs. nothing is decoded.
//!
//! a copy starts where decoding can: at a key frame. where every frame is one (prores, hap, dnx) that is the frame
//! asked for; with long groups (h.264, hevc) it is the key frame at or before it, and the copy says so. it ends where
//! every frame up to the last one asked is stored: with frames stored out of the order they are shown in, that can be
//! a frame or two past it.

use super::clip::{self, Count};
use super::ff::{self, Input, Sample};
use super::meta;
use super::timeline::Timeline;
use ffmpeg_sys_next as ffi;
use std::ffi::{c_int, c_void, CString};
use std::fs::File;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

// the two errno values ffmpeg's error codes are built from, the same on every system we build for
const EIO: c_int = 5;
const EINVAL: c_int = 22;
const IO_BUFFER: usize = 1 << 16;
/// once the picture is done, the sound is read on until the file is this many seconds past the end: it is stored
/// near the picture of the same time
const SOUND_AFTER: f64 = 5.0;

/// how a copy is cut: the frames it keeps, as the player counts them, and where they are in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// the first frame kept and the last, both kept
    pub first: usize,
    pub last: usize,
    /// how many tracks of sound the clip has, which a copy keeps or leaves out
    pub sound: usize,
    /// where in the file the picture kept starts and ends, for how far a copy has got
    bytes: (u64, u64),
    /// the key frame decoding starts at, by its place among the stored samples, and the frame it is: before the
    /// clip's first (-1) when an edit list hides it
    key: usize,
    key_frame: i64,
    /// the time the reader is set by to read from the key frame
    seek: i64,
    /// the place among the stored samples of the last of the picture's packets kept
    end: usize,
    /// when the first frame kept is shown and when the last one ends, in the picture stream's units
    from: i64,
    to: i64,
}

impl Cut {
    pub fn frames(&self) -> usize {
        self.last + 1 - self.first
    }

    /// a cut of these frames of a clip with this many tracks of sound, for the controls' tests
    #[cfg(test)]
    pub fn of(first: usize, last: usize, sound: usize) -> Cut {
        Cut { first, last, sound, bytes: (0, 0), key: 0, key_frame: first as i64, seek: 0, end: 0, from: 0, to: 0 }
    }
}

/// how a copy of frames `asked` (both kept) of the clip at `path` would be cut. it reads the file's table, and the
/// packets about the key frames at either end; setting `stop` ends a count of a file that lists no frames up front.
pub fn plan(path: &str, asked: (usize, usize), stop: &AtomicBool) -> Result<Cut, String> {
    Source::open(path, stop)?.plan(asked)
}

/// copies frames `asked` (both kept) of the clip at `path` into `out`, a file made new for it, with the clip's sound or
/// without it, as the kind of file `name` names by its extension (the source's own). `done` follows how far it has got
/// through the picture's bytes, in millionths, and setting `stop` ends the copy with an error. returns how it was cut.
pub fn copy(path: &str, asked: (usize, usize), sound: bool, out: File, name: &str, done: &AtomicU64, stop: &AtomicBool) -> Result<Cut, String> {
    let mut source = Source::open(path, stop)?;
    let cut = source.plan(asked)?;
    // the picture, and the sound when it is kept. a timecode track is not copied: the muxer makes one again from the
    // timecode, which is the first frame's now (mapped as a stream it kept the source's start, and in mp4 it failed:
    // "could not find tag for codec none")
    let mut kept = vec![source.video];
    if sound {
        kept.extend(sound_streams(&source.input));
    }
    let timecode = source.timecode(&cut);
    let mut writer = Writer::new(out, name)?;
    writer.add(&source.input, &kept, timecode.as_deref())?;
    writer.header()?;
    // a file that keeps only when frames are shown (matroska) gives no decode times, which a muxer wants
    let mut decoded = Decoded { times: vec![ffi::AV_NOPTS_VALUE; source.delay() + 1], step: source.length(cut.first) };
    // every stream's times start again at the first frame kept: what an edit list hid stays hidden, before it
    let ctx = source.input.ctx;
    let tb = move |i: usize| unsafe { (**(*ctx).streams.add(i)).time_base };
    let video_tb = tb(source.video);
    let shift: Vec<i64> = kept.iter().map(|&i| unsafe { ffi::av_rescale_q(cut.from, video_tb, tb(i)) }).collect();
    let ends: Vec<i64> = kept.iter().map(|&i| unsafe { ffi::av_rescale_q(cut.to, video_tb, tb(i)) }).collect();
    let past = seconds(cut.to, video_tb) + SOUND_AFTER;
    source.seek(cut.seek)?;
    let key_pos = source.samples[cut.key].pos as i64;
    let mut packet = Packet::new();
    // the place among the stored samples of the picture's packet in hand, once the key frame is reached
    let mut place: Option<usize> = None;
    let mut ended = vec![false; kept.len()];
    // each sound's last packet that starts before the first frame, kept until the next shows it is the last
    let mut before: Vec<Option<Packet>> = kept.iter().map(|_| None).collect();
    while read(&source.input, &mut packet)? {
        if stop.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let Some(o) = kept.iter().position(|&i| i == packet.stream()) else { continue };
        if ended[0] && seconds(packet.time(), tb(kept[o])) > past {
            break;
        }
        if o == 0 {
            if ended[0] {
                continue;
            }
            // what the reader passes on its way to the key frame is the group before's
            let at = match place {
                Some(at) => at + 1,
                None if packet.pos() == key_pos => cut.key,
                None => continue,
            };
            place = Some(at);
            // a picture stored after the key frame but shown before it belongs to the group before (an open group's)
            if source.frame_of(packet.time()) >= cut.key_frame {
                decoded.fill(&mut packet);
                writer.write(&mut packet, 0, shift[0])?;
            }
            let read = (packet.pos().max(0) as u64).saturating_sub(cut.bytes.0) as u128;
            done.store((read * 1_000_000 / (cut.bytes.1.saturating_sub(cut.bytes.0) as u128).max(1)).min(1_000_000) as u64, Ordering::Relaxed);
            ended[0] = at >= cut.end;
        } else if !ended[o] {
            // the sound, packet for packet (it is not cut inside one): from the last packet that starts before the
            // first frame, which a decoder needs to start clean and the new file hides (mov's edit list, matroska's
            // codec delay), to the last that starts before the last frame ends
            let time = packet.time();
            if time >= ends[o] {
                ended[o] = true;
            } else if time < shift[o] {
                before[o] = Some(packet.take());
            } else {
                if let Some(mut first) = before[o].take() {
                    writer.write(&mut first, o, shift[o])?;
                }
                writer.write(&mut packet, o, shift[o])?;
            }
        }
        if ended.iter().all(|e| *e) {
            break;
        }
    }
    if place.is_none() {
        return Err("the key frame is not where the file's table puts it".into());
    }
    writer.finish()?;
    Ok(cut)
}

/// reads the copy at `written` back the player's own way (adr 0003: its frames from its own table, never ffmpeg's
/// summary fields) and says what differs from what `cut` asked of the clip at `path`: how many frames, how long they
/// last, the first frame's timecode, the sound. None when nothing does.
pub fn check(written: &str, path: &str, cut: &Cut, sound: bool, stop: &AtomicBool) -> Result<Option<String>, String> {
    let source = Source::open(path, stop)?;
    let asked = Timeline { time_base: source.time_base, times: source.times[cut.first..=cut.last].to_vec() };
    let input = Input::open(written)?;
    let video = input.first_video().ok_or("the copy has no picture")?;
    let (samples, shown) = clip::stored(written, &input, &video, &Count::Read { counted: &AtomicUsize::new(0), stop })?;
    let (_, timeline, _) = clip::frames(&video, samples, shown);
    let mut differs = Vec::new();
    if timeline.frames() != cut.frames() {
        differs.push(format!("{} frames, {} asked", timeline.frames(), cut.frames()));
    } else {
        let (got, wanted) = (meta::duration_row(&timeline).value, meta::duration_row(&asked).value);
        if got != wanted {
            differs.push(format!("{got} long, {wanted} asked"));
        }
    }
    match (meta::timecode_tag(&input, video.index), source.timecode(cut)) {
        (got, wanted) if got == wanted => {}
        (Some(got), Some(wanted)) => differs.push(format!("timecode {got}, {wanted} asked")),
        (None, Some(wanted)) => differs.push(format!("no timecode, {wanted} asked")),
        (Some(got), None) => differs.push(format!("timecode {got}, none asked")),
        (None, None) => {}
    }
    let (got, wanted) = (sound_streams(&input).len(), if sound { sound_streams(&source.input).len() } else { 0 });
    if got != wanted {
        differs.push(format!("{got} sound tracks, {wanted} asked"));
    }
    Ok((!differs.is_empty()).then(|| differs.join(", ")))
}

/// how many tracks of sound the clip at `path` has that a copy can keep or leave out.
pub fn sound_tracks(path: &str) -> Result<usize, String> {
    Ok(sound_streams(&Input::open(path)?).len())
}

/// the streams of sound a copy can keep: every one with a codec ffmpeg knows
fn sound_streams(input: &Input) -> Vec<usize> {
    (0..input.streams())
        .filter(|&i| unsafe {
            let par = (**(*input.ctx).streams.add(i)).codecpar;
            (*par).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_AUDIO && (*par).codec_id != ffi::AVCodecID::AV_CODEC_ID_NONE
        })
        .collect()
}

fn seconds(time: i64, tb: ffi::AVRational) -> f64 {
    time as f64 * tb.num as f64 / tb.den as f64
}

/// a clip's picture as a copy reads it: its samples in the order they are stored, and when each of its frames is shown.
struct Source {
    input: Input,
    video: usize,
    time_base: (i64, i64),
    samples: Vec<Sample>,
    /// when each frame the player counts is shown, in the stream's units, as the file's packets have it
    times: Vec<i64>,
    timeline: Timeline,
}

impl Source {
    fn open(path: &str, stop: &AtomicBool) -> Result<Source, String> {
        let mut input = Input::open(path)?;
        let video = input.first_video().ok_or_else(|| format!("{path}: no picture in this file"))?;
        let (samples, shown) = clip::stored(path, &input, &video, &Count::Read { counted: &AtomicUsize::new(0), stop })?;
        let (_, timeline, _) = clip::frames(&video, samples.clone(), shown);
        if timeline.frames() == 0 {
            return Err(format!("{path}: no frames in this file"));
        }
        // a table's times count from the first frame, which is placed by when it is shown: among the first packets,
        // after what an edit list hides, frames being stored at most 16 places from where they are shown
        let hidden = samples.iter().filter(|s| s.hidden).count();
        let first = first_shown(&mut input, video.index, hidden + 16)?;
        let times = timeline.times.iter().map(|t| t - timeline.times[0] + first).collect();
        Ok(Source { input, video: video.index, time_base: video.time_base, samples, times, timeline })
    }

    /// the frame shown at `time`: -1 before the clip's first (what an edit list hides), and its count after its last.
    fn frame_of(&self, time: i64) -> i64 {
        match self.times.binary_search(&time) {
            Ok(i) => i as i64,
            Err(0) => -1,
            Err(i) if i == self.times.len() => i as i64,
            Err(i) if time - self.times[i - 1] <= self.times[i] - time => i as i64 - 1,
            Err(i) => i as i64,
        }
    }

    /// how many places at most a frame is stored after its place in the order frames are shown, by the samples' times:
    /// none for a table's, which are decode times.
    fn delay(&self) -> usize {
        let mut order: Vec<usize> = (0..self.samples.len()).collect();
        order.sort_by_key(|&i| self.samples[i].time);
        order.iter().enumerate().map(|(shown, &stored)| stored.saturating_sub(shown)).max().unwrap_or(0)
    }

    /// how long frame `f` is shown, in the stream's units: until the next, or as long as the one before it.
    fn length(&self, f: usize) -> i64 {
        let t = &self.times;
        t.get(f + 1).map(|next| next - t[f]).or_else(|| (f > 0).then(|| t[f] - t[f - 1])).unwrap_or(1)
    }

    fn plan(&mut self, (from, to): (usize, usize)) -> Result<Cut, String> {
        let frames = self.times.len();
        if from > to || to >= frames {
            return Err(format!("frames {from} to {to} are not in this clip of {frames}"));
        }
        let hidden = self.samples.iter().filter(|s| s.hidden).count();
        let keys: Vec<usize> = (0..self.samples.len()).filter(|&i| self.samples[i].key).collect();
        if keys.is_empty() {
            return Err("this file marks no key frame".into());
        }
        let mut packet = Packet::new();
        // the key frame to start at: the last at or before the first frame asked. a key frame is never stored later
        // than it is shown, so it is among those stored up to that frame's place; one shown after the frame (an open
        // group's) gives way to the one before
        let mut at = keys.iter().rposition(|&k| k <= from + hidden).unwrap_or(0);
        let (key, key_frame, seek) = loop {
            let seek = self.go_to(keys[at], &mut packet)?;
            let frame = self.frame_of(packet.time());
            if frame <= from as i64 || at == 0 {
                break (keys[at], frame, seek);
            }
            at -= 1;
        };
        // where it ends: walked from the last key frame stored up to the last frame asked
        let end_key = keys[keys.iter().rposition(|&k| k <= to + hidden).unwrap_or(0)].max(key);
        self.go_to(end_key, &mut packet)?;
        let mut walk = Walk { last: to as i64, stored: self.samples[..end_key].iter().filter(|s| !s.hidden).count() as i64, latest: i64::MIN };
        let mut end = end_key;
        while !walk.take(self.frame_of(packet.time()), self.samples[end].hidden) && end + 1 < self.samples.len() && self.next_video(&mut packet)? {
            end += 1;
        }
        let first = key_frame.max(0) as usize;
        let last = (walk.latest.max(first as i64) as usize).min(frames - 1);
        let bytes = (self.samples[key].pos, self.samples[end].pos + self.samples[end].size as u64);
        Ok(Cut { first, last, sound: sound_streams(&self.input).len(), bytes, key, key_frame, seek, end, from: self.times[first], to: self.times[last] + self.length(last) })
    }

    /// the timecode a copy's first frame is called by: the file's own at that frame. a file with none gets none, as
    /// does one whose clock the player has no one rate to count by, unless the copy starts where the file does.
    fn timecode(&self, cut: &Cut) -> Option<String> {
        let own = meta::timecode_tag(&self.input, self.video)?;
        if cut.first == 0 {
            return Some(own);
        }
        clip::own_clock(&self.timeline, Some((own, self.input.timecode_base()))).map(|clock| clock.at(cut.first))
    }

    /// sets the reader by `time`, at the key frame at or before it.
    fn seek(&self, time: i64) -> Result<(), String> {
        match unsafe { ffi::av_seek_frame(self.input.ctx, self.video as c_int, time, ffi::AVSEEK_FLAG_BACKWARD) } {
            r if r < 0 => Err(format!("seek: {}", ff::err(r))),
            _ => Ok(()),
        }
    }

    /// reads on to the picture's packet at place `key` and returns the time the reader was set by to get there. a
    /// sample's time is what the reader seeks by, but not always as ffmpeg's reader takes it (a fragmented mp4's
    /// counted times are when frames are shown; it seeks by when they are stored): a reader set past the key frame is
    /// set again by the key frame before, and one set early reads on.
    fn go_to(&self, key: usize, packet: &mut Packet) -> Result<i64, String> {
        let pos = self.samples[key].pos as i64;
        for earlier in (0..=key).rev().filter(|&k| self.samples[k].key) {
            let time = self.samples[earlier].time;
            self.seek(time)?;
            while self.next_video(packet)? {
                if packet.pos() == pos {
                    return Ok(time);
                }
                if packet.pos() > pos {
                    break;
                }
            }
        }
        Err("the key frame is not where the file's table puts it".into())
    }

    /// the picture's next packet. false at the file's end.
    fn next_video(&self, packet: &mut Packet) -> Result<bool, String> {
        while read(&self.input, packet)? {
            if packet.stream() == self.video {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// the time of the first frame shown: the earliest of the picture's first `n` packets that an edit list does not hide.
fn first_shown(input: &mut Input, video: usize, n: usize) -> Result<i64, String> {
    let (mut packet, mut first, mut seen) = (Packet::new(), None::<i64>, 0);
    while seen < n && read(input, &mut packet)? {
        if packet.stream() == video {
            seen += 1;
            if !packet.hidden() {
                first = Some(first.map_or(packet.time(), |t| t.min(packet.time())));
            }
        }
    }
    first.ok_or_else(|| "no frame of this file is shown".to_string())
}

/// the picture's packets taken in stored order, to find where a copy can end: at the first packet, at or after the
/// one of the last frame asked, by which every frame up to the latest seen is stored. with frames stored out of order,
/// a later frame is stored before the earlier ones it is needed for, and is kept with them.
struct Walk {
    last: i64,
    /// how many frames the file stores up to the packet in hand, what an edit list hides left out
    stored: i64,
    latest: i64,
}

impl Walk {
    /// the next packet: the frame it is and whether an edit list hides it. true when the copy can end with it.
    fn take(&mut self, frame: i64, hidden: bool) -> bool {
        self.stored += !hidden as i64;
        self.latest = self.latest.max(frame);
        self.latest >= self.last && self.stored == self.latest + 1
    }
}

/// decode times for packets that have none, as ffmpeg's muxer made them up until it stopped doing so: the earliest of
/// the shown times not given out yet, `times` holding as many as frames can be stored after where they are shown, and one.
struct Decoded {
    times: Vec<i64>,
    /// how long a frame lasts, for a packet that does not say
    step: i64,
}

impl Decoded {
    fn fill(&mut self, packet: &mut Packet) {
        let p = unsafe { &mut *packet.0 };
        if p.dts != ffi::AV_NOPTS_VALUE || p.pts == ffi::AV_NOPTS_VALUE {
            return;
        }
        let (t, step) = (&mut self.times, if p.duration > 0 { p.duration } else { self.step });
        t[0] = p.pts;
        // the first packet: as if the frames before it had come, a frame apart
        for i in 1..t.len() {
            if t[i] == ffi::AV_NOPTS_VALUE {
                t[i] = p.pts + (i as i64 - t.len() as i64) * step;
            }
        }
        for i in 0..t.len() - 1 {
            if t[i] <= t[i + 1] {
                break;
            }
            t.swap(i, i + 1);
        }
        p.dts = t[0];
    }
}

/// a packet of the file, read into again and again.
struct Packet(*mut ffi::AVPacket);

impl Packet {
    fn new() -> Packet {
        Packet(unsafe { ffi::av_packet_alloc() })
    }

    fn stream(&self) -> usize {
        unsafe { (*self.0).stream_index as usize }
    }

    fn pos(&self) -> i64 {
        unsafe { (*self.0).pos }
    }

    /// when it is shown, or failing that when it is decoded, in its stream's units
    fn time(&self) -> i64 {
        unsafe { if (*self.0).pts != ffi::AV_NOPTS_VALUE { (*self.0).pts } else { (*self.0).dts } }
    }

    /// what it holds, moved into a packet of its own: this one is left empty, to be read into again
    fn take(&mut self) -> Packet {
        let taken = Packet::new();
        unsafe { ffi::av_packet_move_ref(taken.0, self.0) };
        taken
    }

    fn hidden(&self) -> bool {
        unsafe { (*self.0).flags & ffi::AV_PKT_FLAG_DISCARD != 0 }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { ffi::av_packet_free(&mut self.0) };
    }
}

/// the file's next packet into `packet`. false at its end.
fn read(input: &Input, packet: &mut Packet) -> Result<bool, String> {
    unsafe {
        ffi::av_packet_unref(packet.0);
        match ffi::av_read_frame(input.ctx, packet.0) {
            ffi::AVERROR_EOF => Ok(false),
            r if r < 0 => Err(ff::err(r)),
            _ => Ok(true),
        }
    }
}

/// what ffmpeg writes through: our own new file and our own position.
struct Out {
    file: File,
    pos: u64,
}

fn write_at(file: &File, mut data: &[u8], mut pos: u64) -> std::io::Result<()> {
    while !data.is_empty() {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_write(file, data, pos)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::write_at(file, data, pos)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        (data, pos) = (&data[n..], pos + n as u64);
    }
    Ok(())
}

unsafe extern "C" fn out_write(opaque: *mut c_void, buf: *const u8, size: c_int) -> c_int {
    let out = &mut *(opaque as *mut Out);
    match write_at(&out.file, std::slice::from_raw_parts(buf, size.max(0) as usize), out.pos) {
        Ok(()) => {
            out.pos += size.max(0) as u64;
            size
        }
        Err(_) => -EIO,
    }
}

unsafe extern "C" fn out_seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let out = &mut *(opaque as *mut Out);
    let len = out.file.metadata().map(|m| m.len() as i64).unwrap_or(-1);
    if whence & ffi::AVSEEK_SIZE as c_int != 0 {
        return len;
    }
    let to = match whence & 3 {
        0 => offset,
        1 => out.pos as i64 + offset,
        2 => len + offset,
        _ => -1,
    };
    if to < 0 {
        return -EINVAL as i64;
    }
    out.pos = to as u64;
    to
}

/// the new file: ffmpeg's muxer for its kind, writing through our own handle.
struct Writer {
    ctx: *mut ffi::AVFormatContext,
    pb: *mut ffi::AVIOContext,
    out: *mut Out,
    /// each stream's units in the source
    from: Vec<ffi::AVRational>,
}

impl Writer {
    fn new(file: File, name: &str) -> Result<Writer, String> {
        let hint = CString::new(name).map_err(|e| e.to_string())?;
        unsafe {
            let mut ctx = ptr::null_mut();
            let r = ffi::avformat_alloc_output_context2(&mut ctx, ptr::null(), ptr::null(), hint.as_ptr());
            if r < 0 || ctx.is_null() {
                return Err(format!("{name}: this kind of file is not written"));
            }
            let out = Box::into_raw(Box::new(Out { file, pos: 0 }));
            let pb = ffi::avio_alloc_context(ffi::av_malloc(IO_BUFFER) as *mut u8, IO_BUFFER as c_int, 1, out as *mut c_void, None, Some(out_write), Some(out_seek));
            (*ctx).pb = pb;
            Ok(Writer { ctx, pb, out, from: Vec::new() })
        }
    }

    /// the source's streams `streams` as they are, the picture first, with the source's metadata (its creation time
    /// with it), rotation and color tags (in the stream's parameters), and `timecode` for the picture's first frame.
    fn add(&mut self, input: &Input, streams: &[usize], timecode: Option<&str>) -> Result<(), String> {
        unsafe {
            ffi::av_dict_copy(&mut (*self.ctx).metadata, (*input.ctx).metadata, 0);
            ffi::av_dict_set(&mut (*self.ctx).metadata, c"timecode".as_ptr(), ptr::null(), 0);
            let tags = (*(*self.ctx).oformat).codec_tag;
            for (o, &i) in streams.iter().enumerate() {
                let from = *(*input.ctx).streams.add(i);
                let to = ffi::avformat_new_stream(self.ctx, ptr::null());
                if to.is_null() {
                    return Err("no room for another stream".into());
                }
                let r = ffi::avcodec_parameters_copy((*to).codecpar, (*from).codecpar);
                if r < 0 {
                    return Err(ff::err(r));
                }
                // the stream's own tag (hvc1 is not hev1) where this kind of file has it for the codec, as ffmpeg's
                // program keeps it
                let par = (*to).codecpar;
                let mut other = 0;
                if !tags.is_null() && ffi::av_codec_get_id(tags, (*par).codec_tag) != (*par).codec_id && ffi::av_codec_get_tag2(tags, (*par).codec_id, &mut other) != 0 {
                    (*par).codec_tag = 0;
                }
                (*to).time_base = (*from).time_base;
                // the rate the muxer counts a timecode track by
                (*to).avg_frame_rate = (*from).avg_frame_rate;
                (*to).r_frame_rate = (*from).r_frame_rate;
                (*to).sample_aspect_ratio = (*from).sample_aspect_ratio;
                (*to).disposition = (*from).disposition;
                ffi::av_dict_copy(&mut (*to).metadata, (*from).metadata, 0);
                ffi::av_dict_set(&mut (*to).metadata, c"timecode".as_ptr(), ptr::null(), 0);
                if let (0, Some(timecode)) = (o, timecode) {
                    let text = CString::new(timecode).map_err(|e| e.to_string())?;
                    ffi::av_dict_set(&mut (*to).metadata, c"timecode".as_ptr(), text.as_ptr(), 0);
                }
                self.from.push((*from).time_base);
            }
        }
        Ok(())
    }

    fn header(&mut self) -> Result<(), String> {
        match unsafe { ffi::avformat_write_header(self.ctx, ptr::null_mut()) } {
            r if r < 0 => Err(ff::err(r)),
            _ => Ok(()),
        }
    }

    /// writes `packet` as stream `stream`, its times `shift` earlier in the source's units.
    fn write(&mut self, packet: &mut Packet, stream: usize, shift: i64) -> Result<(), String> {
        unsafe {
            let p = packet.0;
            if (*p).pts != ffi::AV_NOPTS_VALUE {
                (*p).pts -= shift;
            }
            if (*p).dts != ffi::AV_NOPTS_VALUE {
                (*p).dts -= shift;
            }
            (*p).stream_index = stream as c_int;
            (*p).pos = -1;
            // hidden by its time before the first frame now, as the source hid it
            (*p).flags &= !ffi::AV_PKT_FLAG_DISCARD;
            ffi::av_packet_rescale_ts(p, self.from[stream], (**(*self.ctx).streams.add(stream)).time_base);
            match ffi::av_interleaved_write_frame(self.ctx, p) {
                r if r < 0 => Err(ff::err(r)),
                _ => Ok(()),
            }
        }
    }

    /// ends the file and has the system put it on the disk.
    fn finish(&mut self) -> Result<(), String> {
        unsafe {
            let r = ffi::av_write_trailer(self.ctx);
            ffi::avio_flush(self.pb);
            if r < 0 {
                return Err(ff::err(r));
            }
            if (*self.pb).error < 0 {
                return Err(ff::err((*self.pb).error));
            }
            (*self.out).file.sync_all().map_err(|e| e.to_string())
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        unsafe {
            ffi::avformat_free_context(self.ctx);
            ffi::av_freep(&mut (*self.pb).buffer as *mut *mut u8 as *mut c_void);
            ffi::avio_context_free(&mut self.pb);
            drop(Box::from_raw(self.out));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::media::clip::{Clip, Lane};
    use crate::media::dec::Picture;
    use crate::media::source::Frame;
    use crate::timecode::Timecode;
    use crate::view::Look;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    pub(crate) fn media(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("media").join(name)
    }

    /// a folder of its own for one case, with the test clips it names copied in: nothing is written beside the
    /// repo's own
    pub(crate) fn scratch(case: &str, clips: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eyepiece-copy-{case}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for clip in clips {
            std::fs::copy(media(clip), dir.join(clip)).unwrap();
        }
        dir
    }

    /// removes what a case left, one file at a time, then its empty folder. a failed case leaves them for a look
    pub(crate) fn tidy(dir: &Path) {
        for file in std::fs::read_dir(dir).unwrap().flatten() {
            std::fs::remove_file(file.path()).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    fn at(dir: &Path, name: &str) -> String {
        dir.join(name).to_string_lossy().into_owned()
    }

    /// copies frames `asked` of `from` to `to`, both in `dir`, and checks the copy as the player does.
    fn copied(dir: &Path, from: &str, asked: (usize, usize), sound: bool, to: &str) -> Cut {
        let out = std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join(to)).unwrap();
        let (done, stop) = (AtomicU64::new(0), AtomicBool::new(false));
        let cut = copy(&at(dir, from), asked, sound, out, to, &done, &stop).unwrap_or_else(|e| panic!("{from}: {e}"));
        assert_eq!(plan(&at(dir, from), asked, &stop).as_ref(), Ok(&cut), "{from}: the plan is the cut");
        assert_eq!(check(&at(dir, to), &at(dir, from), &cut, sound, &stop), Ok(None), "{from}: the copy is not what was asked");
        cut
    }

    /// a number that changes when a picture's luma changes.
    fn luma(picture: &Picture, width: usize) -> u64 {
        let layout = picture.layout().expect("layout");
        let (data, stride, rows) = picture.plane(0, layout.chroma_shift);
        let row_bytes = width * (layout.stored.texel_bits as usize / 8);
        (0..rows).flat_map(|r| &data[r * stride..r * stride + row_bytes]).fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
    }

    /// every frame of a clip, in order: decoded in software, or hap's blocks as stored.
    pub(crate) fn pictures(path: &str) -> Vec<u64> {
        let mut c = Clip::open(path).unwrap_or_else(|e| panic!("{e}"));
        c.start(None).unwrap();
        let blocks = |bytes: &[u8]| bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        fn arrived<T>(got: Frame<T>, then: impl Fn(&T) -> u64) -> Result<u64, String> {
            match got {
                Frame::Ready(it) => Ok(then(&it)),
                Frame::Pending => Err("it never arrived".into()),
                Frame::Failed(why) => Err(why),
            }
        }
        (0..c.timeline.frames())
            .map(|frame| {
                let got = match &c.lane {
                    Lane::Video(video) => {
                        video.source.want(&[frame]);
                        arrived(video.source.get(frame, Duration::from_secs(10)), |picture| luma(picture, c.width))
                    }
                    Lane::Hap(hap) => {
                        hap.source.want(&[frame]);
                        arrived(hap.source.get(frame, Duration::from_secs(10)), |stored| blocks(stored))
                    }
                    _ => panic!("{path}: no frames to compare"),
                };
                got.unwrap_or_else(|why| panic!("{path}: frame {frame}: {why}"))
            })
            .collect()
    }

    /// the copy's frames are the source's, from the first kept to the last, picture for picture.
    fn same_pictures(dir: &Path, from: &str, to: &str, cut: &Cut) {
        assert_eq!(pictures(&at(dir, to)), pictures(&at(dir, from))[cut.first..=cut.last], "{to}: not the frames of {from} it should hold");
    }

    /// where every frame is a key frame the copy is exactly the frames asked: prores
    #[test]
    fn a_clip_of_key_frames_alone_is_cut_exactly() {
        let dir = scratch("intra", &["prores_64.mov"]);
        let cut = copied(&dir, "prores_64.mov", (1, 2), true, "prores_64_trim.mov");
        assert_eq!((cut.first, cut.last, cut.frames()), (1, 2, 2));
        same_pictures(&dir, "prores_64.mov", "prores_64_trim.mov", &cut);
        tidy(&dir);
    }

    /// with long groups the copy starts at the key frame at or before the first frame asked, and ends where the
    /// frames stored out of order are whole again, never short of the last frame asked. h264_320.mp4 has key frames
    /// at 0, 8 and 16, and b-frames
    #[test]
    fn a_clip_of_long_groups_starts_at_a_key_frame() {
        let dir = scratch("long", &["h264_320.mp4"]);
        for (asked, first) in [((11, 13), 8), ((8, 8), 8), ((3, 20), 0), ((0, 23), 0)] {
            let to = format!("h264_320_{}_{}.mp4", asked.0, asked.1);
            let cut = copied(&dir, "h264_320.mp4", asked, true, &to);
            assert!(cut.first == first && cut.last >= asked.1 && cut.last <= asked.1 + 3, "{asked:?}: {cut:?}");
            same_pictures(&dir, "h264_320.mp4", &to, &cut);
        }
        tidy(&dir);
    }

    /// in an open group (x265's) the pictures stored after a key frame but shown before it belong to the group before:
    /// a copy from that key frame leaves them out, and every frame it keeps decodes. key frames are shown at 24 and 48
    #[test]
    fn an_open_group_is_cut_at_its_key_frame() {
        let dir = scratch("open", &["hevc_open_320.mp4"]);
        let cut = copied(&dir, "hevc_open_320.mp4", (30, 40), true, "hevc_open_320_trim.mp4");
        assert_eq!(cut.first, 24);
        same_pictures(&dir, "hevc_open_320.mp4", "hevc_open_320_trim.mp4", &cut);
        tidy(&dir);
    }

    /// what an edit list hides stays hidden: the frames before the clip's first are stored and not shown. a cut further
    /// in starts at its key frame, as any other
    #[test]
    fn what_an_edit_list_hides_stays_hidden() {
        let dir = scratch("hidden", &["h264_cut_320.mp4"]);
        let cut = copied(&dir, "h264_cut_320.mp4", (0, 17), true, "whole.mp4");
        assert_eq!((cut.first, cut.last), (0, 17));
        same_pictures(&dir, "h264_cut_320.mp4", "whole.mp4", &cut);
        let cut = copied(&dir, "h264_cut_320.mp4", (5, 9), true, "part.mp4");
        assert_eq!(cut.first, 2);
        same_pictures(&dir, "h264_cut_320.mp4", "part.mp4", &cut);
        tidy(&dir);
    }

    /// files that list no frames up front, whose frames are counted: matroska, and a fragmented mp4
    #[test]
    fn files_that_list_no_frames_are_cut_too() {
        let dir = scratch("counted", &["h264_64.mkv", "h264_frag_64.mp4"]);
        for (from, to) in [("h264_64.mkv", "h264_64_trim.mkv"), ("h264_frag_64.mp4", "h264_frag_64_trim.mp4")] {
            let cut = copied(&dir, from, (5, 9), true, to);
            assert_eq!(cut.first, 4, "{from}");
            same_pictures(&dir, from, to, &cut);
        }
        tidy(&dir);
    }

    /// the copy's first frame is called by its own timecode, drop-frame kept drop-frame; the sound is kept, or left out.
    /// av_tc_64.mov is 29.97 with a drop-frame timecode from 01:00:00;00, key frames every 4, and two tracks of sound
    #[test]
    fn a_copy_has_its_first_frames_timecode_and_its_sound_or_none() {
        let dir = scratch("timecode", &["av_tc_64.mov"]);
        let cut = copied(&dir, "av_tc_64.mov", (5, 9), true, "av_tc_64_trim.mov");
        assert_eq!(cut.first, 4);
        let clip = Clip::open(&at(&dir, "av_tc_64_trim.mov")).unwrap();
        assert_eq!(clip.clock, Some(Timecode { start: 107_892 + 4, base: 30, drop: true, rate: 30 }));
        assert_eq!(clip.clock.unwrap().at(0), "01:00:00;04");
        assert_eq!(sound_streams(&Input::open(&at(&dir, "av_tc_64_trim.mov")).unwrap()).len(), 2);
        copied(&dir, "av_tc_64.mov", (5, 9), false, "av_tc_64_trim_nosound.mov");
        assert!(sound_streams(&Input::open(&at(&dir, "av_tc_64_trim_nosound.mov")).unwrap()).is_empty());
        // the whole clip keeps its own start
        copied(&dir, "av_tc_64.mov", (0, 11), false, "av_tc_64_nosound.mov");
        assert_eq!(Clip::open(&at(&dir, "av_tc_64_nosound.mov")).unwrap().clock.unwrap().at(0), "01:00:00;00");
        tidy(&dir);
    }

    /// an mp4 with a timecode track: copied as a stream, its track is "codec none" there and stops the copy. it is
    /// made again from the timecode. the mp4 here is av_tc_64.mov's picture and timecode as an mp4
    #[test]
    fn an_mp4_with_a_timecode_track_is_copied() {
        let dir = scratch("mp4-timecode", &["av_tc_64.mov"]);
        copied(&dir, "av_tc_64.mov", (0, 11), false, "av_tc_64.mp4");
        let input = Input::open(&at(&dir, "av_tc_64.mp4")).unwrap();
        assert!(input.timecode_base().is_some(), "the mp4 has no timecode track");
        for sound in [true, false] {
            let to = format!("av_tc_64_trim_{sound}.mp4");
            copied(&dir, "av_tc_64.mp4", (5, 9), sound, &to);
            assert_eq!(Clip::open(&at(&dir, &to)).unwrap().clock.unwrap().at(0), "01:00:00;04");
        }
        tidy(&dir);
    }

    /// a copy is turned as its source asks, and keeps the color tags the container states, and states none it does not
    #[test]
    fn a_copy_keeps_its_turn_and_its_color_tags() {
        let dir = scratch("tags", &["h264_320_turn90.mp4", "h264_709_64.mp4", "h264_vui709_64.mp4"]);
        copied(&dir, "h264_320_turn90.mp4", (9, 12), true, "turned.mp4");
        assert_eq!(Clip::open(&at(&dir, "turned.mp4")).unwrap().turn, Look { turns: 1, ..Look::default() });
        for name in ["h264_709_64.mp4", "h264_vui709_64.mp4"] {
            let to = format!("copy_{name}");
            copied(&dir, name, (0, 1), true, &to);
            let color = |path: &str| {
                let input = Input::open(path).unwrap();
                input.color(input.first_video().unwrap().index)
            };
            assert_eq!(color(&at(&dir, &to)), color(&at(&dir, name)), "{name}");
        }
        tidy(&dir);
    }

    /// the source's creation time is the copy's. none of the test clips has one, so a copy of one is given it in its
    /// movie header: seconds from 1904
    #[test]
    fn a_copy_keeps_its_creation_time() {
        let dir = scratch("created", &["prores_64.mov"]);
        let path = dir.join("prores_64.mov");
        let mut bytes = std::fs::read(&path).unwrap();
        let mvhd = bytes.windows(4).position(|w| w == b"mvhd").expect("a movie header");
        assert_eq!(bytes[mvhd + 4], 0, "a version 0 header");
        // 2026-11-26 00:00:00 utc
        bytes[mvhd + 8..mvhd + 12].copy_from_slice(&(1_795_651_200u32 + 2_082_844_800).to_be_bytes());
        std::fs::write(&path, bytes).unwrap();
        let created = |p: &str| Input::open(p).unwrap().tag(None, "creation_time");
        assert_eq!(created(&at(&dir, "prores_64.mov")).as_deref(), Some("2026-11-26T00:00:00.000000Z"));
        copied(&dir, "prores_64.mov", (1, 3), true, "prores_64_trim.mov");
        assert_eq!(created(&at(&dir, "prores_64_trim.mov")), created(&at(&dir, "prores_64.mov")));
        tidy(&dir);
    }

    /// every kind of clip the player plays copies, frame for frame: hap's three (its blocks as stored), vp9 in webm,
    /// frames of two lengths, 10-bit hevc, and clips with sound in mov and mkv
    #[test]
    fn every_kind_the_player_plays_is_copied() {
        let kinds = [("hap_64.mov", (1, 2)), ("hapq_64.mov", (0, 1)), ("hapalpha_64.mov", (1, 1)), ("vp9_64.webm", (5, 9)), ("h264_vfr_64.mp4", (5, 9)), ("hevc10_320.mp4", (10, 14)), ("tone48_stereo.mov", (6, 17)), ("tone48_stereo.mkv", (6, 17)), ("tone44_mono.mov", (6, 17))];
        let dir = scratch("kinds", &kinds.map(|(name, _)| name));
        for (name, asked) in kinds {
            let to = format!("trim_{name}");
            let cut = copied(&dir, name, asked, true, &to);
            assert!(cut.first <= asked.0 && cut.last >= asked.1, "{name}: {cut:?}");
            same_pictures(&dir, name, &to, &cut);
        }
        tidy(&dir);
    }

    /// where the sound kept with a trim is heard from and to, in seconds from the copy's first frame, and the longest of
    /// its packets: what the file hides (a packet an edit list leaves out, the samples a codec's delay skips) is not heard.
    /// a packet that does not say how long it is lasts until the next
    fn sound_span(path: &str) -> (f64, f64, f64) {
        let input = Input::open(path).unwrap();
        let stream = sound_streams(&input)[0];
        let (tb, rate) = unsafe {
            let st = *(*input.ctx).streams.add(stream);
            (seconds(1, (*st).time_base), (*(*st).codecpar).sample_rate as f64)
        };
        // each packet: when it starts, how long it says it is, and how much of it is heard from its start
        let mut packets: Vec<(f64, f64, f64)> = Vec::new();
        let mut packet = Packet::new();
        while read(&input, &mut packet).unwrap() {
            if packet.stream() == stream && !packet.hidden() {
                let skipped = unsafe {
                    let mut size = 0usize;
                    let side = ffi::av_packet_get_side_data(packet.0, ffi::AVPacketSideDataType::AV_PKT_DATA_SKIP_SAMPLES, &mut size);
                    if side.is_null() || size < 4 { 0 } else { u32::from_le_bytes(std::ptr::read_unaligned(side as *const [u8; 4])) }
                };
                packets.push((packet.time() as f64 * tb, unsafe { (*packet.0).duration } as f64 * tb, skipped as f64 / rate));
            }
        }
        let length = |i: usize| match (packets[i].1, packets.get(i + 1)) {
            (said, _) if said > 0.0 => said,
            (_, Some(next)) => next.0 - packets[i].0,
            _ => packets[i].0 - packets[i - 1].0,
        };
        let last = packets.len() - 1;
        (packets[0].0 + packets[0].2, packets[last].0 + length(last), (0..packets.len()).map(length).fold(0.0, f64::max))
    }

    /// the sound kept with a trim lasts as long as its picture, to within one of its packets at either end (packet for
    /// packet: it is not cut inside a packet). these clips are 24 frames a second
    #[test]
    fn the_sound_kept_lasts_as_long_as_its_picture() {
        let names = ["tone48_stereo.mov", "tone48_stereo.mkv", "tone44_mono.mov"];
        let dir = scratch("sound", &names);
        for name in names {
            let to = format!("trim_{name}");
            let picture = copied(&dir, name, (6, 17), true, &to).frames() as f64 / 24.0;
            let (start, end, packet) = sound_span(&at(&dir, &to));
            println!("{name}: sound from {start:.4} s to {end:.4} s of a picture from 0 to {picture:.4} s, in packets of up to {packet:.4} s");
            assert!(start.abs() <= packet + 1e-6 && (end - picture).abs() <= packet + 1e-6, "{name}: sound from {start} to {end}, picture to {picture}, packets of {packet}");
        }
        tidy(&dir);
    }

    /// how long a copy takes, of each clip in EYEPIECE_COPY (separated by `;`): the whole of it without sound, then its
    /// middle third with sound, each written beside it and removed after:
    /// `cargo test --release copies_at_speed -- --ignored --nocapture`. name clips in a scratch folder: copies are
    /// written beside them
    #[test]
    #[ignore]
    fn copies_at_speed() {
        for path in std::env::var("EYEPIECE_COPY").expect("EYEPIECE_COPY names the clips").split(';') {
            let frames = Source::open(path, &AtomicBool::new(false)).unwrap().times.len();
            for (asked, sound, kept) in [((0, frames - 1), false, "nosound"), ((frames / 3, 2 * frames / 3), true, "trim")] {
                let ext = Path::new(path).extension().unwrap().to_string_lossy().into_owned();
                let to = format!("{path}.speed_{kept}.{ext}");
                let out = std::fs::OpenOptions::new().write(true).create_new(true).open(&to).unwrap();
                let started = std::time::Instant::now();
                let cut = copy(path, asked, sound, out, &to, &AtomicU64::new(0), &AtomicBool::new(false)).unwrap();
                let took = started.elapsed();
                let checked = check(&to, path, &cut, sound, &AtomicBool::new(false)).unwrap();
                println!("{path}: {kept}, {} frames, {} MB, in {} ms; {}", cut.frames(), std::fs::metadata(&to).unwrap().len() >> 20, took.as_millis(), checked.unwrap_or_else(|| "as asked".into()));
                std::fs::remove_file(&to).unwrap();
            }
        }
    }

    /// the check says what differs: here a copy held to a cut it was not made from
    #[test]
    fn the_check_says_what_differs() {
        let dir = scratch("differs", &["av_tc_64.mov"]);
        let cut = copied(&dir, "av_tc_64.mov", (4, 7), true, "four.mov");
        let stop = AtomicBool::new(false);
        let other = Cut { last: 8, ..cut.clone() };
        assert_eq!(check(&at(&dir, "four.mov"), &at(&dir, "av_tc_64.mov"), &other, true, &stop), Ok(Some("4 frames, 5 asked".to_string())));
        let later = Cut { first: 5, last: 8, ..cut.clone() };
        assert_eq!(check(&at(&dir, "four.mov"), &at(&dir, "av_tc_64.mov"), &later, false, &stop), Ok(Some("timecode 01:00:00;04, 01:00:00;05 asked, 2 sound tracks, 0 asked".to_string())));
        tidy(&dir);
    }

    /// the walk's end with frames stored out of order: shown 0 3 1 2 6 4 5 as stored, the end after 4 is after 5, where
    /// every frame up to 6 is stored; with none out of order it is the frame itself
    #[test]
    fn a_walk_ends_where_the_frames_stored_are_whole() {
        let ends = |shown: &[i64], last: i64| {
            let mut walk = Walk { last, stored: 0, latest: i64::MIN };
            shown.iter().position(|&f| walk.take(f, false)).map(|at| (at, walk.latest))
        };
        let stored = [0, 3, 1, 2, 6, 4, 5, 9, 7, 8];
        assert_eq!(ends(&stored, 4), Some((6, 6)));
        assert_eq!(ends(&stored, 3), Some((3, 3)));
        assert_eq!(ends(&stored, 6), Some((6, 6)));
        assert_eq!(ends(&[0, 1, 2, 3], 2), Some((2, 2)));
        // what an edit list hides counts for nothing
        let mut walk = Walk { last: 0, stored: 0, latest: i64::MIN };
        assert!(!walk.take(-1, true) && walk.take(0, false));
    }
}
