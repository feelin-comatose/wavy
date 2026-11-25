//! Аккаунт YouTube Music через куки: лайки, история, сохранённые плейлисты и «Моя волна»
//! (радио YouTube Music по твоим лайкам и истории — под аккаунтом оно подбирается по вкусу).
//!
//! Куки берутся одним из способов:
//! - «Войти через Google»: Wavy открывает отдельное окно Chrome/Edge/Brave с чистым профилем,
//!   человек входит как обычно, Wavy забирает куки через DevTools этого окна и закрывает его.
//!   Этим профилем больше никто не пользуется, поэтому YouTube не меняет куки у нас за спиной;
//! - из браузера (Firefox) или файла cookies.txt — те же настройки, что для видео 18+.
//!
//! Все функции блокирующие — вызывай из фонового потока.
use crate::youtube::{rt, ytm_track, YouTube};
use crate::{now_ms, Track};
use rustypipe::client::RustyPipe;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Где открыть вход в Google (сразу ведёт обратно на YouTube Music).
const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";

/// Куки, по которым видно, что в YouTube выполнен вход.
const LOGIN_COOKIES: &[&str] = &["SAPISID", "__Secure-3PAPISID", "LOGIN_INFO"];

/// Плейлист «Мне нравится» в YouTube Music.
pub const LIKED_ID: &str = "LM";

// ---------- вход через Google в отдельном окне браузера ----------

/// Браузер на Chromium, который умеет отдавать куки через DevTools.
pub fn find_browser() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            let Ok(base) = std::env::var(var) else { continue };
            let base = PathBuf::from(base);
            c.push(base.join(r"Google\Chrome\Application\chrome.exe"));
            c.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
            c.push(base.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
            c.push(base.join(r"Chromium\Application\chrome.exe"));
        }
    } else {
        for name in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "brave-browser"] {
            for dir in std::env::var("PATH").unwrap_or_default().split(':') {
                c.push(Path::new(dir).join(name));
            }
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// Открытое окно входа.
pub struct GoogleLogin {
    child: Child,
    port: u16,
}

impl GoogleLogin {
    /// Открыть окно входа: отдельный профиль `profile` (там же он и остаётся до выхода).
    pub fn start(browser: &Path, profile: &Path) -> Result<Self, String> {
        let _ = std::fs::create_dir_all(profile);
        let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(browser);
        // Chromium не запускается от root без этого флага (бывает только на Linux-серверах и в контейнерах)
        if cfg!(unix) && std::env::var("USER").is_ok_and(|u| u == "root") { cmd.arg("--no-sandbox"); }
        let child = cmd
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--remote-debugging-port={port}"))
            .arg("--remote-allow-origins=http://127.0.0.1")
            .args(["--no-first-run", "--no-default-browser-check", "--new-window", LOGIN_URL])
            .spawn()
            .map_err(|e| lf!("браузер не запустился: {e}", "the browser didn't start: {e}"))?;
        Ok(Self { child, port })
    }

    /// Закрыли ли окно сами.
    pub fn closed(&mut self) -> bool { matches!(self.child.try_wait(), Ok(Some(_))) }

    /// Куки окна в формате cookies.txt, когда вход в YouTube уже выполнен (иначе None).
    pub fn poll(&self) -> Result<Option<String>, String> {
        let cookies = cdp(self.port, "Storage.getCookies", json!({}))?;
        let list = cookies["cookies"].as_array().cloned().unwrap_or_default();
        let logged_in = list.iter().any(|c| c["domain"].as_str().is_some_and(|d| d.ends_with("youtube.com")) && LOGIN_COOKIES.contains(&c["name"].as_str().unwrap_or("")));
        Ok(logged_in.then(|| netscape(&list)))
    }

    /// Закрыть окно (сначала вежливо, через DevTools).
    pub fn close(mut self) {
        let _ = cdp(self.port, "Browser.close", json!({}));
        for _ in 0..20 {
            if self.closed() { return; }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = self.child.kill();
    }
}

/// Одна команда Chrome DevTools Protocol на уровне браузера.
fn cdp(port: u16, method: &str, params: Value) -> Result<Value, String> {
    let info: Value = serde_json::from_str(&ureq::get(&format!("http://127.0.0.1:{port}/json/version")).call().map_err(|e| e.to_string())?
        .body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let ws = info["webSocketDebuggerUrl"].as_str().ok_or("DevTools: нет webSocketDebuggerUrl")?;
    let (mut sock, _) = tungstenite::connect(ws).map_err(|e| format!("DevTools: {e}"))?;
    sock.send(tungstenite::Message::text(json!({"id": 1, "method": method, "params": params}).to_string())).map_err(|e| e.to_string())?;
    loop {
        let msg = sock.read().map_err(|e| e.to_string())?;
        let Ok(text) = msg.to_text() else { continue };
        let Ok(v) = serde_json::from_str::<Value>(text) else { continue };
        if v["id"] == 1 {
            let _ = sock.close(None);
            if let Some(e) = v.get("error") { return Err(format!("DevTools: {e}")); }
            return Ok(v["result"].clone());
        }
    }
}

/// Куки DevTools → файл cookies.txt (Netscape), только Google и YouTube.
pub fn netscape(cookies: &[Value]) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n# Wavy: вход через Google\n");
    for c in cookies {
        let domain = c["domain"].as_str().unwrap_or("");
        if !(domain.ends_with("youtube.com") || domain.ends_with("google.com")) { continue; }
        let exp = c["expires"].as_f64().filter(|e| *e > 0.0).unwrap_or(0.0) as i64;
        out += &format!("{domain}\t{}\t{}\t{}\t{exp}\t{}\t{}\n",
            if domain.starts_with('.') { "TRUE" } else { "FALSE" },
            c["path"].as_str().unwrap_or("/"),
            if c["secure"].as_bool().unwrap_or(false) { "TRUE" } else { "FALSE" },
            c["name"].as_str().unwrap_or(""), c["value"].as_str().unwrap_or(""));
    }
    out
}

// ---------- аккаунт ----------

impl YouTube {
    /// Подключить аккаунт по кукам из настроек (браузер или файл cookies.txt).
    pub fn account_connect(&self) -> Result<(), String> {
        let txt = self.cookies_txt()?;
        let dir = self.bin_dir.parent().unwrap_or(&self.bin_dir).join("rustypipe-account");
        let _ = std::fs::create_dir_all(&dir);
        let rp = RustyPipe::builder().storage_dir(dir).build().map_err(|e| e.to_string())?;
        rt().block_on(rp.user_auth_set_cookie_txt(&txt)).map_err(|e| {
            crate::log(&format!("аккаунт YouTube: {e}"));
            l!("в куках нет входа в YouTube (или вход устарел) — войди заново", "the cookies have no YouTube sign-in (or it expired) — sign in again").to_string()
        })?;
        *self.acc.lock().unwrap() = Some(rp);
        Ok(())
    }

    pub fn account_disconnect(&self) {
        if let Some(rp) = self.acc.lock().unwrap().take() { let _ = rt().block_on(rp.user_auth_remove_cookie()); }
    }

    pub fn account_connected(&self) -> bool { self.acc.lock().unwrap().is_some() }

    fn acc(&self) -> Result<RustyPipe, String> {
        self.acc.lock().unwrap().clone().ok_or_else(|| l!("аккаунт YouTube не подключён", "the YouTube account isn't connected").into())
    }

    /// Текст cookies.txt: из файла как есть, из браузера — через yt-dlp (он умеет их читать и расшифровывать).
    fn cookies_txt(&self) -> Result<String, String> {
        let args = self.cookie_args();
        if args.is_empty() { return Err(l!("сначала войди через Google или укажи, откуда брать куки", "first sign in with Google or choose where to take cookies from").into()); }
        if args[0] == "--cookies" { return std::fs::read_to_string(&args[1]).map_err(|e| lf!("не читается файл кук: {e}", "can't read the cookie file: {e}")); }
        let tmp = std::env::temp_dir().join(format!("wavy-cookies-{}.txt", now_ms()));
        let path = tmp.to_string_lossy().to_string();
        // yt-dlp сохраняет куки в файл при выходе, даже если само видео не открылось
        let _ = self.run(&[&args[0], &args[1], "--cookies", &path, "--skip-download", "--simulate", "--no-warnings", "--playlist-items", "0", "https://www.youtube.com/watch?v=jNQXAC9IVRw"]);
        let txt = std::fs::read_to_string(&tmp).map_err(|_| l!("не вышло достать куки из браузера — закрой его и попробуй ещё раз", "couldn't get the cookies from the browser — close it and try again").to_string());
        let _ = std::fs::remove_file(&tmp);
        txt
    }

    /// Лайкнутые песни YouTube Music (новые сверху).
    pub fn liked_tracks(&self, limit: usize) -> Result<Vec<Track>, String> { self.playlist_tracks(LIKED_ID, limit) }

    /// Треки плейлиста аккаунта (в том числе закрытого).
    pub fn playlist_tracks(&self, id: &str, limit: usize) -> Result<Vec<Track>, String> {
        let rp = self.acc()?;
        let q = rp.query().authenticated();
        rt().block_on(async {
            let mut pl = q.music_playlist(id).await?;
            pl.tracks.extend_limit(&q, limit).await?;
            Ok::<_, rustypipe::error::Error>(pl.tracks.items.into_iter().take(limit).map(ytm_track).collect())
        }).map_err(|e| format!("YouTube Music: {e}"))
    }

    /// Сохранённые плейлисты: (id, название, сколько треков).
    pub fn saved_playlists(&self) -> Result<Vec<(String, String, u64)>, String> {
        let rp = self.acc()?;
        let q = rp.query().authenticated();
        rt().block_on(async {
            let mut p = q.music_saved_playlists().await?;
            p.extend_limit(&q, 200).await?;
            Ok::<_, rustypipe::error::Error>(p.items.into_iter().map(|p| (p.id, p.name, p.track_count.unwrap_or(0))).collect())
        }).map_err(|e| format!("YouTube Music: {e}"))
    }

    /// Недавно прослушанное в YouTube Music.
    pub fn history(&self, limit: usize) -> Result<Vec<Track>, String> {
        let rp = self.acc()?;
        let q = rp.query().authenticated();
        rt().block_on(async {
            let h = q.music_history().await?;
            Ok::<_, rustypipe::error::Error>(h.items.into_iter().take(limit).map(|h| ytm_track(h.item)).collect())
        }).map_err(|e| format!("YouTube Music: {e}"))
    }

    /// Радио YouTube Music по треку — под аккаунтом с учётом вкуса.
    fn radio_for(&self, video_id: &str, limit: usize) -> Result<Vec<Track>, String> {
        let rp = self.acc()?;
        let q = rp.query().authenticated();
        rt().block_on(async {
            let r = q.music_radio_track(video_id).await?;
            Ok::<_, rustypipe::error::Error>(r.items.into_iter().take(limit).map(ytm_track).collect())
        }).map_err(|e| format!("YouTube Music: {e}"))
    }

    /// «Моя волна»: радио по нескольким трекам из лайков и истории, вперемешку и без того,
    /// что уже есть в `exclude` (id видео библиотеки и уже сыгранного). `seeds` — свои стартовые треки
    /// (например, последние из прошлой волны), иначе берутся из аккаунта.
    pub fn wave(&self, seeds: &[String], exclude: &HashSet<String>, n: usize) -> Result<Vec<Track>, String> {
        let mut seeds: Vec<String> = seeds.to_vec();
        if seeds.len() < 3 {
            let hist = self.history(20).unwrap_or_default();
            let liked = self.liked_tracks(60).unwrap_or_default();
            if hist.is_empty() && liked.is_empty() { return Err(l!("в аккаунте пока нет лайков и истории — послушай что-нибудь в YouTube Music", "the account has no likes or history yet — play something in YouTube Music").into()); }
            let mut r = Rng(now_ms() | 1);
            seeds.extend(r.pick(&hist, 3).into_iter().chain(r.pick(&liked, 3)).filter_map(|t| t.video_id));
        }
        seeds.dedup();
        let lists: Vec<Vec<Track>> = seeds.iter().take(6).filter_map(|v| self.radio_for(v, 25).map_err(|e| crate::log(&format!("волна: радио {v}: {e}"))).ok()).collect();
        if lists.is_empty() { return Err(l!("YouTube Music не отдал радио — попробуй позже или войди заново", "YouTube Music returned no radio — try later or sign in again").into()); }
        let mut skip = exclude.clone();
        skip.extend(seeds);
        Ok(mix(lists, &skip, n))
    }
}

/// Списки радио → одна очередь: по очереди из каждого списка, без повторов и исключённых.
pub fn mix(lists: Vec<Vec<Track>>, exclude: &HashSet<String>, n: usize) -> Vec<Track> {
    let mut seen = exclude.clone();
    let mut out = Vec::new();
    let mut iters: Vec<_> = lists.into_iter().map(|l| l.into_iter()).collect();
    loop {
        let mut any = false;
        for it in iters.iter_mut() {
            let Some(t) = it.next() else { continue };
            any = true;
            let vid = t.video_id.clone().unwrap_or_default();
            if vid.is_empty() || !seen.insert(vid) { continue; }
            out.push(t);
            if out.len() >= n { return out; }
        }
        if !any { return out; }
    }
}

/// Простой генератор случайных чисел (без лишней зависимости ради выбора нескольких треков).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
    /// k случайных элементов (без повторов), из первых по порядку чаще.
    fn pick<T: Clone>(&mut self, v: &[T], k: usize) -> Vec<T> {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        let mut out = Vec::new();
        while out.len() < k && !idx.is_empty() {
            let i = (self.next() as usize) % idx.len();
            out.push(v[idx.remove(i)].clone());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(v: &str) -> Track {
        Track { id: format!("yt_{v}"), kind: "youtube".into(), path: None, video_id: Some(v.into()), url: None, title: v.into(), artist: String::new(),
            album: String::new(), duration: 0.0, cover: None, added_at: 0, video: false, anime: None, episode: None }
    }
    #[test]
    fn mixes_round_robin_without_repeats() {
        let a = vec![t("a1"), t("x"), t("a2")];
        let b = vec![t("b1"), t("x"), t("lib")];
        let ex: HashSet<String> = ["lib".to_string()].into();
        let ids: Vec<String> = mix(vec![a, b], &ex, 10).into_iter().map(|t| t.title).collect();
        assert_eq!(ids, ["a1", "b1", "x", "a2"]);
        assert_eq!(mix(vec![vec![t("1"), t("2"), t("3")]], &HashSet::new(), 2).len(), 2);
    }
    #[test]
    fn cookie_file() {
        let c = vec![
            json!({"domain": ".youtube.com", "path": "/", "secure": true, "expires": 1900000000.5, "name": "SAPISID", "value": "abc"}),
            json!({"domain": "example.com", "path": "/", "secure": false, "expires": -1, "name": "x", "value": "y"}),
        ];
        let s = netscape(&c);
        assert!(s.contains(".youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tabc\n"));
        assert!(!s.contains("example.com"));
    }
}
