//! Аудио на rodio (symphonia): mp3, flac, wav, ogg/vorbis, m4a/aac, alac.
use std::path::Path;
use std::time::Duration;

pub struct Audio {
    _sink: Option<rodio::MixerDeviceSink>,
    player: Option<rodio::Player>,
    pub duration: f64,
    pub loaded: bool,
    pub error: Option<String>,
}

impl Audio {
    pub fn new() -> Self {
        let mut a = Self { _sink: None, player: None, duration: 0.0, loaded: false, error: None };
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

    pub fn load(&mut self, path: &Path, volume: f32) -> Result<(), String> {
        if self.player.is_none() { self.open(); }
        let p = self.player.as_ref().ok_or_else(|| self.error.clone().unwrap_or_default())?;
        p.clear();
        self.loaded = false;
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

    pub fn stop(&mut self) { if let Some(p) = &self.player { p.clear(); } self.loaded = false; }
    pub fn play(&self) { if let Some(p) = &self.player { p.play() } }
    pub fn pause(&self) { if let Some(p) = &self.player { p.pause() } }
    pub fn is_playing(&self) -> bool { self.loaded && self.player.as_ref().is_some_and(|p| !p.is_paused() && !p.empty()) }
    pub fn position(&self) -> f64 { self.player.as_ref().map(|p| p.get_pos().as_secs_f64()).unwrap_or(0.0) }
    pub fn set_volume(&self, v: f32) { if let Some(p) = &self.player { p.set_volume(v * v) } } // квадратичная кривая — естественнее на слух
    pub fn seek(&self, sec: f64) { if let Some(p) = &self.player { let _ = p.try_seek(Duration::from_secs_f64(sec.max(0.0))); } }
    /// Трек доиграл до конца.
    pub fn finished(&self) -> bool { self.loaded && self.player.as_ref().is_some_and(|p| p.empty()) }
}
