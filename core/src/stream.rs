//! Progressive download: the file downloads into the cache in chunks and can be read right away —
//! the reader waits until the bytes it needs arrive. That way music starts playing from the first
//! kilobytes, and the fully downloaded file stays in the cache for replays.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// YouTube throttles large requests — download in chunks (like yt-dlp).
const CHUNK: u64 = 4 * 1024 * 1024;

#[derive(Default)]
struct State {
    downloaded: u64,
    total: Option<u64>,
    done: bool,
    error: Option<String>,
    cancel: bool,
}

#[derive(Clone)]
pub struct Download {
    shared: Arc<(Mutex<State>, Condvar)>,
    pub path: PathBuf,
}

/// The cached file is fully downloaded (an .ok marker sits next to it).
pub fn is_complete(path: &Path) -> bool {
    path.exists() && marker(path).exists()
}

fn marker(path: &Path) -> PathBuf {
    let mut m = path.as_os_str().to_owned();
    m.push(".ok");
    PathBuf::from(m)
}

/// Downloads in progress: the same file isn't downloaded twice (e.g. preload + playback).
fn active() -> &'static Mutex<std::collections::HashMap<PathBuf, Download>> {
    static A: std::sync::OnceLock<Mutex<std::collections::HashMap<PathBuf, Download>>> = std::sync::OnceLock::new();
    A.get_or_init(Default::default)
}

impl Download {
    /// Start downloading `url` into `path` (or join one already running). `size` — the size, if known.
    pub fn start(url: String, path: PathBuf, size: Option<u64>) -> Self {
        let mut act = active().lock().unwrap();
        if let Some(d) = act.get(&path) {
            let st = d.shared.0.lock().unwrap();
            if !st.cancel && st.error.is_none() { drop(st); return d.clone(); }
        }
        let _ = std::fs::remove_file(marker(&path));
        let d = Self { shared: Arc::new((Mutex::new(State { total: size, ..Default::default() }), Condvar::new())), path };
        act.insert(d.path.clone(), d.clone());
        drop(act);
        let me = d.clone();
        std::thread::Builder::new().name("stream-dl".into()).spawn(move || {
            let r = me.run(&url);
            let (m, cv) = &*me.shared;
            let mut st = m.lock().unwrap();
            match r {
                Ok(()) => { st.done = true; let _ = File::create(marker(&me.path)); }
                Err(e) => { if !st.cancel { crate::log(&format!("загрузка {}: {e}", me.path.display())); } st.error = Some(e); }
            }
            cv.notify_all();
            drop(st);
            active().lock().unwrap().remove(&me.path);
        }).ok();
        d
    }

    /// Download from a process's output (yt-dlp -o -): size unknown, the end is when the process closes its output.
    /// If the same file is already downloading, join it (the process isn't needed then and is terminated).
    pub fn pipe(mut child: std::process::Child, path: PathBuf) -> Self {
        let mut act = active().lock().unwrap();
        if let Some(d) = act.get(&path) {
            let st = d.shared.0.lock().unwrap();
            if !st.cancel && st.error.is_none() { drop(st); let _ = child.kill(); let _ = child.wait(); return d.clone(); }
        }
        let _ = std::fs::remove_file(marker(&path));
        let d = Self { shared: Arc::new((Mutex::new(State::default()), Condvar::new())), path };
        act.insert(d.path.clone(), d.clone());
        drop(act);
        let me = d.clone();
        std::thread::Builder::new().name("stream-pipe".into()).spawn(move || {
            let r = me.run_pipe(&mut child);
            let _ = child.kill();
            let status = child.wait();
            let mut err_text = || {
                let mut e = String::new();
                if let Some(mut se) = child.stderr.take() { let _ = se.read_to_string(&mut e); }
                let lines: Vec<&str> = e.lines().filter(|l| l.contains("ERROR")).collect();
                if lines.is_empty() { e.trim().to_string() } else { lines.join("\n") }
            };
            let (m, cv) = &*me.shared;
            let mut st = m.lock().unwrap();
            let ok = r.is_ok() && st.downloaded > 0 && status.as_ref().is_ok_and(|s| s.success());
            if ok {
                st.done = true;
                st.total = Some(st.downloaded);
                let _ = File::create(marker(&me.path));
            } else {
                let e = r.err().unwrap_or_else(|| { let t = err_text(); if t.is_empty() { l!("yt-dlp ничего не отдал", "yt-dlp returned nothing").into() } else { t } });
                if !st.cancel { crate::log(&format!("поток yt-dlp {}: {e}", me.path.display())); }
                st.error = Some(e);
            }
            cv.notify_all();
            drop(st);
            active().lock().unwrap().remove(&me.path);
        }).ok();
        d
    }

    fn run_pipe(&self, child: &mut std::process::Child) -> Result<(), String> {
        let mut out = child.stdout.take().ok_or(l!("нет вывода", "no output"))?;
        let mut f = File::create(&self.path).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if self.shared.0.lock().unwrap().cancel { return Err(l!("отменено", "cancelled").into()); }
            let n = out.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 { break; }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            let (m, cv) = &*self.shared;
            m.lock().unwrap().downloaded += n as u64;
            cv.notify_all();
        }
        f.flush().map_err(|e| e.to_string())
    }

    fn run(&self, url: &str) -> Result<(), String> {
        let mut f = File::create(&self.path).map_err(|e| e.to_string())?;
        let mut pos = 0u64;
        loop {
            let total = self.shared.0.lock().unwrap().total;
            if total.is_some_and(|t| pos >= t) { break; }
            let end = total.map(|t| (pos + CHUNK).min(t) - 1).unwrap_or(pos + CHUNK - 1);
            let mut attempt = 0;
            let n = loop {
                if self.shared.0.lock().unwrap().cancel { return Err(l!("отменено", "cancelled").into()); }
                match self.fetch(url, pos, end, &mut f) {
                    Ok(n) => break n,
                    // 403/404/410 — the link is invalid, retrying won't help
                    Err(e) if attempt < 3 && !["403", "404", "410"].iter().any(|c| e.contains(c)) => { attempt += 1; crate::log(&format!("загрузка: повтор {attempt} ({e})")); std::thread::sleep(Duration::from_millis(400 * attempt)); }
                    Err(e) => return Err(e),
                }
            };
            pos += n;
            if n == 0 { break; } // the server sent everything
            if total.is_none() && n < end - (pos - n) + 1 { break; } // size unknown and the chunk is shorter — end
        }
        f.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// One chunk: a Range request, written to the file as data arrives.
    fn fetch(&self, url: &str, start: u64, end: u64, f: &mut File) -> Result<u64, String> {
        let resp = crate::http().get(url)
            .header("Range", &format!("bytes={start}-{end}"))
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0")
            .call().map_err(|e| e.to_string())?;
        if let Some(cr) = resp.headers().get("content-range").and_then(|v| v.to_str().ok()) {
            if let Some(t) = cr.rsplit('/').next().and_then(|t| t.parse::<u64>().ok()) {
                self.shared.0.lock().unwrap().total.get_or_insert(t);
            }
        } else if start == 0 {
            if let Some(len) = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()) {
                self.shared.0.lock().unwrap().total.get_or_insert(len);
            }
        }
        f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let mut r = resp.into_body().into_reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut got = 0u64;
        loop {
            if self.shared.0.lock().unwrap().cancel { return Err(l!("отменено", "cancelled").into()); }
            let n = r.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 { break; }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            got += n as u64;
            let (m, cv) = &*self.shared;
            m.lock().unwrap().downloaded = start + got;
            cv.notify_all();
        }
        Ok(got)
    }

    pub fn cancel(&self) {
        let (m, cv) = &*self.shared;
        m.lock().unwrap().cancel = true;
        cv.notify_all();
    }

    pub fn done(&self) -> bool { self.shared.0.lock().unwrap().done }
    pub fn error(&self) -> Option<String> { self.shared.0.lock().unwrap().error.clone() }
    pub fn total(&self) -> Option<u64> { self.shared.0.lock().unwrap().total }

    /// How much is downloaded, in percent (0..=100), if the size is known.
    pub fn progress(&self) -> Option<u8> {
        let st = self.shared.0.lock().unwrap();
        st.total.filter(|t| *t > 0).map(|t| ((st.downloaded * 100) / t).min(100) as u8)
    }

    /// Wait until at least `bytes` (or everything) is downloaded. false — error/cancel/timeout.
    pub fn wait_for(&self, bytes: u64, timeout: Duration) -> bool {
        let (m, cv) = &*self.shared;
        let deadline = std::time::Instant::now() + timeout;
        let mut st = m.lock().unwrap();
        loop {
            if st.done || st.downloaded >= bytes || st.total.is_some_and(|t| st.downloaded >= t) { return true; }
            if st.error.is_some() || st.cancel { return false; }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() { return false; }
            st = cv.wait_timeout(st, left).unwrap().0;
        }
    }

    /// A reader that waits for the bytes it needs to arrive.
    pub fn reader(&self) -> std::io::Result<StreamReader> {
        Ok(StreamReader { dl: self.clone(), file: File::open(&self.path)?, pos: 0 })
    }
}

pub struct StreamReader {
    dl: Download,
    file: File,
    pos: u64,
}

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let (m, cv) = &*self.dl.shared;
        let mut st = m.lock().unwrap();
        loop {
            if st.downloaded > self.pos { break; }
            if st.done || st.total.is_some_and(|t| self.pos >= t) { return Ok(0); }
            if st.cancel || st.error.is_some() { return Err(std::io::Error::other(st.error.clone().unwrap_or_else(|| l!("отменено", "cancelled").into()))); }
            st = cv.wait_timeout(st, Duration::from_secs(30)).unwrap().0;
        }
        let avail = (st.downloaded - self.pos).min(buf.len() as u64) as usize;
        drop(st);
        self.file.seek(SeekFrom::Start(self.pos))?;
        let n = self.file.read(&mut buf[..avail])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let total = self.dl.total();
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => (self.pos as i64 + d).max(0) as u64,
            SeekFrom::End(d) => (total.ok_or_else(|| std::io::Error::other(l!("размер неизвестен", "size unknown")))? as i64 + d).max(0) as u64,
        };
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Mini HTTP server with Range and artificial delay — checks that reading works before the download finishes.
    #[test]
    fn progressive_read() {
        let data: Vec<u8> = (0..10_000_000u32).map(|i| (i % 251) as u8).collect();
        let srv = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = srv.server_addr().to_ip().unwrap().port();
        let d2 = data.clone();
        std::thread::spawn(move || for req in srv.incoming_requests() {
            let r = req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.as_str().to_string()).unwrap();
            let (a, b) = r.trim_start_matches("bytes=").split_once('-').unwrap();
            let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse::<usize>().unwrap().min(d2.len() - 1));
            std::thread::sleep(Duration::from_millis(150)); // "slow network"
            let hdr = tiny_http::Header::from_bytes("Content-Range", format!("bytes {a}-{b}/{}", d2.len())).unwrap();
            let _ = req.respond(tiny_http::Response::from_data(d2[a..=b].to_vec()).with_status_code(206).with_header(hdr));
        });
        let path = std::env::temp_dir().join(format!("wavy-stream-test-{}", crate::now_ms()));
        let dl = Download::start(format!("http://127.0.0.1:{port}/x"), path.clone(), None);
        // the first bytes are available long before the download finishes
        assert!(dl.wait_for(100_000, Duration::from_secs(5)));
        assert!(!dl.done());
        let mut r = dl.reader().unwrap();
        let mut head = vec![0u8; 100_000];
        r.read_exact(&mut head).unwrap();
        assert_eq!(&head[..], &data[..100_000]);
        // seeking forward — wait for the needed chunk
        r.seek(SeekFrom::Start(9_000_000)).unwrap();
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).unwrap();
        assert_eq!(&tail[..], &data[9_000_000..]);
        assert!(dl.wait_for(u64::MAX, Duration::from_secs(10)));
        // the "done" flag and the .ok marker are set right after the last byte — a bit later under load
        let t0 = std::time::Instant::now();
        while !(dl.done() && is_complete(&path)) && t0.elapsed() < Duration::from_secs(10) { std::thread::sleep(Duration::from_millis(20)); }
        assert!(dl.done() && is_complete(&path));
        assert_eq!(std::fs::read(&path).unwrap(), data);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(marker(&path));
    }
}

/// Size of the cache folder (bytes) and number of files.
pub fn dir_size(dir: &std::path::Path) -> (u64, usize) {
    std::fs::read_dir(dir).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).fold((0, 0), |(s, n), m| (s + m.len(), n + 1))).unwrap_or((0, 0))
}

/// Delete old files (by last modification time) until the cache is smaller than `limit` bytes.
/// `keep` — files needed right now (playing/downloading), left alone. Returns what was freed.
pub fn trim_dir(dir: &std::path::Path, limit: u64, keep: &[std::path::PathBuf]) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut files: Vec<(std::path::PathBuf, u64, std::time::SystemTime)> = rd.flatten()
        .filter_map(|e| { let m = e.metadata().ok()?; m.is_file().then(|| (e.path(), m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH))) }).collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.2);
    let mut freed = 0;
    for (p, len, _) in files {
        if total <= limit { break; }
        let stem = p.file_stem().map(|s| s.to_string_lossy().split('.').next().unwrap_or("").to_string()).unwrap_or_default();
        if keep.iter().any(|k| k.file_stem().map(|s| s.to_string_lossy().split('.').next().unwrap_or("").to_string()).as_deref() == Some(stem.as_str())) { continue; }
        if std::fs::remove_file(&p).is_ok() { total -= len; freed += len; }
    }
    freed
}

#[cfg(test)]
mod trim_tests {
    #[test]
    fn trims_oldest_first() {
        let d = std::env::temp_dir().join(format!("wavy-trim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (i, n) in ["a.m4a", "b.m4a", "c.m4a"].iter().enumerate() {
            std::fs::write(d.join(n), vec![0u8; 1000]).unwrap();
            let t = std::time::SystemTime::now() - std::time::Duration::from_secs(1000 - i as u64 * 100);
            std::fs::File::options().write(true).open(d.join(n)).unwrap().set_modified(t).unwrap();
        }
        // a is the oldest but it's "playing": b should be deleted
        let freed = super::trim_dir(&d, 2000, &[d.join("a.m4a")]);
        assert_eq!(freed, 1000);
        assert!(d.join("a.m4a").exists() && !d.join("b.m4a").exists() && d.join("c.m4a").exists());
        assert_eq!(super::dir_size(&d), (2000, 2));
        let _ = std::fs::remove_dir_all(&d);
    }
}
