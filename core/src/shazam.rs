//! Music recognition through Shazam (like the app): a "fingerprint" (spectrum peaks) is built from ~12 seconds of audio
//! and sent to the Shazam server, which returns the title, artist and cover.
//!
//! The fingerprint algorithm is ported from SongRec (https://github.com/marin-m/SongRec, GPL-3.0 — like Wavy),
//! FFT and CRC32 are written here to avoid pulling in extra libraries.
use base64::Engine;
use serde_json::{json, Value};

pub const RATE: usize = 16_000;
/// How much audio to send (the same as SongRec and the Shazam app use).
pub const SECONDS: usize = 12;

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub title: String,
    pub artist: String,
    pub cover: Option<String>,
    pub url: Option<String>,
}

struct Peak {
    pass: u32,
    magnitude: u16,
    bin: u16,
}

/// Fingerprint from 16 kHz mono audio (values −1..1).
pub fn signature(samples: &[f32]) -> Vec<u8> {
    let mut g = Gen::new();
    let s16: Vec<i16> = samples.iter().map(|&x| (x * 32768.0).clamp(-32768.0, 32767.0) as i16).collect();
    for chunk in s16.chunks_exact(128) {
        g.fft(chunk);
        g.spread();
        g.done += 1;
        if g.done >= 46 { g.recognize(); }
    }
    encode(&g.peaks, samples.len() as u32)
}

struct Gen {
    ring: Vec<i16>,
    ring_i: usize,
    window: Vec<f32>,
    fft: Fft,
    outs: Vec<[f32; 1025]>,
    outs_i: u8,
    spread: Vec<[f32; 1025]>,
    spread_i: u8,
    done: u32,
    peaks: [Vec<Peak>; 4],
}

impl Gen {
    fn new() -> Self {
        // 2048-sample Hann window without zeros at the edges (like SongRec's table)
        let window = (0..2048).map(|i| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * (i + 1) as f64 / 2049.0).cos()) as f32).collect();
        Self {
            ring: vec![0; 2048], ring_i: 0, window, fft: Fft::new(2048),
            outs: vec![[0.0; 1025]; 256], outs_i: 0, spread: vec![[0.0; 1025]; 256], spread_i: 0, done: 0,
            peaks: Default::default(),
        }
    }

    fn fft(&mut self, chunk: &[i16]) {
        self.ring[self.ring_i..self.ring_i + 128].copy_from_slice(chunk);
        self.ring_i = (self.ring_i + 128) & 2047;
        let mut re = vec![0f32; 2048];
        let mut im = vec![0f32; 2048];
        for (i, w) in self.window.iter().enumerate() { re[i] = self.ring[(i + self.ring_i) & 2047] as f32 * w; }
        self.fft.run(&mut re, &mut im);
        let out = &mut self.outs[self.outs_i as usize];
        for i in 0..1025 { out[i] = ((re[i] * re[i] + im[i] * im[i]) / (1u32 << 17) as f32).max(1e-10); }
        self.outs_i = self.outs_i.wrapping_add(1);
    }

    fn spread(&mut self) {
        let mut cur = self.outs[self.outs_i.wrapping_sub(1) as usize];
        for p in 0..=1022 { cur[p] = cur[p].max(cur[p + 1]).max(cur[p + 2]); }
        self.spread[self.spread_i as usize] = cur;
        for p in 0..=1024 {
            for back in [1u8, 3, 6] {
                let f = &mut self.spread[self.spread_i.wrapping_sub(back) as usize];
                f[p] = f[p].max(cur[p]);
            }
        }
        self.spread_i = self.spread_i.wrapping_add(1);
    }

    fn recognize(&mut self) {
        let f46 = self.outs[self.outs_i.wrapping_sub(46) as usize];
        let f49 = self.spread[self.spread_i.wrapping_sub(49) as usize];
        for bin in 10..=1014usize {
            if !(f46[bin] >= 1.0 / 64.0 && f46[bin] >= f49[bin - 1]) { continue; }
            let mut max_n = 0f32;
            for off in [-10i32, -7, -4, -3, 1, 2, 5, 8] { max_n = max_n.max(f49[(bin as i32 + off) as usize]); }
            if f46[bin] <= max_n { continue; }
            let mut max_o = max_n;
            for off in [-53i32, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249] {
                let other = &self.spread[((self.spread_i as i32 + off) & 255) as usize];
                max_o = max_o.max(other[bin - 1]);
            }
            if f46[bin] <= max_o { continue; }
            let mag = |v: f32| v.ln().max(1.0 / 64.0) * 1477.3 + 6144.0;
            let (m, before, after) = (mag(f46[bin]), mag(f46[bin - 1]), mag(f46[bin + 1]));
            let var1 = m * 2.0 - before - after;
            if var1 <= 0.0 { continue; }
            let var2 = (after - before) * 32.0 / var1;
            let corrected = (bin as i32 * 64 + var2 as i32) as u16;
            let hz = corrected as f32 * (16000.0 / 2.0 / 1024.0 / 64.0);
            let band = match hz as i32 { 250..=519 => 0, 520..=1449 => 1, 1450..=3499 => 2, 3500..=5500 => 3, _ => continue };
            self.peaks[band].push(Peak { pass: self.done - 46, magnitude: m as u16, bin: corrected });
        }
    }
}

/// Shazam fingerprint binary format (see SongRec signature_format.rs).
fn encode(peaks: &[Vec<Peak>; 4], samples: u32) -> Vec<u8> {
    let mut b: Vec<u8> = Vec::new();
    let w = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
    w(&mut b, 0xcafe2580);
    w(&mut b, 0); // crc32 — later
    w(&mut b, 0); // size without the header — later
    w(&mut b, 0x94119c00);
    for _ in 0..3 { w(&mut b, 0); }
    w(&mut b, 3 << 27); // 16000 Hz
    for _ in 0..2 { w(&mut b, 0); }
    w(&mut b, samples + (16000.0 * 0.24) as u32);
    w(&mut b, (15 << 19) + 0x40000);
    w(&mut b, 0x40000000);
    w(&mut b, 0);
    for (band, list) in peaks.iter().enumerate() {
        if list.is_empty() { continue; }
        let mut pb: Vec<u8> = Vec::new();
        let mut pass = 0u32;
        for p in list {
            if p.pass - pass >= 255 {
                pb.push(0xff);
                pb.extend_from_slice(&p.pass.to_le_bytes());
                pass = p.pass;
            }
            pb.push((p.pass - pass) as u8);
            pb.extend_from_slice(&p.magnitude.to_le_bytes());
            pb.extend_from_slice(&p.bin.to_le_bytes());
            pass = p.pass;
        }
        w(&mut b, 0x60030040 + band as u32);
        w(&mut b, pb.len() as u32);
        let pad = (4 - pb.len() % 4) % 4;
        b.extend_from_slice(&pb);
        b.extend(std::iter::repeat_n(0u8, pad));
    }
    let size = b.len() as u32 - 48;
    b[8..12].copy_from_slice(&size.to_le_bytes());
    b[52..56].copy_from_slice(&size.to_le_bytes());
    let crc = crc32(&b[8..]);
    b[4..8].copy_from_slice(&crc.to_le_bytes());
    b
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &x in data {
        c ^= x as u32;
        for _ in 0..8 { c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 }; }
    }
    !c
}

/// Complex FFT (radix-2) for size 2^n (also used by the player's visualizer).
pub struct Fft { n: usize, cos: Vec<f32>, sin: Vec<f32>, rev: Vec<usize> }

impl Fft {
    pub fn new(n: usize) -> Self {
        let bits = n.trailing_zeros();
        let rev = (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let (cos, sin) = (0..n / 2).map(|k| { let a = -2.0 * std::f64::consts::PI * k as f64 / n as f64; (a.cos() as f32, a.sin() as f32) }).unzip();
        Self { n, cos, sin, rev }
    }
    pub fn run(&self, re: &mut [f32], im: &mut [f32]) {
        for i in 0..self.n { let j = self.rev[i]; if j > i { re.swap(i, j); im.swap(i, j); } }
        let mut len = 2;
        while len <= self.n {
            let step = self.n / len;
            for start in (0..self.n).step_by(len) {
                for k in 0..len / 2 {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + len / 2);
                    let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                    re[b] = re[a] - tr; im[b] = im[a] - ti;
                    re[a] += tr; im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}

fn uuid4() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for (i, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(crate::now_ms() ^ (i as u64) << 32);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Send audio (16 kHz mono) to Shazam. Ok(None) — not recognized.
pub fn recognize(samples: &[f32]) -> Result<Option<Found>, String> {
    let sig = signature(samples);
    let now = crate::now_ms();
    let body = json!({
        "geolocation": { "altitude": 300, "latitude": 45, "longitude": 2 },
        "signature": {
            "samplems": (samples.len() as f64 / RATE as f64 * 1000.0) as u64,
            "timestamp": now,
            "uri": format!("data:audio/vnd.shazam.sig;base64,{}", base64::engine::general_purpose::STANDARD.encode(&sig)),
        },
        "timestamp": now,
        "timezone": "Europe/Moscow",
    });
    let url = format!(
        "https://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3",
        uuid4().to_uppercase(), uuid4()
    );
    let mut r = crate::http().post(&url)
        .header("User-Agent", "Dalvik/2.1.0 (Linux; U; Android 10; SM-G973F Build/QP1A.190711.020)")
        .header("Content-Language", "en_US")
        .config().http_status_as_error(false).build()
        .header("Content-Type", "application/json")
        .send(body.to_string()).map_err(|e| lf!("Shazam недоступен: {e}", "Shazam is unavailable: {e}"))?;
    if r.status().as_u16() == 429 { return Err(l!("Shazam: слишком много запросов, попробуй через минуту", "Shazam: too many requests, try again in a minute").into()); }
    if !r.status().is_success() { return Err(lf!("Shazam ответил {}", "Shazam replied {}", r.status())); }
    let v: Value = r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).ok_or(l!("Shazam: непонятный ответ", "Shazam: unexpected response"))?;
    Ok(parse_response(&v))
}

pub fn parse_response(v: &Value) -> Option<Found> {
    let t = v.get("track")?;
    let title = t["title"].as_str()?.to_string();
    Some(Found {
        title,
        artist: t["subtitle"].as_str().unwrap_or("").to_string(),
        cover: t["images"]["coverarthq"].as_str().or(t["images"]["coverart"].as_str()).map(String::from),
        url: t["url"].as_str().map(String::from),
    })
}

/// Audio → 16 kHz mono: mix down channels, smooth (to avoid aliasing) and resample.
pub fn to_mono_16k(interleaved: &[f32], channels: usize, rate: u32) -> Vec<f32> {
    let ch = channels.max(1);
    let mono: Vec<f32> = interleaved.chunks(ch).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    if rate as usize == RATE { return mono; }
    let ratio = rate as f64 / RATE as f64;
    // moving average about `ratio` wide — a simple low-pass filter before decimation
    let win = ratio.round().max(1.0) as usize;
    let smooth: Vec<f32> = if win > 1 {
        let mut acc = 0f32;
        let mut out = Vec::with_capacity(mono.len());
        for i in 0..mono.len() {
            acc += mono[i];
            if i >= win { acc -= mono[i - win]; }
            out.push(acc / win.min(i + 1) as f32);
        }
        out
    } else { mono };
    let n = (smooth.len() as f64 / ratio) as usize;
    (0..n).map(|i| {
        let x = i as f64 * ratio;
        let j = x as usize;
        let f = (x - j as f64) as f32;
        let a = smooth[j.min(smooth.len() - 1)];
        let b = smooth[(j + 1).min(smooth.len() - 1)];
        a + (b - a) * f
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_fft() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let f = Fft::new(8);
        let mut re = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let mut im = vec![0.0; 8];
        f.run(&mut re, &mut im);
        assert!(re.iter().all(|x| (x - 1.0).abs() < 1e-6));
        // sine on the 4th bin
        let mut re: Vec<f32> = (0..64).map(|i| (2.0 * std::f32::consts::PI * 4.0 * i as f32 / 64.0).cos()).collect();
        let mut im = vec![0.0; 64];
        Fft::new(64).run(&mut re, &mut im);
        assert!((re[4] - 32.0).abs() < 1e-3 && re[5].abs() < 1e-3);
    }

    #[test]
    fn signature_of_tones_has_peaks() {
        // a chord that changes every half second — peaks should be found
        let mut s = Vec::new();
        for k in 0..(SECONDS * 2) {
            let f = [440.0, 660.0, 990.0, 1320.0, 2200.0, 3700.0][k % 6];
            for i in 0..RATE / 2 { let t = i as f32 / RATE as f32; s.push(0.3 * (2.0 * std::f32::consts::PI * f * t).sin()); }
        }
        let sig = signature(&s);
        assert_eq!(&sig[0..4], &0xcafe2580u32.to_le_bytes());
        assert!(sig.len() > 56 + 20, "пиков нет: {}", sig.len());
        assert_eq!(u32::from_le_bytes(sig[8..12].try_into().unwrap()) as usize, sig.len() - 48);
        assert_eq!(u32::from_le_bytes(sig[4..8].try_into().unwrap()), crc32(&sig[8..]));
    }

    #[test]
    fn resample() {
        let x: Vec<f32> = (0..48000 * 2).map(|i| (i % 2) as f32).collect(); // stereo 48 kHz, 1 s
        assert_eq!(to_mono_16k(&x, 2, 48000).len(), 16000);
    }
}

#[cfg(test)]
mod vector {
    /// Deterministic signal (integer-based, so it matches the Kotlin version bit for bit).
    pub fn test_signal() -> Vec<f32> {
        let mut x: u32 = 12345;
        (0..super::RATE * 4).map(|i| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = ((x >> 16) & 0x7ff) as i32 - 1024;
            let period = 16 + (i / 4000) % 40; // "melody": a square wave with a changing period
            let sq = if (i / period) % 2 == 0 { 6000 } else { -6000 };
            (sq + noise) as f32 / 32768.0
        }).collect()
    }

    /// Prints the fingerprint for comparison with Android (LogicTest.shazamMatchesDesktop).
    #[test]
    #[ignore]
    fn dump() {
        let sig = super::signature(&test_signal());
        println!("SIG {}", sig.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }
}
