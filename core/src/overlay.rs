//! Локальный HTTP-сервер оверлея (OBS Browser Source + игровое окно в Tauri).
//! SSE на /events, текущее состояние на /state, обложка на /cover.
use crate::{now_ms, PlayerState};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::{Arc, Mutex};

const HTML: &str = include_str!("../../overlay/overlay.html");
const CSS: &str = include_str!("../../overlay/overlay.css");
const JS: &str = include_str!("../../overlay/overlay.js");

#[derive(Default)]
struct Shared {
    state: PlayerState,
    at: u64,
    config: Value,
    /// что дальше в очереди и текст песни со временем (если включены в настройках)
    queue: Value,
    lyrics: Value,
    clients: Vec<Box<dyn Write + Send>>,
}

#[derive(Clone)]
pub struct OverlayServer {
    shared: Arc<Mutex<Shared>>,
    pub port: Arc<Mutex<Option<u16>>>,
    server: Arc<Mutex<Option<Arc<tiny_http::Server>>>>,
}

impl OverlayServer {
    pub fn new() -> Self {
        let s = Self { shared: Default::default(), port: Default::default(), server: Default::default() };
        { let mut sh = s.shared.lock().unwrap(); sh.config = json!({ "theme": "card", "autoHideSec": 0 }); sh.queue = json!([]); }
        s
    }

    pub fn url(&self) -> Option<String> {
        self.port.lock().unwrap().map(|p| format!("http://127.0.0.1:{p}/overlay"))
    }

    pub fn start(&self, port: u16) -> Result<(), String> {
        if *self.port.lock().unwrap() == Some(port) { return Ok(()); }
        if let Some(old) = self.server.lock().unwrap().take() { old.unblock(); }
        self.shared.lock().unwrap().clients.clear();
        let srv = Arc::new(tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| format!("порт {port} занят ({e})"))?);
        *self.server.lock().unwrap() = Some(srv.clone());
        *self.port.lock().unwrap() = Some(port);
        let me = self.clone();
        std::thread::Builder::new().name("overlay-http".into()).spawn(move || {
            for req in srv.incoming_requests() { let m = me.clone(); std::thread::spawn(move || m.handle(req)); }
        }).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn handle(&self, req: tiny_http::Request) {
        let path = req.url().split('?').next().unwrap_or("/").to_string();
        let hdr = |k: &str, v: &str| tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap();
        let text = |body: &str, ct: &str| tiny_http::Response::from_string(body).with_header(hdr("Content-Type", ct)).with_header(hdr("Access-Control-Allow-Origin", "*"));
        match path.as_str() {
            "/events" => {
                let mut w = req.into_writer();
                let first = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\nConnection: keep-alive\r\n\r\ndata: {}\n\n", self.payload());
                if w.write_all(first.as_bytes()).and_then(|_| w.flush()).is_ok() {
                    self.shared.lock().unwrap().clients.push(w);
                }
            }
            "/state" => { let _ = req.respond(text(&self.payload().to_string(), "application/json")); }
            "/" | "/overlay" | "/overlay.html" => { let _ = req.respond(text(HTML, "text/html; charset=utf-8")); }
            "/overlay.css" => { let _ = req.respond(text(CSS, "text/css")); }
            "/overlay.js" => { let _ = req.respond(text(JS, "text/javascript")); }
            p if p.starts_with("/cover") => {
                let cover = self.shared.lock().unwrap().state.track.as_ref().and_then(|t| t.cover.clone()).filter(|c| !c.starts_with("http"));
                match cover.and_then(|c| std::fs::read(&c).ok().map(|d| (c, d))) {
                    Some((c, d)) => {
                        let ct = if c.ends_with(".png") { "image/png" } else { "image/jpeg" };
                        let _ = req.respond(tiny_http::Response::from_data(d).with_header(hdr("Content-Type", ct)).with_header(hdr("Access-Control-Allow-Origin", "*")));
                    }
                    None => { let _ = req.respond(tiny_http::Response::empty(404)); }
                }
            }
            _ => { let _ = req.respond(tiny_http::Response::empty(404)); }
        }
    }

    fn payload(&self) -> Value {
        let sh = self.shared.lock().unwrap();
        Self::payload_of(&sh)
    }

    fn payload_of(sh: &Shared) -> Value {
        let track = sh.state.track.as_ref().map(|t| {
            let cover = t.cover.as_ref().map(|c| if c.starts_with("http") { c.clone() } else { format!("/cover/{}?v={}", t.id, sh.at) });
            json!({ "id": t.id, "title": t.title, "artist": t.artist, "album": t.album, "cover": cover, "url": t.url, "type": t.kind })
        });
        json!({ "track": track, "playing": sh.state.playing, "position": sh.state.position, "duration": sh.state.duration, "at": sh.at, "config": sh.config, "queue": sh.queue, "lyrics": sh.lyrics })
    }

    pub fn update(&self, state: &PlayerState) {
        let mut sh = self.shared.lock().unwrap();
        sh.state = state.clone();
        sh.at = now_ms();
        Self::broadcast(&mut sh);
    }

    pub fn set_config(&self, theme: &str, auto_hide_sec: u32, show_queue: bool, show_lyrics: bool) {
        let mut sh = self.shared.lock().unwrap();
        sh.config = json!({ "theme": theme, "autoHideSec": auto_hide_sec, "showQueue": show_queue, "showLyrics": show_lyrics });
        Self::broadcast(&mut sh);
    }

    /// Следующие треки: [{title, artist, by}] (by — кто заказал из чата).
    pub fn set_queue(&self, queue: Value) {
        let mut sh = self.shared.lock().unwrap();
        if sh.queue == queue { return; }
        sh.queue = queue;
        Self::broadcast(&mut sh);
    }

    /// Текст песни со временем: [[секунды, строка], …] или null.
    pub fn set_lyrics(&self, lyrics: Value) {
        let mut sh = self.shared.lock().unwrap();
        if sh.lyrics == lyrics { return; }
        sh.lyrics = lyrics;
        Self::broadcast(&mut sh);
    }

    fn broadcast(sh: &mut Shared) {
        let msg = format!("data: {}\n\n", Self::payload_of(sh));
        sh.clients.retain_mut(|c| c.write_all(msg.as_bytes()).and_then(|_| c.flush()).is_ok());
    }

    /// Снимок для нативного оверлей-окна.
    pub fn snapshot(&self) -> (PlayerState, u64) {
        let sh = self.shared.lock().unwrap();
        (sh.state.clone(), sh.at)
    }
}

impl Default for OverlayServer { fn default() -> Self { Self::new() } }

