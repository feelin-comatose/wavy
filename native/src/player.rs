//! Аудио на rodio (symphonia): mp3, flac, wav, ogg/vorbis, m4a/aac, alac.
use std::path::Path;
use std::time::Duration;

pub struct Audio {
    _sink: Option<rodio::MixerDeviceSink>,
    player: Option<rodio::Player>,
    pub duration: f64,
    pub loaded: bool,
    pub error: Option<String>,
    /// играем из потока, который ещё догружается: (загрузка, отложенная перемотка)
    stream: Option<(wavy_core::stream::Download, Option<f64>)>,
    volume: f32,
    /// перемотка в недокачанное: звук на паузе, пока кусок не докачается, потом снова играть
    play_after: bool,
    /// ffmpeg для звука, который не умеет symphonia (Opus, AC3, DTS…)
    pub ffmpeg: Option<std::path::PathBuf>,
}

impl Audio {
    pub fn new() -> Self {
        let mut a = Self { _sink: None, player: None, duration: 0.0, loaded: false, error: None, stream: None, volume: 1.0, play_after: false, ffmpeg: None };
        a.open();
        a
    }

    /// Открыть устройство вывода (повторно — если его не было при запуске).
    fn open(&mut self) {
        match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut sink) => {
                sink.log_on_drop(false);
                self.player = Some(rodio::Player::connect_new(sink.mixer()));
                self._sink = Some(sink);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Нет устройства вывода звука: {e}")),
        }
    }

    /// Есть ли куда выводить звук.
    pub fn has_device(&self) -> bool { self.player.is_some() }

    /// Играть, пока файл ещё качается: декодер читает фрагменты по порядку (без перемотки),
    /// звук начинается с первых килобайт. Перемотка — когда нужный кусок докачается.
    pub fn load_stream(&mut self, dl: wavy_core::stream::Download, volume: f32) -> Result<(), String> {
        if self.player.is_none() { self.open(); }
        let p = self.player.as_ref().ok_or_else(|| self.error.clone().unwrap_or_default())?;
        p.clear();
        self.loaded = false;
        self.volume = volume;
        let reader = dl.reader().map_err(|e| e.to_string())?;
        let dec = rodio::Decoder::builder().with_data(std::io::BufReader::new(reader)).with_seekable(false).with_hint("m4a")
            .build().map_err(|e| format!("не удалось декодировать поток: {e}"))?;
        self.duration = 0.0; // у потока длительность неизвестна — берём из трека
        p.append(dec);
        p.set_volume(volume);
        p.play();
        self.loaded = true;
        self.stream = Some((dl, None));
        Ok(())
    }

    /// Перемотка, ждущая докачки (для индикации «буферизация…»).
    pub fn buffering(&self) -> bool { self.stream.as_ref().is_some_and(|s| s.1.is_some()) }

    /// Вызывать каждый кадр: когда поток докачался, выполнить отложенную перемотку.
    pub fn poll(&mut self) {
        let ready = match &self.stream { Some((dl, Some(_))) => dl.done(), _ => false };
        if ready {
            let (dl, pending) = self.stream.take().unwrap();
            if let Some(sec) = pending {
                self.reopen_at(&dl.path, sec);
                if std::mem::take(&mut self.play_after) { if let Some(p) = &self.player { p.play(); } }
            }
        }
    }

    /// Открыть докачанный файл (уже с перемоткой) и встать на позицию, сохранив паузу/игру.
    /// Переоткрыть тот же файл с другой звуковой дорожкой, оставшись на той же позиции.
    pub fn switch_track(&mut self, path: &Path, audio: usize) -> Result<(), String> {
        let (pos, playing, vol) = (self.position(), self.is_playing(), self.volume);
        self.load_track(path, vol, audio)?;
        if let Some(p) = &self.player { let _ = p.try_seek(Duration::from_secs_f64(pos)); if !playing { p.pause(); } }
        Ok(())
    }

    fn reopen_at(&mut self, path: &Path, sec: f64) {
        let was_playing = self.is_playing() || self.player.as_ref().is_some_and(|p| !p.is_paused());
        let vol = self.volume;
        if self.load(path, vol).is_ok() {
            if let Some(p) = &self.player { let _ = p.try_seek(Duration::from_secs_f64(sec.max(0.0))); if !was_playing { p.pause(); } }
        }
    }

    pub fn load(&mut self, path: &Path, volume: f32) -> Result<(), String> { self.load_track(path, volume, 0) }

    /// Открыть файл; для видео — с выбранной звуковой дорожкой (номер среди звуковых).
    pub fn load_track(&mut self, path: &Path, volume: f32, audio: usize) -> Result<(), String> {
        if self.player.is_none() { self.open(); }
        self.stream = None;
        self.volume = volume;
        let p = self.player.as_ref().ok_or_else(|| self.error.clone().unwrap_or_default())?;
        p.clear();
        self.loaded = false;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if wavy_core::local::VIDEO_EXT.contains(&ext.as_str()) {
            // видеофайл: rodio взял бы первую дорожку (видео), поэтому выбираем звуковую сами
            match AudioOfVideo::open(path, audio) {
                Ok(src) => { self.duration = src.duration.map(|d| d.as_secs_f64()).unwrap_or(0.0); p.append(src); }
                Err(e) if e.starts_with(wavy_core::ffmpeg::NEED) => {
                    let ff = self.ffmpeg.clone().ok_or(e)?;
                    let src = FfAudio::open(ff, path, audio, 0.0)?;
                    self.duration = src.duration.unwrap_or(0.0);
                    p.append(src);
                }
                Err(e) => return Err(e),
            }
            p.set_volume(volume);
            p.play();
            self.loaded = true;
            return Ok(());
        }
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let mut b = rodio::Decoder::builder().with_data(std::io::BufReader::new(f)).with_byte_len(len).with_seekable(true);
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) { b = b.with_hint(ext); }
        let dec = match b.build() {
            Ok(d) => d,
            // формат, которого нет в symphonia (Opus, AC3, DTS…) — через ffmpeg, если он есть
            Err(e) => {
                let Some(ff) = self.ffmpeg.clone() else { return Err(format!("{}не удалось декодировать: {e}", wavy_core::ffmpeg::NEED)) };
                let src = FfAudio::open(ff, path, 0, 0.0)?;
                self.duration = src.duration.unwrap_or(0.0);
                p.append(src);
                p.set_volume(volume);
                p.play();
                self.loaded = true;
                return Ok(());
            }
        };
        self.duration = rodio::Source::total_duration(&dec).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        p.append(dec);
        p.set_volume(volume);
        p.play();
        self.loaded = true;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.play_after = false;
        if let Some(p) = &self.player { p.clear(); }
        self.loaded = false;
        // недокачанный поток прошлого трека больше не нужен
        if let Some((dl, _)) = self.stream.take() { if !dl.done() { dl.cancel(); } }
    }
    pub fn play(&mut self) {
        if self.buffering() { self.play_after = true; return; } // заиграет с нужного места, когда докачается
        if let Some(p) = &self.player { p.play() }
    }
    pub fn pause(&mut self) { self.play_after = false; if let Some(p) = &self.player { p.pause() } }
    pub fn is_playing(&self) -> bool { self.loaded && self.player.as_ref().is_some_and(|p| !p.is_paused() && !p.empty()) }
    pub fn position(&self) -> f64 { self.player.as_ref().map(|p| p.get_pos().as_secs_f64()).unwrap_or(0.0) }
    pub fn set_volume(&mut self, v: f32) { self.volume = v * v; if let Some(p) = &self.player { p.set_volume(v * v) } } // квадратичная кривая — естественнее на слух
    pub fn seek(&mut self, sec: f64) {
        if let Some((dl, pending)) = &mut self.stream {
            if dl.done() {
                let path = dl.path.clone();
                self.stream = None;
                self.reopen_at(&path, sec);
            } else {
                *pending = Some(sec); // перемотаем, когда докачается
                // а пока молчим: иначе играло бы старое место (после обрыва — песня заново с начала)
                if let Some(p) = &self.player { if !p.is_paused() { p.pause(); self.play_after = true; } }
            }
            return;
        }
        if let Some(p) = &self.player { let _ = p.try_seek(Duration::from_secs_f64(sec.max(0.0))); }
    }
    /// Трек доиграл до конца.
    pub fn finished(&self) -> bool { self.loaded && self.player.as_ref().is_some_and(|p| p.empty()) }
}

/// Звуковая дорожка из видеофайла (MP4/MOV) через symphonia — с явным выбором аудиодорожки.
pub struct AudioOfVideo {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track: u32,
    buf: Vec<f32>,
    pos: usize,
    channels: std::num::NonZero<u16>,
    rate: std::num::NonZero<u32>,
    pub duration: Option<Duration>,
}

impl AudioOfVideo {
    pub fn open(path: &Path, nth: usize) -> Result<Self, String> {
        use symphonia::core::{codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(e) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(e); }
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
            .map_err(|e| format!("не удалось открыть файл: {e}"))?;
        let format = probed.format;
        let codecs = symphonia::default::get_codecs();
        let audio: Vec<_> = format.tracks().iter().filter(|t| t.codec_params.sample_rate.is_some()).collect();
        // выбранная дорожка в формате, которого нет в symphonia (Opus, AC3, DTS…) — это работа для ffmpeg
        let track = match audio.get(nth) {
            Some(t) if codecs.get_codec(t.codec_params.codec).is_some() => *t,
            Some(_) => return Err(format!("{}звук в формате, который нужен ffmpeg", wavy_core::ffmpeg::NEED)),
            None => audio.iter().copied().find(|t| codecs.get_codec(t.codec_params.codec).is_some())
                .ok_or_else(|| format!("{}в видео нет звуковой дорожки, которую можно воспроизвести без ffmpeg", wavy_core::ffmpeg::NEED))?,
        };
        let decoder = codecs.make(&track.codec_params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
        let rate = track.codec_params.sample_rate.and_then(std::num::NonZero::new).ok_or("неизвестная частота")?;
        let channels = track.codec_params.channels.map(|c| c.count() as u16).and_then(std::num::NonZero::new).unwrap_or(std::num::NonZero::new(2).unwrap());
        let duration = match (track.codec_params.time_base, track.codec_params.n_frames) {
            (Some(tb), Some(n)) => { let t = tb.calc_time(n); Some(Duration::from_secs_f64(t.seconds as f64 + t.frac)) }
            _ => None,
        };
        let id = track.id;
        let mut s = Self { format, decoder, track: id, buf: Vec::new(), pos: 0, channels, rate, duration };
        s.refill();
        Ok(s)
    }

    fn refill(&mut self) -> bool {
        use symphonia::core::{audio::SampleBuffer, errors::Error};
        loop {
            let packet = match self.format.next_packet() { Ok(p) => p, Err(_) => return false };
            if packet.track_id() != self.track { continue; }
            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    // число каналов и частоту берём из декодированного звука: в MP4 их часто не указывают
                    let spec = *audio.spec();
                    if let Some(c) = std::num::NonZero::new(spec.channels.count() as u16) { self.channels = c; }
                    if let Some(r) = std::num::NonZero::new(spec.rate) { self.rate = r; }
                    let mut sb = SampleBuffer::<f32>::new(audio.capacity() as u64, spec);
                    sb.copy_interleaved_ref(audio);
                    self.buf.clear();
                    self.buf.extend_from_slice(sb.samples());
                    self.pos = 0;
                    if !self.buf.is_empty() { return true; }
                }
                Err(Error::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
    }
}

impl Iterator for AudioOfVideo {
    type Item = rodio::Sample;
    fn next(&mut self) -> Option<rodio::Sample> {
        if self.pos >= self.buf.len() && !self.refill() { return None; }
        let v = self.buf[self.pos];
        self.pos += 1;
        Some(v as rodio::Sample)
    }
}

impl rodio::Source for AudioOfVideo {
    fn current_span_len(&self) -> Option<usize> { Some(self.buf.len().saturating_sub(self.pos).max(1)) }
    fn channels(&self) -> rodio::ChannelCount { self.channels }
    fn sample_rate(&self) -> rodio::SampleRate { self.rate }
    fn total_duration(&self) -> Option<Duration> { self.duration }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        use symphonia::core::{formats::{SeekMode, SeekTo}, units::Time};
        let time = Time::new(pos.as_secs(), pos.subsec_nanos() as f64 / 1e9);
        let _ = self.format.seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track) });
        self.decoder.reset();
        self.buf.clear();
        self.pos = 0;
        Ok(())
    }
}

/// Кусок звука из файла (любого: музыка, MP4, MKV) для Shazam: `secs` секунд с позиции `start`, моно 16 кГц.
/// `audio` — номер звуковой дорожки (для видео с несколькими озвучками).
pub fn sample_for_shazam(path: &Path, start: f64, secs: f64, audio: usize, ffmpeg: Option<&Path>) -> Result<Vec<f32>, String> {
    match sample_symphonia(path, start, secs, audio) {
        Err(e) if e.starts_with(wavy_core::ffmpeg::NEED) => {
            let ff = ffmpeg.ok_or_else(|| wavy_core::ffmpeg::human(&e).to_string())?;
            let out = wavy_core::ffmpeg::cmd(ff).args(["-nostdin", "-hide_banner", "-loglevel", "error", "-ss", &format!("{:.3}", start.max(0.0)), "-t", &format!("{secs}")])
                .arg("-i").arg(path).arg("-map").arg(format!("0:a:{audio}?")).args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"])
                .output().map_err(|e| format!("ffmpeg: {e}"))?;
            let v: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            if v.len() < 16000 * 3 { return Err("слишком мало звука".into()); }
            Ok(v)
        }
        r => r,
    }
}

fn sample_symphonia(path: &Path, start: f64, secs: f64, audio: usize) -> Result<Vec<f32>, String> {
    use symphonia::core::{audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::{FormatOptions, SeekMode, SeekTo}, io::MediaSourceStream, meta::MetadataOptions, probe::Hint, units::Time};
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(e); }
    let mut format = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("{}не удалось открыть файл: {e}", wavy_core::ffmpeg::NEED))?.format;
    let codecs = symphonia::default::get_codecs();
    let audio_tracks: Vec<_> = format.tracks().iter().filter(|t| t.codec_params.sample_rate.is_some()).cloned().collect();
    let track = match audio_tracks.get(audio) {
        Some(t) if codecs.get_codec(t.codec_params.codec).is_some() => t.clone(),
        Some(_) => return Err(format!("{}звук в формате, который нужен ffmpeg", wavy_core::ffmpeg::NEED)),
        None => audio_tracks.iter().find(|t| codecs.get_codec(t.codec_params.codec).is_some()).cloned()
            .ok_or_else(|| format!("{}нет звуковой дорожки, которую можно разобрать", wavy_core::ffmpeg::NEED))?,
    };
    let mut dec = codecs.make(&track.codec_params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
    if start > 0.5 {
        let t = Time::new(start as u64, start.fract());
        let _ = format.seek(SeekMode::Coarse, SeekTo::Time { time: t, track_id: Some(track.id) });
    }
    let (mut pcm, mut ch, mut rate) = (Vec::<f32>::new(), 2usize, track.codec_params.sample_rate.unwrap_or(44100));
    loop {
        if pcm.len() as f64 >= secs * rate as f64 * ch as f64 { break; }
        let packet = match format.next_packet() { Ok(p) => p, Err(_) => break };
        if packet.track_id() != track.id { continue; }
        match dec.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                ch = spec.channels.count().max(1);
                rate = spec.rate;
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(buf);
                pcm.extend_from_slice(sb.samples());
            }
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    if pcm.len() < rate as usize * ch * 3 { return Err("слишком мало звука".into()); }
    Ok(wavy_core::shazam::to_mono_16k(&pcm, ch, rate))
}

/// Звук через ffmpeg (Opus, AC3/E-AC3, DTS, TrueHD…): f32 стерео 48 кГц по трубе. Перемотка — перезапуск с -ss.
pub struct FfAudio {
    ff: std::path::PathBuf,
    path: std::path::PathBuf,
    track: usize,
    child: Option<std::process::Child>,
    out: Option<std::io::BufReader<std::process::ChildStdout>>,
    pub duration: Option<f64>,
}

impl FfAudio {
    pub fn open(ff: std::path::PathBuf, path: &Path, track: usize, at: f64) -> Result<Self, String> {
        let duration = wavy_core::media::duration_of(path);
        let mut s = Self { ff, path: path.to_path_buf(), track, child: None, out: None, duration };
        s.spawn(at)?;
        Ok(s)
    }

    fn spawn(&mut self, at: f64) -> Result<(), String> {
        use std::process::Stdio;
        self.kill();
        let mut c = wavy_core::ffmpeg::cmd(&self.ff);
        c.args(["-nostdin", "-hide_banner", "-nostats", "-loglevel", "error"]);
        if at > 0.05 { c.arg("-ss").arg(format!("{at:.3}")); }
        c.arg("-i").arg(&self.path).arg("-map").arg(format!("0:a:{}?", self.track))
            .args(["-vn", "-sn", "-dn", "-ac", "2", "-ar", "48000", "-f", "f32le", "-"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = c.spawn().map_err(|e| format!("не удалось запустить ffmpeg: {e}"))?;
        self.out = child.stdout.take().map(|o| std::io::BufReader::with_capacity(256 * 1024, o));
        self.child = Some(child);
        Ok(())
    }

    fn kill(&mut self) {
        self.out = None;
        if let Some(mut c) = self.child.take() { let _ = c.kill(); let _ = c.wait(); }
    }
}

impl Drop for FfAudio { fn drop(&mut self) { self.kill(); } }

impl Iterator for FfAudio {
    type Item = rodio::Sample;
    fn next(&mut self) -> Option<rodio::Sample> {
        use std::io::Read;
        let mut b = [0u8; 4];
        self.out.as_mut()?.read_exact(&mut b).ok()?;
        Some(f32::from_le_bytes(b) as rodio::Sample)
    }
}

impl rodio::Source for FfAudio {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> rodio::ChannelCount { std::num::NonZero::new(2).unwrap() }
    fn sample_rate(&self) -> rodio::SampleRate { std::num::NonZero::new(48000).unwrap() }
    fn total_duration(&self) -> Option<Duration> { self.duration.map(Duration::from_secs_f64) }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        let _ = self.spawn(pos.as_secs_f64());
        Ok(())
    }
}
