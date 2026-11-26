//! Built-in video player: MP4/MOV/MKV with H.264, OpenH264 decoder (built into the exe, nothing to download).
//! Audio plays as usual (rodio) and the video follows the audio position.
//! Decoding runs on a separate thread; only a few frames are kept in memory.
//! Subtitles: from MKV (ASS/SRT, collected while reading the file) or an external .ass/.srt.
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use openh264::decoder::{Decoder, DecoderConfig, Flush};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wavy_core::media::{Cue, SubSrc};

/// Subtitles the video thread collects as it goes (shared with the UI).
pub type Subs = Arc<Mutex<Vec<Cue>>>;

struct Frame {
    pts: f64,
    gen: u32,
    size: [usize; 2],
    rgba: Vec<u8>,
}

enum Ctrl {
    Seek(f64, u32),
}

pub struct Video {
    frames: Receiver<Result<Frame, String>>,
    ctrl: Sender<Ctrl>,
    queue: VecDeque<Frame>,
    gen: u32,
    shown_pts: Option<f64>,
    /// after a seek, show the very first ready frame (even when paused)
    fresh: bool,
    /// where we were last asked to seek (or where we opened from) and when
    target: (f64, std::time::Instant),
    pub texture: Option<TextureHandle>,
    pub error: Option<String>,
    pub subs: Subs,
    /// the displayed frame (RGBA) — for screenshots; it's the same buffer, no extra copy
    last: Option<([usize; 2], Vec<u8>)>,
}

impl Video {
    /// `sub` — which subtitles to show (embedded MKV track or external file).
    /// `ffmpeg` — for formats OpenH264 can't handle (HEVC, 10-bit, VP9, AV1…); without it — the `ffmpeg::NEED` error.
    pub fn open(path: PathBuf, start: f64, ctx: &egui::Context, sub: Option<SubSrc>, ffmpeg: Option<PathBuf>) -> Self {
        let (ftx, frx) = sync_channel(3);
        let (ctl_tx, ctl_rx) = channel();
        let c = ctx.clone();
        let subs: Subs = Arc::new(Mutex::new(Vec::new()));
        let s2 = subs.clone();
        std::thread::Builder::new()
            .name("video".into())
            .spawn(move || {
                let embedded = match sub {
                    Some(SubSrc::External(p)) => { *s2.lock().unwrap() = wavy_core::media::load_sub_file(&p); None }
                    Some(SubSrc::Embedded(n)) => Some(n),
                    None => None,
                };
                let mut r = if wavy_core::media::is_mkv(&path) { decode_mkv(&path, start, &ftx, &ctl_rx, &c, embedded, &s2) } else { decode_loop(&path, start, &ftx, &ctl_rx, &c) };
                if let (Err(e), Some(ff)) = (&r, &ffmpeg) {
                    if e.starts_with(wavy_core::ffmpeg::NEED) {
                        // the built-in decoder can't do it — play through ffmpeg; MKV subtitles are collected in a separate pass
                        if let (Some(n), true) = (embedded, wavy_core::media::is_mkv(&path)) {
                            let (p2, s3, c2) = (path.clone(), s2.clone(), c.clone());
                            std::thread::Builder::new().name("subs".into()).spawn(move || scan_subs(&p2, n, &s3, &c2)).ok();
                        }
                        r = decode_ffmpeg(ff, &path, start, &ftx, &ctl_rx, &c);
                    }
                }
                if let Err(e) = r {
                    wavy_core::log(&format!("видео {}: {e}", path.display()));
                    let _ = ftx.send(Err(e));
                    c.request_repaint();
                }
            })
            .ok();
        Self { frames: frx, ctrl: ctl_tx, queue: VecDeque::new(), gen: 0, shown_pts: None, fresh: false, texture: None, error: None, subs, target: (start, std::time::Instant::now()), last: None }
    }

    /// Time of the displayed frame (for checks in examples/).
    #[allow(dead_code)]
    pub fn shown(&self) -> Option<f64> { self.shown_pts }

    /// Current frame: size and RGBA pixels.
    pub fn frame(&self) -> Option<([usize; 2], &[u8])> { self.last.as_ref().map(|(s, p)| (*s, p.as_slice())) }

    /// Subtitles that should be on screen at time `t`.
    pub fn subs_at(&self, t: f64) -> Vec<Cue> {
        let s = self.subs.lock().unwrap();
        let mut v: Vec<Cue> = s.iter().filter(|c| c.start <= t && t < c.end).cloned().collect();
        v.sort_by(|a, b| a.start.total_cmp(&b.start));
        v
    }

    /// Seek: the decoder starts from the nearest keyframe.
    pub fn seek(&mut self, t: f64) {
        self.gen += 1;
        self.queue.clear();
        // otherwise the old (later) frame looks like "video ran ahead" → endless seeking
        self.shown_pts = None;
        self.fresh = true;
        self.target = (t.max(0.0), std::time::Instant::now());
        let _ = self.ctrl.send(Ctrl::Seek(t.max(0.0), self.gen));
    }

    /// Call every UI frame with the current audio position.
    pub fn update(&mut self, clock: f64, ctx: &egui::Context) {
        loop {
            if self.queue.len() >= 3 { break; }
            match self.frames.try_recv() {
                Ok(Ok(f)) if f.gen == self.gen => self.queue.push_back(f),
                Ok(Ok(_)) => continue, // a frame from before the seek
                Ok(Err(e)) => { self.error = Some(e); break; }
                Err(_) => break,
            }
        }
        // opened at the wrong place (e.g. at the previous episode's old position) and there are no frames — catch up with the audio
        let lost = self.queue.is_empty() && (self.shown_pts.is_none() || self.fresh) && (clock - self.target.0).abs() > 1.0 && self.target.1.elapsed() > Duration::from_millis(400);
        // audio went back (or far forward) from what we have — seek the video
        let behind = lost || self.shown_pts.is_some_and(|p| p > clock + 1.0);
        let far_ahead = self.queue.back().is_some_and(|f| f.pts + 2.0 < clock);
        if behind || far_ahead {
            self.seek(clock);
            return;
        }
        // the newest frame whose time has already come
        let mut pick = None;
        while self.queue.front().is_some_and(|f| f.pts <= clock + 0.015) { pick = self.queue.pop_front(); }
        // before playback starts, show the first frame so the screen isn't black
        if pick.is_none() && (self.texture.is_none() || self.fresh) { pick = self.queue.pop_front(); }
        if let Some(f) = pick {
            let img = ColorImage::from_rgba_unmultiplied(f.size, &f.rgba);
            match &mut self.texture {
                Some(t) if t.size() == f.size => t.set(img, TextureOptions::LINEAR),
                _ => self.texture = Some(ctx.load_texture("video", img, TextureOptions::LINEAR)),
            }
            self.shown_pts = Some(f.pts);
            self.fresh = false;
            self.last = Some((f.size, f.rgba));
        }
    }
}

enum Flow {
    Go,
    Seek(f64, u32),
    Stop,
}

fn check_ctrl(ctrl: &Receiver<Ctrl>) -> Flow {
    let mut last = None;
    loop {
        match ctrl.try_recv() {
            Ok(Ctrl::Seek(t, g)) => last = Some((t, g)),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => return Flow::Stop,
        }
    }
    match last { Some((t, g)) => Flow::Seek(t, g), None => Flow::Go }
}

fn decode_loop(path: &Path, start: f64, out: &SyncSender<Result<Frame, String>>, ctrl: &Receiver<Ctrl>, ctx: &egui::Context) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| lf!("не удалось открыть видео: {e}", "couldn't open the video: {e}"))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    // couldn't parse the MP4 (e.g. HEVC in hvc1) — let ffmpeg try
    let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), size).map_err(|e| format!("{}MP4: {e}", wavy_core::ffmpeg::NEED))?;
    let (track_id, timescale, nal_len, params) = {
        let t = mp4.tracks().values()
            .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H264)))
            .ok_or_else(|| lf!("{}видео не в H.264", "{}the video isn't H.264", wavy_core::ffmpeg::NEED))?;
        let avcc = &t.trak.mdia.minf.stbl.stsd.avc1.as_ref().ok_or(l!("нет описания H.264", "no H.264 description"))?.avcc;
        if let Some(p) = unsupported_profile(avcc.avc_profile_indication) { return Err(lf!("{}видео в {p}", "{}the video is {p}", wavy_core::ffmpeg::NEED)); }
        let mut params = Vec::new();
        for nal in avcc.sequence_parameter_sets.iter().chain(avcc.picture_parameter_sets.iter()) {
            params.extend_from_slice(&[0, 0, 0, 1]);
            params.extend_from_slice(&nal.bytes);
        }
        (t.track_id(), t.timescale().max(1) as f64, (avcc.length_size_minus_one & 3) as usize + 1, params)
    };
    let count = mp4.sample_count(track_id).map_err(|e| e.to_string())?;

    // frame index: presentation time and "is keyframe" — needed for seeking
    // for fragmented MP4 (as YouTube serves it) the library computes frame times and positions
    // in the file wrong — we compute them ourselves and read frames directly
    let mut locs: Option<(std::fs::File, Vec<(u64, u32)>)> = None;
    let mut index: Vec<(f64, bool)> = if mp4.is_fragmented() {
        let moofs = moof_offsets(path).map_err(|e| lf!("не удалось разобрать MP4: {e}", "couldn't parse the MP4: {e}"))?;
        let (idx, l) = fragmented_index(&mp4, track_id, timescale, &moofs);
        locs = Some((std::fs::File::open(path).map_err(|e| e.to_string())?, l));
        idx
    } else {
        let mut v = Vec::with_capacity(count as usize);
        for i in 1..=count {
            match mp4.read_sample(track_id, i) {
                Ok(Some(s)) => v.push(((s.start_time as i64 + s.rendering_offset as i64) as f64 / timescale, s.is_sync)),
                _ => v.push((v.last().map(|l: &(f64, bool)| l.0).unwrap_or(0.0), false)),
            }
        }
        v
    };
    index.resize(count as usize, (index.last().map(|l| l.0).unwrap_or(0.0), false));
    if !index.iter().any(|x| x.1) { if let Some(first) = index.first_mut() { first.1 = true; } }
    // the first frame is zero (MP4 can have an offset because of B-frames)
    let t0 = index.iter().take(32).map(|x| x.0).fold(f64::INFINITY, f64::min);
    let t0 = if t0.is_finite() { t0 } else { 0.0 };

    let (mut target, mut gen) = (start, 0u32);
    let mut buf = Vec::new();
    let mut logged_err = false;
    'outer: loop {
        let k = index.iter().enumerate().filter(|(_, (t, s))| *s && *t - t0 <= target + 0.001).map(|(i, _)| i).last().unwrap_or(0);
        let mut dec = Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new().flush_after_decode(Flush::NoFlush))
            .map_err(|e| lf!("декодер H.264: {e}", "H.264 decoder: {e}"))?;
        let mut pending: BinaryHeap<Reverse<i64>> = BinaryHeap::new(); // frames come out in presentation order
        let to_key = |t: f64| (t * 1_000_000.0) as i64;

        for i in k..count as usize {
            match check_ctrl(ctrl) {
                Flow::Seek(t, g) => { target = t; gen = g; continue 'outer; }
                Flow::Stop => return Ok(()),
                Flow::Go => {}
            }
            let sample: Vec<u8>;
            let data: &[u8] = match &mut locs {
                Some((f, l)) => {
                    use std::io::{Read, Seek, SeekFrom};
                    let Some(&(off, size)) = l.get(i) else { continue };
                    let mut v = vec![0u8; size as usize];
                    if f.seek(SeekFrom::Start(off)).and_then(|_| f.read_exact(&mut v)).is_err() { continue; }
                    sample = v;
                    &sample
                }
                None => {
                    let Ok(Some(s)) = mp4.read_sample(track_id, i as u32 + 1) else { continue };
                    sample = s.bytes.to_vec();
                    &sample
                }
            };
            buf.clear();
            if i == k || index[i].1 { buf.extend_from_slice(&params); }
            let mut p = 0;
            while p + nal_len <= data.len() {
                let n = data[p..p + nal_len].iter().fold(0usize, |a, b| (a << 8) | *b as usize);
                p += nal_len;
                if p + n > data.len() { break; }
                buf.extend_from_slice(&[0, 0, 0, 1]);
                buf.extend_from_slice(&data[p..p + n]);
                p += n;
            }
            let pts = index[i].0 - t0;
            match dec.decode(&buf) {
                Ok(img) => {
                    pending.push(Reverse(to_key(pts)));
                    if let Some(img) = img {
                        let Some(Reverse(key)) = pending.pop() else { continue };
                        match emit(&img, key as f64 / 1_000_000.0, target, gen, out, ctrl, ctx) {
                            Flow::Go => {}
                            Flow::Seek(t, g) => { target = t; gen = g; continue 'outer; }
                            Flow::Stop => return Ok(()),
                        }
                    }
                }
                Err(e) => if !logged_err { logged_err = true; wavy_core::log(&format!("видео: кадр {i} не декодировался: {e}")); },
            }
        }
        // the tail of the decoder buffer
        if let Ok(rest) = dec.flush_remaining() {
            for img in rest {
                let Some(Reverse(key)) = pending.pop() else { break };
                match emit(&img, key as f64 / 1_000_000.0, target, gen, out, ctrl, ctx) {
                    Flow::Go => {}
                    Flow::Seek(t, g) => { target = t; gen = g; continue 'outer; }
                    Flow::Stop => return Ok(()),
                }
            }
        }
        // end of video: wait for a seek or close
        match ctrl.recv() {
            Ok(Ctrl::Seek(t, g)) => { target = t; gen = g; }
            Err(_) => return Ok(()),
        }
    }
}

/// Positions of moof boxes in the file (MP4 top level).
fn moof_offsets(path: &Path) -> std::io::Result<Vec<u64>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let (mut pos, mut out) = (0u64, Vec::new());
    while pos + 8 <= len {
        f.seek(SeekFrom::Start(pos))?;
        let mut h = [0u8; 16];
        f.read_exact(&mut h[..8])?;
        let mut size = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        if size == 1 { f.read_exact(&mut h[8..16])?; size = u64::from_be_bytes(h[8..16].try_into().unwrap()); }
        if size == 0 { size = len - pos; }
        if size < 8 { break; }
        if &h[4..8] == b"moof" { out.push(pos); }
        pos += size;
    }
    Ok(out)
}

/// Presentation time, "is keyframe" and location in the file (offset, size) for each frame
/// of a fragmented MP4 — from the moof/traf/tfhd/tfdt/trun boxes, as in the ISO BMFF standard.
fn fragmented_index<R>(mp4: &mp4::Mp4Reader<R>, track_id: u32, timescale: f64, moof_pos: &[u64]) -> (Vec<(f64, bool)>, Vec<(u64, u32)>) {
    let trex = mp4.moov.mvex.as_ref().map(|m| &m.trex);
    let (mut idx, mut locs) = (Vec::new(), Vec::new());
    let mut dts: u64 = 0;
    for (moof, &moof_start) in mp4.moofs.iter().zip(moof_pos) {
        let mut prev_end = moof_start;
        for (ti, traf) in moof.trafs.iter().enumerate() {
            let tfhd = &traf.tfhd;
            let base = match tfhd.base_data_offset {
                Some(b) => b,
                None if tfhd.flags & 0x02_0000 != 0 || ti == 0 => moof_start, // default-base-is-moof
                None => prev_end,
            };
            let Some(trun) = &traf.trun else { continue };
            let mut off = match trun.data_offset { Some(d) => (base as i64 + d as i64) as u64, None => base };
            let def_dur = tfhd.default_sample_duration.or(trex.map(|t| t.default_sample_duration)).unwrap_or(0);
            let def_size = tfhd.default_sample_size.or(trex.map(|t| t.default_sample_size)).unwrap_or(0);
            let def_flags = tfhd.default_sample_flags.or(trex.map(|t| t.default_sample_flags)).unwrap_or(0);
            if tfhd.track_id == track_id { if let Some(tfdt) = &traf.tfdt { dts = tfdt.base_media_decode_time; } }
            for j in 0..trun.sample_count as usize {
                let size = trun.sample_sizes.get(j).copied().unwrap_or(def_size);
                if tfhd.track_id == track_id {
                    let dur = trun.sample_durations.get(j).copied().unwrap_or(def_dur) as u64;
                    let cts = trun.sample_cts.get(j).map(|c| if trun.version == 1 { *c as i32 as i64 } else { *c as i64 }).unwrap_or(0);
                    let flags = if j == 0 { trun.first_sample_flags.or(trun.sample_flags.first().copied()).unwrap_or(def_flags) }
                        else { trun.sample_flags.get(j).copied().unwrap_or(def_flags) };
                    idx.push(((dts as i64 + cts) as f64 / timescale, flags & 0x0001_0000 == 0)); // sample_is_non_sync bit
                    locs.push((off, size));
                    dts += dur;
                }
                off += size as u64;
            }
            prev_end = off;
        }
    }
    (idx, locs)
}

/// H.264 profiles OpenH264 can't handle: 10-bit (Hi10P), 4:2:2, 4:4:4.
fn unsupported_profile(profile: u8) -> Option<&'static str> {
    match profile {
        110 => Some(l!("10-битное H.264 (Hi10P)", "10-bit H.264 (Hi10P)")),
        122 | 244 | 44 => Some("H.264 4:2:2/4:4:4"),
        _ => None,
    }
}

/// avcC (CodecPrivate in MKV): NAL length prefix size and SPS/PPS as Annex B.
fn parse_avcc(c: &[u8]) -> Result<(usize, Vec<u8>, u8), String> {
    if c.len() < 7 { return Err(l!("нет описания H.264", "no H.264 description").into()); }
    let profile = c[1];
    let nal_len = (c[4] & 3) as usize + 1;
    let mut params = Vec::new();
    let mut p = 5;
    for round in 0..2 {
        let n = if round == 0 { (c[p] & 0x1f) as usize } else { c[p] as usize };
        p += 1;
        for _ in 0..n {
            if p + 2 > c.len() { break; }
            let len = u16::from_be_bytes([c[p], c[p + 1]]) as usize;
            p += 2;
            if p + len > c.len() { break; }
            params.extend_from_slice(&[0, 0, 0, 1]);
            params.extend_from_slice(&c[p..p + len]);
            p += len;
        }
        if p >= c.len() { break; }
    }
    Ok((nal_len, params, profile))
}

/// Whether the frame contains an IDR (keyframe) — for MKV blocks without the keyframe flag.
fn has_idr(data: &[u8], nal_len: usize) -> bool {
    let mut p = 0;
    while p + nal_len <= data.len() {
        let n = data[p..p + nal_len].iter().fold(0usize, |a, b| (a << 8) | *b as usize);
        p += nal_len;
        if p < data.len() && data[p] & 0x1f == 5 { return true; }
        p += n;
    }
    false
}

/// Length-prefixed NALs → Annex B (what OpenH264 consumes).
fn to_annexb(data: &[u8], nal_len: usize, buf: &mut Vec<u8>) {
    let mut p = 0;
    while p + nal_len <= data.len() {
        let n = data[p..p + nal_len].iter().fold(0usize, |a, b| (a << 8) | *b as usize);
        p += nal_len;
        if p + n > data.len() { break; }
        buf.extend_from_slice(&[0, 0, 0, 1]);
        buf.extend_from_slice(&data[p..p + n]);
        p += n;
    }
}

fn video_codec_error(codec: &str) -> String {
    let name = match codec {
        "V_MPEGH/ISO/HEVC" => "H.265 (HEVC)",
        "V_AV1" => "AV1",
        "V_VP9" => "VP9",
        "V_VP8" => "VP8",
        c => c,
    };
    lf!("{}видео в {name}", "{}the video is {name}", wavy_core::ffmpeg::NEED)
}

/// MKV: H.264 + subtitles. The header (codec, avcC) is read by matroska-demuxer, while packets and seeking
/// come from symphonia (its seeking is reliable). Seeking goes to the nearest keyframe before the target.
fn decode_mkv(path: &Path, start: f64, out: &SyncSender<Result<Frame, String>>, ctrl: &Receiver<Ctrl>, ctx: &egui::Context, sub_track: Option<u64>, subs: &Subs) -> Result<(), String> {
    use matroska_demuxer::{MatroskaFile, TrackType};
    use symphonia::core::{formats::{FormatOptions, SeekMode, SeekTo}, io::MediaSourceStream, meta::MetadataOptions, probe::Hint, units::Time};
    let (vnum, nal_len, params, sub_codec) = {
        let file = std::fs::File::open(path).map_err(|e| lf!("не удалось открыть видео: {e}", "couldn't open the video: {e}"))?;
        let mkv = MatroskaFile::open(BufReader::new(file)).map_err(|e| lf!("не удалось прочитать MKV: {e}", "couldn't read the MKV: {e}"))?;
        let vt = mkv.tracks().iter().find(|t| t.track_type() == TrackType::Video).ok_or(l!("в файле нет видео", "the file has no video"))?;
        let codec = vt.codec_id().to_string();
        if codec != "V_MPEG4/ISO/AVC" { return Err(video_codec_error(&codec)); }
        let (nal_len, params, profile) = parse_avcc(vt.codec_private().unwrap_or(&[]))?;
        if let Some(p) = unsupported_profile(profile) { return Err(lf!("{}видео в {p}", "{}the video is {p}", wavy_core::ffmpeg::NEED)); }
        let sub_codec = sub_track.and_then(|n| mkv.tracks().iter().find(|t| t.track_number().get() == n)).map(|t| t.codec_id().to_string()).unwrap_or_default();
        (vt.track_number().get() as u32, nal_len, params, sub_codec)
    };
    let file = std::fs::File::open(path).map_err(|e| lf!("не удалось открыть видео: {e}", "couldn't open the video: {e}"))?;
    let mut hint = Hint::new();
    hint.with_extension("mkv");
    let mut fmt = symphonia::default::get_probe().format(&hint, MediaSourceStream::new(Box::new(file), Default::default()), &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| lf!("не удалось прочитать MKV: {e}", "couldn't read the MKV: {e}"))?.format;
    let tb = fmt.tracks().iter().find(|t| t.id == vnum).and_then(|t| t.codec_params.time_base).map(|t| t.numer as f64 / t.denom as f64).unwrap_or(0.001);
    let sub_num = sub_track.map(|n| n as u32);
    let push_sub = |p: &symphonia::core::formats::Packet| {
        if Some(p.track_id()) != sub_num { return; }
        let (st, dur) = (p.ts() as f64 * tb, if p.dur() > 0 { p.dur() as f64 * tb } else { 3.0 });
        let cue = if sub_codec.contains("UTF8") { wavy_core::media::mkv_srt_block(&p.data, st, dur) } else { wavy_core::media::mkv_ass_block(&p.data, st, dur) };
        if let Some(c) = cue {
            let mut s = subs.lock().unwrap();
            if !s.iter().any(|x| x.start == c.start && x.text == c.text) { s.push(c); }
        }
    };
    let seek = |fmt: &mut Box<dyn symphonia::core::formats::FormatReader>, sec: f64| {
        let sec = sec.max(0.0);
        fmt.seek(SeekMode::Accurate, SeekTo::Time { time: Time::new(sec as u64, sec.fract()), track_id: Some(vnum) }).is_ok()
    };

    let (mut target, mut gen) = (start, 0u32);
    let mut buf = Vec::new();
    let mut logged_err = false;
    'outer: loop {
        // a keyframe no later than target: look at the window before it (15 s, then wider)
        let mut key: Option<f64> = None;
        if target > 0.3 {
            for back in [15.0, 60.0, 600.0] {
                if !seek(&mut fmt, target - back) { break; }
                while let Ok(p) = fmt.next_packet() {
                    if p.track_id() == vnum {
                        let t = p.ts() as f64 * tb;
                        if t > target + 0.001 { break; }
                        if has_idr(&p.data, nal_len) { key = Some(t); }
                    } else { push_sub(&p); }
                }
                if key.is_some() || target - back <= 0.0 { break; }
            }
        }
        seek(&mut fmt, key.unwrap_or(0.0));
        let mut dec = Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new().flush_after_decode(Flush::NoFlush))
            .map_err(|e| lf!("декодер H.264: {e}", "H.264 decoder: {e}"))?;
        let mut pending: BinaryHeap<Reverse<i64>> = BinaryHeap::new();
        let mut started = false;
        loop {
            match check_ctrl(ctrl) {
                Flow::Seek(t, g) => { target = t; gen = g; continue 'outer; }
                Flow::Stop => return Ok(()),
                Flow::Go => {}
            }
            let p = match fmt.next_packet() { Ok(p) => p, Err(_) => break };
            if p.track_id() != vnum { push_sub(&p); continue; }
            let key = has_idr(&p.data, nal_len);
            if !started { if !key { continue; } started = true; }
            buf.clear();
            if key { buf.extend_from_slice(&params); }
            to_annexb(&p.data, nal_len, &mut buf);
            let pts = p.ts() as f64 * tb;
            match dec.decode(&buf) {
                Ok(img) => {
                    pending.push(Reverse((pts * 1_000_000.0) as i64));
                    if let Some(img) = img {
                        let Some(Reverse(k)) = pending.pop() else { continue };
                        match emit(&img, k as f64 / 1_000_000.0, target, gen, out, ctrl, ctx) {
                            Flow::Go => {}
                            Flow::Seek(t, g) => { target = t; gen = g; continue 'outer; }
                            Flow::Stop => return Ok(()),
                        }
                    }
                }
                Err(e) => if !logged_err { logged_err = true; wavy_core::log(&format!("видео: кадр {pts:.2} с не декодировался: {e}")); },
            }
        }
        if let Ok(rest) = dec.flush_remaining() {
            for img in rest {
                let Some(Reverse(k)) = pending.pop() else { break };
                match emit(&img, k as f64 / 1_000_000.0, target, gen, out, ctrl, ctx) {
                    Flow::Go => {}
                    Flow::Seek(t, g) => { target = t; gen = g; continue 'outer; }
                    Flow::Stop => return Ok(()),
                }
            }
        }
        match ctrl.recv() {
            Ok(Ctrl::Seek(t, g)) => { target = t; gen = g; }
            Err(_) => return Ok(()),
        }
    }
}

/// Subtitles from MKV when the video goes through ffmpeg: one quick pass over the file (subtitle packets only).
fn scan_subs(path: &Path, track: u64, subs: &Subs, ctx: &egui::Context) {
    use symphonia::core::{formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};
    let codec = std::fs::File::open(path).ok().and_then(|f| matroska_demuxer::MatroskaFile::open(BufReader::new(f)).ok())
        .and_then(|m| m.tracks().iter().find(|t| t.track_number().get() == track).map(|t| t.codec_id().to_string())).unwrap_or_default();
    let Ok(file) = std::fs::File::open(path) else { return };
    let mut hint = Hint::new();
    hint.with_extension("mkv");
    let Ok(probed) = symphonia::default::get_probe().format(&hint, MediaSourceStream::new(Box::new(file), Default::default()), &FormatOptions::default(), &MetadataOptions::default()) else { return };
    let mut fmt = probed.format;
    let tb = fmt.tracks().iter().find(|t| t.id == track as u32).and_then(|t| t.codec_params.time_base).map(|t| t.numer as f64 / t.denom as f64).unwrap_or(0.001);
    let mut batch = Vec::new();
    while let Ok(p) = fmt.next_packet() {
        if p.track_id() != track as u32 { continue; }
        let (st, dur) = (p.ts() as f64 * tb, if p.dur() > 0 { p.dur() as f64 * tb } else { 3.0 });
        let cue = if codec.contains("UTF8") { wavy_core::media::mkv_srt_block(&p.data, st, dur) } else { wavy_core::media::mkv_ass_block(&p.data, st, dur) };
        batch.extend(cue);
        if batch.len() >= 50 {
            subs.lock().unwrap().append(&mut batch);
            ctx.request_repaint();
        }
        if Arc::strong_count(subs) == 1 { return; } // the video was closed
    }
    subs.lock().unwrap().append(&mut batch);
}

/// An ffmpeg process that's killed together with the video thread.
struct Child(std::process::Child);
impl Drop for Child { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

/// Video through ffmpeg: RGBA frames over a pipe; frame time and size come from the showinfo filter.
/// Seeking restarts with -ss (ffmpeg decodes from the keyframe to the target itself).
/// -hwaccel auto: if the GPU supports the codec, it decodes (that's what matters on a weak PC).
/// Memory: ffmpeg outputs YUV 4:2:0 (2.7× smaller than RGBA) and decodes on 2 threads — ~100 MB for 1080p HEVC
/// instead of ~350 MB by default; we convert to RGBA ourselves.
fn decode_ffmpeg(ff: &Path, path: &Path, start: f64, out: &SyncSender<Result<Frame, String>>, ctrl: &Receiver<Ctrl>, ctx: &egui::Context) -> Result<(), String> {
    use std::io::{BufRead, Read};
    use std::process::Stdio;
    let (mut target, mut gen) = (start, 0u32);
    let mut ever = false; // at least one frame overall (seeking past the end of the file isn't an error)
    'outer: loop {
        let mut c = wavy_core::ffmpeg::cmd(ff);
        c.args(["-nostdin", "-hide_banner", "-nostats", "-loglevel", "info", "-hwaccel", "auto", "-threads", "2", "-copyts"]);
        if target > 0.3 { c.arg("-ss").arg(format!("{target:.3}")); }
        c.arg("-i").arg(path)
            .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf", "scale=w='min(iw,1920)':h=-2:flags=bilinear,format=yuv420p,showinfo=checksum=0", "-filter_threads", "1", "-f", "rawvideo", "-"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = Child(c.spawn().map_err(|e| lf!("не удалось запустить ffmpeg: {e}", "couldn't start ffmpeg: {e}"))?);
        let mut stdout = std::io::BufReader::with_capacity(1 << 20, child.0.stdout.take().unwrap());
        let stderr = child.0.stderr.take().unwrap();
        let (mtx, mrx) = channel::<(f64, usize, usize)>();
        let tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let t2 = tail.clone();
        std::thread::Builder::new().name("ffmpeg-log".into()).spawn(move || {
            for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                match wavy_core::ffmpeg::parse_showinfo(&line) {
                    Some(m) => { if mtx.send(m).is_err() { break; } }
                    None => { let mut t = t2.lock().unwrap(); t.push_back(line); if t.len() > 8 { t.pop_front(); } }
                }
            }
        }).ok();
        let mut frames = 0usize;
        let mut yuv = Vec::new();
        loop {
            match check_ctrl(ctrl) {
                Flow::Seek(t, g) => { target = t; gen = g; continue 'outer; }
                Flow::Stop => return Ok(()),
                Flow::Go => {}
            }
            let (pts, w, h) = match mrx.recv_timeout(Duration::from_millis(100)) {
                Ok(m) => m,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            yuv.resize(w * h + 2 * (w.div_ceil(2) * h.div_ceil(2)), 0);
            if stdout.read_exact(&mut yuv).is_err() { break; }
            let rgba = yuv420_to_rgba(&yuv, w, h);
            frames += 1;
            ever = true;
            match send(Frame { pts, gen, size: [w, h], rgba }, out, ctrl, ctx) {
                Flow::Go => {}
                Flow::Seek(t, g) => { target = t; gen = g; continue 'outer; }
                Flow::Stop => return Ok(()),
            }
        }
        if frames == 0 && !ever {
            let _ = child.0.wait();
            let t = tail.lock().unwrap().iter().filter(|l| !l.starts_with(' ')).cloned().collect::<Vec<_>>().join(" | ");
            return Err(lf!("ffmpeg не смог показать видео: {t}", "ffmpeg couldn't show the video: {t}"));
        }
        drop(child);
        match ctrl.recv() {
            Ok(Ctrl::Seek(t, g)) => { target = t; gen = g; }
            Err(_) => return Ok(()),
        }
    }
}

/// YUV 4:2:0 (planar) → RGBA. BT.709 matrix for HD, BT.601 for SD, "TV" range 16–235.
fn yuv420_to_rgba(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (yp, rest) = src.split_at(w * h);
    let (up, vp) = rest.split_at(cw * ch);
    // coefficients ×1024
    let (rv, gu, gv, bu) = if h >= 720 { (1613, 192, 479, 1900) } else { (1436, 352, 731, 1815) };
    let mut out = vec![255u8; w * h * 4];
    for row in 0..h {
        let (yr, cr) = (&yp[row * w..row * w + w], row / 2 * cw);
        let o = &mut out[row * w * 4..(row + 1) * w * 4];
        for x in 0..w {
            let y = (yr[x] as i32 - 16) * 1192; // 255/219 ×1024
            let u = up[cr + x / 2] as i32 - 128;
            let v = vp[cr + x / 2] as i32 - 128;
            let r = (y + rv * v * 1192 / 1024) >> 10;
            let g = (y - (gu * u + gv * v) * 1192 / 1024) >> 10;
            let b = (y + bu * u * 1192 / 1024) >> 10;
            let p = &mut o[x * 4..x * 4 + 3];
            p[0] = r.clamp(0, 255) as u8;
            p[1] = g.clamp(0, 255) as u8;
            p[2] = b.clamp(0, 255) as u8;
        }
    }
    out
}

/// Hand a ready frame to the UI without blocking forever (listen for seeks while waiting).
fn send(mut frame: Frame, out: &SyncSender<Result<Frame, String>>, ctrl: &Receiver<Ctrl>, ctx: &egui::Context) -> Flow {
    loop {
        match out.try_send(Ok(frame)) {
            Ok(()) => { ctx.request_repaint(); return Flow::Go; }
            Err(TrySendError::Disconnected(_)) => return Flow::Stop,
            Err(TrySendError::Full(Ok(f))) => {
                frame = f;
                match check_ctrl(ctrl) {
                    Flow::Go => std::thread::sleep(Duration::from_millis(8)),
                    other => return other,
                }
            }
            Err(TrySendError::Full(Err(_))) => return Flow::Go,
        }
    }
}

fn emit(img: &openh264::decoder::DecodedYUV<'_>, pts: f64, target: f64, gen: u32, out: &SyncSender<Result<Frame, String>>, ctrl: &Receiver<Ctrl>, ctx: &egui::Context) -> Flow {
    if pts + 0.05 < target { return Flow::Go; } // catching up to the seek point — frames before it aren't needed
    let (w, h) = img.dimensions();
    let mut rgba = vec![0u8; w * h * 4];
    img.write_rgba8(&mut rgba);
    let mut frame = Frame { pts, gen, size: [w, h], rgba };
    loop {
        match out.try_send(Ok(frame)) {
            Ok(()) => { ctx.request_repaint(); return Flow::Go; }
            Err(TrySendError::Disconnected(_)) => return Flow::Stop,
            Err(TrySendError::Full(Ok(f))) => {
                frame = f;
                match check_ctrl(ctrl) {
                    Flow::Go => std::thread::sleep(Duration::from_millis(8)),
                    other => return other,
                }
            }
            Err(TrySendError::Full(Err(_))) => return Flow::Go,
        }
    }
}
