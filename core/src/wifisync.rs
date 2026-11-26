//! Wi-Fi sync with the phone: the PC listens on a LAN port, the phone connects by address
//! and PIN (shown in Wavy's settings on the PC). Data uses the same format as the wavy-export.json file:
//!   GET  /wavy/sync?pin=…  → the PC's playlists and tracks
//!   POST /wavy/sync?pin=…  ← the phone's playlists and tracks (added on the PC)
//! Remote (the phone controls the PC):
//!   GET  /wavy/remote?pin=…              → what's playing, position, volume, queue
//!   POST /wavy/remote?pin=…&cmd=next&v=… ← toggle | next | prev | volume (0..1) | seek (s) | jump (index in the queue)
//!   GET  /wavy/remote/cover?pin=…        → cover of the current track (if it's a file on the PC)
use crate::store::Store;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

pub const PORT: u16 = 3738;

/// The PC's LAN address (from the outbound route; no packets are actually sent).
pub fn local_ip() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

pub fn new_pin() -> String { format!("{:06}", crate::now_ms().wrapping_mul(2_654_435_761) % 1_000_000) }

pub struct Server { srv: Arc<tiny_http::Server> }

/// Remote: snapshot of the player state (updated by the app) and where to send commands.
#[derive(Clone)]
pub struct Remote {
    pub state: Arc<Mutex<serde_json::Value>>,
    /// (command, number) → the app; `wake` wakes its UI
    pub cmds: std::sync::mpsc::Sender<(String, f64)>,
    pub wake: Arc<dyn Fn() + Send + Sync>,
    /// path to the current track's cover (a local file), if any
    pub cover: Arc<Mutex<Option<String>>>,
}

impl Drop for Server { fn drop(&mut self) { self.srv.unblock(); } }

/// Start the server. `on_sync` — after receiving data from the phone (refresh the screen).
pub fn start(store: Arc<Mutex<Store>>, pin: String, remote: Remote, on_sync: impl Fn(String) + Send + Sync + 'static) -> Result<Server, String> {
    let srv = Arc::new(tiny_http::Server::http(("0.0.0.0", PORT)).map_err(|e| lf!("порт {PORT} занят: {e}", "port {PORT} is busy: {e}"))?);
    let s2 = srv.clone();
    let on_sync = Arc::new(on_sync);
    std::thread::Builder::new().name("wifi-sync".into()).spawn(move || {
        for mut req in s2.incoming_requests() {
            let url = req.url().to_string();
            let (path, query) = url.split_once('?').unwrap_or((&url, ""));
            let ok_pin = url::form_urlencoded::parse(query.as_bytes()).any(|(k, v)| k == "pin" && v == pin.as_str());
            let json = |code: u16, body: String| tiny_http::Response::from_string(body).with_status_code(code)
                .with_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap());
            if !path.starts_with("/wavy/") { let _ = req.respond(json(404, "{}".into())); continue; }
            if !ok_pin { let _ = req.respond(json(403, r#"{"error":"неверный PIN"}"#.into())); continue; }
            if path == "/wavy/remote" {
                if *req.method() == tiny_http::Method::Post {
                    let q: std::collections::HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
                    let cmd = q.get("cmd").cloned().unwrap_or_default();
                    if !["toggle", "next", "prev", "volume", "seek", "jump"].contains(&cmd.as_str()) { let _ = req.respond(json(400, r#"{"error":"неизвестная команда"}"#.into())); continue; }
                    let v = q.get("v").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                    let _ = remote.cmds.send((cmd, v));
                    (remote.wake)();
                    // the response is the state a bit later, once the app has run the command
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                let body = remote.state.lock().unwrap().to_string();
                let _ = req.respond(json(200, body));
                continue;
            }
            if path == "/wavy/remote/cover" {
                let cover = remote.cover.lock().unwrap().clone();
                match cover.and_then(|c| std::fs::read(&c).ok()) {
                    Some(d) => { let _ = req.respond(tiny_http::Response::from_data(d).with_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"image/jpeg"[..]).unwrap())); }
                    None => { let _ = req.respond(json(404, "{}".into())); }
                }
                continue;
            }
            if path != "/wavy/sync" { let _ = req.respond(json(404, "{}".into())); continue; }
            match req.method() {
                tiny_http::Method::Get => { let body = crate::sync::export(&store.lock().unwrap()); let _ = req.respond(json(200, body)); }
                tiny_http::Method::Post => {
                    let mut text = String::new();
                    use std::io::Read;
                    if req.as_reader().take(50 * 1024 * 1024).read_to_string(&mut text).is_err() { let _ = req.respond(json(400, "{}".into())); continue; }
                    let r = crate::sync::import(&mut store.lock().unwrap(), &text);
                    match r {
                        Ok(s) => {
                            let msg = lf!("Телефон: {} треков, новых плейлистов {}, обновлено {}", "Phone: {} tracks, new playlists {}, updated {}", s.tracks, s.new_playlists, s.updated_playlists);
                            crate::log(&format!("wi-fi синхронизация: {msg}"));
                            on_sync(msg);
                            let _ = req.respond(json(200, serde_json::json!({"tracks": s.tracks, "newPlaylists": s.new_playlists, "updatedPlaylists": s.updated_playlists}).to_string()));
                        }
                        Err(e) => { let _ = req.respond(json(400, serde_json::json!({"error": e}).to_string())); }
                    }
                }
                _ => { let _ = req.respond(json(405, "{}".into())); }
            }
        }
    }).map_err(|e| e.to_string())?;
    Ok(Server { srv })
}
