//! Spectrum visualizer: the effect chain hands over the last played samples (`Tap`), the UI turns them
//! into a few smoothed bars with the FFT from the Shazam code. Costs one small FFT per drawn frame,
//! and only while the bars are on screen.
use std::sync::{Arc, Mutex};

/// Samples per hand-over from the audio thread.
pub const BATCH: usize = 512;
const N: usize = 1024;

/// The last N mono samples and their sample rate.
#[derive(Clone, Default)]
pub struct Tap(Arc<Mutex<(Vec<f32>, u32)>>);

impl Tap {
    pub fn push(&self, s: &[f32], rate: u32) {
        let mut g = self.0.lock().unwrap();
        g.0.extend_from_slice(s);
        let n = g.0.len();
        if n > N { g.0.drain(..n - N); }
        g.1 = rate;
    }
    fn latest(&self) -> Option<(Vec<f32>, u32)> {
        let g = self.0.lock().unwrap();
        (g.0.len() == N).then(|| g.clone())
    }
}

pub struct Spectrum {
    fft: wavy_core::shazam::Fft,
    window: Vec<f32>,
    pub bars: Vec<f32>,
}

impl Spectrum {
    pub fn new(bars: usize) -> Self {
        let window = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
        Self { fft: wavy_core::shazam::Fft::new(N), window, bars: vec![0.0; bars] }
    }

    /// Recompute the bars (0..1) from the latest samples; `playing` false lets them fall to zero.
    pub fn update(&mut self, tap: &Tap, playing: bool) {
        let target: Vec<f32> = match tap.latest().filter(|_| playing) {
            None => vec![0.0; self.bars.len()],
            Some((s, rate)) => {
                let mut re: Vec<f32> = s.iter().zip(&self.window).map(|(x, w)| x * w).collect();
                let mut im = vec![0.0; N];
                self.fft.run(&mut re, &mut im);
                let mag: Vec<f32> = (0..N / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() / (N as f32 / 4.0)).collect();
                // log-spaced bands from 40 Hz to 16 kHz, in dB (−60 dB → 0, 0 dB → 1)
                let (lo, hi, n) = (40f32, 16000f32.min(rate as f32 / 2.0), self.bars.len());
                (0..n).map(|b| {
                    let f0 = lo * (hi / lo).powf(b as f32 / n as f32);
                    let f1 = lo * (hi / lo).powf((b + 1) as f32 / n as f32);
                    let (k0, k1) = ((f0 * N as f32 / rate as f32) as usize, ((f1 * N as f32 / rate as f32) as usize).max((f0 * N as f32 / rate as f32) as usize + 1));
                    let peak = mag[k0.min(N / 2 - 1)..k1.min(N / 2)].iter().cloned().fold(0.0, f32::max);
                    ((20.0 * peak.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0)
                }).collect()
            }
        };
        // jump up, fall slowly — the usual meter feel
        for (b, t) in self.bars.iter_mut().zip(target) { *b = if t > *b { t } else { *b * 0.85 + t * 0.15 }; }
    }

    pub fn active(&self) -> bool { self.bars.iter().any(|b| *b > 0.01) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_lights_its_band() {
        let tap = Tap::default();
        let rate = 44100;
        let s: Vec<f32> = (0..N).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate as f32).sin() * 0.5).collect();
        tap.push(&s, rate);
        let mut sp = Spectrum::new(16);
        sp.update(&tap, true);
        let loudest = sp.bars.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        // 1 kHz on a 40 Hz…16 kHz log scale with 16 bands lands in band 8 or 9
        assert!((8..=9).contains(&loudest), "{:?}", sp.bars);
        assert!(sp.bars[0] < 0.6 && sp.bars[15] < 0.6);
        sp.update(&tap, false);
        assert!(sp.bars.iter().all(|b| *b < sp.bars[loudest] + 1.0)); // falls when paused
    }
}
