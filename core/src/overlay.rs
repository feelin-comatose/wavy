//! Local HTTP server for the overlay (OBS Browser Source + in-game window in Tauri).
//! SSE on /events, current state on /state, cover on /cover.
//! Also serves the addon API under /api/v1/ (token required, see docs/addons.md).
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
    /// what's next in the queue and timed lyrics (if enabled in settings)
    queue: Value,
    lyrics: Value,
    clients: Vec<Box<dyn Write + Send>>,
    /// addon API: token, full player state (set by the app) and where commands go
    api_token: String,
    api_state: Value,
    api_cmds: Option<std::sync::mpsc::Sender<(String, Value)>>,
    /// wakes the app's UI so it handles a command right away (an idle window doesn't redraw)
    api_wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Commands the addon API accepts (POST /api/v1/command).
pub const API_COMMANDS: &[&str] = &["toggle", "play", "pause", "next", "prev", "seek", "volume", "jump", "like", "add"];

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
        let srv = Arc::new(tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| lf!("порт {port} занят ({e})", "port {port} is busy ({e})"))?);
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
        if path.starts_with("/api/") { return self.api(req, &path); }
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

    /// Enable the addon API: requests must carry `token`; commands are sent to `cmds`, then `wake` is called.
    pub fn set_api(&self, token: String, cmds: std::sync::mpsc::Sender<(String, Value)>, wake: Arc<dyn Fn() + Send + Sync>) {
        let mut sh = self.shared.lock().unwrap();
        sh.api_token = token;
        sh.api_cmds = Some(cmds);
        sh.api_wake = Some(wake);
    }

    /// Full player state for GET /api/v1/state (the app refreshes it a few times a second).
    pub fn set_api_state(&self, v: Value) { self.shared.lock().unwrap().api_state = v; }

    pub fn api_url(&self) -> Option<String> { self.port.lock().unwrap().map(|p| format!("http://127.0.0.1:{p}/api/v1")) }

    fn api(&self, mut req: tiny_http::Request, path: &str) {
        let hdr = |k: &str, v: &str| tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap();
        let cors = |r: tiny_http::Response<std::io::Cursor<Vec<u8>>>| r
            .with_header(hdr("Access-Control-Allow-Origin", "*"))
            .with_header(hdr("Access-Control-Allow-Headers", "Authorization, Content-Type"))
            .with_header(hdr("Access-Control-Allow-Methods", "GET, POST, OPTIONS"));
        let reply = |code: u16, v: Value| cors(tiny_http::Response::from_string(v.to_string()).with_status_code(code).with_header(hdr("Content-Type", "application/json")));
        // browser preflight for addons that are web pages
        if *req.method() == tiny_http::Method::Options { let _ = req.respond(cors(tiny_http::Response::from_string("").with_status_code(204))); return; }
        // token: "Authorization: Bearer <token>", or ?token= for EventSource, which can't set headers
        let token = self.shared.lock().unwrap().api_token.clone();
        let from_header = req.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.as_str().trim_start_matches("Bearer ").trim().to_string());
        let from_query = req.url().split_once('?').and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("token=").map(str::to_string)));
        if token.is_empty() || (from_header.as_deref() != Some(token.as_str()) && from_query.as_deref() != Some(token.as_str())) {
            let _ = req.respond(reply(401, json!({"error": "bad or missing token"})));
            return;
        }
        match (req.method().clone(), path) {
            (tiny_http::Method::Get, "/api/v1/ping") => { let _ = req.respond(reply(200, json!({"app": "wavy", "api": 1}))); }
            (tiny_http::Method::Get, "/api/v1/state") => { let v = self.shared.lock().unwrap().api_state.clone(); let _ = req.respond(reply(200, v)); }
            (tiny_http::Method::Get, "/api/v1/events") => {
                // the same stream as the overlay's /events: one message per change of track/playing/position/queue
                let mut w = req.into_writer();
                let first = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\nConnection: keep-alive\r\n\r\ndata: {}\n\n", self.payload());
                if w.write_all(first.as_bytes()).and_then(|_| w.flush()).is_ok() { self.shared.lock().unwrap().clients.push(w); }
            }
            (tiny_http::Method::Post, "/api/v1/command") => {
                let mut body = String::new();
                use std::io::Read;
                let parsed = req.as_reader().take(64 * 1024).read_to_string(&mut body).ok().and_then(|_| serde_json::from_str::<Value>(&body).ok());
                let Some(v) = parsed else { let _ = req.respond(reply(400, json!({"error": "body must be JSON: {\"cmd\": …}"}))); return };
                let cmd = v["cmd"].as_str().unwrap_or("").to_string();
                if !API_COMMANDS.contains(&cmd.as_str()) { let _ = req.respond(reply(400, json!({"error": "unknown cmd", "commands": API_COMMANDS}))); return; }
                let (sent, wake) = { let sh = self.shared.lock().unwrap(); (sh.api_cmds.as_ref().is_some_and(|tx| tx.send((cmd, v)).is_ok()), sh.api_wake.clone()) };
                if let Some(w) = wake { w(); }
                let _ = req.respond(if sent { reply(202, json!({"ok": true})) } else { reply(503, json!({"error": "player not ready"})) });
            }
            _ => { let _ = req.respond(reply(404, json!({"error": "unknown endpoint"}))); }
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
        json!({ "track": track, "playing": sh.state.playing, "position": sh.state.position, "duration": sh.state.duration, "at": sh.at, "config": sh.config, "queue": sh.queue, "lyrics": sh.lyrics, "lang": if crate::i18n::en() { "en" } else { "ru" } })
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

    /// Upcoming tracks: [{title, artist, by}] (by — who requested it from chat).
    pub fn set_queue(&self, queue: Value) {
        let mut sh = self.shared.lock().unwrap();
        if sh.queue == queue { return; }
        sh.queue = queue;
        Self::broadcast(&mut sh);
    }

    /// Timed lyrics: [[seconds, line], …] or null.
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

    /// Snapshot for the native overlay window.
    pub fn snapshot(&self) -> (PlayerState, u64) {
        let sh = self.shared.lock().unwrap();
        (sh.state.clone(), sh.at)
    }
}

impl Default for OverlayServer { fn default() -> Self { Self::new() } }

