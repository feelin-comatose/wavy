//! YouTube / YouTube Music / SoundCloud / … через yt-dlp.
//! yt-dlp ищется в PATH, иначе один раз скачивается в data_dir/bin.
//! Все функции блокирующие — вызывай из фонового потока.
use crate::{now_ms, sha16, Track};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

const RELEASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";

pub struct YouTube {
    bin_dir: PathBuf,
    bin: Mutex<Option<String>>,
    streams: Mutex<HashMap<String, (String, u64)>>, // url -> (stream, expires_ms)
    deno: Arc<Mutex<Deno>>,
    rp: std::sync::OnceLock<Option<rustypipe::client::RustyPipe>>,
    /// куки для видео 18+ (браузер или файл cookies.txt) — только для повторной попытки такого видео
    cookies: Mutex<String>,
}

/// Браузеры, из которых yt-dlp умеет брать куки.
pub const COOKIE_BROWSERS: &[(&str, &str)] = &[("firefox", "Firefox"), ("chrome", "Chrome"), ("edge", "Edge"), ("brave", "Brave"), ("opera", "Opera"), ("vivaldi", "Vivaldi"), ("chromium", "Chromium")];

/// Прямая ссылка на поток (без скачивания): адрес и размер, если известен.
#[derive(Clone, Debug)]
pub struct Direct {
    pub url: String,
    pub size: Option<u64>,
}

/// Общий tokio-рантайм для rustypipe (один рабочий поток — экономим память).
fn rt() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).thread_name("rustypipe").enable_all().build().expect("tokio"))
}

/// JS-движок для yt-dlp: без него YouTube постепенно перестаёт отдавать форматы.
#[derive(Clone, Debug)]
pub enum Deno {
    Unknown,
    Installing,
    Ready(String),
    Failed(String),
}

pub struct UrlImport {
    pub name: Option<String>,
    pub source: String,
    pub tracks: Vec<Track>,
}

impl YouTube {
    pub fn new(bin_dir: PathBuf) -> Self {
        Self { bin_dir, bin: Mutex::new(None), streams: Mutex::new(HashMap::new()), deno: Arc::new(Mutex::new(Deno::Unknown)), rp: std::sync::OnceLock::new(), cookies: Mutex::new(String::new()) }
    }

    pub fn deno_status(&self) -> Deno { self.deno.lock().unwrap().clone() }

    /// Куки YouTube: имя браузера или путь к cookies.txt (пусто — без кук).
    pub fn set_cookies(&self, v: &str) { *self.cookies.lock().unwrap() = v.trim().to_string(); }
    pub fn has_cookies(&self) -> bool { !self.cookies.lock().unwrap().is_empty() }

    fn cookie_args(&self) -> Vec<String> {
        let v = self.cookies.lock().unwrap().clone();
        if v.is_empty() { return vec![]; }
        if COOKIE_BROWSERS.iter().any(|b| b.0 == v) { vec!["--cookies-from-browser".into(), v] } else { vec!["--cookies".into(), v] }
    }

    fn rustypipe(&self) -> Result<&rustypipe::client::RustyPipe, String> {
        self.rp.get_or_init(|| {
            let dir = self.bin_dir.parent().unwrap_or(&self.bin_dir).join("rustypipe");
            let _ = std::fs::create_dir_all(&dir);
            match rustypipe::client::RustyPipe::builder().storage_dir(dir).build() {
                Ok(c) => Some(c),
                Err(e) => { crate::log(&format!("rustypipe: {e}")); None }
            }
        }).as_ref().ok_or_else(|| "rustypipe недоступен".to_string())
    }

    /// Быстрый путь без yt-dlp: прямая ссылка через rustypipe (0.2–1 с).
    /// video=false — звук m4a (AAC), video=true — видео без звука H.264 до 720p.
    pub fn direct(&self, video_id: &str, video: bool) -> Result<Direct, String> {
        use rustypipe::model::{AudioFormat, VideoCodec, VideoFormat};
        use rustypipe::param::StreamFilter;
        let key = format!("{}:{video_id}", if video { "v" } else { "a" });
        if let Some((u, exp)) = self.streams.lock().unwrap().get(&key) {
            if *exp > now_ms() + 60_000 { return Ok(serde_json::from_str::<(String, Option<u64>)>(u).map(|(url, size)| Direct { url, size }).map_err(|e| e.to_string())?); }
        }
        let rp = self.rustypipe()?;
        let t = std::time::Instant::now();
        let player = rt().block_on(rp.query().player(video_id)).map_err(|e| format!("rustypipe: {e}"))?;
        let d = if video {
            let f = StreamFilter::new().video_formats(vec![VideoFormat::Mp4]).video_codecs(vec![VideoCodec::Avc1]).video_max_res(720);
            let v = player.select_video_only_stream(&f).ok_or("нет видео H.264")?;
            Direct { url: v.url.clone(), size: v.size }
        } else {
            let f = StreamFilter::new().audio_formats(vec![AudioFormat::M4a]);
            let a = player.select_audio_stream(&f).ok_or("нет звука m4a")?;
            Direct { url: a.url.clone(), size: Some(a.size) }
        };
        crate::log(&format!("rustypipe {key}: ссылка за {:.2} с", t.elapsed().as_secs_f32()));
        let exp = url::Url::parse(&d.url).ok()
            .and_then(|u| u.query_pairs().find(|(k, _)| k == "expire").and_then(|(_, v)| v.parse::<u64>().ok()))
            .map(|e| e * 1000).unwrap_or(now_ms() + 3_600_000);
        self.streams.lock().unwrap().insert(key, (serde_json::to_string(&(d.url.clone(), d.size)).unwrap_or_default(), exp));
        Ok(d)
    }

    /// Забыть закэшированные ссылки трека (протухли / 403).
    pub fn forget(&self, video_id: &str) {
        let mut s = self.streams.lock().unwrap();
        s.remove(&format!("a:{video_id}"));
        s.remove(&format!("v:{video_id}"));
    }

    /// Найти deno (PATH или data_dir/bin); если нет — скачать в фоне. Не блокирует.
    pub fn ensure_deno_background(&self) {
        let mut st = self.deno.lock().unwrap();
        if !matches!(*st, Deno::Unknown | Deno::Failed(_)) { return; }
        let local = self.bin_dir.join(if cfg!(windows) { "deno.exe" } else { "deno" });
        if cmd("deno").arg("--version").output().is_ok_and(|o| o.status.success()) {
            *st = Deno::Ready("deno".into());
            return;
        }
        if local.exists() {
            *st = Deno::Ready(local.to_string_lossy().into_owned());
            return;
        }
        let Some(target) = deno_target() else { *st = Deno::Failed("deno не поддерживает эту платформу".into()); return };
        *st = Deno::Installing;
        drop(st);
        let (state, dir) = (self.deno.clone(), self.bin_dir.clone());
        std::thread::Builder::new().name("deno-install".into()).spawn(move || {
            let r = install_deno(&dir, &local, target);
            crate::log(&format!("deno: {}", match &r { Ok(()) => "скачан".to_string(), Err(e) => format!("ошибка: {e}") }));
            *state.lock().unwrap() = match r { Ok(()) => Deno::Ready(local.to_string_lossy().into_owned()), Err(e) => Deno::Failed(e) };
        }).ok();
    }

    fn deno_path(&self) -> Option<String> {
        match &*self.deno.lock().unwrap() { Deno::Ready(p) => Some(p.clone()), _ => None }
    }

    /// Подождать, пока deno докачается (если качается). true — если в итоге готов.
    fn wait_deno(&self, max: std::time::Duration) -> bool {
        let t = std::time::Instant::now();
        while matches!(*self.deno.lock().unwrap(), Deno::Installing) && t.elapsed() < max {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        self.deno_path().is_some()
    }

    fn asset_name() -> &'static str {
        if cfg!(windows) { "yt-dlp.exe" }
        else if cfg!(target_os = "macos") { "yt-dlp_macos" }
        else if cfg!(target_arch = "aarch64") { "yt-dlp_linux_aarch64" }
        else { "yt-dlp_linux" }
    }

    pub fn ensure_binary(&self, status: &dyn Fn(&str)) -> Result<String, String> {
        let mut guard = self.bin.lock().unwrap();
        if let Some(b) = guard.as_ref() { return Ok(b.clone()); }
        if cmd("yt-dlp").arg("--version").output().is_ok_and(|o| o.status.success()) {
            *guard = Some("yt-dlp".into());
            return Ok("yt-dlp".into());
        }
        let local = self.bin_dir.join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" });
        if !local.exists() {
            status("Скачиваю yt-dlp (один раз)...");
            std::fs::create_dir_all(&self.bin_dir).map_err(|e| e.to_string())?;
            let resp = crate::http().get(&format!("{RELEASE}{}", Self::asset_name())).call().map_err(|e| { crate::log(&format!("yt-dlp: ошибка скачивания: {e}")); format!("Не удалось скачать yt-dlp: {e}") })?;
            let part = local.with_extension("part");
            let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            std::io::copy(&mut resp.into_body().into_reader(), &mut f).map_err(|e| e.to_string())?;
            drop(f);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755));
            }
            std::fs::rename(&part, &local).map_err(|e| e.to_string())?;
        }
        let s = local.to_string_lossy().into_owned();
        *guard = Some(s.clone());
        Ok(s)
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let bin = self.ensure_binary(&|_| {})?;
        self.ensure_deno_background();
        let had_deno = self.deno_path().is_some();
        match self.run_once(&bin, args) {
            // deno ещё качался — дождёмся и попробуем ещё раз уже с ним
            Err(e) if !had_deno && args != ["--version"] && self.wait_deno(std::time::Duration::from_secs(180)) => self.run_once(&bin, args).map_err(|e2| if e2.is_empty() { e } else { e2 }),
            r => r,
        }
    }

    fn run_once(&self, bin: &str, args: &[&str]) -> Result<String, String> {
        let mut c = cmd(bin);
        if let Some(d) = self.deno_path() { c.arg("--js-runtimes").arg(format!("deno:{d}")); }
        let out = c.args(args).output().map_err(|e| { crate::log(&format!("yt-dlp не запустился: {e}")); e.to_string() })?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            crate::log(&format!("yt-dlp {:?} (deno: {}) завершился с кодом {:?}:\n{}", args, self.deno_path().is_some(), out.status.code(), err.trim()));
            let lines: Vec<&str> = err.lines().filter(|l| l.contains("ERROR")).collect();
            return Err(if lines.is_empty() { err.trim().to_string() } else { lines.join("\n") });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn version(&self) -> Result<String, String> {
        Ok(self.run(&["--version"])?.trim().to_string())
    }

    pub fn update(&self) -> Result<String, String> {
        if self.ensure_binary(&|_| {})? == "yt-dlp" {
            return Ok("yt-dlp установлен в системе — обнови его своим пакетным менеджером".into());
        }
        Ok(self.run(&["-U"])?.trim().to_string())
    }

    /// Плейлист / альбом / одиночное видео по ссылке.
    pub fn import_url(&self, url: &str, status: &dyn Fn(&str)) -> Result<UrlImport, String> {
        self.ensure_binary(status)?;
        status("Читаю плейлист...");
        let json: Value = serde_json::from_str(&self.run(&["--flat-playlist", "-J", "--no-warnings", url])?).map_err(|e| e.to_string())?;
        let is_music = url.contains("music.youtube.com");
        let entries: Vec<&Value> = match json.get("entries").and_then(Value::as_array) {
            Some(a) => a.iter().filter(|e| !e.is_null()).collect(),
            None => vec![&json],
        };
        let base = now_ms();
        let n = entries.len() as u64;
        let tracks = entries.iter().enumerate()
            .filter(|(_, e)| !matches!(s(e, "title").as_deref(), Some("[Private video]") | Some("[Deleted video]")) && !is_live(e))
            .map(|(i, e)| { let mut t = to_track(e, &json, is_music); t.added_at = base + n - i as u64; t })
            .collect();
        Ok(UrlImport {
            name: json.get("entries").map(|_| s(&json, "title").unwrap_or_else(|| "Импортированный плейлист".into())),
            source: url.to_string(),
            tracks,
        })
    }

    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<Track>, String> {
        let q = format!("ytsearch{limit}:{query}");
        let json: Value = serde_json::from_str(&self.run(&["--flat-playlist", "-J", "--no-warnings", &q])?).map_err(|e| e.to_string())?;
        Ok(json.get("entries").and_then(Value::as_array).map(|a| a.iter().filter(|e| !e.is_null() && !is_live(e)).map(|e| to_track(e, &json, false)).collect()).unwrap_or_default())
    }

    /// Один трек по ссылке или по названию (первый результат поиска) — для заказов из чата.
    pub fn lookup(&self, query: &str) -> Result<Track, String> {
        if query.starts_with("http") {
            let json: Value = serde_json::from_str(&self.run(&["--flat-playlist", "-J", "--no-warnings", "--playlist-end", "1", query])?).map_err(|e| e.to_string())?;
            let e = json.get("entries").and_then(Value::as_array).and_then(|a| a.first()).cloned().unwrap_or_else(|| json.clone());
            if e.is_null() || is_live(&e) { return Err("по ссылке нет видео".into()); }
            return Ok(to_track(&e, &json, false));
        }
        self.search(query, 1)?.into_iter().next().ok_or_else(|| "ничего не нашлось".into())
    }

    /// Радио по треку: микс YouTube (похожие песни), без самого трека.
    pub fn radio(&self, video_id: &str, limit: u32) -> Result<Vec<Track>, String> {
        let url = format!("https://www.youtube.com/watch?v={video_id}&list=RD{video_id}");
        let end = (limit + 1).to_string();
        let json: Value = serde_json::from_str(&self.run(&["--flat-playlist", "-J", "--no-warnings", "--playlist-end", &end, &url])?).map_err(|e| e.to_string())?;
        Ok(json.get("entries").and_then(Value::as_array).map(|a| a.iter()
            .filter(|e| !e.is_null() && !is_live(e) && e["id"].as_str() != Some(video_id))
            .map(|e| to_track(e, &json, false)).collect()).unwrap_or_default())
    }

    /// Прямая ссылка на аудиопоток (для webview-плеера).
    pub fn resolve_stream(&self, track: &Track) -> Result<String, String> {
        let key = track.url.clone().ok_or("У трека нет ссылки")?;
        if let Some((u, exp)) = self.streams.lock().unwrap().get(&key) {
            if *exp > now_ms() + 60_000 { return Ok(u.clone()); }
        }
        let out = self.run(&["-f", "bestaudio[ext=m4a]/bestaudio/best", "-g", "--no-playlist", "--no-warnings", &key])?;
        let stream = out.lines().next().unwrap_or("").trim().to_string();
        if stream.is_empty() { return Err("yt-dlp не вернул ссылку на поток".into()); }
        let exp = url::Url::parse(&stream).ok()
            .and_then(|u| u.query_pairs().find(|(k, _)| k == "expire").and_then(|(_, v)| v.parse::<u64>().ok()))
            .map(|e| e * 1000).unwrap_or(now_ms() + 3_600_000);
        self.streams.lock().unwrap().insert(key, (stream.clone(), exp));
        Ok(stream)
    }

    /// Звук потоком через yt-dlp (-o -): он сам проходит защиту YouTube (deno), а мы играем с первых
    /// килобайт, пока остальное докачивается в `path`. Только m4a (AAC) — его декодирует symphonia.
    /// `auth` — с куками (для видео 18+, если они заданы в настройках).
    pub fn stream_audio(&self, track: &Track, path: &Path, auth: bool) -> Result<crate::stream::Download, String> {
        use std::process::Stdio;
        let key = track.url.clone().ok_or("У трека нет ссылки")?;
        let bin = self.ensure_binary(&|_| {})?;
        self.ensure_deno_background();
        // deno нужен для подписи ссылок; если он ещё качается — подождём немного
        self.wait_deno(std::time::Duration::from_secs(60));
        let mut c = cmd(&bin);
        if let Some(d) = self.deno_path() { c.arg("--js-runtimes").arg(format!("deno:{d}")); }
        if auth { c.args(self.cookie_args()); }
        c.args(["-f", "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]", "--no-playlist", "--no-warnings", "--no-progress", "--quiet",
                "--no-part", "--match-filter", "!is_live", "-o", "-", &key])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = c.spawn().map_err(|e| format!("yt-dlp не запустился: {e}"))?;
        Ok(crate::stream::Download::pipe(child, path.to_path_buf()))
    }

    /// Один трек не качаем двумя yt-dlp сразу (предзагрузка + воспроизведение): в Windows второй
    /// не может переименовать .part, пока его держит первый («WinError 32»). Второй ждёт первого.
    fn key_lock(&self, key: &str) -> Arc<Mutex<()>> {
        static LOCKS: std::sync::OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = std::sync::OnceLock::new();
        LOCKS.get_or_init(Default::default).lock().unwrap().entry(key.to_string()).or_default().clone()
    }

    /// Скачать аудио в кэш (для нативного плеера). Возвращает путь к файлу.
    /// Берём m4a (AAC) — его умеет декодировать symphonia.
    pub fn download_audio(&self, track: &Track, cache: &Path) -> Result<PathBuf, String> {
        let key = track.url.clone().ok_or("У трека нет ссылки")?;
        let lock = self.key_lock(&format!("a:{key}"));
        let _guard = lock.lock().unwrap();
        let _ = std::fs::create_dir_all(cache);
        let stem = sha16(key.as_bytes());
        if let Some(p) = std::fs::read_dir(cache).ok().and_then(|rd| rd.flatten().map(|e| e.path())
            .find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == stem) && p.extension().is_some_and(|e| e != "part"))) {
            return Ok(p);
        }
        let tpl = cache.join(format!("{stem}.%(ext)s"));
        let out = self.run(&[
            "-f", "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio[ext=mp3]/bestaudio",
            "--no-playlist", "--no-warnings", "--no-progress", "--match-filter", "!is_live", "-o", &tpl.to_string_lossy(),
            "--print", "after_move:filepath", &key,
        ])?;
        let p = PathBuf::from(out.lines().last().unwrap_or("").trim());
        if p.exists() { Ok(p) } else { Err("не удалось скачать (прямые эфиры не поддерживаются)".into()) }
    }
}

/// Прямые эфиры и премьеры пропускаем: их нельзя скачать целиком.
fn is_live(e: &Value) -> bool {
    matches!(e.get("live_status").and_then(Value::as_str), Some("is_live" | "is_upcoming"))
}

impl YouTube {
    /// Скачать видеодорожку (без звука) для показа клипа. H.264 в MP4 до 720p —
    /// его умеет встроенный декодер, и он не грузит процессор.
    pub fn download_video(&self, track: &Track, cache: &Path) -> Result<PathBuf, String> {
        let key = track.url.clone().ok_or("У трека нет ссылки")?;
        let lock = self.key_lock(&format!("v:{key}"));
        let _guard = lock.lock().unwrap();
        let _ = std::fs::create_dir_all(cache);
        let path = cache.join(format!("{}.video.mp4", sha16(key.as_bytes())));
        if path.exists() { return Ok(path); }
        self.run(&[
            "-f", "bestvideo[vcodec^=avc1][height<=720][ext=mp4]/bestvideo[vcodec^=avc1][ext=mp4]",
            "--no-playlist", "--no-warnings", "--no-progress", "--match-filter", "!is_live",
            "-o", &path.to_string_lossy(), &key,
        ])?;
        if path.exists() { Ok(path) } else { Err("YouTube не отдал видео в формате H.264".into()) }
    }
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string).filter(|x| !x.is_empty())
}

fn to_track(e: &Value, parent: &Value, is_music: bool) -> Track {
    let ie = s(e, "ie_key").or_else(|| s(parent, "extractor_key")).unwrap_or_default().to_lowercase();
    let id = s(e, "id").unwrap_or_default();
    let artists = e.get("artists").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).filter(|x| !x.is_empty());
    let artist = artists.or_else(|| s(e, "artist")).or_else(|| s(e, "channel")).or_else(|| s(e, "uploader")).or_else(|| s(parent, "uploader")).unwrap_or_default();
    let artist = artist.trim_end_matches(" - Topic").trim().to_string();
    let title = s(e, "track").or_else(|| s(e, "title")).unwrap_or_default();
    let duration = e.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
    let album = s(e, "album").unwrap_or_default();
    if ie.starts_with("youtube") && id.len() == 11 {
        let host = if is_music { "https://music.youtube.com" } else { "https://www.youtube.com" };
        return Track {
            id: format!("yt_{id}"), kind: "youtube".into(), path: None,
            url: Some(format!("{host}/watch?v={id}")),
            cover: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
            video_id: Some(id), title, artist: if artist.is_empty() { "YouTube".into() } else { artist },
            album, duration, added_at: now_ms(), video: false, anime: None, episode: None,
        };
    }
    let url = s(e, "webpage_url").or_else(|| s(e, "url")).or_else(|| s(parent, "webpage_url")).unwrap_or_default();
    let cover = s(e, "thumbnail").or_else(|| e.get("thumbnails").or_else(|| parent.get("thumbnails")).and_then(Value::as_array).and_then(|a| a.last()).and_then(|t| s(t, "url")));
    Track {
        id: format!("ext_{}", sha16(url.as_bytes())), kind: "stream".into(), path: None, video_id: None,
        title: if title.is_empty() { url.clone() } else { title }, url: Some(url),
        artist: if artist.is_empty() { s(e, "extractor_key").unwrap_or_else(|| "Stream".into()) } else { artist },
        album, duration, cover, added_at: now_ms(), video: false, anime: None, episode: None,
    }
}

fn deno_target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    })
}

fn install_deno(dir: &Path, dest: &Path, target: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let url = format!("https://github.com/denoland/deno/releases/latest/download/deno-{target}.zip");
    let resp = crate::http().get(&url).call().map_err(|e| format!("Не удалось скачать deno: {e}"))?;
    let zip_path = dir.join("deno.zip.part");
    let mut f = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    std::io::copy(&mut resp.into_body().into_reader(), &mut f).map_err(|e| e.to_string())?;
    drop(f);
    let res = (|| {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&zip_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let name = if cfg!(windows) { "deno.exe" } else { "deno" };
        let mut entry = zip.by_name(name).map_err(|e| format!("в архиве нет {name}: {e}"))?;
        let part = dest.with_extension("part");
        let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        drop(out);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755));
        }
        std::fs::rename(&part, dest).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&zip_path);
    res
}

fn cmd(bin: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW — без мигающей консоли
    }
    c
}
