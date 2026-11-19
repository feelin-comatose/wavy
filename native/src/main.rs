// Wavy — нативная версия (egui + rodio). Без браузерного движка: минимум памяти.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod media;
mod update;
mod video;
mod player;
mod thumbs;

use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use global_hotkey::{hotkey::{Code, HotKey, Modifiers}, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wavy_core::{discord::{Discord, DiscordStatus}, fmt_time, local, overlay::OverlayServer, store::Store, youtube::YouTube, PlayerState, Track};

// ---------- палитра ----------
const BG: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0c);
const BG2: Color32 = Color32::from_rgb(0x10, 0x10, 0x13);
const BG3: Color32 = Color32::from_rgb(0x18, 0x18, 0x1c);
const LINE: Color32 = Color32::from_rgb(0x23, 0x23, 0x29);
const TEXT: Color32 = Color32::from_rgb(0xe4, 0xe4, 0xe8);
const MUTED: Color32 = Color32::from_rgb(0x7a, 0x7a, 0x86);
const ACCENT: Color32 = Color32::from_rgb(0xf4, 0xf4, 0xf5); // акцент — белый
const ACCENT_DIM: Color32 = Color32::from_rgb(0xf4, 0xf4, 0xf5); // заливка основных кнопок
const ACCENT_SOFT: Color32 = Color32::from_rgb(0xa1, 0xa1, 0xaa);
const BAD: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);
const OK: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80);

const AUDIO_FILTER: &[&str] = &["mp3", "flac", "wav", "ogg", "oga", "opus", "m4a", "aac", "alac", "aiff", "mp4", "m4v", "mov", "m3u", "m3u8"];

#[derive(Clone, PartialEq)]
enum View { Library, Playlist(String), Search }

#[derive(Clone)]
enum UpdState { Checking, Downloading(u32), Ready(u32), UpToDate, Error(String) }

enum Bg {
    Status(String),
    OpenPlaylist(String),
    Search(Result<Vec<Track>, String>),
    Ready { token: u64, path: PathBuf },
    LoadFailed { token: u64, err: String },
    Discord(DiscordStatus),
    YtVersion(Result<String, String>),
    VideoReady { id: String, path: PathBuf },
    VideoFailed { id: String, err: String },
    Update(UpdState),
    ImportDone(Result<(), String>),
}

enum Modal {
    ImportUrl { url: String, into: bool, busy: bool, err: Option<String> },
    NewPlaylist { name: String, ids: Vec<String> },
    Rename { id: String, name: String },
    Delete { id: String },
    Settings { client_id: String, port: String },
}

enum Act {
    Play(Vec<String>, usize, bool),
    TogglePlay, Next, Prev, Seek(f64), Volume(f32), SaveVolume, Shuffle, Repeat,
    View(View), ImportFiles, ImportFolder, ImportPaths(Vec<PathBuf>, Option<String>), ImportUrl(String, Option<String>),
    Search(String), AddTracks(Vec<Track>), PlayNext(Vec<String>), Enqueue(Vec<String>),
    AddTo(String, Vec<String>), RemoveFrom(String, Vec<String>), RemoveTracks(Vec<String>),
    CreatePlaylist(String, Vec<String>), Rename(String, String), DeletePlaylist(String), Sync(String),
    Reveal(String), OpenUrl(String), Settings(serde_json::Value), ToggleOverlay, ToggleLock, JumpQueue(usize),
    Modal(Modal), YtUpdate, ToggleVideo, Fullscreen(bool), CheckUpdate, Restart,
}

struct App {
    store: Arc<Mutex<Store>>,
    yt: Arc<YouTube>,
    discord: Discord,
    discord_status: DiscordStatus,
    overlay: OverlayServer,
    audio: player::Audio,
    thumbs: thumbs::Thumbs,
    tx: Sender<Bg>,
    rx: Receiver<Bg>,
    covers: PathBuf,
    cache: PathBuf,

    view: View,
    filter: String,
    search_q: String,
    searching: bool,
    results: Vec<Track>,
    selected: HashSet<String>,
    last_clicked: Option<String>,

    queue: Vec<String>,
    qi: usize,
    current: Option<Track>,
    token: u64,
    loading: bool,
    errors_in_row: u32,
    pending_next: bool,
    seek_drag: Option<f64>,
    last_pushed: (Option<String>, bool),
    shown_at: Instant,

    modal: Option<Modal>,
    toast: Option<(String, bool, Instant)>,
    show_queue: bool,
    overlay_open: bool,
    overlay_init: [f32; 4],
    yt_version: Option<Result<String, String>>,

    media: media::Media,
    media_rx: Receiver<media::MediaCmd>,
    resume: Option<(String, f64, bool)>, // (id трека, позиция, автоплей) — продолжить с того же места
    session_saved: Instant,
    spin: f32,
    show_video: bool,
    fullscreen: bool,
    video: Option<(String, video::Video)>,
    video_loading: Option<String>,
    video_error: Option<(String, String)>,
    upd: Option<UpdState>,
    upd_checked: Option<Instant>,
    started: Instant,
    _hotkeys: Option<GlobalHotKeyManager>,
    hk_rx: Receiver<u32>,
    hk_ids: (u32, u32),
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let ctx = cc.egui_ctx.clone();
        style(&ctx);
        let dir = wavy_core::data_dir();
        let store = Store::open(&dir);
        let (tx, rx) = channel();
        let (t2, c2) = (tx.clone(), ctx.clone());
        let discord = Discord::start(move |s| { let _ = t2.send(Bg::Discord(s.clone())); c2.request_repaint(); });
        let overlay = OverlayServer::new();
        let s = store.lib.settings.clone();
        discord.configure(s.discord.enabled, &s.discord.client_id, s.discord.show_buttons);
        overlay.set_config(&s.overlay.theme, s.overlay.auto_hide_sec);
        let mut toast = None;
        if let Err(e) = overlay.start(s.overlay.port) { wavy_core::log(&format!("оверлей: {e}")); toast = Some((format!("Оверлей: {e}"), true, Instant::now())); }

        // глобальные хоткеи: Ctrl+Shift+O — оверлей, Ctrl+Shift+L — закрепить/двигать
        let (hk_tx, hk_rx) = channel();
        let ko = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyO);
        let kl = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyL);
        let hotkeys = GlobalHotKeyManager::new().ok().filter(|m| m.register(ko).is_ok() && m.register(kl).is_ok());
        let c3 = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed { let _ = hk_tx.send(e.id); c3.request_repaint(); }
        }));

        let cache = dir.join("cache");
        std::thread::spawn({ let c = cache.clone(); move || trim_cache(&c, 1_500_000_000, 1_000_000_000) });

        let (media_tx, media_rx) = channel();
        let media = media::Media::new(cc, media_tx);
        let audio = player::Audio::new();
        if let Some(e) = &audio.error { wavy_core::log(e); toast = Some((e.clone(), true, Instant::now())); }
        let mut app = Self {
            overlay_open: false,
            overlay_init: s.overlay_window.bounds.unwrap_or([24.0, 24.0, 440.0, 124.0]),
            store: Arc::new(Mutex::new(store)),
            yt: Arc::new(YouTube::new(dir.join("bin"))),
            discord, discord_status: Default::default(), overlay, audio,
            thumbs: thumbs::Thumbs::new(&ctx),
            tx, rx, covers: dir.join("covers"), cache,
            view: View::Library, filter: String::new(), search_q: String::new(), searching: false, results: vec![],
            selected: HashSet::new(), last_clicked: None,
            queue: vec![], qi: 0, current: None, token: 0, loading: false, errors_in_row: 0, pending_next: false, seek_drag: None,
            last_pushed: (None, false), shown_at: Instant::now(),
            modal: None, toast, show_queue: false, yt_version: None,
            media, media_rx, resume: None, session_saved: Instant::now(), spin: 0.0,
            show_video: false, fullscreen: false, video: None, video_loading: None, video_error: None, upd: None, upd_checked: None, started: Instant::now(),
            _hotkeys: hotkeys, hk_rx, hk_ids: (ko.id(), kl.id()),
        };
        app.overlay_open = s.overlay_window.visible;
        app.restore_session(&ctx);
        // «Открыть с помощью Wavy»
        let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).filter(|p| p.exists()).collect();
        if !args.is_empty() { app.apply(Act::ImportPaths(args, None), &ctx); }
        app
    }

    fn session_file() -> PathBuf { wavy_core::data_dir().join("session.json") }

    fn save_session(&mut self) {
        self.session_saved = Instant::now();
        let pos = self.resume.as_ref().map(|r| r.1).unwrap_or_else(|| self.audio.position());
        let v = json!({ "queue": self.queue, "qi": self.qi, "position": pos });
        let _ = std::fs::write(Self::session_file(), v.to_string());
    }

    fn restore_session(&mut self, ctx: &egui::Context) {
        let Some(v) = std::fs::read_to_string(Self::session_file()).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) else { return };
        let queue: Vec<String> = serde_json::from_value(v["queue"].clone()).unwrap_or_default();
        let queue: Vec<String> = { let s = self.store.lock().unwrap(); queue.into_iter().filter(|id| s.lib.tracks.contains_key(id)).collect() };
        if queue.is_empty() { return; }
        self.qi = (v["qi"].as_u64().unwrap_or(0) as usize).min(queue.len() - 1);
        self.queue = queue;
        let pos = v["position"].as_f64().unwrap_or(0.0);
        let id = self.queue[self.qi].clone();
        self.current = self.track(&id);
        self.resume = Some((id, pos, false));
        // локальный файл открываем сразу на паузе; YouTube — при нажатии play
        if self.current.as_ref().is_some_and(|t| t.is_local()) { self.load_current(ctx); }
    }

    fn settings(&self) -> wavy_core::store::Settings { self.store.lock().unwrap().lib.settings.clone() }
    fn track(&self, id: &str) -> Option<Track> {
        self.store.lock().unwrap().lib.tracks.get(id).cloned().or_else(|| self.results.iter().find(|t| t.id == id).cloned())
    }
    fn toast(&mut self, m: impl Into<String>, err: bool) {
        let m = m.into();
        if err { wavy_core::log(&format!("ошибка: {m}")); }
        self.toast = Some((m, err, Instant::now()));
    }

    fn list_ids(&self) -> Vec<String> {
        let s = self.store.lock().unwrap();
        let ids = match &self.view {
            View::Library => s.library_ids(),
            View::Playlist(id) => s.lib.playlists.iter().find(|p| &p.id == id).map(|p| p.track_ids.iter().filter(|t| s.lib.tracks.contains_key(*t)).cloned().collect()).unwrap_or_default(),
            View::Search => return self.results.iter().map(|t| t.id.clone()).collect(),
        };
        let q = self.filter.trim().to_lowercase();
        if q.is_empty() { return ids; }
        ids.into_iter().filter(|id| s.lib.tracks.get(id).is_some_and(|t| format!("{} {} {}", t.title, t.artist, t.album).to_lowercase().contains(&q))).collect()
    }

    // ---------- воспроизведение ----------
    fn play_list(&mut self, ids: Vec<String>, start: usize, force_shuffle: bool, ctx: &egui::Context) {
        if ids.is_empty() { return; }
        self.resume = None;
        let mut q = ids;
        let mut start = start.min(q.len() - 1);
        let shuffle = self.settings().shuffle;
        if force_shuffle || shuffle {
            let first = if force_shuffle && start == 0 { None } else { Some(q.remove(start)) };
            shuffle_vec(&mut q);
            if let Some(f) = first { q.insert(0, f); }
            start = 0;
            if force_shuffle && !shuffle { self.store.lock().unwrap().patch_settings(&json!({"shuffle": true})); }
        }
        self.queue = q;
        self.qi = start;
        self.load_current(ctx);
    }

    fn load_current(&mut self, ctx: &egui::Context) {
        let Some(id) = self.queue.get(self.qi).cloned() else { return };
        let Some(t) = self.track(&id) else { return };
        self.token += 1;
        let token = self.token;
        self.current = Some(t.clone());
        self.shown_at = Instant::now();
        self.audio.stop();
        if t.is_local() {
            let path = PathBuf::from(t.path.clone().unwrap_or_default());
            self.start_file(&path);
        } else {
            self.loading = true;
            self.push_state();
            let (yt, tx, cache, c) = (self.yt.clone(), self.tx.clone(), self.cache.clone(), ctx.clone());
            let next = self.queue.get(self.qi + 1).and_then(|n| self.track(n)).filter(|n| !n.is_local());
            std::thread::spawn(move || {
                let r = yt.download_audio(&t, &cache);
                let _ = tx.send(match r { Ok(path) => Bg::Ready { token, path }, Err(err) => Bg::LoadFailed { token, err } });
                c.request_repaint();
                if let Some(n) = next { let _ = yt.download_audio(&n, &cache); } // предзагрузка следующего
            });
        }
    }

    fn start_file(&mut self, path: &Path) {
        let vol = self.settings().volume;
        match self.audio.load(path, vol * vol) {
            Ok(()) => {
                self.errors_in_row = 0;
                let cur = self.current.as_ref().map(|t| t.id.clone());
                if let Some((id, pos, autoplay)) = self.resume.take() {
                    if Some(&id) == cur.as_ref() {
                        if pos > 1.0 { self.audio.seek(pos); }
                        if !autoplay { self.audio.pause(); }
                    }
                }
            }
            Err(e) => self.track_failed(e),
        }
        self.loading = false;
        self.push_state();
    }

    fn track_failed(&mut self, err: String) {
        let title = self.current.as_ref().map(|t| t.title.clone()).unwrap_or_default();
        self.toast(format!("Не получилось воспроизвести «{title}»: {err}"), true);
        self.errors_in_row += 1;
        self.loading = false;
        // без звукового устройства листать очередь бессмысленно — останавливаемся
        if self.audio.has_device() && self.errors_in_row < 5 && self.qi + 1 < self.queue.len() { self.pending_next = true; }
    }

    fn next(&mut self, auto: bool, ctx: &egui::Context) {
        if self.queue.is_empty() { return; }
        self.resume = None;
        let repeat = self.settings().repeat;
        if auto && repeat == "one" && self.errors_in_row == 0 { self.audio.seek(0.0); if self.audio.finished() { self.load_current(ctx); } return; }
        if self.qi + 1 < self.queue.len() { self.qi += 1; }
        else if repeat == "all" || !auto { self.qi = 0; }
        else { self.audio.stop(); self.push_state(); return; }
        self.load_current(ctx);
    }

    fn prev(&mut self, ctx: &egui::Context) {
        if self.queue.is_empty() { return; }
        self.resume = None;
        if self.audio.position() > 3.0 { self.audio.seek(0.0); self.push_state(); return; }
        self.qi = if self.qi > 0 { self.qi - 1 } else { self.queue.len() - 1 };
        self.load_current(ctx);
    }

    fn push_state(&mut self) {
        let duration = if self.audio.duration > 0.0 { self.audio.duration } else { self.current.as_ref().map(|t| t.duration).unwrap_or(0.0) };
        let position = self.resume.as_ref().map(|r| r.1).unwrap_or_else(|| self.audio.position());
        let st = PlayerState { track: self.current.clone(), playing: self.audio.is_playing(), position, duration };
        self.media.update(st.track.as_ref(), st.playing, st.position, st.duration);
        self.last_pushed = (st.track.as_ref().map(|t| t.id.clone()), st.playing);
        self.discord.update(&st);
        self.overlay.update(&st);
    }

    // ---------- фоновая работа ----------
    fn bg(&self, ctx: &egui::Context, f: impl FnOnce(&Sender<Bg>) -> Result<(), String> + Send + 'static) {
        let (tx, c) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = f(&tx);
            let _ = tx.send(Bg::ImportDone(r));
            c.request_repaint();
        });
    }

    fn apply(&mut self, a: Act, ctx: &egui::Context) {
        match a {
            Act::Play(ids, i, sh) => self.play_list(ids, i, sh, ctx),
            Act::TogglePlay => {
                if self.current.is_none() { let ids = self.list_ids(); self.play_list(ids, 0, false, ctx); }
                else if self.audio.is_playing() { self.audio.pause(); }
                else if self.audio.loaded && !self.audio.finished() { self.audio.play(); }
                else if !self.loading {
                    if let Some(r) = self.resume.as_mut() { r.2 = true; }
                    self.load_current(ctx);
                }
                self.shown_at = Instant::now();
                self.push_state();
            }
            Act::Next => self.next(false, ctx),
            Act::Prev => self.prev(ctx),
            Act::Seek(s) => { self.audio.seek(s); if let Some((_, v)) = &mut self.video { v.seek(s); } self.push_state(); }
            Act::Volume(v) => { self.audio.set_volume(v); self.store.lock().unwrap().lib.settings.volume = v; }
            Act::SaveVolume => { let v = self.settings().volume; self.store.lock().unwrap().patch_settings(&json!({"volume": v})); }
            Act::Shuffle => {
                let v = !self.settings().shuffle;
                self.store.lock().unwrap().patch_settings(&json!({"shuffle": v}));
                if v && self.qi + 1 < self.queue.len() { let mut rest = self.queue.split_off(self.qi + 1); shuffle_vec(&mut rest); self.queue.extend(rest); }
            }
            Act::Repeat => {
                let r = match self.settings().repeat.as_str() { "off" => "all", "all" => "one", _ => "off" };
                self.store.lock().unwrap().patch_settings(&json!({"repeat": r}));
            }
            Act::View(v) => { self.view = v; self.selected.clear(); self.filter.clear(); }
            Act::ImportFiles | Act::ImportFolder => {
                let pid = self.cur_playlist();
                let dlg = rfd::FileDialog::new();
                let picked = if matches!(a, Act::ImportFiles) { dlg.set_title("Выбери музыку").add_filter("Аудио и плейлисты", AUDIO_FILTER).pick_files() } else { dlg.set_title("Выбери папку с музыкой").pick_folders() };
                if let Some(p) = picked { self.apply(Act::ImportPaths(p, pid), ctx); }
            }
            Act::ImportPaths(paths, pid) => {
                self.toast("Импорт…", false);
                let (store, covers) = (self.store.clone(), self.covers.clone());
                self.bg(ctx, move |tx| {
                    let res = local::import_paths(&paths, &covers, |i, n| { if n > 30 && i % 25 == 0 { let _ = tx.send(Bg::Status(format!("Импорт: {i} / {n}"))); } });
                    let n = res.tracks.len();
                    let mut s = store.lock().unwrap();
                    let ids: Vec<String> = res.tracks.iter().map(|t| t.id.clone()).collect();
                    s.add_tracks(res.tracks);
                    for (name, ids) in res.playlists { s.create_playlist(&name, ids, Some("m3u".into())); }
                    if let Some(pid) = pid { s.add_to_playlist(&pid, &ids); }
                    let _ = tx.send(Bg::Status(format!("Импортировано треков: {n}")));
                    Ok(())
                });
            }
            Act::ImportUrl(url, pid) => {
                let (store, yt) = (self.store.clone(), self.yt.clone());
                self.bg(ctx, move |tx| {
                    let t2 = tx.clone();
                    let res = yt.import_url(url.trim(), &move |m| { let _ = t2.send(Bg::Status(m.into())); })?;
                    let ids: Vec<String> = res.tracks.iter().map(|t| t.id.clone()).collect();
                    let n = ids.len();
                    let mut s = store.lock().unwrap();
                    s.add_tracks(res.tracks);
                    let target = if let Some(pid) = pid { s.add_to_playlist(&pid, &ids); Some(pid) }
                        else { res.name.map(|name| s.create_playlist(&name, ids, Some(res.source)).id) };
                    let _ = tx.send(Bg::Status(format!("Добавлено {n} трек(ов)")));
                    if let Some(t) = target { let _ = tx.send(Bg::OpenPlaylist(t)); }
                    Ok(())
                });
            }
            Act::Search(q) => {
                self.searching = true;
                let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
                std::thread::spawn(move || { let _ = tx.send(Bg::Search(yt.search(&q, 25))); c.request_repaint(); });
            }
            Act::AddTracks(t) => { self.store.lock().unwrap().add_tracks(t); self.toast("Добавлено в библиотеку", false); }
            Act::PlayNext(ids) => {
                if self.current.is_none() { return self.play_list(ids, 0, false, ctx); }
                let n = ids.len();
                for (k, id) in ids.into_iter().enumerate() { self.queue.insert((self.qi + 1 + k).min(self.queue.len()), id); }
                self.toast(if n > 1 { format!("{n} треков сыграют следующими") } else { "Сыграет следующим".into() }, false);
            }
            Act::Enqueue(ids) => {
                if self.current.is_none() { return self.play_list(ids, 0, false, ctx); }
                self.queue.extend(ids);
                self.toast("Добавлено в очередь", false);
            }
            Act::AddTo(pid, ids) => {
                self.ensure_in_library(&ids);
                if let Some(n) = self.store.lock().unwrap().add_to_playlist(&pid, &ids) { self.toast = Some((format!("Добавлено в «{n}»"), false, Instant::now())); }
            }
            Act::RemoveFrom(pid, ids) => self.store.lock().unwrap().remove_from_playlist(&pid, &ids),
            Act::RemoveTracks(ids) => {
                self.store.lock().unwrap().remove_tracks(&ids);
                let cur = self.queue.get(self.qi).cloned();
                self.queue.retain(|q| !ids.contains(q) || Some(q) == cur.as_ref());
                self.qi = cur.and_then(|c| self.queue.iter().position(|q| *q == c)).unwrap_or(0);
                self.selected.clear();
            }
            Act::CreatePlaylist(name, ids) => {
                self.ensure_in_library(&ids);
                let p = self.store.lock().unwrap().create_playlist(&name, ids.clone(), None);
                if ids.is_empty() { self.view = View::Playlist(p.id); } else { self.toast(format!("Плейлист «{name}» создан"), false); }
            }
            Act::Rename(id, name) => self.store.lock().unwrap().rename_playlist(&id, &name),
            Act::DeletePlaylist(id) => { self.store.lock().unwrap().delete_playlist(&id); self.view = View::Library; }
            Act::Sync(id) => {
                let (store, yt) = (self.store.clone(), self.yt.clone());
                self.bg(ctx, move |tx| {
                    let src = store.lock().unwrap().lib.playlists.iter().find(|p| p.id == id).and_then(|p| p.source.clone()).filter(|s| s.starts_with("http")).ok_or("У плейлиста нет ссылки")?;
                    let t2 = tx.clone();
                    let res = yt.import_url(&src, &move |m| { let _ = t2.send(Bg::Status(m.into())); })?;
                    let ids: Vec<String> = res.tracks.iter().map(|t| t.id.clone()).collect();
                    let n = ids.len();
                    let mut s = store.lock().unwrap();
                    s.add_tracks(res.tracks);
                    s.set_playlist_tracks(&id, ids);
                    let _ = tx.send(Bg::Status(format!("Синхронизировано: {n} треков")));
                    Ok(())
                });
            }
            Act::Reveal(p) => reveal(&p),
            Act::OpenUrl(u) => open_url(&u),
            Act::Settings(patch) => {
                self.store.lock().unwrap().patch_settings(&patch);
                let s = self.settings();
                self.discord.configure(s.discord.enabled, &s.discord.client_id, s.discord.show_buttons);
                self.overlay.set_config(&s.overlay.theme, s.overlay.auto_hide_sec);
                if let Err(e) = self.overlay.start(s.overlay.port) { self.toast(format!("Оверлей: {e}"), true); }
            }
            Act::ToggleOverlay => {
                self.overlay_open = !self.overlay_open;
                self.overlay_init = self.settings().overlay_window.bounds.unwrap_or(self.overlay_init);
                self.shown_at = Instant::now();
                self.store.lock().unwrap().patch_settings(&json!({"overlayWindow": {"visible": self.overlay_open}}));
            }
            Act::ToggleLock => {
                let l = !self.settings().overlay_window.locked;
                self.store.lock().unwrap().patch_settings(&json!({"overlayWindow": {"locked": l}}));
                if !self.overlay_open && !l { self.apply(Act::ToggleOverlay, ctx); }
            }
            Act::JumpQueue(i) => { self.resume = None; self.qi = i; self.load_current(ctx); }
            Act::Modal(m) => {
                if matches!(m, Modal::Settings { .. }) && self.yt_version.is_none() {
                    let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
                    std::thread::spawn(move || { let _ = tx.send(Bg::YtVersion(yt.version())); c.request_repaint(); });
                }
                self.modal = Some(m);
            }
            Act::ToggleVideo => {
                self.show_video = !self.show_video;
                if !self.show_video && self.fullscreen { self.apply(Act::Fullscreen(false), ctx); }
            }
            Act::Fullscreen(on) => {
                self.fullscreen = on;
                if on { self.show_video = true; }
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
            }
            Act::CheckUpdate => { self.upd_checked = Some(Instant::now()); self.start_update(ctx); }
            Act::Restart => {
                self.save_session();
                update::restart();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Act::YtUpdate => {
                let yt = self.yt.clone();
                self.yt_version = None;
                self.bg(ctx, move |tx| { let m = yt.update()?; let _ = tx.send(Bg::Status(m)); let _ = tx.send(Bg::YtVersion(yt.version())); Ok(()) });
            }
        }
    }

    fn ensure_in_library(&mut self, ids: &[String]) {
        let missing: Vec<Track> = ids.iter().filter(|id| !self.store.lock().unwrap().lib.tracks.contains_key(*id)).filter_map(|id| self.results.iter().find(|t| &t.id == id).cloned()).collect();
        if !missing.is_empty() { self.store.lock().unwrap().add_tracks(missing); }
    }

    fn cur_playlist(&self) -> Option<String> { if let View::Playlist(p) = &self.view { Some(p.clone()) } else { None } }

    /// Сообщения фоновых потоков, конец трека, хоткеи, drag & drop.
    fn tick(&mut self, ctx: &egui::Context) {
        self.thumbs.poll(ctx);
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Bg::Status(s) => self.toast(s, false),
                Bg::OpenPlaylist(id) => {
                    self.view = View::Playlist(id);
                    if matches!(self.modal, Some(Modal::ImportUrl { .. })) { self.modal = None; }
                }
                Bg::Search(r) => {
                    self.searching = false;
                    match r { Ok(v) => self.results = v, Err(e) => self.toast(e, true) }
                }
                Bg::Ready { token, path } => if token == self.token { self.start_file(&path); },
                Bg::LoadFailed { token, err } => if token == self.token { self.track_failed(err); },
                Bg::Discord(s) => self.discord_status = s,
                Bg::YtVersion(v) => self.yt_version = Some(v),
                Bg::VideoReady { id, path } => {
                    if self.video_loading.as_deref() == Some(id.as_str()) { self.video_loading = None; }
                    let cur = self.current.as_ref().map(|t| t.id.clone());
                    if cur.as_deref() == Some(id.as_str()) && (self.show_video || self.fullscreen) {
                        let pos = self.audio.position();
                        self.video = Some((id, video::Video::open(path, pos, ctx)));
                    }
                }
                Bg::VideoFailed { id, err } => {
                    if self.video_loading.as_deref() == Some(id.as_str()) { self.video_loading = None; }
                    self.video_error = Some((id, err));
                }
                Bg::Update(st) => {
                    if let UpdState::Ready(n) = &st { self.toast(format!("Новая версия (сборка {n}) установлена — перезапусти плеер"), false); }
                    if let UpdState::Error(e) = &st { wavy_core::log(&format!("обновление: {e}")); }
                    self.upd = Some(st);
                }
                Bg::ImportDone(r) => {
                    if let Err(e) = r {
                        if let Some(Modal::ImportUrl { busy, err, .. }) = &mut self.modal { *busy = false; *err = Some(e); } else { self.toast(e, true); }
                    } else if let Some(Modal::ImportUrl { busy: true, .. }) = &self.modal { self.modal = None; }
                }
            }
        }
        if std::mem::take(&mut self.pending_next) { self.next(true, ctx); }
        if self.audio.finished() && !self.loading { self.next(true, ctx); }
        let state = (self.current.as_ref().map(|t| t.id.clone()), self.audio.is_playing());
        if state != self.last_pushed && !self.loading { self.push_state(); }

        while let Ok(cmd) = self.media_rx.try_recv() {
            use media::MediaCmd::*;
            match cmd {
                Toggle => self.apply(Act::TogglePlay, ctx),
                Play => if !self.audio.is_playing() { self.apply(Act::TogglePlay, ctx) },
                Pause => if self.audio.is_playing() { self.apply(Act::TogglePlay, ctx) },
                Next => self.next(false, ctx),
                Prev => self.prev(ctx),
                SeekTo(s) => self.apply(Act::Seek(s), ctx),
                SeekBy(d) => { let p = self.audio.position(); self.apply(Act::Seek((p + d).max(0.0)), ctx) }
            }
        }
        if self.audio.is_playing() && self.session_saved.elapsed() > Duration::from_secs(15) { self.save_session(); }
        while let Ok(id) = self.hk_rx.try_recv() {
            if id == self.hk_ids.0 { self.apply(Act::ToggleOverlay, ctx); } else if id == self.hk_ids.1 { self.apply(Act::ToggleLock, ctx); }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            let pid = self.cur_playlist();
            self.apply(Act::ImportPaths(dropped, pid), ctx);
        }
        if let Some((_, _, at)) = &self.toast { if at.elapsed() > Duration::from_secs(4) { self.toast = None; } }
        self.video_tick(ctx);
        // автообновление: через 5 с после запуска и потом раз в 6 часов
        if self.settings().auto_update && update::current_build().is_some() {
            let due = match self.upd_checked { None => self.started.elapsed() > Duration::from_secs(5), Some(t) => t.elapsed() > Duration::from_secs(6 * 3600) };
            let busy = matches!(self.upd, Some(UpdState::Checking | UpdState::Downloading(_) | UpdState::Ready(_)));
            if due && !busy { self.upd_checked = Some(Instant::now()); self.start_update(ctx); }
            // окно может простаивать без перерисовок — просим разбудить к следующей проверке
            let next = match self.upd_checked { None => Duration::from_secs(5).saturating_sub(self.started.elapsed()), Some(t) => Duration::from_secs(6 * 3600).saturating_sub(t.elapsed()) };
            ctx.request_repaint_after(next + Duration::from_millis(100));
        }
    }

    fn start_update(&mut self, ctx: &egui::Context) {
        let (tx, c) = (self.tx.clone(), ctx.clone());
        self.upd = Some(UpdState::Checking);
        std::thread::spawn(move || {
            let send = |s: UpdState| { let _ = tx.send(Bg::Update(s)); c.request_repaint(); };
            match update::check() {
                Ok(Some(rel)) => {
                    send(UpdState::Downloading(rel.build));
                    send(match update::install(&rel) { Ok(()) => UpdState::Ready(rel.build), Err(e) => UpdState::Error(e) });
                }
                Ok(None) => send(UpdState::UpToDate),
                Err(e) => send(UpdState::Error(e)),
            }
        });
    }

    /// Видео для текущего трека: открываем, когда открыт просмотр, и выгружаем, когда закрыт (память).
    fn video_tick(&mut self, ctx: &egui::Context) {
        let want = if self.show_video || self.fullscreen { self.current.clone().filter(|t| t.has_video()) } else { None };
        let want_id = want.as_ref().map(|t| t.id.clone());
        if self.video.as_ref().map(|v| &v.0) != want_id.as_ref() { self.video = None; }
        let Some(t) = want else { return };
        let failed = self.video_error.as_ref().is_some_and(|e| e.0 == t.id);
        if self.video.is_none() && !failed {
            if t.is_local() {
                let path = PathBuf::from(t.path.clone().unwrap_or_default());
                let pos = self.audio.position();
                self.video = Some((t.id.clone(), video::Video::open(path, pos, ctx)));
            } else if self.video_loading.as_deref() != Some(t.id.as_str()) {
                self.video_loading = Some(t.id.clone());
                let (yt, tx, cache, c) = (self.yt.clone(), self.tx.clone(), self.cache.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let msg = match yt.download_video(&t, &cache) {
                        Ok(path) => Bg::VideoReady { id: t.id.clone(), path },
                        Err(err) => Bg::VideoFailed { id: t.id.clone(), err },
                    };
                    let _ = tx.send(msg);
                    c.request_repaint();
                });
            }
        }
        let clock = self.resume.as_ref().map(|r| r.1).unwrap_or_else(|| self.audio.position());
        let playing = self.audio.is_playing();
        if let Some((id, v)) = &mut self.video {
            v.update(clock, ctx);
            if let Some(e) = v.error.take() { self.video_error = Some((id.clone(), e)); self.video = None; }
            else if playing { ctx.request_repaint_after(Duration::from_millis(15)); }
        }
    }
}

// ---------- UI ----------
impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] { [0.0; 4] } // прозрачность для оверлей-окна

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) { self.save_session(); }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.tick(&ctx);
        let mut acts = Vec::new();
        self.keys(&ctx, &mut acts);

        if self.fullscreen {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ui, |ui| self.video_view(ui, &mut acts));
            self.modal_ui(&ctx, &mut acts);
            self.toast_ui(&ctx);
            if self.overlay_open { self.overlay_window(&ctx, &mut acts); }
            for a in acts { self.apply(a, &ctx); }
            return;
        }
        egui::Panel::bottom("player").exact_size(84.0).frame(egui::Frame::new().fill(BG2).stroke(Stroke::new(1.0, LINE)).inner_margin(egui::Margin::symmetric(18, 10)))
            .show(ui, |ui| self.player_bar(ui, &mut acts));
        egui::Panel::left("side").exact_size(232.0).resizable(false).frame(egui::Frame::new().fill(BG2).inner_margin(egui::Margin::symmetric(12, 16)))
            .show(ui, |ui| self.sidebar(ui, &mut acts));
        if self.show_queue {
            egui::Panel::right("queue").exact_size(290.0).resizable(false).frame(egui::Frame::new().fill(BG2).inner_margin(12))
                .show(ui, |ui| self.queue_ui(ui, &mut acts));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 28, right: 20, top: 24, bottom: 0 }))
            .show(ui, |ui| if self.show_video { self.video_view(ui, &mut acts) } else { self.main_view(ui, &mut acts) });

        self.modal_ui(&ctx, &mut acts);
        self.toast_ui(&ctx);
        if self.overlay_open { self.overlay_window(&ctx, &mut acts); }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) { drop_hint(&ctx); }

        for a in acts { self.apply(a, &ctx); }
        if self.audio.is_playing() || self.loading || self.searching || self.toast.is_some() { ctx.request_repaint_after(Duration::from_millis(250)); }
    }
}

impl App {
    fn keys(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        if ctx.egui_wants_keyboard_input() || self.modal.is_some() { return; }
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Space) { acts.push(Act::TogglePlay); }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::ArrowRight) { acts.push(Act::Next); }
            else if i.key_pressed(egui::Key::ArrowRight) { acts.push(Act::Seek(self.audio.position() + 5.0)); }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::ArrowLeft) { acts.push(Act::Prev); }
            else if i.key_pressed(egui::Key::ArrowLeft) { acts.push(Act::Seek((self.audio.position() - 5.0).max(0.0))); }
            for ev in &i.events {
                if let egui::Event::Paste(t) = ev { if t.trim().starts_with("http") { acts.push(Act::Modal(Modal::ImportUrl { url: t.trim().into(), into: false, busy: false, err: None })); } }
            }
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
            ui.painter().rect_filled(r, 8, ACCENT);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E340}", FontId::new(15.0, egui::FontFamily::Name("fill".into())), BG);
            ui.add_space(4.0);
            ui.label(RichText::new("wavy").size(23.0).family(bold()).color(TEXT));
        });
        ui.add_space(14.0);
        if nav(ui, "\u{E340}  Библиотека", self.view == View::Library, None).clicked() { acts.push(Act::View(View::Library)); }
        if nav(ui, "\u{E30C}  Поиск на YouTube", self.view == View::Search, None).clicked() { acts.push(Act::View(View::Search)); }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("ПЛЕЙЛИСТЫ").size(11.0).color(MUTED));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_btn(ui, "+", 18.0, false).on_hover_text("Новый плейлист").clicked() { acts.push(Act::Modal(Modal::NewPlaylist { name: String::new(), ids: vec![] })); }
            });
        });
        let pls = self.store.lock().unwrap().lib.playlists.clone();
        let foot = 92.0;
        egui::ScrollArea::vertical().max_height(ui.available_height() - foot).auto_shrink([false, true]).show(ui, |ui| {
            if pls.is_empty() { ui.label(RichText::new("Пока пусто").color(MUTED)); }
            for p in &pls {
                let icon = if p.is_remote() { "\u{E4FC}" } else { "\u{E6AA}" };
                let r = nav(ui, &format!("{icon}  {}", p.name), self.view == View::Playlist(p.id.clone()), Some(p.track_ids.len()));
                if r.clicked() { acts.push(Act::View(View::Playlist(p.id.clone()))); }
            }
        });
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            let d = if self.discord_status.connected { "\u{E61A}" } else { "" };
            if nav(ui, &format!("\u{E272}  Настройки  {d}"), false, None).clicked() {
                let s = self.settings();
                acts.push(Act::Modal(Modal::Settings { client_id: s.discord.client_id, port: s.overlay.port.to_string() }));
            }
            let s = self.settings();
            let st = if !self.overlay_open { "выкл" } else if s.overlay_window.locked { "вкл" } else { "двигай" };
            if nav(ui, &format!("\u{E466}  Оверлей · {st}"), self.overlay_open, None).clicked() {
                acts.push(Act::Modal(Modal::Settings { client_id: s.discord.client_id.clone(), port: s.overlay.port.to_string() }));
            }
            ui.separator();
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, ids: &[String], acts: &mut Vec<Act>) {
        let (eyebrow, title, pl) = {
            let s = self.store.lock().unwrap();
            match &self.view {
                View::Library => ("КОЛЛЕКЦИЯ", "Библиотека".to_string(), None),
                View::Search => ("YOUTUBE", "Поиск".to_string(), None),
                View::Playlist(id) => {
                    let p = s.lib.playlists.iter().find(|p| &p.id == id).cloned();
                    (if p.as_ref().is_some_and(|p| p.is_remote()) { "ПЛЕЙЛИСТ · ИМПОРТ" } else { "ПЛЕЙЛИСТ" }, p.as_ref().map(|p| p.name.clone()).unwrap_or_default(), p)
                }
            }
        };
        let total: f64 = ids.iter().filter_map(|i| self.track(i)).map(|t| t.duration).sum();
        let cover = ids.iter().filter_map(|i| self.track(i)).find_map(|t| t.cover.clone().map(|c| if t.video_id.is_some() { format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", t.video_id.unwrap()) } else { c }));
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(120.0), Sense::hover());
            match cover.filter(|_| pl.is_some()).and_then(|c| self.thumbs.get(&c, 240)) {
                Some(t) => { ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE); }
                None => {
                    ui.painter().rect_filled(r, 14, BG3);
                    ui.painter().rect_stroke(r, 14, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, if self.view == View::Search { "\u{E30C}" } else { "\u{E340}" }, FontId::proportional(44.0), ACCENT);
                }
            }
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(eyebrow).size(11.0).color(MUTED));
                ui.label(RichText::new(&title).size(34.0).family(bold()).color(TEXT));
                if self.view == View::Search { ui.label(RichText::new("Ищи треки и добавляй в библиотеку или в плейлист").color(MUTED)); }
                else { ui.label(RichText::new(format!("{} трек(ов) · {}", ids.len(), fmt_time(total))).color(MUTED)); }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if !ids.is_empty() && self.view != View::Search {
                        if pill(ui, "\u{E3D0}  Слушать", true).clicked() { acts.push(Act::Play(ids.to_vec(), 0, false)); }
                        if pill(ui, "\u{E422}  Вперемешку", false).clicked() { acts.push(Act::Play(ids.to_vec(), 0, true)); }
                    }
                    if let Some(p) = &pl {
                        if p.is_remote() && pill(ui, "\u{E094}  Синхронизировать", false).clicked() { acts.push(Act::Sync(p.id.clone())); }
                        if pill(ui, "\u{E3B4}  Переименовать", false).clicked() { acts.push(Act::Modal(Modal::Rename { id: p.id.clone(), name: p.name.clone() })); }
                        if pill(ui, "\u{E4A6}  Удалить", false).clicked() { acts.push(Act::Modal(Modal::Delete { id: p.id.clone() })); }
                    }
                });
            });
        });
        ui.add_space(14.0);
    }

    fn main_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        if let Some(UpdState::Ready(n)) = self.upd.clone() {
            egui::Frame::new().fill(BG3).stroke(Stroke::new(1.0, LINE)).corner_radius(12).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("\u{E036}  Новая версия Wavy (сборка {n}) уже скачана")).color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, "Перезапустить", true).clicked() { acts.push(Act::Restart); } });
                });
            });
            ui.add_space(12.0);
        }
        let ids = self.list_ids();
        self.header(ui, &ids, acts);
        if self.view == View::Search {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.search_q).hint_text("Найти трек на YouTube… или вставь ссылку").desired_width(480.0).margin(egui::Margin::symmetric(12, 8)));
                let go = pill(ui, if self.searching { "Ищу…" } else { "Искать" }, true).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if go && !self.search_q.trim().is_empty() {
                    let q = self.search_q.trim().to_string();
                    if q.starts_with("http") { acts.push(Act::Modal(Modal::ImportUrl { url: q, into: false, busy: false, err: None })); } else { acts.push(Act::Search(q)); }
                }
            });
            ui.add_space(10.0);
            return self.search_results(ui, acts);
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("\u{E30C} Фильтр по названию, артисту, альбому…").desired_width(360.0).margin(egui::Margin::symmetric(12, 8)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if pill(ui, "\u{E2E2}  По ссылке", false).clicked() { acts.push(Act::Modal(Modal::ImportUrl { url: String::new(), into: self.cur_playlist().is_some(), busy: false, err: None })); }
                if pill(ui, "\u{E25A}  Папка", false).clicked() { acts.push(Act::ImportFolder); }
                if pill(ui, "\u{EA20}  Файлы", false).clicked() { acts.push(Act::ImportFiles); }
            });
        });
        ui.add_space(8.0);
        if ids.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                let filt = !self.filter.is_empty();
                ui.label(RichText::new(if filt { "Ничего не нашлось" } else if self.cur_playlist().is_some() { "Плейлист пустой" } else { "Тут пока тихо" }).size(22.0).family(bold()).color(TEXT));
                ui.label(RichText::new("Перетащи файлы/папки в окно или импортируй плейлист с YouTube Music по ссылке").color(MUTED));
                ui.add_space(12.0);
                if !filt && pill(ui, "\u{E3D0}  Плейлист с YouTube", true).clicked() { acts.push(Act::Modal(Modal::ImportUrl { url: String::new(), into: self.cur_playlist().is_some(), busy: false, err: None })); }
            });
            return;
        }
        self.track_table(ui, &ids, acts);
    }

    fn track_table(&mut self, ui: &mut egui::Ui, ids: &[String], acts: &mut Vec<Act>) {
        let cur = self.current.as_ref().map(|t| t.id.clone());
        let playing = self.audio.is_playing();
        let pid = self.cur_playlist();
        let pls: Vec<(String, String)> = self.store.lock().unwrap().lib.playlists.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        let mods = ui.input(|i| i.modifiers);
        TableBuilder::new(ui)
            .striped(false)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(36.0))
            .column(Column::remainder().at_least(200.0).clip(true))
            .column(Column::exact(220.0).clip(true))
            .column(Column::exact(56.0))
            .max_scroll_height(f32::INFINITY)
            .header(26.0, |mut h| {
                for t in ["#", "НАЗВАНИЕ", "АЛЬБОМ", "\u{E19A}"] { h.col(|ui| { ui.label(RichText::new(t).size(11.0).color(MUTED)); }); }
            })
            .body(|body| {
                body.rows(52.0, ids.len(), |mut row| {
                    let i = row.index();
                    let id = &ids[i];
                    let Some(t) = self.store.lock().unwrap().lib.tracks.get(id).cloned() else { return };
                    let is_cur = cur.as_deref() == Some(id.as_str());
                    row.set_selected(self.selected.contains(id));
                    row.col(|ui| {
                        if is_cur { ui.label(RichText::new(if playing { "\u{E340}" } else { "\u{E39E}" }).color(ACCENT)); }
                        else { ui.label(RichText::new((i + 1).to_string()).color(MUTED)); }
                    });
                    row.col(|ui| {
                        self.cover(ui, &t, 40.0);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.add_space(6.0);
                            ui.add(egui::Label::new(RichText::new(&t.title).color(if is_cur { ACCENT } else { TEXT }).size(14.0)).truncate());
                            ui.add(egui::Label::new(RichText::new(&t.artist).color(MUTED).size(12.5)).truncate());
                        });
                    });
                    row.col(|ui| {
                        let album = if !t.album.is_empty() { t.album.clone() } else if t.kind == "youtube" { "YouTube".into() } else if t.kind == "stream" { "Стрим".into() } else { String::new() };
                        ui.add(egui::Label::new(RichText::new(album).color(MUTED).size(13.0)).truncate());
                    });
                    row.col(|ui| { ui.label(RichText::new(fmt_time(t.duration)).color(MUTED)); });
                    let r = row.response();
                    if r.double_clicked() { acts.push(Act::Play(ids.to_vec(), i, false)); }
                    else if r.clicked() {
                        if mods.shift { if let Some(a) = self.last_clicked.as_ref().and_then(|l| ids.iter().position(|x| x == l)) { let (x, y) = (a.min(i), a.max(i)); self.selected.extend(ids[x..=y].iter().cloned()); } }
                        else if mods.command { if !self.selected.remove(id) { self.selected.insert(id.clone()); } self.last_clicked = Some(id.clone()); }
                        else { self.selected = HashSet::from([id.clone()]); self.last_clicked = Some(id.clone()); }
                    }
                    if r.secondary_clicked() && !self.selected.contains(id) { self.selected = HashSet::from([id.clone()]); }
                    let sel: Vec<String> = if self.selected.contains(id) { ids.iter().filter(|x| self.selected.contains(*x)).cloned().collect() } else { vec![id.clone()] };
                    r.context_menu(|ui| track_menu(ui, &sel, &t, pid.as_deref(), &pls, acts));
                });
            });
    }

    fn search_results(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        if self.results.is_empty() {
            ui.add_space(50.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(if self.searching { "Ищу…" } else { "Найди что-нибудь" }).size(22.0).family(bold()).color(TEXT));
                ui.label(RichText::new("Например: «lofi hip hop», «Daft Punk», или вставь ссылку на плейлист").color(MUTED));
            });
            return;
        }
        let results = self.results.clone();
        let ids: Vec<String> = results.iter().map(|t| t.id.clone()).collect();
        let pls: Vec<(String, String)> = self.store.lock().unwrap().lib.playlists.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        let cur = self.current.as_ref().map(|t| t.id.clone());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, t) in results.iter().enumerate() {
                let in_lib = self.store.lock().unwrap().lib.tracks.contains_key(&t.id);
                let r = ui.horizontal(|ui| {
                    ui.set_min_height(52.0);
                    self.cover(ui, t, 40.0);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 330.0).max(120.0));
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(RichText::new(&t.title).color(if cur.as_deref() == Some(&t.id) { ACCENT } else { TEXT })).truncate());
                        ui.add(egui::Label::new(RichText::new(&t.artist).color(MUTED).size(12.5)).truncate());
                    });
                    ui.label(RichText::new(fmt_time(t.duration)).color(MUTED));
                    ui.add_space(10.0);
                    if pill(ui, "\u{E3D0}", false).clicked() { acts.push(Act::Play(ids.clone(), i, false)); }
                    if ui.add_enabled(!in_lib, egui::Button::new(if in_lib { "В библиотеке" } else { "+ В библиотеку" }).corner_radius(99)).clicked() { acts.push(Act::AddTracks(vec![t.clone()])); }
                    ui.menu_button("\u{E1FE}", |ui| track_menu(ui, &[t.id.clone()], t, None, &pls, acts));
                }).response;
                let _ = r;
            }
        });
    }

    fn cover(&mut self, ui: &mut egui::Ui, t: &Track, size: f32) {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        let src = if size <= 64.0 { t.thumb() } else { t.cover.clone() };
        match src.and_then(|s| self.thumbs.get(&s, if size <= 64.0 { 80 } else { 160 })) {
            Some(tex) => { egui::Image::new(&tex).corner_radius(6).paint_at(ui, r); }
            None => {
                ui.painter().rect_filled(r, 6, BG3);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E33C}", FontId::proportional(size * 0.4), MUTED);
            }
        }
        if !t.is_local() { ui.painter().rect_filled(Rect::from_min_size(r.right_bottom() - Vec2::splat(10.0), Vec2::splat(10.0)), 3, Color32::from_rgb(0xff, 0, 0x33)); }
    }

    fn player_bar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let w = ui.available_width();
        let s = self.settings();
        ui.columns_const(|[left, mid, right]| {
            // сейчас играет — клик открывает/закрывает просмотр видео (или большой обложки)
            let np = left.horizontal(|ui| {
                ui.set_max_width(w * 0.3);
                match self.current.clone() {
                    Some(t) => {
                        self.cover(ui, &t, 56.0);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.add_space(8.0);
                            ui.add(egui::Label::new(RichText::new(&t.title).family(semi()).color(TEXT)).truncate());
                            ui.add(egui::Label::new(RichText::new(if self.loading { "загрузка…".to_string() } else { t.artist.clone() }).color(MUTED).size(12.5)).truncate());
                        });
                    }
                    None => { ui.vertical(|ui| { ui.add_space(10.0); ui.label(RichText::new("Ничего не играет").family(semi())); ui.label(RichText::new("Закинь музыку — и погнали").color(MUTED).size(12.5)); }); }
                }
            }).response;
            if self.current.is_some() {
                let has_video = self.current.as_ref().is_some_and(|t| t.has_video());
                let r = left.interact(np.rect.expand(4.0), Id::unique("now-playing"), Sense::click());
                if r.hovered() || self.show_video {
                    left.painter().rect_stroke(np.rect.expand(4.0), 10, Stroke::new(1.0, if self.show_video { TEXT } else { LINE }), egui::StrokeKind::Outside);
                }
                let hint = match (self.show_video, has_video) { (true, _) => "Скрыть", (false, true) => "Смотреть видео", (false, false) => "Показать обложку" };
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hint).clicked() { acts.push(Act::ToggleVideo); }
            }
            // кнопки + перемотка
            mid.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    let bw = 34.0 * 4.0 + 44.0 + 4.0 * 14.0;
                    ui.add_space(((ui.available_width() - bw) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 14.0;
                    if icon_btn(ui, "\u{E422}", 34.0, s.shuffle).on_hover_text("Перемешать").clicked() { acts.push(Act::Shuffle); }
                    if icon_btn(ui, "\u{E5A4}", 34.0, false).clicked() { acts.push(Act::Prev); }
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 21.0, if resp.hovered() { Color32::WHITE } else { TEXT });
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, if self.loading { "…" } else if self.audio.is_playing() { "\u{E39E}" } else { "\u{E3D0}" }, FontId::new(20.0, egui::FontFamily::Name("fill".into())), Color32::BLACK);
                    if resp.clicked() { acts.push(Act::TogglePlay); }
                    if icon_btn(ui, "\u{E5A6}", 34.0, false).clicked() { acts.push(Act::Next); }
                    let rep = match s.repeat.as_str() { "one" => "\u{E3F8}", _ => "\u{E3F6}" };
                    if icon_btn(ui, rep, 34.0, s.repeat != "off").on_hover_text("Повтор").clicked() { acts.push(Act::Repeat); }
                });
                ui.horizontal(|ui| {
                    let dur = if self.audio.duration > 0.0 { self.audio.duration } else { self.current.as_ref().map(|t| t.duration).unwrap_or(0.0) };
                    let pos = self.seek_drag.unwrap_or(self.audio.position()).min(dur.max(0.0));
                    ui.label(RichText::new(fmt_time(pos)).size(12.0).color(MUTED));
                    let sw = (ui.available_width() - 48.0).max(60.0);
                    let (r, new) = bar(ui, if dur > 0.0 { (pos / dur) as f32 } else { 0.0 }, sw);
                    if let Some(f) = new { if r.dragged() { self.seek_drag = Some(f as f64 * dur); } }
                    if r.drag_stopped() || r.clicked() {
                        if let Some(v) = new.map(|f| f as f64 * dur).or(self.seek_drag) { acts.push(Act::Seek(v)); }
                        self.seek_drag = None;
                    }
                    ui.label(RichText::new(fmt_time(dur)).size(12.0).color(MUTED));
                });
            });
            // громкость, очередь
            right.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(4.0);
                let (r, new) = bar(ui, s.volume, 110.0);
                let v = new.unwrap_or(s.volume);
                if let Some(f) = new { acts.push(Act::Volume(f)); }
                if r.drag_stopped() || r.clicked() { acts.push(Act::SaveVolume); }
                ui.label(RichText::new(if v == 0.0 { "\u{E45C}" } else { "\u{E44A}" }).color(MUTED));
                if icon_btn(ui, "\u{E6AC}", 34.0, self.show_queue).on_hover_text("Очередь").clicked() { self.show_queue = !self.show_queue; }
            });
        });
    }

    /// Просмотр: видео (клип с YouTube или локальный файл) либо большая обложка.
    fn video_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click());
        let p = ui.painter().clone();
        let area = if self.fullscreen { rect } else { rect.shrink2(Vec2::new(0.0, 4.0)).with_max_y(rect.bottom() - 16.0) };
        if !self.fullscreen { p.rect_filled(area, 14, Color32::BLACK); }
        let Some(t) = self.current.clone() else {
            p.text(area.center(), Align2::CENTER_CENTER, "Ничего не играет", FontId::new(18.0, semi()), MUTED);
            return;
        };
        let fit = |size: Vec2| { let k = (area.width() / size.x).min(area.height() / size.y); Rect::from_center_size(area.center(), size * k) };
        let tex = self.video.as_ref().and_then(|(_, v)| v.texture.clone());
        let err = self.video_error.as_ref().filter(|e| e.0 == t.id).map(|e| e.1.clone());
        if let Some(tex) = tex {
            p.image(tex.id(), fit(tex.size_vec2()), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        } else {
            // большая обложка (пока видео грузится, при ошибке или если видео нет)
            let side = (area.height() * 0.62).min(area.width() * 0.6).max(80.0);
            let cr = Rect::from_center_size(area.center() - Vec2::new(0.0, 36.0), Vec2::splat(side));
            match t.cover.clone().and_then(|c| self.thumbs.get(&c, 512)) {
                Some(tx) => { egui::Image::new(&tx).corner_radius(14).paint_at(ui, cr); }
                None => { p.rect_filled(cr, 14, BG3); p.text(cr.center(), Align2::CENTER_CENTER, "\u{E340}", FontId::proportional(side * 0.35), MUTED); }
            }
            let y = cr.bottom() + 16.0;
            p.text(Pos2::new(area.center().x, y), Align2::CENTER_TOP, ellipsize(&t.title, area.width() - 40.0, 20.0), FontId::new(20.0, bold()), TEXT);
            p.text(Pos2::new(area.center().x, y + 28.0), Align2::CENTER_TOP, ellipsize(&t.artist, area.width() - 40.0, 14.0), FontId::proportional(14.0), MUTED);
            let note = if let Some(e) = &err { Some((format!("Видео не открылось: {e}"), BAD)) }
                else if t.has_video() { Some(("Загружаю видео…".to_string(), MUTED)) }
                else { None };
            if let Some((n, c)) = note {
                p.text(Pos2::new(area.center().x, y + 54.0), Align2::CENTER_TOP, ellipsize(&n, area.width() - 40.0, 13.0), FontId::proportional(13.0), c);
                if err.is_none() { ui.ctx().request_repaint_after(Duration::from_millis(200)); }
            }
        }
        // кнопка «во весь экран» и подсказка
        let btn = Rect::from_min_size(area.right_top() + Vec2::new(-46.0, 12.0), Vec2::splat(34.0));
        let b = ui.interact(btn, Id::unique("fs-btn"), Sense::click());
        p.circle_filled(btn.center(), 17.0, Color32::from_black_alpha(if b.hovered() { 200 } else { 120 }));
        p.text(btn.center(), Align2::CENTER_CENTER, if self.fullscreen { "\u{E1CE}" } else { "\u{E1D0}" }, FontId::proportional(17.0), Color32::WHITE);
        if b.on_hover_text(if self.fullscreen { "Выйти из полноэкранного (Esc)" } else { "Во весь экран (двойной клик)" }).clicked() || resp.double_clicked() {
            acts.push(Act::Fullscreen(!self.fullscreen));
        }
        if self.fullscreen {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) { acts.push(Act::Fullscreen(false)); }
            // название и прогресс — только когда двигали мышью
            if ui.input(|i| i.pointer.time_since_last_movement()) < 2.5 {
                let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
                let pos = self.audio.position();
                let bar = Rect::from_min_size(Pos2::new(area.left() + 24.0, area.bottom() - 28.0), Vec2::new(area.width() - 48.0, 4.0));
                p.rect_filled(Rect::from_min_max(Pos2::new(area.left(), area.bottom() - 90.0), area.right_bottom()), 0, Color32::from_black_alpha(140));
                p.text(bar.left_top() - Vec2::new(0.0, 34.0), Align2::LEFT_TOP, format!("{} — {}", t.title, t.artist), FontId::new(16.0, semi()), Color32::WHITE);
                p.rect_filled(bar, 2, Color32::from_white_alpha(50));
                if dur > 0.0 { p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * (pos / dur).clamp(0.0, 1.0) as f32, 4.0)), 2, Color32::WHITE); }
                p.text(bar.right_top() - Vec2::new(0.0, 22.0), Align2::RIGHT_TOP, format!("{} / {}", fmt_time(pos), fmt_time(dur)), FontId::proportional(12.0), Color32::from_white_alpha(200));
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
        }
    }

    fn queue_ui(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Очередь").family(semi()).size(15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if icon_btn(ui, "\u{E4F6}", 24.0, false).clicked() { self.show_queue = false; } });
        });
        ui.separator();
        let q = self.queue.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 44.0, q.len(), |ui, range| {
            for i in range {
                let Some(t) = self.track(&q[i]) else { continue };
                let r = ui.horizontal(|ui| {
                    ui.set_min_height(42.0);
                    self.cover(ui, &t, 34.0);
                    ui.vertical(|ui| {
                        let c = if i == self.qi { ACCENT } else if i < self.qi { MUTED.gamma_multiply(0.6) } else { TEXT };
                        ui.add(egui::Label::new(RichText::new(&t.title).color(c)).truncate());
                        ui.add(egui::Label::new(RichText::new(&t.artist).color(MUTED).size(12.0)).truncate());
                    });
                }).response.interact(Sense::click());
                if r.clicked() { acts.push(Act::JumpQueue(i)); }
            }
        });
    }

    fn modal_ui(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(mut m) = self.modal.take() else { return };
        let mut close = false;
        let pls_name = |id: &str| self.store.lock().unwrap().lib.playlists.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
        let cur_pl = self.cur_playlist();
        let s = self.settings();
        let resp = egui::Modal::new(Id::unique("modal")).frame(egui::Frame::popup(&ctx.global_style()).fill(BG2).inner_margin(22).corner_radius(16)).show(ctx, |ui| {
            ui.set_width(500.0);
            match &mut m {
                Modal::ImportUrl { url, into, busy, err } => {
                    ui.label(RichText::new("Импорт по ссылке").size(20.0).family(bold()));
                    ui.label(RichText::new("YouTube Music, YouTube (плейлист, альбом, видео), SoundCloud, Bandcamp — всё, что умеет yt-dlp.").color(MUTED));
                    ui.add_space(10.0);
                    let r = ui.add(egui::TextEdit::singleline(url).hint_text("https://music.youtube.com/playlist?list=...").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                    if url.is_empty() { r.request_focus(); }
                    if let Some(p) = &cur_pl { ui.checkbox(into, format!("Добавить в текущий плейлист «{}»", pls_name(p))); }
                    if *busy { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Загружаю… большие плейлисты — до минуты").color(MUTED)); }); }
                    if let Some(e) = err { ui.label(RichText::new(e.as_str()).color(BAD)); }
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.add_enabled(!*busy, pill_btn("Импортировать", true)).clicked() || (enter && !*busy) {
                            if url.trim().starts_with("http") { *busy = true; *err = None; acts.push(Act::ImportUrl(url.clone(), if *into { cur_pl.clone() } else { None })); }
                            else { *err = Some("Нужна ссылка, начинающаяся с https://".into()); }
                        }
                        if pill(ui, "Отмена", false).clicked() { close = true; }
                    });
                }
                Modal::NewPlaylist { name, ids } => {
                    ui.label(RichText::new("Новый плейлист").size(20.0).family(bold()));
                    ui.add_space(8.0);
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("Название").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                    r.request_focus();
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (pill(ui, "Создать", true).clicked() || enter) && !name.trim().is_empty() { acts.push(Act::CreatePlaylist(name.trim().into(), ids.clone())); close = true; }
                        if pill(ui, "Отмена", false).clicked() { close = true; }
                    });
                }
                Modal::Rename { id, name } => {
                    ui.label(RichText::new("Переименовать плейлист").size(20.0).family(bold()));
                    ui.add_space(8.0);
                    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8))).request_focus();
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (pill(ui, "Сохранить", true).clicked() || enter) && !name.trim().is_empty() { acts.push(Act::Rename(id.clone(), name.trim().into())); close = true; }
                        if pill(ui, "Отмена", false).clicked() { close = true; }
                    });
                }
                Modal::Delete { id } => {
                    ui.label(RichText::new(format!("Удалить «{}»?", pls_name(id))).size(20.0).family(bold()));
                    ui.label(RichText::new("Треки останутся в библиотеке.").color(MUTED));
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, "Удалить", true).clicked() { acts.push(Act::DeletePlaylist(id.clone())); close = true; }
                        if pill(ui, "Отмена", false).clicked() { close = true; }
                    });
                }
                Modal::Settings { client_id, port } => {
                    ui.label(RichText::new("Настройки").size(20.0).family(bold()));
                    egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                        section(ui, "\u{E61A}  Discord Rich Presence");
                        let mut en = s.discord.enabled;
                        if ui.checkbox(&mut en, "Показывать, что я слушаю, в профиле Discord").changed() { acts.push(Act::Settings(json!({"discord": {"enabled": en}}))); }
                        ui.label(RichText::new("APPLICATION ID").size(11.0).color(MUTED));
                        let r = ui.add(egui::TextEdit::singleline(client_id).hint_text("например 1234567890123456789").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 7)));
                        if r.changed() { client_id.retain(|c| c.is_ascii_digit()); } // вставили с пробелами/мусором — оставляем цифры
                        // сохраняем, когда ID похож на полный (Discord ID — 17–20 цифр) или при уходе из поля
                        let full = (17..=20).contains(&client_id.len());
                        if (r.changed() && full) || r.lost_focus() { acts.push(Act::Settings(json!({"discord": {"clientId": client_id.as_str()}}))); }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("Создай приложение на").color(MUTED).size(12.5));
                            if ui.link("discord.com/developers").clicked() { acts.push(Act::OpenUrl("https://discord.com/developers/applications".into())); }
                            ui.label(RichText::new("(его название будет после «Слушает»), скопируй Application ID.").color(MUTED).size(12.5));
                        });
                        let mut b = s.discord.show_buttons;
                        if ui.checkbox(&mut b, "Кнопка «Открыть трек» (для YouTube-треков)").changed() { acts.push(Act::Settings(json!({"discord": {"showButtons": b}}))); }
                        let (txt, c) = if !s.discord.enabled { ("Выключено".to_string(), MUTED) } else if s.discord.client_id.is_empty() { ("Укажи Application ID".into(), MUTED) }
                            else if self.discord_status.connected { ("\u{E182} Подключено".into(), OK) } else { (self.discord_status.error.clone().map(|e| format!("\u{E4F6} {e}")).unwrap_or("Подключаюсь…".into()), BAD) };
                        ui.label(RichText::new(txt).color(c));

                        section(ui, "\u{E466}  Оверлей");
                        ui.label(RichText::new("ДЛЯ OBS — BROWSER SOURCE").size(11.0).color(MUTED));
                        let url = self.overlay.url().unwrap_or_else(|| format!("http://127.0.0.1:{port}/overlay"));
                        ui.horizontal(|ui| {
                            let mut u = url.clone();
                            ui.add(egui::TextEdit::singleline(&mut u).desired_width(340.0).margin(egui::Margin::symmetric(10, 7)));
                            if pill(ui, "Копировать", false).clicked() { ui.ctx().copy_text(url.clone()); acts.push(Act::Settings(json!({}))); self.toast = Some(("Ссылка скопирована — вставь в OBS → Browser Source".into(), false, Instant::now())); }
                        });
                        ui.label(RichText::new("Ширина 500, высота 160. Можно добавить ?theme=minimal или ?theme=vinyl").color(MUTED).size(12.0));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label("Стиль OBS:");
                            let mut th = s.overlay.theme.clone();
                            egui::ComboBox::from_id_salt("theme").selected_text(match th.as_str() { "minimal" => "Минимал", "vinyl" => "Винил", _ => "Карточка" }).show_ui(ui, |ui| {
                                ui.selectable_value(&mut th, "card".into(), "Карточка");
                                ui.selectable_value(&mut th, "minimal".into(), "Минимал");
                                ui.selectable_value(&mut th, "vinyl".into(), "Винил");
                            });
                            if th != s.overlay.theme { acts.push(Act::Settings(json!({"overlay": {"theme": th}}))); }
                            ui.add_space(12.0);
                            ui.label("Скрывать через, сек:");
                            let mut h = s.overlay.auto_hide_sec;
                            if ui.add(egui::DragValue::new(&mut h).range(0..=600)).changed() { acts.push(Act::Settings(json!({"overlay": {"autoHideSec": h}}))); }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Порт:");
                            let r = ui.add(egui::TextEdit::singleline(port).desired_width(70.0));
                            if r.lost_focus() { if let Ok(p) = port.parse::<u16>() { acts.push(Act::Settings(json!({"overlay": {"port": p}}))); } }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if pill(ui, if self.overlay_open { "Скрыть оверлей поверх игр" } else { "Показать оверлей поверх игр" }, self.overlay_open).clicked() { acts.push(Act::ToggleOverlay); }
                            if pill(ui, if s.overlay_window.locked { "\u{E306} Двигать" } else { "\u{E2FA} Закрепить" }, false).clicked() { acts.push(Act::ToggleLock); }
                        });
                        ui.label(RichText::new("Ctrl+Shift+O — показать/скрыть, Ctrl+Shift+L — закрепить/двигать. Поверх игр в режиме «оконный без рамки».").color(MUTED).size(12.0));

                        section(ui, "\u{E4FC}  YouTube (yt-dlp)");
                        match &self.yt_version {
                            None => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Проверяю… (при первом запуске скачивается yt-dlp)").color(MUTED)); }); }
                            Some(Ok(v)) => { ui.label(RichText::new(format!("yt-dlp {v}")).color(OK)); }
                            Some(Err(e)) => { ui.label(RichText::new(e.as_str()).color(BAD)); }
                        }
                        match self.yt.deno_status() {
                            wavy_core::youtube::Deno::Ready(_) => { ui.label(RichText::new("JS-движок deno: готов").color(OK)); }
                            wavy_core::youtube::Deno::Installing => {
                                ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Скачиваю JS-движок deno (~42 МБ, один раз)…").color(MUTED)); });
                                ui.ctx().request_repaint_after(Duration::from_millis(500));
                            }
                            wavy_core::youtube::Deno::Failed(e) => { ui.label(RichText::new(format!("deno: {e}")).color(BAD)); }
                            wavy_core::youtube::Deno::Unknown => {}
                        }
                        ui.horizontal(|ui| {
                            if pill(ui, "Обновить yt-dlp", false).clicked() { acts.push(Act::YtUpdate); }
                            if pill(ui, "Открыть папку с логами", false).on_hover_text("wavy.log — пришли его, если что-то не работает").clicked() {
                                acts.push(Act::Reveal(wavy_core::data_dir().join("wavy.log").to_string_lossy().into_owned()));
                            }
                        });
                        ui.label(RichText::new("Если YouTube перестал играть — почти всегда помогает обновление yt-dlp.").color(MUTED).size(12.0));

                        section(ui, "\u{E036}  Обновления");
                        match update::current_build() {
                            Some(b) => { ui.label(RichText::new(format!("Версия: сборка {b}")).color(TEXT)); }
                            None => { ui.label(RichText::new("Локальная сборка — обновляется только exe из GitHub Releases").color(MUTED)); }
                        }
                        let mut au = s.auto_update;
                        if ui.checkbox(&mut au, "Обновляться автоматически").changed() { acts.push(Act::Settings(json!({"autoUpdate": au}))); }
                        match &self.upd {
                            Some(UpdState::Checking) => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Проверяю…").color(MUTED)); }); }
                            Some(UpdState::Downloading(n)) => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(format!("Скачиваю сборку {n}…")).color(MUTED)); }); }
                            Some(UpdState::Ready(n)) => { ui.label(RichText::new(format!("Сборка {n} установлена, осталось перезапустить")).color(OK)); }
                            Some(UpdState::UpToDate) => { ui.label(RichText::new("У тебя последняя версия").color(OK)); }
                            Some(UpdState::Error(e)) => { ui.label(RichText::new(e.as_str()).color(BAD)); }
                            None => {}
                        }
                        ui.horizontal(|ui| {
                            let busy = matches!(self.upd, Some(UpdState::Checking | UpdState::Downloading(_)));
                            if ui.add_enabled(update::current_build().is_some() && !busy, pill_btn("Проверить обновления", false)).clicked() { acts.push(Act::CheckUpdate); }
                            if matches!(self.upd, Some(UpdState::Ready(_))) && pill(ui, "Перезапустить", true).clicked() { acts.push(Act::Restart); }
                        });
                    });
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, "Готово", true).clicked() { close = true; } });
                }
            }
        });
        if resp.should_close() && !matches!(m, Modal::ImportUrl { busy: true, .. }) { close = true; }
        if !close { self.modal = Some(m); }
    }

    fn toast_ui(&self, ctx: &egui::Context) {
        let Some((msg, err, _)) = &self.toast else { return };
        egui::Area::new(Id::unique("toast")).anchor(Align2::CENTER_BOTTOM, [0.0, -100.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::new().fill(BG3).stroke(Stroke::new(1.0, if *err { BAD } else { LINE })).corner_radius(99).inner_margin(egui::Margin::symmetric(18, 10)).show(ui, |ui| {
                ui.set_max_width(640.0);
                ui.label(RichText::new(msg.as_str()).color(TEXT));
            });
        });
    }

    /// Игровой оверлей: отдельное прозрачное окно поверх всего.
    fn overlay_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let s = self.settings();
        let locked = s.overlay_window.locked;
        let [x, y, w, h] = self.overlay_init;
        let vb = egui::ViewportBuilder::default()
            .with_title("Wavy Overlay").with_decorations(false).with_transparent(true).with_always_on_top()
            .with_taskbar(false).with_resizable(!locked).with_mouse_passthrough(locked).with_active(false)
            .with_position([x, y]).with_inner_size([w, h]);
        let hide_after = s.overlay.auto_hide_sec;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("wavy-overlay"), vb, |ui, _| {
            let rect = ui.max_rect();
            let p = ui.painter().clone();
            let ctx = ui.ctx().clone();
            if !locked {
                p.rect_filled(rect, 8, Color32::from_white_alpha(12));
                p.rect_stroke(rect.shrink(1.0), 8, Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
                p.text(rect.center_top() + Vec2::new(0.0, 10.0), Align2::CENTER_CENTER, "тяни — двигать · угол — ресайз · Ctrl+Shift+L — закрепить", FontId::proportional(11.0), ACCENT_SOFT);
                let resp = ui.interact(rect, Id::unique("ov-drag"), Sense::click_and_drag());
                let grip = Rect::from_min_size(rect.right_bottom() - Vec2::splat(16.0), Vec2::splat(16.0));
                p.line_segment([grip.right_top(), grip.left_bottom()], Stroke::new(2.0, ACCENT));
                if resp.drag_started() {
                    let at = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
                    ctx.send_viewport_cmd(if grip.contains(at) { egui::ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast) } else { egui::ViewportCommand::StartDrag });
                }
                // запоминаем положение
                if let (Some(o), Some(i)) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect)) {
                    let b = [o.min.x, o.min.y, i.width(), i.height()];
                    if b != s.overlay_window.bounds.unwrap_or_default() && !ctx.input(|i| i.pointer.any_down()) {
                        acts.push(Act::Settings(json!({"overlayWindow": {"bounds": b}})));
                    }
                }
            }
            if ctx.input(|i| i.viewport().close_requested()) { acts.push(Act::ToggleOverlay); }
            let Some(t) = self.current.clone() else { return };
            let fade = if hide_after > 0 && locked { let e = self.shown_at.elapsed().as_secs_f32() - hide_after as f32; (1.0 - e.max(0.0) / 0.5).clamp(0.0, 1.0) } else { 1.0 };
            if fade <= 0.0 { return; }
            let theme = s.overlay.theme.as_str();
            let playing = self.audio.is_playing();
            let card = rect.shrink2(Vec2::new(8.0, 8.0)).with_min_y(rect.min.y + if locked { 8.0 } else { 20.0 });
            let a = |c: Color32| c.gamma_multiply(fade);
            let minimal = theme == "minimal";
            if !minimal {
                p.rect_filled(card, 16, a(Color32::from_rgba_unmultiplied(10, 10, 12, 220)));
                p.rect_stroke(card, 16, Stroke::new(1.0, a(Color32::from_white_alpha(20))), egui::StrokeKind::Inside);
            }
            let cs = (card.height() - 20.0).clamp(40.0, 120.0);
            let cr = Rect::from_min_size(card.min + Vec2::new(10.0, (card.height() - cs) / 2.0), Vec2::splat(cs));
            let tex = t.cover.clone().and_then(|c| self.thumbs.get(&c, 160));
            let tx = if minimal { card.left() + 4.0 } else { cr.right() + 14.0 };
            match theme {
                "minimal" => {}
                "vinyl" => {
                    if playing { self.spin = (self.spin + ctx.input(|i| i.stable_dt).min(0.1) * std::f32::consts::TAU / 6.0) % std::f32::consts::TAU; }
                    vinyl(&p, tex.as_ref().map(|t| t.id()), cr.center(), cs / 2.0, self.spin, fade);
                    if playing { ctx.request_repaint_after(Duration::from_millis(33)); }
                }
                _ => match &tex {
                    Some(tex) => { egui::Image::new(tex).corner_radius(10).tint(a(Color32::WHITE)).paint_at(ui, cr); }
                    None => { p.rect_filled(cr, 10, a(BG3)); p.text(cr.center(), Align2::CENTER_CENTER, "\u{E340}", FontId::proportional(cs * 0.4), a(ACCENT)); }
                },
            }
            let tw = card.right() - tx - 14.0;
            let mut yy = if minimal { card.center().y - 38.0 } else { cr.top() + 2.0 };
            // в минимале текст прямо поверх игры — даём ему тень
            let text = |pos: Pos2, align: Align2, txt: String, font: FontId, c: Color32| {
                if minimal { p.text(pos + Vec2::new(1.0, 1.5), align, &txt, font.clone(), Color32::from_black_alpha((200.0 * fade) as u8)); }
                p.text(pos, align, txt, font, c);
            };
            text(Pos2::new(tx, yy), Align2::LEFT_TOP, if playing { "\u{E340}  СЕЙЧАС ИГРАЕТ".into() } else { "\u{E39E}  ПАУЗА".into() }, FontId::proportional(10.5), a(ACCENT_SOFT));
            yy += 16.0;
            text(Pos2::new(tx, yy), Align2::LEFT_TOP, ellipsize(&t.title, tw, 18.0), FontId::new(18.0, bold()), a(Color32::WHITE));
            yy += 24.0;
            text(Pos2::new(tx, yy), Align2::LEFT_TOP, ellipsize(&t.artist, tw, 13.5), FontId::proportional(13.5), a(Color32::from_white_alpha(190)));
            yy += 24.0;
            let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
            let pos = self.resume.as_ref().map(|r| r.1).unwrap_or_else(|| self.audio.position());
            let bw = if minimal { tw.min(320.0) } else { tw };
            let bar = Rect::from_min_size(Pos2::new(tx, yy), Vec2::new(bw, 4.0));
            p.rect_filled(bar, 2, a(Color32::from_white_alpha(40)));
            if dur > 0.0 { p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bw * (pos / dur).clamp(0.0, 1.0) as f32, 4.0)), 2, a(ACCENT)); }
            if !minimal {
                p.text(Pos2::new(tx, yy + 8.0), Align2::LEFT_TOP, fmt_time(pos), FontId::proportional(10.5), a(Color32::from_white_alpha(140)));
                p.text(Pos2::new(tx + tw, yy + 8.0), Align2::RIGHT_TOP, fmt_time(dur), FontId::proportional(10.5), a(Color32::from_white_alpha(140)));
            }
            if hide_after > 0 && fade > 0.0 { ctx.request_repaint_after(Duration::from_millis(100)); }
        });
    }
}

// ---------- мелкие виджеты ----------
fn track_menu(ui: &mut egui::Ui, ids: &[String], t: &Track, pid: Option<&str>, pls: &[(String, String)], acts: &mut Vec<Act>) {
    let ids = ids.to_vec();
    if ui.button("Играть далее").clicked() { acts.push(Act::PlayNext(ids.clone())); ui.close(); }
    if ui.button("Добавить в очередь").clicked() { acts.push(Act::Enqueue(ids.clone())); ui.close(); }
    ui.separator();
    ui.menu_button("Добавить в плейлист", |ui| {
        if ui.button("+ Новый плейлист…").clicked() { acts.push(Act::Modal(Modal::NewPlaylist { name: String::new(), ids: ids.clone() })); ui.close(); }
        if !pls.is_empty() { ui.separator(); }
        for (id, name) in pls { if ui.button(name).clicked() { acts.push(Act::AddTo(id.clone(), ids.clone())); ui.close(); } }
    });
    if let Some(p) = pid { if ui.button("Убрать из плейлиста").clicked() { acts.push(Act::RemoveFrom(p.into(), ids.clone())); ui.close(); } }
    if ids.len() == 1 {
        if let Some(p) = &t.path { if ui.button("Показать в папке").clicked() { acts.push(Act::Reveal(p.clone())); ui.close(); } }
        if let Some(u) = &t.url { if ui.button("Открыть в браузере").clicked() { acts.push(Act::OpenUrl(u.clone())); ui.close(); } }
    }
    ui.separator();
    let label = if ids.len() > 1 { format!("Удалить из библиотеки ({})", ids.len()) } else { "Удалить из библиотеки".into() };
    if ui.button(RichText::new(label).color(BAD)).clicked() { acts.push(Act::RemoveTracks(ids)); ui.close(); }
}

/// Тонкая полоска-слайдер (перемотка, громкость): белое заполнение, ручка при наведении.
fn bar(ui: &mut egui::Ui, frac: f32, width: f32) -> (egui::Response, Option<f32>) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::click_and_drag());
    let new = if resp.dragged() || resp.clicked() {
        resp.interact_pointer_pos().map(|p| ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0))
    } else { None };
    let f = new.unwrap_or(frac).clamp(0.0, 1.0);
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 4.0));
    let active = resp.hovered() || resp.dragged();
    ui.painter().rect_filled(track, 2, Color32::from_rgb(0x2c, 0x2c, 0x33));
    ui.painter().rect_filled(Rect::from_min_size(track.min, Vec2::new(track.width() * f, 4.0)), 2, if active { Color32::WHITE } else { TEXT });
    if active { ui.painter().circle_filled(Pos2::new(track.left() + track.width() * f, track.center().y), 6.0, Color32::WHITE); }
    (resp, new)
}

fn nav(ui: &mut egui::Ui, text: &str, active: bool, count: Option<usize>) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    if active || resp.hovered() { ui.painter().rect_filled(r, 8, BG3); }
    let c = if active || resp.hovered() { TEXT } else { MUTED };
    let w = r.width() - if count.is_some() { 50.0 } else { 20.0 };
    ui.painter().text(r.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, ellipsize(text, w, 14.0), FontId::proportional(14.0), c);
    if let Some(n) = count { ui.painter().text(r.right_center() - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER, n.to_string(), FontId::proportional(12.0), MUTED); }
    resp
}

fn icon_btn(ui: &mut egui::Ui, icon: &str, size: f32, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if resp.hovered() { ui.painter().circle_filled(r.center(), size / 2.0, BG3); }
    let c = if on { ACCENT } else if resp.hovered() { TEXT } else { MUTED };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(size * 0.5), c);
    if on { ui.painter().circle_filled(r.center_bottom() - Vec2::new(0.0, 2.0), 2.0, ACCENT); }
    resp
}

fn pill_btn(text: &str, primary: bool) -> egui::Button<'_> {
    let b = egui::Button::new(RichText::new(text).color(if primary { BG } else { TEXT }).family(semi())).corner_radius(99).min_size(Vec2::new(0.0, 32.0));
    if primary { b.fill(ACCENT_DIM).stroke(Stroke::NONE) } else { b.fill(BG3).stroke(Stroke::new(1.0, LINE)) }
}
fn pill(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response { ui.add(pill_btn(text, primary)) }

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(15.0).family(semi()).color(TEXT));
    ui.add_space(4.0);
}

/// Пластинка: обложка натягивается на круг (меш-веер) и крутится.
fn vinyl(p: &egui::Painter, tex: Option<egui::TextureId>, c: Pos2, r: f32, rot: f32, fade: f32) {
    let a = |col: Color32| col.gamma_multiply(fade);
    p.circle_filled(c, r, a(Color32::from_rgb(12, 12, 14)));
    let inner = r * 0.94;
    match tex {
        Some(id) => {
            let mut m = egui::Mesh::with_texture(id);
            let n = 64;
            m.vertices.push(egui::epaint::Vertex { pos: c, uv: Pos2::new(0.5, 0.5), color: a(Color32::WHITE) });
            for i in 0..=n {
                let ang = i as f32 / n as f32 * std::f32::consts::TAU;
                let (sn, cs) = ang.sin_cos();
                let (us, uc) = (ang + rot).sin_cos();
                m.vertices.push(egui::epaint::Vertex { pos: c + Vec2::new(cs, sn) * inner, uv: Pos2::new(0.5 + 0.5 * uc, 0.5 + 0.5 * us), color: a(Color32::WHITE) });
                if i > 0 { m.add_triangle(0, i as u32, i as u32 + 1); }
            }
            p.add(m);
        }
        None => { p.circle_filled(c, inner, a(ACCENT)); }
    }
    for k in [0.55f32, 0.75] { p.circle_stroke(c, r * k, Stroke::new(1.0, a(Color32::from_black_alpha(60)))); }
    p.circle_filled(c, r * 0.16, a(Color32::from_rgb(12, 12, 14)));
    p.circle_stroke(c, r * 0.16, Stroke::new(1.5, a(Color32::from_white_alpha(60))));
    // блик
    let (sn, cs) = rot.sin_cos();
    p.circle_filled(c + Vec2::new(cs, sn) * r * 0.62, r * 0.05, a(Color32::from_white_alpha(70)));
}

fn drop_hint(ctx: &egui::Context) {
    egui::Area::new(Id::unique("drop")).fixed_pos(Pos2::ZERO).order(egui::Order::Foreground).show(ctx, |ui| {
        let r = ctx.content_rect();
        ui.painter().rect_filled(r, 0, Color32::from_rgba_unmultiplied(10, 10, 12, 220));
        ui.painter().rect_stroke(r.shrink(40.0), 24, Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "Отпусти, чтобы добавить музыку \u{E340}", FontId::proportional(22.0), TEXT);
    });
}

fn ellipsize(s: &str, width: f32, size: f32) -> String {
    let max = (width / (size * 0.52)).max(3.0) as usize;
    if s.chars().count() <= max { s.to_string() } else { s.chars().take(max - 1).collect::<String>() + "…" }
}

fn shuffle_vec<T>(v: &mut [T]) {
    let mut seed = wavy_core::now_ms() ^ 0x9E37_79B9_7F4A_7C15;
    for i in (1..v.len()).rev() {
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        v.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

fn semi() -> egui::FontFamily { egui::FontFamily::Name("semi".into()) }
fn bold() -> egui::FontFamily { egui::FontFamily::Name("bold".into()) }

fn style(ctx: &egui::Context) {
    // Inter (OFL) — основной шрифт; иконки Phosphor (MIT) — запасной, плюс «fill»-вариант для кнопки play
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("inter".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/Inter-Regular.otf"))));
    fonts.font_data.insert("inter-semi".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/Inter-SemiBold.otf"))));
    fonts.font_data.insert("inter-bold".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/Inter-Bold.otf"))));
    // из Inter вырезаны его глифы в Private Use Area (assets/README), чтобы не перекрывать иконки
    fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().splice(0..0, ["inter".to_string(), "phosphor".to_string()]);
    fonts.font_data.insert("phosphor".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/Phosphor.ttf"))));
    fonts.font_data.insert("phosphor-fill".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/Phosphor-Fill.ttf"))));
    let mut fill = fonts.families[&egui::FontFamily::Proportional].clone();
    fill.insert(0, "phosphor-fill".into());
    fonts.families.insert(egui::FontFamily::Name("fill".into()), fill);
    for (name, font) in [("semi", "inter-semi"), ("bold", "inter-bold")] {
        let mut f = fonts.families[&egui::FontFamily::Proportional].clone();
        f[0] = font.into(); // [inter-…, phosphor, …]
        fonts.families.insert(egui::FontFamily::Name(name.into()), f);
    }
    ctx.set_fonts(fonts);
    ctx.set_visuals(egui::Visuals::dark());
    ctx.all_styles_mut(|s| {
        let v = &mut s.visuals;
        v.panel_fill = BG;
        v.window_fill = BG2;
        v.extreme_bg_color = BG3;
        v.faint_bg_color = BG2;
        v.selection.bg_fill = Color32::from_white_alpha(14);
        v.selection.stroke = Stroke::new(1.0, TEXT);
        v.hyperlink_color = ACCENT;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.inactive.bg_fill = BG3;
        v.widgets.inactive.weak_bg_fill = BG3;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x20, 0x20, 0x26);
        v.widgets.hovered.bg_fill = Color32::from_rgb(0x2a, 0x2a, 0x30);
        v.widgets.active.bg_fill = Color32::from_rgb(0x34, 0x34, 0x3a);
        v.widgets.inactive.corner_radius = CornerRadius::same(8);
        v.widgets.hovered.corner_radius = CornerRadius::same(8);
        v.widgets.active.corner_radius = CornerRadius::same(8);
        v.slider_trailing_fill = true;
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(14.0, 6.0);
        s.text_styles.insert(egui::TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(14.0));
    });
}

fn trim_cache(dir: &Path, max: u64, target: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd.flatten().filter_map(|e| { let m = e.metadata().ok()?; Some((m.modified().ok()?, m.len(), e.path())) }).collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max { return; }
    files.sort();
    for (_, len, p) in files { if total <= target { break; } if std::fs::remove_file(&p).is_ok() { total -= len; } }
}

fn reveal(path: &str) {
    #[cfg(windows)] { let _ = std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn(); }
    #[cfg(target_os = "macos")] { let _ = std::process::Command::new("open").args(["-R", path]).spawn(); }
    #[cfg(all(unix, not(target_os = "macos")))] { if let Some(d) = Path::new(path).parent() { let _ = std::process::Command::new("xdg-open").arg(d).spawn(); } }
}

fn open_url(url: &str) {
    if !url.starts_with("http") { return; }
    #[cfg(windows)] { let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn(); }
    #[cfg(target_os = "macos")] { let _ = std::process::Command::new("open").arg(url).spawn(); }
    #[cfg(all(unix, not(target_os = "macos")))] { let _ = std::process::Command::new("xdg-open").arg(url).spawn(); }
}

fn main() -> eframe::Result {
    // запуск после автообновления: даём старому процессу закрыться (порт оверлея, файлы)
    if std::env::args().any(|a| a == "--restarted") { std::thread::sleep(Duration::from_millis(1500)); }
    // паника → в лог (в Windows-сборке нет консоли)
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        wavy_core::log(&format!("ПАДЕНИЕ: {info}\n{}", std::backtrace::Backtrace::force_capture()));
        default_hook(info);
    }));
    wavy_core::log(&format!("запуск Wavy {} сборка {:?} ({} {})", env!("CARGO_PKG_VERSION"), update::current_build(), std::env::consts::OS, std::env::consts::ARCH));
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Wavy").with_inner_size([1200.0, 780.0]).with_min_inner_size([860.0, 560.0]).with_drag_and_drop(true)
            .with_icon(Arc::new(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default())),
        ..Default::default()
    };
    eframe::run_native("Wavy", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
