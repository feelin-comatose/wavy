//! Потоковая загрузка: файл качается кусками в кэш, а читать его можно сразу —
//! читатель ждёт, пока нужные байты докачаются. Так музыка начинает играть с первых
//! килобайт, а целиком скачанный файл остаётся в кэше для повторного прослушивания.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// YouTube режет скорость на больших запросах — качаем кусками (как yt-dlp).
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

/// Файл в кэше докачан до конца (рядом лежит метка .ok).
pub fn is_complete(path: &Path) -> bool {
    path.exists() && marker(path).exists()
}

fn marker(path: &Path) -> PathBuf {
    let mut m = path.as_os_str().to_owned();
    m.push(".ok");
    PathBuf::from(m)
}

/// Идущие загрузки: один и тот же файл не качается дважды (например, предзагрузка + воспроизведение).
fn active() -> &'static Mutex<std::collections::HashMap<PathBuf, Download>> {
    static A: std::sync::OnceLock<Mutex<std::collections::HashMap<PathBuf, Download>>> = std::sync::OnceLock::new();
    A.get_or_init(Default::default)
}

impl Download {
    /// Начать загрузку `url` в `path` (или присоединиться к уже идущей). `size` — размер, если известен.
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

    /// Загрузка из вывода процесса (yt-dlp -o -): размер неизвестен, конец — когда процесс закрыл вывод.
    /// Если идёт загрузка этого же файла — присоединяемся к ней (процесс тогда не нужен и завершается).
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
                let e = r.err().unwrap_or_else(|| { let t = err_text(); if t.is_empty() { "yt-dlp ничего не отдал".into() } else { t } });
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
        let mut out = child.stdout.take().ok_or("нет вывода")?;
        let mut f = File::create(&self.path).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if self.shared.0.lock().unwrap().cancel { return Err("отменено".into()); }
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
                if self.shared.0.lock().unwrap().cancel { return Err("отменено".into()); }
                match self.fetch(url, pos, end, &mut f) {
                    Ok(n) => break n,
                    // 403/404/410 — ссылка недействительна, повтор не поможет
                    Err(e) if attempt < 3 && !["403", "404", "410"].iter().any(|c| e.contains(c)) => { attempt += 1; crate::log(&format!("загрузка: повтор {attempt} ({e})")); std::thread::sleep(Duration::from_millis(400 * attempt)); }
                    Err(e) => return Err(e),
                }
            };
            pos += n;
            if n == 0 { break; } // сервер отдал всё
            if total.is_none() && n < end - (pos - n) + 1 { break; } // размер неизвестен и кусок короче — конец
        }
        f.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Один кусок: Range-запрос, запись в файл по мере прихода данных.
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
            if self.shared.0.lock().unwrap().cancel { return Err("отменено".into()); }
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

    /// Сколько скачано, в процентах (0..=100), если размер известен.
    pub fn progress(&self) -> Option<u8> {
        let st = self.shared.0.lock().unwrap();
        st.total.filter(|t| *t > 0).map(|t| ((st.downloaded * 100) / t).min(100) as u8)
    }

    /// Подождать, пока скачается хотя бы `bytes` (или всё). false — ошибка/отмена/таймаут.
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

    /// Читатель, который ждёт докачки нужных байт.
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
            if st.cancel || st.error.is_some() { return Err(std::io::Error::other(st.error.clone().unwrap_or_else(|| "отменено".into()))); }
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
            SeekFrom::End(d) => (total.ok_or_else(|| std::io::Error::other("размер неизвестен"))? as i64 + d).max(0) as u64,
        };
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Мини HTTP-сервер с Range и искусственной задержкой — проверяем, что читать можно до конца загрузки.
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
            std::thread::sleep(Duration::from_millis(150)); // «медленная сеть»
            let hdr = tiny_http::Header::from_bytes("Content-Range", format!("bytes {a}-{b}/{}", d2.len())).unwrap();
            let _ = req.respond(tiny_http::Response::from_data(d2[a..=b].to_vec()).with_status_code(206).with_header(hdr));
        });
        let path = std::env::temp_dir().join(format!("wavy-stream-test-{}", crate::now_ms()));
        let dl = Download::start(format!("http://127.0.0.1:{port}/x"), path.clone(), None);
        // первые байты доступны задолго до конца загрузки
        assert!(dl.wait_for(100_000, Duration::from_secs(5)));
        assert!(!dl.done());
        let mut r = dl.reader().unwrap();
        let mut head = vec![0u8; 100_000];
        r.read_exact(&mut head).unwrap();
        assert_eq!(&head[..], &data[..100_000]);
        // перемотка вперёд — ждём нужный кусок
        r.seek(SeekFrom::Start(9_000_000)).unwrap();
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).unwrap();
        assert_eq!(&tail[..], &data[9_000_000..]);
        assert!(dl.wait_for(u64::MAX, Duration::from_secs(10)) && dl.done() && is_complete(&path));
        assert_eq!(std::fs::read(&path).unwrap(), data);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(marker(&path));
    }
}
