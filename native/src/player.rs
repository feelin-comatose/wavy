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
}

impl Audio {
    pub fn new() -> Self {
        let mut a = Self { _sink: None, player: None, duration: 0.0, loaded: false, error: None, stream: None, volume: 1.0 };
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
            if let Some(sec) = pending { self.reopen_at(&dl.path, sec); }
        }
    }

    /// Открыть докачанный файл (уже с перемоткой) и встать на позицию, сохранив паузу/игру.
    fn reopen_at(&mut self, path: &Path, sec: f64) {
        let was_playing = self.is_playing() || self.player.as_ref().is_some_and(|p| !p.is_paused());
        let vol = self.volume;
        if self.load(path, vol).is_ok() {
            if let Some(p) = &self.player { let _ = p.try_seek(Duration::from_secs_f64(sec.max(0.0))); if !was_playing { p.pause(); } }
        }
    }

    pub fn load(&mut self, path: &Path, volume: f32) -> Result<(), String> {
        if self.player.is_none() { self.open(); }
        self.stream = None;
        self.volume = volume;
        let p = self.player.as_ref().ok_or_else(|| self.error.clone().unwrap_or_default())?;
        p.clear();
        self.loaded = false;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if wavy_core::local::VIDEO_EXT.contains(&ext.as_str()) {
            // видеофайл: rodio взял бы первую дорожку (видео), поэтому выбираем звуковую сами
            let src = AudioOfVideo::open(path)?;
            self.duration = src.duration.map(|d| d.as_secs_f64()).unwrap_or(0.0);
            p.append(src);
            p.set_volume(volume);
            p.play();
            self.loaded = true;
            return Ok(());
        }
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let mut b = rodio::Decoder::builder().with_data(std::io::BufReader::new(f)).with_byte_len(len).with_seekable(true);
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) { b = b.with_hint(ext); }
        let dec = b.build().map_err(|e| format!("не удалось декодировать: {e}"))?;
        self.duration = rodio::Source::total_duration(&dec).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        p.append(dec);
        p.set_volume(volume);
        p.play();
        self.loaded = true;
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(p) = &self.player { p.clear(); }
        self.loaded = false;
        // недокачанный поток прошлого трека больше не нужен
        if let Some((dl, _)) = self.stream.take() { if !dl.done() { dl.cancel(); } }
    }
    pub fn play(&self) { if let Some(p) = &self.player { p.play() } }
    pub fn pause(&self) { if let Some(p) = &self.player { p.pause() } }
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
    pub fn open(path: &Path) -> Result<Self, String> {
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
        let track = format.tracks().iter()
            .find(|t| t.codec_params.sample_rate.is_some() && codecs.get_codec(t.codec_params.codec).is_some())
            .ok_or("в видео нет звуковой дорожки (или её формат не поддерживается)")?;
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
