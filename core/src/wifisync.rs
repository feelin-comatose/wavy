//! Синхронизация с телефоном по Wi-Fi: ПК слушает порт в локальной сети, телефон заходит по адресу
//! и PIN-коду (виден в настройках Wavy на ПК). Обмен — тем же форматом, что и файл wavy-export.json:
//!   GET  /wavy/sync?pin=…  → плейлисты и треки ПК
//!   POST /wavy/sync?pin=…  ← плейлисты и треки телефона (добавляются на ПК)
use crate::store::Store;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

pub const PORT: u16 = 3738;

/// Адрес ПК в локальной сети (по маршруту «наружу», пакеты при этом не отправляются).
pub fn local_ip() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

pub fn new_pin() -> String { format!("{:06}", crate::now_ms().wrapping_mul(2_654_435_761) % 1_000_000) }

pub struct Server { srv: Arc<tiny_http::Server> }

impl Drop for Server { fn drop(&mut self) { self.srv.unblock(); } }

/// Запустить сервер. `on_sync` — после приёма данных с телефона (обновить экран).
pub fn start(store: Arc<Mutex<Store>>, pin: String, on_sync: impl Fn(String) + Send + Sync + 'static) -> Result<Server, String> {
    let srv = Arc::new(tiny_http::Server::http(("0.0.0.0", PORT)).map_err(|e| format!("порт {PORT} занят: {e}"))?);
    let s2 = srv.clone();
    let on_sync = Arc::new(on_sync);
    std::thread::Builder::new().name("wifi-sync".into()).spawn(move || {
        for mut req in s2.incoming_requests() {
            let url = req.url().to_string();
            let (path, query) = url.split_once('?').unwrap_or((&url, ""));
            let ok_pin = url::form_urlencoded::parse(query.as_bytes()).any(|(k, v)| k == "pin" && v == pin.as_str());
            let json = |code: u16, body: String| tiny_http::Response::from_string(body).with_status_code(code)
                .with_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap());
            if path != "/wavy/sync" { let _ = req.respond(json(404, "{}".into())); continue; }
            if !ok_pin { let _ = req.respond(json(403, r#"{"error":"неверный PIN"}"#.into())); continue; }
            match req.method() {
                tiny_http::Method::Get => { let body = crate::sync::export(&store.lock().unwrap()); let _ = req.respond(json(200, body)); }
                tiny_http::Method::Post => {
                    let mut text = String::new();
                    use std::io::Read;
                    if req.as_reader().take(50 * 1024 * 1024).read_to_string(&mut text).is_err() { let _ = req.respond(json(400, "{}".into())); continue; }
                    let r = crate::sync::import(&mut store.lock().unwrap(), &text);
                    match r {
                        Ok(s) => {
                            let msg = format!("Телефон: {} треков, новых плейлистов {}, обновлено {}", s.tracks, s.new_playlists, s.updated_playlists);
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
