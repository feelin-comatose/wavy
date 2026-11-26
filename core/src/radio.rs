//! Internet radio (Icecast / SHOUTcast / plain HTTP audio streams).
//! A background thread reads the station into a small buffer and strips the ICY metadata that stations
//! mix into the audio (the current song: "StreamTitle='Artist - Song';"). The decoder reads from the
//! buffer, so a slow network can't block the audio output thread.
use crate::{now_ms, sha16, Track};
use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Buffer: enough to ride out short network hiccups (~30 s of 128 kbit/s), small enough for the memory budget.
const MAX_BUF: usize = 512 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Station {
    /// the actual audio stream (after resolving .pls/.m3u)
    pub stream_url: String,
    /// icy-name, if the station sends one
    pub name: String,
    /// decoder hint: mp3 / aac / ogg
    pub codec: String,
}

/// Decoder hint from the Content-Type.
fn codec_of(content_type: &str) -> Option<&'static str> {
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("mpeg") || ct.contains("mp3") { Some("mp3") }
    else if ct.contains("aac") || ct.contains("mp4") { Some("aac") }
    else if ct.contains("ogg") || ct.contains("opus") || ct.contains("vorbis") { Some("ogg") }
    else if ct.contains("flac") { Some("flac") }
    else { None }
}

/// First stream URL inside a .pls / .m3u playlist.
pub fn parse_playlist(text: &str) -> Option<String> {
    text.lines().map(str::trim).find_map(|l| {
        let u = l.split_once('=').filter(|(k, _)| k.to_ascii_lowercase().starts_with("file")).map(|(_, v)| v.trim()).unwrap_or(l);
        (u.starts_with("http://") || u.starts_with("https://")).then(|| u.to_string())
    })
}

/// "StreamTitle='Artist - Song';StreamUrl='';" → "Artist - Song".
pub fn parse_icy_title(meta: &str) -> Option<String> {
    let start = meta.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &meta[start..];
    let end = rest.find("';").unwrap_or(rest.len());
    let t = rest[..end].trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    crate::http().get(url).header("Icy-MetaData", "1").header("User-Agent", "Wavy").call().map_err(|e| e.to_string())
}

/// Is this link a radio station? Follows .pls/.m3u playlists. Reads only the response headers
/// (and a playlist body), not the audio. Ok(None) — not an audio stream (let yt-dlp try it).
pub fn probe(url: &str) -> Result<Option<Station>, String> {
    let mut url = url.trim().to_string();
    for _ in 0..3 {
        let mut r = get(&url)?;
        let h = |k: &str| r.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let ct = h("content-type");
        if let Some(codec) = codec_of(&ct) {
            return Ok(Some(Station { stream_url: url, name: h("icy-name").trim().to_string(), codec: codec.into() }));
        }
        let lower = url.to_ascii_lowercase();
        let is_list = ct.contains("scpls") || ct.contains("mpegurl") || lower.ends_with(".pls") || lower.ends_with(".m3u") || lower.ends_with(".m3u8");
        if !is_list { return Ok(None); }
        let text = r.body_mut().with_config().limit(64 * 1024).read_to_string().map_err(|e| e.to_string())?;
        match parse_playlist(&text) { Some(u) if u != url => url = u, _ => return Ok(None) }
    }
    Ok(None)
}

/// Library track for a station. The id comes from the link, so adding it twice doesn't duplicate it.
pub fn station_track(page_url: &str, st: &Station) -> Track {
    let host = page_url.split("://").nth(1).and_then(|r| r.split(['/', ':']).next()).unwrap_or(page_url).to_string();
    Track {
        id: format!("radio_{}", sha16(page_url.trim().as_bytes())), kind: "radio".into(), path: None, video_id: None,
        url: Some(page_url.trim().to_string()),
        title: if st.name.is_empty() { host } else { st.name.clone() },
        artist: "Radio".into(), album: String::new(), duration: 0.0, cover: None, added_at: now_ms(),
        video: false, anime: None, episode: None,
    }
}

struct Buf { data: VecDeque<u8>, ended: bool, error: Option<String> }

/// A playing station: the reader for the decoder plus the current song title.
pub struct Radio {
    shared: Arc<(Mutex<Buf>, Condvar)>,
    stop: Arc<AtomicBool>,
    /// current song from the ICY metadata ("" until the station sends one)
    pub title: Arc<Mutex<String>>,
}

impl Radio {
    /// Connect and start buffering. Waits until there's a little audio, so the decoder can probe the format.
    pub fn open(st: &Station) -> Result<Radio, String> {
        let resp = get(&st.stream_url)?;
        let metaint: usize = resp.headers().get("icy-metaint").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let mut body = resp.into_body().into_reader();
        let shared = Arc::new((Mutex::new(Buf { data: VecDeque::new(), ended: false, error: None }), Condvar::new()));
        let (stop, title) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(String::new())));
        let (sh, st2, ti) = (shared.clone(), stop.clone(), title.clone());
        std::thread::Builder::new().name("radio".into()).spawn(move || {
            let mut chunk = vec![0u8; 16 * 1024];
            let mut until_meta = metaint;
            let finish = |err: Option<String>| { let mut b = sh.0.lock().unwrap(); b.ended = true; b.error = err; sh.1.notify_all(); };
            while !st2.load(Ordering::Relaxed) {
                // audio up to the next metadata block
                let want = if metaint > 0 { until_meta.min(chunk.len()) } else { chunk.len() };
                let n = match body.read(&mut chunk[..want]) { Ok(0) => return finish(None), Ok(n) => n, Err(e) => return finish(Some(e.to_string())) };
                {
                    let mut b = sh.0.lock().unwrap();
                    // the player stalled (paused): wait instead of growing the buffer
                    while b.data.len() + n > MAX_BUF && !st2.load(Ordering::Relaxed) { b = sh.1.wait_timeout(b, Duration::from_millis(200)).unwrap().0; }
                    b.data.extend(&chunk[..n]);
                    sh.1.notify_all();
                }
                if metaint == 0 { continue; }
                until_meta -= n;
                if until_meta > 0 { continue; }
                until_meta = metaint;
                // metadata block: one length byte (×16), then text
                let mut len = [0u8; 1];
                if body.read_exact(&mut len).is_err() { return finish(None); }
                let mut meta = vec![0u8; len[0] as usize * 16];
                if body.read_exact(&mut meta).is_err() { return finish(None); }
                if let Some(t) = parse_icy_title(&String::from_utf8_lossy(&meta)) { *ti.lock().unwrap() = t; }
            }
        }).map_err(|e| e.to_string())?;
        let r = Radio { shared, stop, title };
        // ~1 s of audio before handing it to the decoder
        let (lock, cv) = &*r.shared;
        let mut b = lock.lock().unwrap();
        let t0 = std::time::Instant::now();
        while b.data.len() < 24 * 1024 && !b.ended && t0.elapsed() < Duration::from_secs(10) { b = cv.wait_timeout(b, Duration::from_millis(100)).unwrap().0; }
        if b.data.is_empty() { return Err(b.error.clone().unwrap_or_else(|| l!("станция не отвечает", "the station doesn't respond").into())); }
        drop(b);
        Ok(r)
    }

    /// Reader for the decoder: blocks briefly when the buffer is empty; EOF when the station drops or `stop`.
    pub fn reader(&self) -> RadioReader { RadioReader { shared: self.shared.clone(), stop: self.stop.clone(), pos: 0 } }
}

impl Drop for Radio {
    fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed); self.shared.1.notify_all(); }
}

pub struct RadioReader { shared: Arc<(Mutex<Buf>, Condvar)>, stop: Arc<AtomicBool>, pos: u64 }

impl Read for RadioReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let (lock, cv) = &*self.shared;
        let mut b = lock.lock().unwrap();
        let t0 = std::time::Instant::now();
        while b.data.is_empty() && !b.ended && !self.stop.load(Ordering::Relaxed) {
            // a network stall: wait up to 15 s, then end (the app can reconnect)
            if t0.elapsed() > Duration::from_secs(15) { return Ok(0); }
            b = cv.wait_timeout(b, Duration::from_millis(100)).unwrap().0;
        }
        let n = out.len().min(b.data.len());
        for (i, x) in b.data.drain(..n).enumerate() { out[i] = x; }
        cv.notify_all();
        self.pos += n as u64;
        Ok(n)
    }
}

/// The decoder needs Seek, but a live stream can't seek: only "where are we" is answered.
impl std::io::Seek for RadioReader {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        match to {
            std::io::SeekFrom::Current(0) => Ok(self.pos),
            _ => Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "live radio can't seek")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parsing() {
        assert_eq!(parse_icy_title("StreamTitle='Daft Punk - Veridis Quo';StreamUrl='';"), Some("Daft Punk - Veridis Quo".into()));
        assert_eq!(parse_icy_title("StreamTitle='';"), None);
        assert_eq!(parse_playlist("[playlist]\nNumberOfEntries=1\nFile1=http://ice.example:8000/live\nTitle1=X\n"), Some("http://ice.example:8000/live".into()));
        assert_eq!(parse_playlist("#EXTM3U\n#EXTINF:-1,Station\nhttps://s.example/stream.mp3\n"), Some("https://s.example/stream.mp3".into()));
        assert_eq!(codec_of("audio/mpeg"), Some("mp3"));
        assert_eq!(codec_of("audio/aacp"), Some("aac"));
        assert_eq!(codec_of("text/html; charset=utf-8"), None);
    }

    /// A fake Icecast server: metadata every 1000 bytes; a .pls pointing at it.
    #[test]
    fn stream_with_metadata() {
        let srv = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = srv.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in srv.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut c = c;
                    let mut req = [0u8; 2048];
                    let n = c.read(&mut req).unwrap_or(0);
                    let req = String::from_utf8_lossy(&req[..n]).to_string();
                    if req.starts_with("GET /list.pls") {
                        let body = format!("[playlist]\nFile1=http://127.0.0.1:{port}/live\n");
                        let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: audio/x-scpls\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                        return;
                    }
                    let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nicy-name: Test FM\r\nicy-metaint: 1000\r\n\r\n");
                    let meta = b"StreamTitle='Artist - Song';";
                    let mut block = vec![((meta.len() + 15) / 16) as u8];
                    block.extend(meta);
                    block.resize(1 + block[0] as usize * 16, 0);
                    for _ in 0..40 {
                        if c.write_all(&[0x55; 1000]).is_err() || c.write_all(&block).is_err() { return; }
                    }
                });
            }
        });
        let st = probe(&format!("http://127.0.0.1:{port}/list.pls")).unwrap().unwrap();
        assert_eq!((st.name.as_str(), st.codec.as_str()), ("Test FM", "mp3"));
        assert!(st.stream_url.ends_with("/live"));
        let radio = Radio::open(&st).unwrap();
        let mut audio = Vec::new();
        radio.reader().read_to_end(&mut audio).unwrap();
        // all audio bytes, none of the metadata
        assert_eq!(audio.len(), 40_000);
        assert!(audio.iter().all(|&b| b == 0x55));
        assert_eq!(radio.title.lock().unwrap().as_str(), "Artist - Song");
        assert!(probe(&format!("http://127.0.0.1:{port}/none")).is_ok());
    }
}
