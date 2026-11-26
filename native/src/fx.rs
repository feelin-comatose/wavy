//! Audio effects on top of any source: speed (nightcore/slowed), 10-band equalizer,
//! volume normalization and a soft limiter. Plus an honest position in the track: rodio's built-in one
//! counts real time, so at speed ≠ 1 the seek bar would be wrong.
//!
//! Settings change on the fly: the effect rereads them every ~2048 frames (≈40 ms).
use rodio::Source;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const BANDS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub speed: f32,
    pub eq_on: bool,
    /// band gains, dB (−12…+12)
    pub eq: [f32; 10],
    pub normalize: bool,
}

impl Default for Settings {
    fn default() -> Self { Self { speed: 1.0, eq_on: false, eq: [0.0; 10], normalize: false } }
}

/// Settings shared by all tracks (changed from the UI).
#[derive(Clone, Default)]
pub struct Shared {
    settings: Arc<Mutex<(Settings, u64)>>,
    /// the last played samples, for the visualizer
    pub tap: crate::viz::Tap,
}

impl Shared {
    pub fn set(&self, s: Settings) {
        let mut g = self.settings.lock().unwrap();
        if g.0 != s { g.0 = s; g.1 += 1; }
    }
    fn snapshot(&self) -> (Settings, u64) { self.settings.lock().unwrap().clone() }
}

/// Position in the track (source seconds), updated by the effect.
#[derive(Clone, Default)]
pub struct Pos(Arc<AtomicU64>);
impl Pos {
    pub fn get(&self) -> f64 { f64::from_bits(self.0.load(Ordering::Relaxed)) }
    fn set(&self, v: f64) { self.0.store(v.to_bits(), Ordering::Relaxed) }
}

#[derive(Clone, Copy, Default)]
struct Biquad { b0: f32, b1: f32, b2: f32, a1: f32, a2: f32 }

impl Biquad {
    /// Peaking filter (RBJ cookbook).
    fn peaking(f: f32, q: f32, db: f32, fs: f32) -> Self {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * PI * f / fs;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Self { b0: (1.0 + alpha * a) / a0, b1: -2.0 * cs / a0, b2: (1.0 - alpha * a) / a0, a1: -2.0 * cs / a0, a2: (1.0 - alpha / a) / a0 }
    }
}

const MAX_CH: usize = 8;
const SPAN: usize = 2048;

pub struct Fx<S: Source> {
    inner: S,
    shared: Shared,
    pos: Pos,
    version: u64,
    settings: Settings,
    // current chunk
    ch: usize,
    rate: u32,
    left: usize, // samples until the end of the chunk
    frame_i: usize,
    // equalizer: active bands and their per-channel state (x1, x2, y1, y2)
    filters: Vec<Biquad>,
    state: Vec<[[f32; 4]; MAX_CH]>,
    // volume normalization
    ms: f32,
    gain: f32,
    heard: f32,
    secs: f64,
    frame: [f32; MAX_CH],
    /// ReplayGain from the file's tags (volume factor): with normalization on it replaces the adaptive gain
    replaygain: Option<f32>,
    /// mono samples collected for the visualizer, handed over in batches (one lock per batch, not per sample)
    tap_buf: Vec<f32>,
}

impl<S: Source> Fx<S> {
    pub fn new(inner: S, shared: Shared, pos: Pos) -> Self {
        let (settings, version) = shared.snapshot();
        let mut fx = Self {
            inner, shared, pos, version: u64::MAX, settings, ch: 2, rate: 44100, left: 0, frame_i: 0,
            filters: Vec::new(), state: Vec::new(), ms: 0.0, gain: 1.0, heard: 0.0, secs: 0.0, frame: [0.0; MAX_CH], replaygain: None, tap_buf: Vec::with_capacity(crate::viz::BATCH),
        };
        fx.version = version.wrapping_sub(1);
        fx.pos.set(0.0);
        fx
    }

    /// Use the file's ReplayGain (when normalization is on) instead of measuring loudness on the fly.
    pub fn with_replaygain(mut self, g: Option<f32>) -> Self { self.replaygain = g; self }

    fn rebuild(&mut self) {
        let fs = self.rate as f32;
        let old = std::mem::take(&mut self.filters);
        self.filters = if self.settings.eq_on {
            BANDS.iter().zip(self.settings.eq.iter())
                .filter(|(f, db)| db.abs() > 0.05 && **f < fs * 0.45)
                .map(|(f, db)| Biquad::peaking(*f, 1.1, *db, fs)).collect()
        } else { Vec::new() };
        if self.filters.len() != old.len() { self.state = vec![[[0.0; 4]; MAX_CH]; self.filters.len()]; }
    }

    /// Start of a new chunk: reread settings and format.
    fn new_span(&mut self) {
        let (ch, rate) = (self.inner.channels().get() as usize, self.inner.sample_rate().get());
        let (s, v) = self.shared.snapshot();
        let fmt_changed = ch != self.ch || rate != self.rate;
        self.ch = ch.min(MAX_CH);
        self.rate = rate;
        if v != self.version || fmt_changed {
            let eq_changed = s.eq != self.settings.eq || s.eq_on != self.settings.eq_on;
            self.settings = s;
            self.version = v;
            if eq_changed || fmt_changed || self.filters.is_empty() == self.settings.eq_on { self.rebuild(); }
        }
        let inner_left = self.inner.current_span_len().unwrap_or(usize::MAX);
        let per = SPAN * self.ch;
        self.left = inner_left.min(per).max(self.ch);
        self.left -= self.left % self.ch.max(1);
        if self.left == 0 { self.left = self.ch; }
    }

    fn process_frame(&mut self) -> bool {
        let ch = self.ch;
        for c in 0..ch {
            match self.inner.next() { Some(x) => self.frame[c] = x, None => return false }
        }
        // equalizer
        for (bi, f) in self.filters.iter().enumerate() {
            let st = &mut self.state[bi];
            for c in 0..ch {
                let s = &mut st[c];
                let x = self.frame[c];
                let y = f.b0 * x + f.b1 * s[0] + f.b2 * s[1] - f.a1 * s[2] - f.a2 * s[3];
                s[1] = s[0]; s[0] = x; s[3] = s[2]; s[2] = y;
                self.frame[c] = y;
            }
        }
        // normalization: average power over ~3 s → gain toward the target level (smoothly, within ×0.3…×3)
        let dt = 1.0 / self.rate as f32;
        if let (true, Some(rg)) = (self.settings.normalize, self.replaygain) {
            // the file knows its own loudness: a fixed gain, reached smoothly
            self.gain += (rg - self.gain) * (dt / 0.3).min(1.0);
        } else if self.settings.normalize {
            let p = self.frame[..ch].iter().map(|x| x * x).sum::<f32>() / ch as f32;
            self.ms += (p - self.ms) * (dt / 3.0).min(1.0);
            self.heard += dt;
            if self.heard > 1.0 && self.ms > 1e-7 {
                let want = (0.16 / self.ms.sqrt()).clamp(0.3, 3.0);
                self.gain += (want - self.gain) * (dt / 1.5).min(1.0);
            }
        } else {
            self.gain += (1.0 - self.gain) * (dt / 0.3).min(1.0);
        }
        let needs_limit = self.gain > 1.0 || !self.filters.is_empty();
        for c in 0..ch {
            let mut y = self.frame[c] * self.gain;
            // soft limiting: louder than 0.9 is smoothly pressed toward 1.0 (no crackle)
            if needs_limit && y.abs() > 0.9 { y = y.signum() * (0.9 + 0.1 * ((y.abs() - 0.9) / 0.1).tanh()); }
            self.frame[c] = y;
        }
        self.secs += dt as f64;
        if self.frame_i % 256 == 0 { self.pos.set(self.secs); }
        self.tap_buf.push(self.frame[..ch].iter().sum::<f32>() / ch as f32);
        if self.tap_buf.len() >= crate::viz::BATCH { self.shared.tap.push(&self.tap_buf, self.rate); self.tap_buf.clear(); }
        true
    }
}

impl<S: Source> Iterator for Fx<S> {
    type Item = rodio::Sample;
    fn next(&mut self) -> Option<rodio::Sample> {
        if self.frame_i == 0 {
            if self.left == 0 { self.new_span(); }
            if !self.process_frame() { self.pos.set(self.secs); return None; }
        }
        let v = self.frame[self.frame_i];
        self.frame_i += 1;
        if self.frame_i >= self.ch { self.frame_i = 0; }
        self.left = self.left.saturating_sub(1);
        Some(v)
    }
}

impl<S: Source> Source for Fx<S> {
    fn current_span_len(&self) -> Option<usize> { Some(if self.left == 0 { SPAN * self.ch } else { self.left }) }
    fn channels(&self) -> rodio::ChannelCount { std::num::NonZero::new(self.ch as u16).unwrap_or(std::num::NonZero::<u16>::MIN) }
    /// speed = sample rate times the factor (pitch changes along with speed)
    fn sample_rate(&self) -> rodio::SampleRate {
        let r = (self.rate as f32 * self.settings.speed.clamp(0.5, 2.0)).round() as u32;
        std::num::NonZero::new(r.max(1000)).unwrap_or(std::num::NonZero::<u32>::MIN)
    }
    fn total_duration(&self) -> Option<Duration> { self.inner.total_duration() }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)?;
        self.secs = pos.as_secs_f64();
        self.pos.set(self.secs);
        self.frame_i = 0;
        self.left = 0;
        for s in &mut self.state { *s = [[0.0; 4]; MAX_CH]; }
        Ok(())
    }
}

/// Equalizer presets.
pub const PRESETS: &[(&str, [f32; 10])] = &[
    ("Ровно", [0.0; 10]),
    ("Бас", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Вокал", [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
    ("Электроника", [5.0, 4.0, 1.0, 0.0, -2.0, 1.0, 0.0, 2.0, 4.0, 5.0]),
    ("Рок", [4.0, 3.0, 1.0, -1.0, -2.0, -1.0, 1.0, 3.0, 4.0, 4.0]),
    ("Мягче верхи", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -3.0, -5.0, -6.0]),
    ("Наушники-затычки", [3.0, 2.0, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 1.0, 0.0]),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    fn tone(f: f32, secs: f32) -> SamplesBuffer {
        let n = (44100.0 * secs) as usize;
        let v: Vec<f32> = (0..n).flat_map(|i| { let x = (2.0 * PI * f * i as f32 / 44100.0).sin() * 0.5; [x, x] }).collect();
        SamplesBuffer::new(std::num::NonZero::new(2).unwrap(), std::num::NonZero::new(44100).unwrap(), v)
    }

    fn rms(v: &[f32]) -> f32 { (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt() }

    #[test]
    fn passthrough_and_position() {
        let sh = Shared::default();
        let pos = Pos::default();
        let fx = Fx::new(tone(440.0, 1.0), sh, pos.clone());
        let out: Vec<f32> = fx.collect();
        assert_eq!(out.len(), 88200);
        assert!((rms(&out) - 0.3535).abs() < 0.01);
        assert!((pos.get() - 1.0).abs() < 0.01, "{}", pos.get());
    }

    #[test]
    fn eq_boosts_band() {
        let sh = Shared::default();
        let mut s = Settings::default();
        s.eq_on = true;
        s.eq[2] = 12.0; // 125 Hz
        sh.set(s);
        let out: Vec<f32> = Fx::new(tone(125.0, 1.0), sh.clone(), Pos::default()).collect();
        let hi: Vec<f32> = Fx::new(tone(4000.0, 1.0), sh, Pos::default()).collect();
        assert!(rms(&out[20000..]) > 0.6, "буст не сработал: {}", rms(&out[20000..]));
        assert!((rms(&hi[20000..]) - 0.3535).abs() < 0.03, "задело другие частоты: {}", rms(&hi[20000..]));
        assert!(out.iter().all(|x| x.abs() <= 1.0));
    }

    #[test]
    fn speed_reports_rate_and_keeps_track_time() {
        let sh = Shared::default();
        sh.set(Settings { speed: 1.25, ..Default::default() });
        let pos = Pos::default();
        let mut fx = Fx::new(tone(440.0, 2.0), sh, pos.clone());
        let _ = fx.next();
        assert_eq!(fx.sample_rate().get(), 55125);
        let _: Vec<f32> = fx.collect();
        assert!((pos.get() - 2.0).abs() < 0.01); // position is track time, not real time
    }

    #[test]
    fn replaygain_is_a_fixed_gain() {
        let sh = Shared::default();
        sh.set(Settings { normalize: true, ..Default::default() });
        let quiet: Vec<f32> = tone(440.0, 4.0).map(|x| x * 0.1).collect();
        let level = rms(&quiet[quiet.len() - 44100..]);
        let src = SamplesBuffer::new(std::num::NonZero::new(2).unwrap(), std::num::NonZero::new(44100).unwrap(), quiet);
        // −6 dB from the tags: half as loud, not lifted like the adaptive mode would
        let out: Vec<f32> = Fx::new(src, sh, Pos::default()).with_replaygain(Some(0.5)).collect();
        let tail = rms(&out[out.len() - 44100..]);
        assert!((tail / level - 0.5).abs() < 0.02, "{tail} vs {level}");
    }

    #[test]
    fn normalize_lifts_quiet() {
        let sh = Shared::default();
        sh.set(Settings { normalize: true, ..Default::default() });
        let quiet: Vec<f32> = tone(440.0, 8.0).map(|x| x * 0.1).collect();
        let src = SamplesBuffer::new(std::num::NonZero::new(2).unwrap(), std::num::NonZero::new(44100).unwrap(), quiet);
        let out: Vec<f32> = Fx::new(src, sh, Pos::default()).collect();
        let tail = rms(&out[out.len() - 44100..]);
        assert!(tail > 0.1, "тихий трек не подтянулся: {tail}");
    }
}
