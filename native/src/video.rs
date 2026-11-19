//! Встроенный видеоплеер: MP4/MOV с H.264, декодер OpenH264 (вшит в exe, ничего не докачивается).
//! Звук играет как обычно (rodio), а видео подстраивается под позицию звука.
//! Декодирование — в отдельном потоке, в памяти держим всего несколько кадров.
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use openh264::decoder::{Decoder, DecoderConfig, Flush};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::time::Duration;

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
    pub texture: Option<TextureHandle>,
    pub error: Option<String>,
}

impl Video {
    pub fn open(path: PathBuf, start: f64, ctx: &egui::Context) -> Self {
        let (ftx, frx) = sync_channel(3);
        let (ctl_tx, ctl_rx) = channel();
        let c = ctx.clone();
        std::thread::Builder::new()
            .name("video".into())
            .spawn(move || {
                if let Err(e) = decode_loop(&path, start, &ftx, &ctl_rx, &c) {
                    wavy_core::log(&format!("видео {}: {e}", path.display()));
                    let _ = ftx.send(Err(e));
                    c.request_repaint();
                }
            })
            .ok();
        Self { frames: frx, ctrl: ctl_tx, queue: VecDeque::new(), gen: 0, shown_pts: None, texture: None, error: None }
    }

    /// Перемотка: декодер начинает с ближайшего ключевого кадра.
    pub fn seek(&mut self, t: f64) {
        self.gen += 1;
        self.queue.clear();
        let _ = self.ctrl.send(Ctrl::Seek(t.max(0.0), self.gen));
    }

    /// Вызывать каждый кадр интерфейса с текущей позицией звука.
    pub fn update(&mut self, clock: f64, ctx: &egui::Context) {
        loop {
            if self.queue.len() >= 3 { break; }
            match self.frames.try_recv() {
                Ok(Ok(f)) if f.gen == self.gen => self.queue.push_back(f),
                Ok(Ok(_)) => continue, // кадр из до-перемоточной эпохи
                Ok(Err(e)) => { self.error = Some(e); break; }
                Err(_) => break,
            }
        }
        // звук ушёл назад (или сильно вперёд) от того, что у нас есть — перематываем видео
        let behind = self.shown_pts.is_some_and(|p| p > clock + 1.0);
        let far_ahead = self.queue.back().is_some_and(|f| f.pts + 2.0 < clock);
        if behind || far_ahead {
            self.seek(clock);
            return;
        }
        // самый свежий кадр, время которого уже наступило
        let mut pick = None;
        while self.queue.front().is_some_and(|f| f.pts <= clock + 0.015) { pick = self.queue.pop_front(); }
        // до старта воспроизведения показываем первый кадр, чтобы не было чёрного экрана
        if pick.is_none() && self.texture.is_none() { pick = self.queue.pop_front(); }
        if let Some(f) = pick {
            let img = ColorImage::from_rgba_unmultiplied(f.size, &f.rgba);
            match &mut self.texture {
                Some(t) if t.size() == f.size => t.set(img, TextureOptions::LINEAR),
                _ => self.texture = Some(ctx.load_texture("video", img, TextureOptions::LINEAR)),
            }
            self.shown_pts = Some(f.pts);
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
    let file = std::fs::File::open(path).map_err(|e| format!("не удалось открыть видео: {e}"))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), size).map_err(|e| format!("не удалось прочитать MP4: {e}"))?;
    let (track_id, timescale, nal_len, params) = {
        let t = mp4.tracks().values()
            .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H264)))
            .ok_or("Видео не в формате H.264 — такое пока не поддерживается (нужен MP4/MOV с H.264)")?;
        let avcc = &t.trak.mdia.minf.stbl.stsd.avc1.as_ref().ok_or("нет описания H.264")?.avcc;
        let mut params = Vec::new();
        for nal in avcc.sequence_parameter_sets.iter().chain(avcc.picture_parameter_sets.iter()) {
            params.extend_from_slice(&[0, 0, 0, 1]);
            params.extend_from_slice(&nal.bytes);
        }
        (t.track_id(), t.timescale().max(1) as f64, (avcc.length_size_minus_one & 3) as usize + 1, params)
    };
    let count = mp4.sample_count(track_id).map_err(|e| e.to_string())?;

    // индекс кадров: время показа и «ключевой ли» — нужен для перемотки
    // для фрагментированного MP4 (так отдаёт YouTube) библиотека неверно считает время и место кадров
    // в файле — считаем сами и читаем кадры напрямую
    let mut locs: Option<(std::fs::File, Vec<(u64, u32)>)> = None;
    let mut index: Vec<(f64, bool)> = if mp4.is_fragmented() {
        let moofs = moof_offsets(path).map_err(|e| format!("не удалось разобрать MP4: {e}"))?;
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
    // первый кадр — это ноль (в MP4 бывает сдвиг из-за B-кадров)
    let t0 = index.iter().take(32).map(|x| x.0).fold(f64::INFINITY, f64::min);
    let t0 = if t0.is_finite() { t0 } else { 0.0 };

    let (mut target, mut gen) = (start, 0u32);
    let mut buf = Vec::new();
    let mut logged_err = false;
    'outer: loop {
        let k = index.iter().enumerate().filter(|(_, (t, s))| *s && *t - t0 <= target + 0.001).map(|(i, _)| i).last().unwrap_or(0);
        let mut dec = Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new().flush_after_decode(Flush::NoFlush))
            .map_err(|e| format!("декодер H.264: {e}"))?;
        let mut pending: BinaryHeap<Reverse<i64>> = BinaryHeap::new(); // кадры выходят в порядке показа
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
        // хвост буфера декодера
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
        // конец видео: ждём перемотку или закрытие
        match ctrl.recv() {
            Ok(Ctrl::Seek(t, g)) => { target = t; gen = g; }
            Err(_) => return Ok(()),
        }
    }
}

/// Позиции блоков moof в файле (верхний уровень MP4).
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

/// Время показа, «ключевой ли» и место в файле (смещение, размер) для каждого кадра
/// фрагментированного MP4 — по блокам moof/traf/tfhd/tfdt/trun, как в стандарте ISO BMFF.
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
                    idx.push(((dts as i64 + cts) as f64 / timescale, flags & 0x0001_0000 == 0)); // бит sample_is_non_sync
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

fn emit(img: &openh264::decoder::DecodedYUV<'_>, pts: f64, target: f64, gen: u32, out: &SyncSender<Result<Frame, String>>, ctrl: &Receiver<Ctrl>, ctx: &egui::Context) -> Flow {
    if pts + 0.05 < target { return Flow::Go; } // догоняем точку перемотки — кадры до неё не нужны
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
