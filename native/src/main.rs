// Wavy — нативная версия (egui + rodio). Без браузерного движка: минимум памяти.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod media;
mod update;
mod video;
mod player;
mod thumbs;
mod export;
mod tray;
mod fx;

use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use global_hotkey::{hotkey::{Code, HotKey, Modifiers}, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wavy_core::{anime, discord::{Discord, DiscordStatus}, fmt_time, local, media::{MediaInfo, SubSrc}, overlay::OverlayServer, shazam::Found, shikimori::ShikiAnime, store::Store, youtube::YouTube, PlayerState, Track};

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

const AUDIO_FILTER: &[&str] = &["mp3", "flac", "wav", "ogg", "oga", "opus", "m4a", "aac", "alac", "aiff", "mp4", "m4v", "mov", "mkv", "m3u", "m3u8"];
/// Плейлист, куда Shazam складывает распознанные опенинги.
const OPENINGS: &str = "Опенинги";

#[derive(Clone, Copy, PartialEq)]
enum Sleep { At(Instant), EndOfTrack }

#[derive(Clone, PartialEq)]
enum View { Library, Liked, Stats, Playlist(String), Search, Anime, AnimeTitle(String) }

#[derive(Clone)]
enum ShazamState { Listening, Found(Found), NotFound, Error(String), Adding }

#[derive(Clone)]
enum UpdState { Checking, Downloading(u32), Ready(u32), UpToDate, Error(String) }

enum Bg {
    Status(String),
    OpenPlaylist(String),
    Search(Result<Vec<Track>, String>),
    Ready { token: u64, path: PathBuf },
    /// видео трека пропало с YouTube — нашли такой же трек в другом видео
    Replaced(Track),
    ReadyStream { token: u64, dl: wavy_core::stream::Download },
    VideoDownloading { id: String, dl: wavy_core::stream::Download },
    LoadFailed { token: u64, err: String },
    Discord(DiscordStatus),
    YtVersion(Result<String, String>),
    VideoReady { id: String, path: PathBuf },
    VideoFailed { id: String, err: String },
    Update(UpdState),
    ImportDone(Result<(), String>),
    OpenAnime(Option<String>),
    ShikiResults(Result<Vec<ShikiAnime>, String>),
    Shazam(Result<Option<Found>, String>),
    Exported(Result<export::Report, String>),
    Changes(Result<Vec<(String, String)>, String>),
    Lyrics(String, Result<wavy_core::lyrics::Lyrics, String>),
    Radio(String, Result<Vec<Track>, String>),
    Skips(String, Vec<wavy_core::aniskip::Skip>),
    /// вход/синхронизация с Шикимори: обновлённый аккаунт (токены) или ошибка
    Shiki(Result<wavy_core::shikimori::Account, String>, bool),
}

enum Modal {
    ImportUrl { url: String, into: bool, busy: bool, err: Option<String> },
    NewPlaylist { name: String, ids: Vec<String> },
    Rename { id: String, name: String },
    Delete { id: String },
    Settings { client_id: String, port: String, sp_id: String, sp_secret: String },
    AnimeMatch { id: String, query: String, results: Vec<ShikiAnime>, busy: bool, err: Option<String> },
    DeleteAnime { id: String },
    Shazam { state: ShazamState, to_openings: bool },
    /// изменения между сборками: (с, по, загруженный список)
    WhatsNew { from: u32, to: u32, items: Option<Result<Vec<(String, String)>, String>> },
    EditTrack { id: String, title: String, artist: String, album: String },
    Dupes { groups: Vec<Vec<String>> },
    Folder { id: String, name: String },
}

enum Act {
    Play(Vec<String>, usize, bool),
    TogglePlay, Next, Prev, Seek(f64), Volume(f32), SaveVolume, Shuffle, Repeat,
    View(View), ImportFiles, ImportFolder, ImportPaths(Vec<PathBuf>, Option<String>), ImportUrl(String, Option<String>),
    Search(String), AddTracks(Vec<Track>), PlayNext(Vec<String>), Enqueue(Vec<String>),
    AddTo(String, Vec<String>), RemoveFrom(String, Vec<String>), RemoveTracks(Vec<String>),
    CreatePlaylist(String, Vec<String>), Rename(String, String), DeletePlaylist(String), Sync(String),
    Reveal(String), OpenUrl(String), Settings(serde_json::Value), ToggleOverlay, ToggleLock, JumpQueue(usize),
    Modal(Modal), YtUpdate, SyncExport, SyncImport(Option<PathBuf>), ToggleVideo, Fullscreen(bool), CheckUpdate, Restart,
    ImportAnime(Vec<PathBuf>), ImportAnimeFiles, ImportAnimeFolder, PlayAnime(String, Option<String>), AnimeSearch(String), AnimeApply(String, ShikiAnime),
    DeleteAnime(String), MarkWatched(String, bool), AudioTrack(usize), Subs(Option<usize>), Shazam, ShazamAdd(Found, bool),
    /// треки → MP3 в папку; Some(название) — плейлист целиком (своя подпапка)
    Export(Vec<String>, Option<String>),
    /// (ключ: "library" или id плейлиста, способ)
    Sort(String, String),
    PlaylistCover(String), PlaylistCoverReset(String),
    Like(Vec<String>, bool),
    /// перетаскивание в текущем списке (плейлист / «Любимое»): треки, куда (индекс)
    Move(Vec<String>, usize),
    MoveQueue(usize, usize),
    WhatsNew(u32, u32), ToggleMini, ShowWindow, Quit,
    EditTrack(String, String, String, String), TrackCover(String), MergeDupes(Vec<Vec<String>>), SetFolder(String, Option<String>), Radio(String),
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
    /// Что и когда последний раз ушло в оверлей: (позиция, играло ли, когда) — оверлей досчитывает время сам, мы сверяем.
    pushed_clock: (f64, bool, Instant),
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
    video_dl: Option<(String, wavy_core::stream::Download)>,
    video_error: Option<(String, String)>,
    upd: Option<UpdState>,
    upd_checked: Option<Instant>,
    started: Instant,
    /// положение и размер окна до полного экрана — чтобы вернуть их после
    pre_fullscreen: Option<(Pos2, Vec2, bool)>,
    restore_window: Option<(Pos2, Vec2, bool, u8)>,
    _hotkeys: Option<GlobalHotKeyManager>,
    hk_rx: Receiver<u32>,
    hk_ids: (u32, u32),
    /// Дорожки текущего видео и что выбрано (звук — номер среди звуковых, субтитры — номер в списке).
    media_info: Option<(String, MediaInfo)>,
    audio_sel: usize,
    sub_sel: Option<usize>,
    /// серия, уже отмеченная просмотренной (чтобы не писать на диск каждый кадр)
    watched_mark: Option<String>,
    /// ffmpeg для HEVC/10 бит/VP9/AV1 и звука Opus/AC3/DTS (качается по требованию)
    ff: wavy_core::ffmpeg::Ffmpeg,
    /// звук не открылся без ffmpeg — переоткрыть трек, когда он скачается
    reload_after_ff: bool,
    exporting: bool,
    /// (трек, сколько раз уже восстанавливали после обрыва)
    cut_retries: Option<(String, u8)>,
    /// трек, прослушивание которого уже засчитано (после 30 секунд)
    counted: Option<String>,
    /// перетаскиваемые треки и куда бросить; строка под мышью (для сердечка)
    drag_ids: Option<Vec<String>>,
    drop_at: Option<usize>,
    hover_row: Option<String>,
    drag_queue: Option<usize>,
    muted_vol: f32,
    focus_filter: bool,
    quitting: bool,
    sleep: Option<Sleep>,
    /// треки радио (не в библиотеке, только в очереди)
    radio: Vec<Track>,
    show_lyrics: bool,
    lyrics: Option<(String, Option<Result<wavy_core::lyrics::Lyrics, String>>)>,
    lyrics_line: Option<usize>,
    stats_period: u8,
    skips: Option<(String, Vec<wavy_core::aniskip::Skip>)>,
    auto_skipped: HashSet<String>,
    /// «следующая серия через…»: (серия, когда переключить); серия, где человек выбрал остаться
    next_ep: Option<(String, Instant)>,
    stay_ep: Option<String>,
    shiki_code: String,
    /// трек, для которого уже начат кроссфейд
    xfaded: Option<String>,
    tray: Option<(tray::Tray, Receiver<tray::Cmd>)>,
    /// шрифт с иероглифами уже подгружен (или искали и не нашли)
    cjk_loaded: bool,
    cjk_check: Instant,
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
        let ff = wavy_core::ffmpeg::Ffmpeg::new(dir.join("bin"));
        let mut audio = player::Audio::new();
        audio.ffmpeg = ff.path();
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
            last_pushed: (None, false), pushed_clock: (0.0, false, Instant::now()), shown_at: Instant::now(),
            modal: None, toast, show_queue: false, yt_version: None,
            media, media_rx, resume: None, session_saved: Instant::now(), spin: 0.0,
            show_video: false, fullscreen: false, video: None, video_loading: None, video_dl: None, video_error: None, upd: None, upd_checked: None, started: Instant::now(), pre_fullscreen: None, restore_window: None,
            _hotkeys: hotkeys, hk_rx, hk_ids: (ko.id(), kl.id()),
            media_info: None, audio_sel: 0, sub_sel: None, watched_mark: None,
            ff, reload_after_ff: false, exporting: false, cut_retries: None, counted: None, drag_ids: None, drop_at: None, hover_row: None, drag_queue: None, muted_vol: 0.0, focus_filter: false, quitting: false, sleep: None, radio: Vec::new(), show_lyrics: false, lyrics: None, lyrics_line: None, stats_period: 0, skips: None, auto_skipped: HashSet::new(), next_ep: None, stay_ep: None, shiki_code: String::new(), xfaded: None, tray: None, cjk_loaded: false, cjk_check: Instant::now() - Duration::from_secs(60),
        };
        app.fix_episode_numbers();
        app.overlay_open = s.overlay_window.visible;
        app.tray = tray::new(cc);
        // после обновления — показать, что поменялось
        if let Some(cur) = update::current_build() {
            match s.last_build { Some(m) if m < cur => app.apply(Act::WhatsNew(m, cur), &ctx), _ => {} }
            if s.last_build != Some(cur) { app.store.lock().unwrap().patch_settings(&json!({"lastBuild": cur})); }
        }
        app.restore_session(&ctx);
        // «Открыть с помощью Wavy»
        let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).filter(|p| p.exists()).collect();
        if !args.is_empty() { app.apply(Act::ImportPaths(args, None), &ctx); }
        app
    }

    /// Серии, у которых раньше не распознался номер (например, «Title_[01]_[AniLibria_TV]»), — разобрать заново.
    fn fix_episode_numbers(&mut self) {
        let mut s = self.store.lock().unwrap();
        let fix: Vec<(String, anime::Parsed)> = s.lib.tracks.values().filter(|t| t.anime.is_some() && t.episode.is_none())
            .filter_map(|t| { let p = anime::parse_path(Path::new(t.path.as_deref()?)); p.episode.map(|_| (t.id.clone(), p)) }).collect();
        if fix.is_empty() { return; }
        let mut ids = HashSet::new();
        for (id, p) in &fix {
            let path = PathBuf::from(s.lib.tracks[id].path.clone().unwrap_or_default());
            let fresh = anime::episode_track(&path, p, "");
            if let Some(t) = s.lib.tracks.get_mut(id) { t.title = fresh.title; t.episode = fresh.episode; ids.extend(t.anime.clone()); }
        }
        for a in ids { s.sort_anime(&a); }
        s.save();
    }

    /// Нужен ffmpeg: скачать (один раз), по готовности — перерисовать.
    fn need_ffmpeg(&mut self, ctx: &egui::Context) {
        let c = ctx.clone();
        self.ff.install_background(move || c.request_repaint());
    }

    fn session_file() -> PathBuf { wavy_core::data_dir().join("session.json") }

    fn save_session(&mut self) {
        self.session_saved = Instant::now();
        let pos = self.resume.as_ref().map(|r| r.1).unwrap_or_else(|| self.audio.position());
        let v = json!({ "queue": self.queue, "qi": self.qi, "position": pos });
        let _ = std::fs::write(Self::session_file(), v.to_string());
        if let Some(t) = self.current.as_ref().filter(|t| t.anime.is_some()) { self.store.lock().unwrap().set_progress(&t.id, pos); }
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
        self.store.lock().unwrap().lib.tracks.get(id).cloned().or_else(|| self.results.iter().chain(self.radio.iter()).find(|t| t.id == id).cloned())
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
            View::Liked => s.lib.liked.iter().filter(|t| s.lib.tracks.contains_key(*t)).cloned().collect(),
            View::Playlist(id) => s.lib.playlists.iter().find(|p| &p.id == id).map(|p| p.track_ids.iter().filter(|t| s.lib.tracks.contains_key(*t)).cloned().collect()).unwrap_or_default(),
            View::Search => return self.results.iter().map(|t| t.id.clone()).collect(),
            View::Anime | View::AnimeTitle(_) | View::Stats => return vec![],
        };
        let (key, pl) = match &self.view { View::Playlist(id) => (id.clone(), true), View::Liked => ("liked".to_string(), true), _ => ("library".to_string(), false) };
        let ids = s.sorted(ids, s.lib.settings.sort.get(&key).map(String::as_str).unwrap_or(""), pl);
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
        // новый запуск трека — новое прослушивание (а продолжение после обрыва — то же самое)
        if self.resume.as_ref().map(|r| &r.0) != Some(&id) { self.counted = None; }
        self.token += 1;
        let token = self.token;
        self.current = Some(t.clone());
        self.shown_at = Instant::now();
        self.audio.stop();
        self.prepare_media(&t);
        if t.is_local() {
            let path = PathBuf::from(t.path.clone().unwrap_or_default());
            self.start_file(&path);
        } else {
            self.loading = true;
            self.push_state();
            let (yt, tx, cache, c) = (self.yt.clone(), self.tx.clone(), self.cache.clone(), ctx.clone());
            let next = self.queue.get(self.qi + 1).and_then(|n| self.track(n)).filter(|n| !n.is_local());
            std::thread::spawn(move || {
                let t0 = Instant::now();
                let mut res = fetch_audio(&yt, &t, &cache);
                // видео удалено / закрыто / недоступно в стране — ищем этот же трек в другом видео
                if let Err(e) = &res {
                    if gone_from_youtube(e) && t.video_id.is_some() {
                        wavy_core::log(&format!("«{}»: видео недоступно ({e}) — ищу замену", t.title));
                        let _ = tx.send(Bg::Status(format!("«{}» больше нет на YouTube — ищу такой же трек…", t.title)));
                        c.request_repaint();
                        if let Ok(Some(n)) = wavy_core::spotify::find_replacement(&yt, &t) {
                            wavy_core::log(&format!("«{}»: замена {:?}", t.title, n.video_id));
                            let r2 = fetch_audio(&yt, &n, &cache);
                            if r2.is_ok() { let _ = tx.send(Bg::Replaced(n)); res = r2; }
                        }
                    }
                }
                let msg = match res {
                    Ok(AudioSrc::File(path)) => Bg::Ready { token, path },
                    Ok(AudioSrc::Stream(dl)) => Bg::ReadyStream { token, dl },
                    Err(err) => Bg::LoadFailed { token, err },
                };
                wavy_core::log(&format!("трек «{}»: старт через {:.1} с", t.title, t0.elapsed().as_secs_f32()));
                let _ = tx.send(msg);
                c.request_repaint();
                // предзагрузка следующего трека целиком — переключение будет мгновенным
                if let Some(n) = next { if let Ok(AudioSrc::Stream(dl)) = fetch_audio(&yt, &n, &cache) { dl.wait_for(u64::MAX, Duration::from_secs(600)); } }
            });
        }
    }

    /// Дорожки видеофайла: какую озвучку и какие субтитры включить (для аниме — как в прошлой серии).
    fn prepare_media(&mut self, t: &Track) {
        self.media_info = None;
        self.audio_sel = 0;
        self.sub_sel = None;
        if !(t.is_local() && t.video) { return; }
        let info = wavy_core::media::probe(Path::new(t.path.as_deref().unwrap_or_default()));
        let (ap, sp) = t.anime.as_ref().and_then(|a| self.store.lock().unwrap().anime(a).map(|a| (a.audio, a.subs.clone()))).unwrap_or_default();
        self.audio_sel = wavy_core::media::pick_audio(&info, ap);
        self.sub_sel = wavy_core::media::pick_subs(&info, sp.as_deref());
        if t.anime.is_some() { self.store.lock().unwrap().set_progress(&t.id, 0.0); }
        self.media_info = Some((t.id.clone(), info));
    }

    fn sub_src(&self) -> Option<SubSrc> {
        let (_, info) = self.media_info.as_ref()?;
        Some(info.subs.get(self.sub_sel?)?.src.clone())
    }

    fn start_stream(&mut self, dl: wavy_core::stream::Download) {
        let vol = self.settings().volume;
        match self.audio.load_stream(dl, vol * vol) {
            Ok(()) => {
                self.errors_in_row = 0;
                let cur = self.current.as_ref().map(|t| t.id.clone());
                if let Some((id, pos, autoplay)) = self.resume.take() {
                    if Some(&id) == cur.as_ref() {
                        if pos > 1.0 { self.audio.seek(pos); } // прыгнет, когда этот кусок докачается
                        if !autoplay { self.audio.pause(); }
                    }
                }
            }
            Err(e) => self.track_failed(e),
        }
        self.loading = false;
        self.push_state();
    }

    fn start_file(&mut self, path: &Path) {
        let vol = self.settings().volume;
        match self.audio.load_track(path, vol * vol, self.audio_sel) {
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
        if err.starts_with(wavy_core::ffmpeg::NEED) && matches!(self.ff.state(), wavy_core::ffmpeg::State::Missing | wavy_core::ffmpeg::State::Downloading(_)) {
            // звук в Opus/AC3/DTS… — качаем декодер и переоткроем трек с того же места
            self.reload_after_ff = true;
            self.loading = false;
            self.toast("Скачиваю декодер для этого формата (≈6 МБ, один раз)…", false);
            return;
        }
        let err = wavy_core::ffmpeg::human(&err).to_string();
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

    /// Трек «доиграл», хотя до конца далеко: оборвалась загрузка с YouTube (ссылка протухла, 403, сеть).
    /// Раньше плеер в этот момент просто включал следующую песню. Теперь перезапускает загрузку и
    /// продолжает с того же места — до двух раз на трек, потом уже следующая.
    fn recover_cut(&mut self, ctx: &egui::Context) -> bool {
        let Some(t) = self.current.clone() else { return false };
        if t.is_local() || t.duration < 15.0 { return false; }
        let pos = self.audio.position();
        let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
        if pos >= dur - 6.0 { return false; }
        let n = match &self.cut_retries { Some((id, n)) if *id == t.id => *n, _ => 0 };
        if n >= 2 { wavy_core::log(&format!("«{}» снова оборвался на {pos:.0} с — дальше", t.title)); return false; }
        self.cut_retries = Some((t.id.clone(), n + 1));
        wavy_core::log(&format!("«{}» оборвался на {pos:.0} из {dur:.0} с — докачиваю и продолжаю с того же места", t.title));
        if let Some(v) = &t.video_id { self.yt.forget(v); }
        self.resume = Some((t.id.clone(), pos, true));
        self.load_current(ctx);
        true
    }

    fn push_state(&mut self) {
        let duration = if self.audio.duration > 0.0 { self.audio.duration } else { self.current.as_ref().map(|t| t.duration).unwrap_or(0.0) };
        let position = self.resume.as_ref().map(|r| r.1).unwrap_or_else(|| self.audio.position());
        let st = PlayerState { track: self.current.clone(), playing: self.audio.is_playing(), position, duration };
        self.media.update(st.track.as_ref(), st.playing, st.position, st.duration);
        self.last_pushed = (st.track.as_ref().map(|t| t.id.clone()), st.playing);
        self.pushed_clock = (st.position, st.playing, Instant::now());
        self.discord.update(&st);
        self.overlay.update(&st);
    }

    /// Оверлей (OBS) и системный плеер сами досчитывают время от последнего обновления. Если трек долго
    /// грузился или позиция ушла (докачка, перемотка в недокачанное), их часы расходятся со звуком —
    /// раз в секунду сверяем и поправляем. Discord не трогаем: у него лимит на частоту обновлений.
    fn sync_clock(&mut self) {
        let (pos, playing, at) = self.pushed_clock;
        if self.loading || self.current.is_none() || at.elapsed().as_secs_f64() < 1.0 { return; }
        let expected = pos + if playing { at.elapsed().as_secs_f64() } else { 0.0 };
        let actual = self.audio.position();
        if (expected - actual).abs() < 0.75 { return; }
        let duration = if self.audio.duration > 0.0 { self.audio.duration } else { self.current.as_ref().map(|t| t.duration).unwrap_or(0.0) };
        let st = PlayerState { track: self.current.clone(), playing: self.audio.is_playing(), position: actual, duration };
        self.media.update(st.track.as_ref(), st.playing, st.position, st.duration);
        self.pushed_clock = (actual, st.playing, Instant::now());
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
            Act::View(v) => { self.view = v; self.selected.clear(); self.filter.clear(); self.show_video = false; } // переход в раздел сворачивает видео
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
                    let (name, source, tracks, note) = fetch_url(&url, &yt, &store, tx)?;
                    let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
                    let n = ids.len();
                    let mut s = store.lock().unwrap();
                    s.add_tracks(tracks);
                    let target = if let Some(pid) = pid { s.add_to_playlist(&pid, &ids); Some(pid) }
                        else { name.map(|name| s.create_playlist(&name, ids, Some(source)).id) };
                    let _ = tx.send(Bg::Status(format!("Добавлено {n} трек(ов){note}")));
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
                    let (_, _, tracks, note) = fetch_url(&src, &yt, &store, tx)?;
                    let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
                    let n = ids.len();
                    let mut s = store.lock().unwrap();
                    s.add_tracks(tracks);
                    s.set_playlist_tracks(&id, ids);
                    let _ = tx.send(Bg::Status(format!("Синхронизировано: {n} треков{note}")));
                    Ok(())
                });
            }
            Act::Export(ids, name) => {
                if self.exporting { self.toast("Экспорт уже идёт — дождись, пока закончится", true); return; }
                let tracks: Vec<Track> = ids.iter().filter_map(|i| self.track(i)).collect();
                if tracks.is_empty() { return; }
                let title = match &name { Some(n) => format!("Куда сохранить плейлист «{n}»? (создам в ней папку)"), None => "Куда сохранить MP3".into() };
                let Some(base) = rfd::FileDialog::new().set_title(title).pick_folder() else { return };
                let dir = match &name { Some(n) => base.join(export::safe_name(n)), None => base };
                self.exporting = true;
                self.toast(format!("Экспорт MP3: {} трек(ов)…", tracks.len()), false);
                let (ff, yt, cache) = (self.ff.clone(), self.yt.clone(), self.cache.clone());
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::Builder::new().name("export".into()).spawn(move || {
                    let r = export::run(&tracks, &dir, &ff, &yt, &cache, |s| { let _ = tx.send(Bg::Status(s)); c.request_repaint(); });
                    let _ = tx.send(Bg::Exported(r));
                    c.request_repaint();
                }).ok();
            }
            Act::Sort(key, how) => { self.store.lock().unwrap().patch_settings(&json!({ "sort": { key: how } })); }
            Act::PlaylistCover(id) => {
                let Some(src) = rfd::FileDialog::new().set_title("Картинка для обложки").add_filter("Картинки", &["jpg", "jpeg", "png", "webp"]).pick_file() else { return };
                let (store, covers, tx, c) = (self.store.clone(), self.covers.clone(), self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    match playlist_cover(&src, &covers, &id) {
                        Ok(path) => {
                            let mut s = store.lock().unwrap();
                            if let Some(p) = s.lib.playlists.iter_mut().find(|p| p.id == id) {
                                if let Some(old) = p.cover.replace(path.to_string_lossy().into_owned()) { let _ = std::fs::remove_file(old); }
                            }
                            s.save();
                        }
                        Err(e) => { let _ = tx.send(Bg::Status(format!("Не вышло поставить обложку: {e}"))); }
                    }
                    c.request_repaint();
                });
            }
            Act::PlaylistCoverReset(id) => {
                let mut s = self.store.lock().unwrap();
                if let Some(old) = s.lib.playlists.iter_mut().find(|p| p.id == id).and_then(|p| p.cover.take()) { let _ = std::fs::remove_file(old); }
                s.save();
            }
            Act::Like(ids, on) => {
                // лайк трека не из библиотеки (поиск, радио) — сначала добавить его в библиотеку
                let extra: Vec<Track> = ids.iter().filter(|i| !self.store.lock().unwrap().lib.tracks.contains_key(*i)).filter_map(|i| self.track(i)).collect();
                let mut s = self.store.lock().unwrap();
                if on && !extra.is_empty() { s.add_tracks(extra); }
                s.set_liked(&ids, on);
            }
            Act::EditTrack(id, title, artist, album) => self.store.lock().unwrap().edit_track(&id, &title, &artist, &album, None),
            Act::TrackCover(id) => {
                let Some(src) = rfd::FileDialog::new().set_title("Картинка для обложки").add_filter("Картинки", &["jpg", "jpeg", "png", "webp"]).pick_file() else { return };
                match playlist_cover(&src, &self.covers, &format!("t_{}", wavy_core::sha16(id.as_bytes()))) {
                    Ok(path) => {
                        let Some(t) = self.track(&id) else { return };
                        self.store.lock().unwrap().edit_track(&id, &t.title, &t.artist, &t.album, Some(Some(path.to_string_lossy().into_owned())));
                        if self.current.as_ref().is_some_and(|c| c.id == id) { self.current = self.track(&id); }
                    }
                    Err(e) => self.toast(format!("Не вышло поставить обложку: {e}"), true),
                }
            }
            Act::MergeDupes(groups) => { let n = self.store.lock().unwrap().merge_duplicates(&groups); self.toast(format!("Убрано дублей: {n}"), false); }
            Act::SetFolder(id, f) => self.store.lock().unwrap().set_folder(&id, f),
            Act::Radio(id) => {
                let Some(t) = self.track(&id) else { return };
                let Some(vid) = t.video_id.clone() else { self.toast("Радио работает для треков с YouTube", true); return };
                self.toast(format!("Собираю радио по «{}»…", t.title), false);
                let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
                std::thread::spawn(move || { let _ = tx.send(Bg::Radio(id, yt.radio(&vid, 30))); c.request_repaint(); });
            }
            Act::Move(ids, to) => {
                let mut s = self.store.lock().unwrap();
                match &self.view { View::Playlist(p) => s.move_in_playlist(p, &ids, to), View::Liked => s.move_liked(&ids, to), _ => {} }
            }
            Act::MoveQueue(from, to) => {
                if from < self.queue.len() && from != to {
                    // двигаем номера позиций, чтобы играющий трек остался тем же (в очереди бывают повторы)
                    let mut idx: Vec<usize> = (0..self.queue.len()).collect();
                    let x = idx.remove(from);
                    idx.insert(if to > from { to - 1 } else { to }.min(idx.len()), x);
                    let old = self.queue.clone();
                    self.queue = idx.iter().map(|&i| old[i].clone()).collect();
                    self.qi = idx.iter().position(|&i| i == self.qi).unwrap_or(self.qi);
                }
            }
            Act::WhatsNew(from, to) => {
                self.modal = Some(Modal::WhatsNew { from, to, items: None });
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || { let _ = tx.send(Bg::Changes(update::changes(from, to))); c.request_repaint(); });
            }
            Act::ToggleMini => { let on = !self.settings().mini; self.store.lock().unwrap().patch_settings(&json!({"mini": on})); }
            Act::ShowWindow => tray::show(ctx),
            Act::Quit => { self.quitting = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
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
            Act::SyncExport => {
                let text = wavy_core::sync::export(&self.store.lock().unwrap());
                if let Some(path) = rfd::FileDialog::new().set_title("Экспорт плейлистов для телефона").set_file_name("wavy-export.json").add_filter("Wavy", &["json"]).save_file() {
                    match std::fs::write(&path, text) {
                        Ok(()) => self.toast("Готово: открой этот файл в Wavy на телефоне (Настройки → Синхронизация → Импорт)", false),
                        Err(e) => self.toast(format!("Не удалось сохранить: {e}"), true),
                    }
                }
            }
            Act::SyncImport(path) => {
                let path = path.or_else(|| rfd::FileDialog::new().set_title("Импорт плейлистов с телефона").add_filter("Wavy", &["json"]).pick_file());
                if let Some(path) = path {
                    let res = std::fs::read_to_string(&path).map_err(|e| e.to_string())
                        .and_then(|t| wavy_core::sync::import(&mut self.store.lock().unwrap(), &t));
                    match res {
                        Ok(r) => self.toast(format!("Импортировано: {} треков, новых плейлистов {}, обновлено {}{}", r.tracks, r.new_playlists, r.updated_playlists,
                            if r.skipped_local > 0 { format!(" (файлы с телефона не переносятся: {})", r.skipped_local) } else { String::new() }), false),
                        Err(e) => self.toast(e, true),
                    }
                }
            }
            Act::ToggleVideo => {
                self.show_video = !self.show_video;
                if !self.show_video && self.fullscreen { self.apply(Act::Fullscreen(false), ctx); }
            }
            Act::Fullscreen(on) => {
                if on == self.fullscreen { return; }
                if on {
                    // запоминаем окно: после выхода из полного экрана система ставит размер по умолчанию
                    self.pre_fullscreen = ctx.input(|i| {
                        let v = i.viewport();
                        Some((v.outer_rect?.min, v.inner_rect?.size(), v.maximized.unwrap_or(false)))
                    });
                    self.show_video = true;
                } else if let Some((pos, size, max)) = self.pre_fullscreen.take() {
                    self.restore_window = Some((pos, size, max, 3)); // применяем несколько кадров подряд, пока окно не «устаканится»
                }
                self.fullscreen = on;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
            }
            Act::CheckUpdate => { self.upd_checked = Some(Instant::now()); self.start_update(ctx); }
            Act::Restart => {
                self.save_session();
                update::restart();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Act::ImportAnimeFiles | Act::ImportAnimeFolder => {
                let dlg = rfd::FileDialog::new();
                let picked = if matches!(a, Act::ImportAnimeFiles) { dlg.set_title("Выбери серии").add_filter("Видео", anime::EPISODE_EXT).pick_files() } else { dlg.set_title("Выбери папку с аниме").pick_folders() };
                if let Some(p) = picked { self.apply(Act::ImportAnime(p), ctx); }
            }
            Act::ImportAnime(paths) => {
                self.toast("Ищу серии…", false);
                let (store, covers) = (self.store.clone(), self.covers.clone());
                self.bg(ctx, move |tx| {
                    let groups = anime::scan(&paths, |i, n| { if n > 30 && i % 25 == 0 { let _ = tx.send(Bg::Status(format!("Серии: {i} / {n}"))); } });
                    if groups.is_empty() { return Err("Не нашлось видео (mkv, mp4, mov)".into()); }
                    let eps: usize = groups.iter().map(|g| g.tracks.len()).sum();
                    let mut ids = Vec::new();
                    for g in groups {
                        let n = g.tracks.len();
                        let (id, _) = store.lock().unwrap().add_episodes(&g.key, &g.parsed, g.tracks);
                        ids.push(id.clone());
                        let need = store.lock().unwrap().anime(&id).is_some_and(|a| !a.matched);
                        if !need { continue; }
                        let _ = tx.send(Bg::Status(format!("Ищу «{}» на Шикимори…", g.parsed.title)));
                        match wavy_core::shikimori::search(&anime::search_query(&g.parsed), 10) {
                            Ok(list) => {
                                let best = wavy_core::shikimori::best_match(&g.parsed.title, g.parsed.season, n, &list).cloned();
                                let mut a = store.lock().unwrap().anime(&id).cloned().unwrap_or_default();
                                match best {
                                    Some(b) => wavy_core::shikimori::apply(&mut a, &b, &covers),
                                    None => { a.matched = true; wavy_core::log(&format!("Шикимори: «{}» не нашлось", g.parsed.title)); }
                                }
                                let mut s = store.lock().unwrap();
                                if let Some(x) = s.anime_mut(&id) { *x = a; }
                                s.retitle_episodes(&id);
                                s.save();
                            }
                            Err(e) => wavy_core::log(&format!("Шикимори: {e}")),
                        }
                    }
                    let _ = tx.send(Bg::Status(format!("Добавлено серий: {eps}")));
                    let _ = tx.send(Bg::OpenAnime(if ids.len() == 1 { ids.pop() } else { None }));
                    Ok(())
                });
            }
            Act::PlayAnime(id, ep) => {
                let Some(a) = self.store.lock().unwrap().anime(&id).cloned() else { return };
                if a.episodes.is_empty() { return; }
                // продолжить: последняя серия (если досмотрена — следующая), с того же места
                let start = match &ep {
                    Some(e) => a.episodes.iter().position(|x| x == e).unwrap_or(0),
                    None => match a.last.as_ref().and_then(|l| a.episodes.iter().position(|x| x == l)) {
                        Some(i) if a.watched.contains(&a.episodes[i]) && i + 1 < a.episodes.len() => i + 1,
                        Some(i) => i,
                        None => a.episodes.iter().position(|e| !a.watched.contains(e)).unwrap_or(0),
                    },
                };
                let resume_pos = if ep.is_none() && a.last.as_deref() == Some(a.episodes[start].as_str()) && a.last_pos > 5.0 { a.last_pos } else { 0.0 };
                self.queue = a.episodes.clone();
                self.qi = start;
                self.resume = (resume_pos > 0.0).then(|| (a.episodes[start].clone(), resume_pos, true));
                self.video = None;
                self.show_video = true;
                self.load_current(ctx);
            }
            Act::AnimeSearch(q) => {
                if let Some(Modal::AnimeMatch { busy, err, .. }) = &mut self.modal { *busy = true; *err = None; }
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || { let _ = tx.send(Bg::ShikiResults(wavy_core::shikimori::search(&q, 12))); c.request_repaint(); });
            }
            Act::AnimeApply(id, sh) => {
                let (store, covers) = (self.store.clone(), self.covers.clone());
                let name = sh.display().to_string();
                self.bg(ctx, move |tx| {
                    let mut a = store.lock().unwrap().anime(&id).cloned().ok_or("Тайтл не найден")?;
                    wavy_core::shikimori::apply(&mut a, &sh, &covers);
                    let mut s = store.lock().unwrap();
                    if let Some(x) = s.anime_mut(&id) { *x = a; }
                    s.retitle_episodes(&id);
                    s.save();
                    let _ = tx.send(Bg::Status(format!("Теперь это «{name}»")));
                    Ok(())
                });
            }
            Act::DeleteAnime(id) => {
                let eps = self.store.lock().unwrap().anime(&id).map(|a| a.episodes.clone()).unwrap_or_default();
                if self.current.as_ref().is_some_and(|t| eps.contains(&t.id)) { self.audio.stop(); self.current = None; self.queue.clear(); self.video = None; self.show_video = false; self.push_state(); }
                self.store.lock().unwrap().delete_anime(&id);
                self.view = View::Anime;
            }
            Act::MarkWatched(tid, w) => self.store.lock().unwrap().mark_episode(&tid, w),
            Act::AudioTrack(i) => {
                let Some(t) = self.current.clone() else { return };
                self.audio_sel = i;
                if let Err(e) = self.audio.switch_track(Path::new(t.path.as_deref().unwrap_or_default()), i) { self.track_failed(e); }
                if let Some(aid) = &t.anime { if let Some(a) = self.store.lock().unwrap().anime_mut(aid) { a.audio = Some(i); } self.store.lock().unwrap().save(); }
                self.push_state();
            }
            Act::Subs(sel) => {
                self.sub_sel = sel;
                self.video = None; // откроется заново с нужными субтитрами на той же позиции
                let key = match (sel, &self.media_info) { (Some(i), Some((_, info))) => info.subs.get(i).map(|s| s.key()), _ => Some("off".to_string()) };
                if let Some(aid) = self.current.as_ref().and_then(|t| t.anime.clone()) {
                    let mut s = self.store.lock().unwrap();
                    if let Some(a) = s.anime_mut(&aid) { a.subs = key; }
                    s.save();
                }
            }
            Act::Shazam => {
                let Some(t) = self.current.clone() else { return };
                let path = match t.path.clone() {
                    Some(p) => PathBuf::from(p),
                    None => self.cache.join(format!("{}.s.m4a", wavy_core::sha16(t.url.clone().unwrap_or_default().as_bytes()))),
                };
                if !path.exists() { self.toast("Shazam работает с файлами на компьютере (и уже загруженными треками)", true); return; }
                let (pos, track, ff) = (self.audio.position(), self.audio_sel, self.ff.path());
                self.modal = Some(Modal::Shazam { state: ShazamState::Listening, to_openings: t.anime.is_some() });
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    // берём чуть до текущего места и вперёд — нажимают обычно, когда опенинг уже идёт
                    let r = player::sample_for_shazam(&path, (pos - 3.0).max(0.0), wavy_core::shazam::SECONDS as f64, track, ff.as_deref()).and_then(|s| wavy_core::shazam::recognize(&s));
                    if let Err(e) = &r { wavy_core::log(&format!("Shazam: {e}")); }
                    let _ = tx.send(Bg::Shazam(r));
                    c.request_repaint();
                });
            }
            Act::ShazamAdd(f, to_pl) => {
                if let Some(Modal::Shazam { state, .. }) = &mut self.modal { *state = ShazamState::Adding; }
                let (store, yt) = (self.store.clone(), self.yt.clone());
                self.bg(ctx, move |tx| {
                    let q = format!("{} {}", f.artist, f.title);
                    let list = yt.search(&q, 8)?;
                    let low = |s: &str| s.to_lowercase();
                    let best = list.iter().find(|t| low(&t.title).contains(&low(&f.title)) && (60.0..900.0).contains(&t.duration))
                        .or_else(|| list.iter().find(|t| (60.0..900.0).contains(&t.duration))).or(list.first()).cloned()
                        .ok_or_else(|| format!("На YouTube не нашлось «{q}»"))?;
                    let mut t = best;
                    t.title = f.title.clone();
                    if !f.artist.is_empty() { t.artist = f.artist.clone(); }
                    let id = t.id.clone();
                    let mut s = store.lock().unwrap();
                    s.add_tracks(vec![t]);
                    let mut note = String::new();
                    if to_pl {
                        let pid = match s.lib.playlists.iter().find(|p| p.name == OPENINGS) { Some(p) => p.id.clone(), None => s.create_playlist(OPENINGS, vec![], None).id };
                        s.add_to_playlist(&pid, &[id]);
                        note = format!(" и в «{OPENINGS}»");
                    }
                    let _ = tx.send(Bg::Status(format!("{} — {}: добавлено в библиотеку{note}", f.artist, f.title)));
                    Ok(())
                });
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
        let st = self.settings();
        self.audio.fx.set(fx::Settings { speed: st.speed, eq_on: st.eq_on, eq: st.eq, normalize: st.normalize });
        if self.audio.tick() { ctx.request_repaint_after(Duration::from_millis(40)); }
        // таймер сна
        if let Some(Sleep::At(t)) = self.sleep {
            if Instant::now() >= t { self.sleep = None; if self.audio.is_playing() { self.audio.pause(); self.push_state(); } self.toast("Таймер сна: спокойной ночи", false); }
            else { ctx.request_repaint_after(Duration::from_secs(1)); }
        }
        // кроссфейд: за N секунд до конца запускаем следующий трек, этот плавно затихает
        if st.crossfade > 0.0 && !self.loading && self.audio.is_playing() && self.sleep != Some(Sleep::EndOfTrack) && st.repeat != "one" {
            if let Some(t) = self.current.clone() {
                let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
                let has_next = self.qi + 1 < self.queue.len() || st.repeat == "all";
                if has_next && dur > st.crossfade as f64 * 3.0 && self.audio.position() >= dur - st.crossfade as f64 && self.xfaded.as_deref() != Some(t.id.as_str()) {
                    self.xfaded = Some(t.id.clone());
                    self.audio.begin_crossfade(st.crossfade);
                    self.next(true, ctx);
                }
            }
        }
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Bg::Status(s) => self.toast(s, false),
                Bg::Lyrics(id, r) => if let Some((cur, slot)) = &mut self.lyrics { if *cur == id { *slot = Some(r); self.lyrics_line = None; } },
                Bg::Radio(id, r) => match r {
                    Ok(list) if !list.is_empty() => {
                        let ids: Vec<String> = list.iter().map(|t| t.id.clone()).collect();
                        self.radio.retain(|t| !ids.contains(&t.id));
                        self.radio.extend(list);
                        let n = ids.len();
                        if self.current.as_ref().is_some_and(|c| c.id == id) {
                            // играет этот трек — радио встаёт следующим в очереди вместо остального
                            self.queue.truncate(self.qi + 1);
                            self.queue.extend(ids);
                        } else {
                            let mut q = vec![id];
                            q.extend(ids);
                            self.play_list(q, 0, false, ctx);
                        }
                        self.toast(format!("Радио: {n} похожих треков в очереди"), false);
                    }
                    Ok(_) => self.toast("Похожих треков не нашлось", true),
                    Err(e) => self.toast(format!("Радио не собралось: {e}"), true),
                },
                Bg::Skips(id, v) => if let Some((cur, slot)) = &mut self.skips { if *cur == id { *slot = v; } },
                Bg::Shiki(r, login) => match r {
                    Ok(acc) => {
                        if login { self.toast(format!("Шикимори: вошёл как {}", acc.nickname), false); self.shiki_code.clear(); }
                        self.store.lock().unwrap().patch_settings(&json!({ "shiki": acc }));
                    }
                    Err(e) => self.toast(e, true),
                },
                Bg::Changes(r) => if let Some(Modal::WhatsNew { items, .. }) = &mut self.modal { *items = Some(r); },
                Bg::Exported(r) => {
                    self.exporting = false;
                    match r {
                        Ok(rep) => {
                            let msg = if rep.failed.is_empty() { format!("Готово: {} MP3 в папке «{}»", rep.done, rep.dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()) }
                                else { format!("Сохранено {} MP3, не вышло {}: {} — {}", rep.done, rep.failed.len(), rep.failed[0].0, rep.failed[0].1) };
                            self.toast(msg, !rep.failed.is_empty());
                            if rep.done > 0 { open_folder(&rep.dir); }
                        }
                        Err(e) => self.toast(format!("Экспорт не вышел: {e}"), true),
                    }
                }
                Bg::OpenPlaylist(id) => {
                    self.view = View::Playlist(id);
                    if matches!(self.modal, Some(Modal::ImportUrl { .. })) { self.modal = None; }
                }
                Bg::Search(r) => {
                    self.searching = false;
                    match r { Ok(v) => self.results = v, Err(e) => self.toast(e, true) }
                }
                Bg::Replaced(t) => {
                    // запоминаем новое видео в библиотеке (id тот же — плейлисты не трогаем)
                    let mut s = self.store.lock().unwrap();
                    if s.lib.tracks.contains_key(&t.id) { s.add_tracks(vec![t.clone()]); }
                    drop(s);
                    if self.current.as_ref().is_some_and(|c| c.id == t.id) { self.current = Some(t.clone()); }
                    self.video_error = None;
                    self.toast(format!("«{}»: старое видео пропало с YouTube, нашёл замену", t.title), false);
                }
                Bg::Ready { token, path } => if token == self.token { self.start_file(&path); },
                Bg::ReadyStream { token, dl } => if token == self.token { self.start_stream(dl); } else if !dl.done() { dl.cancel(); },
                Bg::VideoDownloading { id, dl } => {
                    if self.video_loading.as_deref() == Some(id.as_str()) { self.video_dl = Some((id, dl)); }
                }
                Bg::LoadFailed { token, err } => if token == self.token { self.track_failed(err); },
                Bg::Discord(s) => self.discord_status = s,
                Bg::YtVersion(v) => self.yt_version = Some(v),
                Bg::VideoReady { id, path } => {
                    if self.video_loading.as_deref() == Some(id.as_str()) { self.video_loading = None; }
                    let cur = self.current.as_ref().map(|t| t.id.clone());
                    if cur.as_deref() == Some(id.as_str()) && (self.show_video || self.fullscreen) {
                        let pos = self.audio.position();
                        self.video = Some((id, video::Video::open(path, pos, ctx, None, self.ff.path())));
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
                        match &mut self.modal {
                            Some(Modal::ImportUrl { busy, err, .. }) => { *busy = false; *err = Some(e); }
                            Some(Modal::Shazam { state, .. }) => *state = ShazamState::Error(e),
                            _ => self.toast(e, true),
                        }
                    } else if matches!(self.modal, Some(Modal::ImportUrl { busy: true, .. }) | Some(Modal::Shazam { state: ShazamState::Adding, .. })) { self.modal = None; }
                }
                Bg::OpenAnime(id) => self.view = match id { Some(id) => View::AnimeTitle(id), None => View::Anime },
                Bg::ShikiResults(r) => {
                    if let Some(Modal::AnimeMatch { results, busy, err, .. }) = &mut self.modal {
                        *busy = false;
                        match r { Ok(v) => { if v.is_empty() { *err = Some("Ничего не нашлось — попробуй другое написание (можно по-русски или по-английски)".into()); } *results = v; } Err(e) => *err = Some(e) }
                    }
                }
                Bg::Shazam(r) => {
                    if let Some(Modal::Shazam { state, .. }) = &mut self.modal {
                        *state = match r { Ok(Some(f)) => ShazamState::Found(f), Ok(None) => ShazamState::NotFound, Err(e) => ShazamState::Error(e) };
                    }
                }
            }
        }
        if std::mem::take(&mut self.pending_next) { self.next(true, ctx); }
        if self.audio.finished() && !self.loading {
            if self.recover_cut(ctx) {}
            else if self.sleep == Some(Sleep::EndOfTrack) {
                self.sleep = None;
                self.audio.stop();
                self.push_state();
                self.toast("Таймер сна: трек доиграл, спокойной ночи", false);
            } else { self.next(true, ctx); }
        }
        let state = (self.current.as_ref().map(|t| t.id.clone()), self.audio.is_playing());
        if state != self.last_pushed && !self.loading { self.push_state(); }
        self.sync_clock();
        if let Some(t) = self.current.as_ref().filter(|t| t.anime.is_none() && self.counted.as_deref() != Some(t.id.as_str())) {
            if self.audio.is_playing() && self.audio.position() > 30.0 { let id = t.id.clone(); self.store.lock().unwrap().count_play(&id); self.counted = Some(id); }
        }

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
        let mut dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        // wavy-export.json с телефона — это синхронизация, а не музыка
        let exports: Vec<PathBuf> = dropped.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))).cloned().collect();
        dropped.retain(|p| !exports.contains(p));
        for e in exports { self.apply(Act::SyncImport(Some(e)), ctx); }
        if !dropped.is_empty() {
            if matches!(self.view, View::Anime | View::AnimeTitle(_)) { self.apply(Act::ImportAnime(dropped), ctx); }
            else { let pid = self.cur_playlist(); self.apply(Act::ImportPaths(dropped, pid), ctx); }
        }
        // серия досмотрена (последние 10 % или титры) — отмечаем
        if let Some(t) = self.current.as_ref().filter(|t| t.anime.is_some()) {
            let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
            let pos = self.audio.position();
            if dur > 60.0 && self.audio.loaded && (pos > dur * 0.9 || (dur > 600.0 && dur - pos < 100.0)) && self.watched_mark.as_deref() != Some(t.id.as_str()) {
                let id = t.id.clone();
                self.store.lock().unwrap().mark_episode(&id, true);
                self.watched_mark = Some(id);
                self.shiki_sync(t.anime.clone().unwrap_or_default(), ctx);
            }
        }
        self.anime_tick(ctx);
        if let Some((_, _, at)) = &self.toast { if at.elapsed() > Duration::from_secs(4) { self.toast = None; } }
        self.audio.poll();
        if self.audio.buffering() { ctx.request_repaint_after(Duration::from_millis(200)); }
        // загрузка видео: показываем проценты, по готовности — открываем
        if let Some((id, dl)) = self.video_dl.clone() {
            if dl.done() {
                self.video_dl = None;
                let _ = self.tx.send(Bg::VideoReady { id, path: dl.path.clone() });
                ctx.request_repaint();
            } else if let Some(e) = dl.error() {
                self.video_dl = None;
                let (yt, tx, cache, c) = (self.yt.clone(), self.tx.clone(), self.cache.clone(), ctx.clone());
                if let Some(t) = self.track(&id) {
                    wavy_core::log(&format!("видео: быстрая загрузка не вышла ({e}), пробую yt-dlp"));
                    std::thread::spawn(move || {
                        let msg = match yt.download_video(&t, &cache) { Ok(path) => Bg::VideoReady { id: t.id.clone(), path }, Err(err) => Bg::VideoFailed { id: t.id.clone(), err } };
                        let _ = tx.send(msg); c.request_repaint();
                    });
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
        self.ffmpeg_tick(ctx);
        self.video_tick(ctx);
        self.cjk_tick(ctx);
        if let Some((pos, size, max, left)) = self.restore_window.take() {
            if max { ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true)); }
            else {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            }
            if left > 1 { self.restore_window = Some((pos, size, max, left - 1)); ctx.request_repaint_after(Duration::from_millis(60)); }
        }
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

    /// Японские/китайские/корейские названия: раз в пару секунд смотрим, не появились ли такие
    /// (библиотека, плейлисты, поиск, аниме), и тогда один раз подгружаем системный шрифт.
    fn cjk_tick(&mut self, ctx: &egui::Context) {
        if self.cjk_loaded || self.cjk_check.elapsed() < Duration::from_secs(2) { return; }
        self.cjk_check = Instant::now();
        let found = {
            let s = self.store.lock().unwrap();
            s.lib.tracks.values().any(|t| has_cjk(&t.title) || has_cjk(&t.artist) || has_cjk(&t.album))
                || s.lib.playlists.iter().any(|p| has_cjk(&p.name))
                || s.lib.anime.iter().any(|a| has_cjk(a.display()))
        } || self.results.iter().any(|t| has_cjk(&t.title) || has_cjk(&t.artist))
            || has_cjk(&self.filter) || has_cjk(&self.search_q)
            || self.toast.as_ref().is_some_and(|t| has_cjk(&t.0))
            || matches!(&self.modal, Some(Modal::Shazam { state: ShazamState::Found(f), .. }) if has_cjk(&f.title) || has_cjk(&f.artist))
            || matches!(&self.modal, Some(Modal::AnimeMatch { results, .. }) if results.iter().any(|r| has_cjk(r.display())));
        if !found { return; }
        self.cjk_loaded = true;
        let fonts = load_cjk_fonts();
        if fonts.is_empty() { wavy_core::log("шрифт для иероглифов не найден"); return; }
        ctx.set_fonts(font_defs(&fonts));
    }

    /// Декодер ffmpeg: качаем, если он понадобился, и повторяем то, что без него не открылось.
    fn ffmpeg_tick(&mut self, ctx: &egui::Context) {
        use wavy_core::ffmpeg::{State, NEED};
        let need_video = self.video_error.as_ref().is_some_and(|e| e.1.starts_with(NEED));
        if !(need_video || self.reload_after_ff) { return; }
        match self.ff.state() {
            State::Missing => self.need_ffmpeg(ctx),
            State::Downloading(_) => ctx.request_repaint_after(Duration::from_millis(300)),
            State::Ready(p) => {
                self.audio.ffmpeg = Some(p);
                if need_video { self.video_error = None; self.video = None; } // video_tick откроет заново
                if std::mem::take(&mut self.reload_after_ff) {
                    let pos = self.audio.position();
                    if let (Some(t), None) = (self.current.clone(), &self.resume) { self.resume = Some((t.id, pos, true)); }
                    self.load_current(ctx);
                }
            }
            State::Failed(e) => {
                if need_video { if let Some(v) = &mut self.video_error { v.1 = format!("{} — не скачался декодер: {e}", wavy_core::ffmpeg::human(&v.1)); } }
                if std::mem::take(&mut self.reload_after_ff) { self.toast(format!("Не скачался декодер: {e}"), true); }
            }
        }
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
                let pos = self.resume.as_ref().map(|r| r.1).unwrap_or_else(|| self.audio.position());
                let sub = self.sub_src();
                self.video = Some((t.id.clone(), video::Video::open(path, pos, ctx, sub, self.ff.path())));
            } else if self.video_loading.as_deref() != Some(t.id.as_str()) {
                self.video_loading = Some(t.id.clone());
                let (yt, tx, cache, c) = (self.yt.clone(), self.tx.clone(), self.cache.clone(), ctx.clone());
                std::thread::spawn(move || {
                    // быстрый путь: прямая ссылка + загрузка кусками с процентами
                    if let Some(vid) = t.video_id.clone() {
                        let path = cache.join(format!("{}.v.mp4", wavy_core::sha16(t.url.clone().unwrap_or_default().as_bytes())));
                        if wavy_core::stream::is_complete(&path) { let _ = tx.send(Bg::VideoReady { id: t.id.clone(), path }); c.request_repaint(); return; }
                        if let Ok(d) = yt.direct(&vid, true) {
                            let dl = wavy_core::stream::Download::start(d.url, path, d.size);
                            let _ = tx.send(Bg::VideoDownloading { id: t.id.clone(), dl });
                            c.request_repaint();
                            return;
                        }
                    }
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
        if let Some((_, rx)) = &self.tray {
            while let Ok(c) = rx.try_recv() {
                acts.push(match c { tray::Cmd::Show => continue, tray::Cmd::Toggle => Act::TogglePlay, tray::Cmd::Next => Act::Next, tray::Cmd::Prev => Act::Prev, tray::Cmd::Quit => Act::Quit });
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.tray.is_some() && self.settings().close_to_tray {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide(&ctx);
        }
        if self.settings().mini { self.mini_window(&ctx, &mut acts); }
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
        if self.show_lyrics {
            egui::Panel::right("lyrics").exact_size(340.0).resizable(false).frame(egui::Frame::new().fill(BG2).inner_margin(14))
                .show(ui, |ui| self.lyrics_ui(ui, &mut acts));
        } else if self.show_queue {
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
            let vol = self.settings().volume;
            if i.key_pressed(egui::Key::ArrowUp) { acts.push(Act::Volume((vol + 0.05).min(1.0))); acts.push(Act::SaveVolume); }
            if i.key_pressed(egui::Key::ArrowDown) { acts.push(Act::Volume((vol - 0.05).max(0.0))); acts.push(Act::SaveVolume); }
            if i.key_pressed(egui::Key::M) {
                if vol > 0.0 { self.muted_vol = vol; acts.push(Act::Volume(0.0)); } else { acts.push(Act::Volume(if self.muted_vol > 0.0 { self.muted_vol } else { 0.8 })); }
                acts.push(Act::SaveVolume);
            }
            if i.key_pressed(egui::Key::L) {
                if let Some(t) = self.current.as_ref().filter(|t| t.anime.is_none()) { let on = self.store.lock().unwrap().is_liked(&t.id); acts.push(Act::Like(vec![t.id.clone()], !on)); }
            }
            if i.key_pressed(egui::Key::Q) { self.show_queue = !self.show_queue; if self.show_queue { self.show_lyrics = false; } }
            if i.key_pressed(egui::Key::T) { self.show_lyrics = !self.show_lyrics; if self.show_lyrics { self.show_queue = false; } }
            if i.key_pressed(egui::Key::S) && !i.modifiers.ctrl { acts.push(Act::Shuffle); }
            if i.key_pressed(egui::Key::R) && !i.modifiers.ctrl { acts.push(Act::Repeat); }
            if i.modifiers.command && i.key_pressed(egui::Key::F) { self.focus_filter = true; }
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
        let n_liked = { let s = self.store.lock().unwrap(); s.lib.liked.iter().filter(|t| s.lib.tracks.contains_key(*t)).count() };
        if nav(ui, "\u{E2A8}  Любимое", self.view == View::Liked, (n_liked > 0).then_some(n_liked)).clicked() { acts.push(Act::View(View::Liked)); }
        if nav(ui, "\u{E150}  Итоги", self.view == View::Stats, None).clicked() { acts.push(Act::View(View::Stats)); }
        if nav(ui, "\u{E30C}  Поиск на YouTube", self.view == View::Search, None).clicked() { acts.push(Act::View(View::Search)); }
        let n_anime = self.store.lock().unwrap().lib.anime.len();
        if nav(ui, "\u{E754}  Аниме", matches!(self.view, View::Anime | View::AnimeTitle(_)), (n_anime > 0).then_some(n_anime)).clicked() { acts.push(Act::View(View::Anime)); }
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
            let mut folders: Vec<String> = pls.iter().filter_map(|p| p.folder.clone()).collect();
            folders.sort_by_key(|f| f.to_lowercase());
            folders.dedup();
            let item = |ui: &mut egui::Ui, p: &wavy_core::store::Playlist, acts: &mut Vec<Act>, view: &View| {
                let icon = if p.is_remote() { "\u{E4FC}" } else { "\u{E6AA}" };
                let r = nav(ui, &format!("{icon}  {}", p.name), *view == View::Playlist(p.id.clone()), Some(p.track_ids.len()));
                if r.clicked() { acts.push(Act::View(View::Playlist(p.id.clone()))); }
                r.context_menu(|ui| {
                    if ui.button("В папку…").clicked() { acts.push(Act::Modal(Modal::Folder { id: p.id.clone(), name: p.folder.clone().unwrap_or_default() })); ui.close(); }
                    if p.folder.is_some() && ui.button("Убрать из папки").clicked() { acts.push(Act::SetFolder(p.id.clone(), None)); ui.close(); }
                    if ui.button("Переименовать…").clicked() { acts.push(Act::Modal(Modal::Rename { id: p.id.clone(), name: p.name.clone() })); ui.close(); }
                    if ui.button(RichText::new("Удалить…").color(BAD)).clicked() { acts.push(Act::Modal(Modal::Delete { id: p.id.clone() })); ui.close(); }
                });
            };
            for f in &folders {
                egui::CollapsingHeader::new(RichText::new(format!("\u{E24A}  {f}")).color(MUTED)).id_salt(("folder", f)).default_open(true).show(ui, |ui| {
                    for p in pls.iter().filter(|p| p.folder.as_ref() == Some(f)) { item(ui, p, acts, &self.view); }
                });
            }
            for p in pls.iter().filter(|p| p.folder.is_none()) { item(ui, p, acts, &self.view); }
        });
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            let d = if self.discord_status.connected { "\u{E61A}" } else { "" };
            if nav(ui, &format!("\u{E272}  Настройки  {d}"), false, None).clicked() {
                let s = self.settings();
                acts.push(Act::Modal(Modal::Settings { client_id: s.discord.client_id, port: s.overlay.port.to_string(), sp_id: s.spotify.client_id, sp_secret: s.spotify.client_secret }));
            }
            let s = self.settings();
            let st = if !self.overlay_open { "выкл" } else if s.overlay_window.locked { "вкл" } else { "двигай" };
            if nav(ui, &format!("\u{E466}  Оверлей · {st}"), self.overlay_open, None).clicked() {
                acts.push(Act::Modal(Modal::Settings { client_id: s.discord.client_id.clone(), port: s.overlay.port.to_string(), sp_id: s.spotify.client_id.clone(), sp_secret: s.spotify.client_secret.clone() }));
            }
            ui.separator();
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, ids: &[String], acts: &mut Vec<Act>) {
        let (eyebrow, title, pl) = {
            let s = self.store.lock().unwrap();
            match &self.view {
                View::Library => ("КОЛЛЕКЦИЯ", "Библиотека".to_string(), None),
                View::Liked => ("КОЛЛЕКЦИЯ", "Любимое".to_string(), None),
                View::Stats => ("ТВОЯ МУЗЫКА", "Итоги".to_string(), None),
                View::Search => ("YOUTUBE", "Поиск".to_string(), None),
                View::Anime | View::AnimeTitle(_) => ("АНИМЕ", "Аниме".to_string(), None),
                View::Playlist(id) => {
                    let p = s.lib.playlists.iter().find(|p| &p.id == id).cloned();
                    (if p.as_ref().is_some_and(|p| p.is_remote()) { "ПЛЕЙЛИСТ · ИМПОРТ" } else { "ПЛЕЙЛИСТ" }, p.as_ref().map(|p| p.name.clone()).unwrap_or_default(), p)
                }
            }
        };
        let total: f64 = ids.iter().filter_map(|i| self.track(i)).map(|t| t.duration).sum();
        let own = pl.as_ref().and_then(|p| p.cover.clone());
        let cover = own.clone().or_else(|| ids.iter().filter_map(|i| self.track(i)).find_map(|t| t.cover.clone().map(|c| if t.video_id.is_some() { format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", t.video_id.unwrap()) } else { c })));
        ui.horizontal(|ui| {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(120.0), if pl.is_some() { Sense::click() } else { Sense::hover() });
            if let Some(p) = &pl {
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Поставить свою картинку");
                if resp.clicked() { acts.push(Act::PlaylistCover(p.id.clone())); }
                resp.context_menu(|ui| {
                    if ui.button("Поставить свою картинку…").clicked() { acts.push(Act::PlaylistCover(p.id.clone())); ui.close(); }
                    if own.is_some() && ui.button("Вернуть обложку из треков").clicked() { acts.push(Act::PlaylistCoverReset(p.id.clone())); ui.close(); }
                });
            }
            let hovered = pl.is_some() && ui.rect_contains_pointer(r);
            match cover.filter(|_| pl.is_some()).and_then(|c| self.thumbs.get(&c, 240)) {
                Some(t) => { ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE); }
                None => {
                    ui.painter().rect_filled(r, 14, BG3);
                    ui.painter().rect_stroke(r, 14, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, match self.view { View::Search => "\u{E30C}", View::Liked => "\u{E2A8}", _ => "\u{E340}" }, FontId::new(44.0, if self.view == View::Liked { egui::FontFamily::Name("fill".into()) } else { egui::FontFamily::Proportional }), ACCENT);
                }
            }
            if hovered {
                ui.painter().rect_filled(r, 0, Color32::from_black_alpha(150));
                glyph(ui.painter(), r.center() - Vec2::new(0.0, 10.0), "\u{E3B4}", FontId::proportional(26.0), TEXT);
                ui.painter().text(r.center() + Vec2::new(0.0, 18.0), Align2::CENTER_CENTER, "Своя обложка", FontId::proportional(12.0), TEXT);
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
                    if self.view == View::Library && !ids.is_empty() && pill(ui, "\u{E1CA}  Дубли", false).on_hover_text("Найти одинаковые треки").clicked() {
                        let groups = self.store.lock().unwrap().duplicates();
                        if groups.is_empty() { self.toast("Дублей нет — всё чисто", false); } else { acts.push(Act::Modal(Modal::Dupes { groups })); }
                    }
                    if let Some(p) = &pl {
                        if p.is_remote() && pill(ui, "\u{E094}  Синхронизировать", false).clicked() { acts.push(Act::Sync(p.id.clone())); }
                        if !ids.is_empty() && pill(ui, "\u{E20C}  Скачать MP3", false).on_hover_text("Сохранить весь плейлист в MP3 в одну папку").clicked() { acts.push(Act::Export(ids.to_vec(), Some(p.name.clone()))); }
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
        match self.view.clone() {
            View::Anime => return self.anime_grid(ui, acts),
            View::Stats => return self.stats_page(ui, acts),
            View::AnimeTitle(id) => return self.anime_page(ui, &id, acts),
            _ => {}
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
            let fw = (ui.available_width() - 420.0).clamp(160.0, 360.0);
            let fr = ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("\u{E30C} Фильтр (Ctrl+F)…").desired_width(fw).margin(egui::Margin::symmetric(12, 8)));
            if std::mem::take(&mut self.focus_filter) { fr.request_focus(); }
            if fr.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) { self.filter.clear(); fr.surrender_focus(); }
            // сортировка (своя для библиотеки и для каждого плейлиста)
            let (key, is_pl) = match &self.view { View::Playlist(id) => (id.clone(), true), View::Liked => ("liked".to_string(), true), _ => ("library".to_string(), false) };
            let cur = self.settings().sort.get(&key).cloned().unwrap_or_default();
            let opts: &[(&str, &str)] = if is_pl {
                &[("", if key == "liked" { "Как лайкал" } else { "Как в плейлисте" }), ("plays", "Часто слушаю"), ("newest", "Сначала новые"), ("title", "По названию"), ("artist", "По исполнителю"), ("album", "По альбому")]
            } else {
                &[("", "Сначала новые"), ("plays", "Часто слушаю"), ("oldest", "Сначала старые"), ("title", "По названию"), ("artist", "По исполнителю"), ("album", "По альбому")]
            };
            let label = opts.iter().find(|o| o.0 == cur).map(|o| o.1).unwrap_or(opts[0].1);
            ui.add_space(6.0);
            let text = format!("\u{E444}  {label}");
            let r = ui.add(pill_btn(&text, false)).on_hover_text("Сортировка");
            egui::Popup::menu(&r).show(|ui| {
                ui.set_min_width(180.0);
                for (k, name) in opts {
                    let on = *k == cur.as_str() || (k.is_empty() && !opts.iter().any(|o| o.0 == cur));
                    if ui.selectable_label(on, *name).clicked() { acts.push(Act::Sort(key.clone(), k.to_string())); ui.close(); }
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // узкое окно (открыт текст/очередь) — только значки
                let narrow = ui.available_width() < 330.0;
                let t = |icon: &str, text: &str| if narrow { icon.to_string() } else { format!("{icon}  {text}") };
                if pill(ui, &t("\u{E2E2}", "По ссылке"), false).on_hover_text("Добавить по ссылке").clicked() { acts.push(Act::Modal(Modal::ImportUrl { url: String::new(), into: self.cur_playlist().is_some(), busy: false, err: None })); }
                if pill(ui, &t("\u{E25A}", "Папка"), false).on_hover_text("Добавить папку").clicked() { acts.push(Act::ImportFolder); }
                if pill(ui, &t("\u{EA20}", "Файлы"), false).on_hover_text("Добавить файлы").clicked() { acts.push(Act::ImportFiles); }
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
        let liked: HashSet<String> = self.store.lock().unwrap().lib.liked.iter().cloned().collect();
        // перетаскивать можно в плейлисте и «Любимом», если порядок свой (без сортировки и фильтра)
        let key = match &self.view { View::Playlist(id) => Some(id.clone()), View::Liked => Some("liked".to_string()), _ => None };
        let can_drag = key.as_ref().is_some_and(|k| self.settings().sort.get(k).is_none_or(|v| v.is_empty())) && self.filter.trim().is_empty();
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let mut line: Option<(f32, f32, f32)> = None;
        let hover_prev = self.hover_row.take();
        TableBuilder::new(ui)
            .striped(false)
            .sense(if can_drag { Sense::click_and_drag() } else { Sense::click() })
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(36.0))
            .column(Column::remainder().at_least(200.0).clip(true))
            .column(Column::exact(220.0).clip(true))
            .column(Column::exact(34.0))
            .column(Column::exact(56.0))
            .max_scroll_height(f32::INFINITY)
            .header(26.0, |mut h| {
                for t in ["#", "НАЗВАНИЕ", "АЛЬБОМ", "", "\u{E19A}"] { h.col(|ui| { ui.label(RichText::new(t).size(11.0).color(MUTED)); }); }
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
                    row.col(|ui| {
                        let on = liked.contains(id);
                        if on || hover_prev.as_deref() == Some(id.as_str()) {
                            let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                            let fam = if on { egui::FontFamily::Name("fill".into()) } else { egui::FontFamily::Proportional };
                            glyph(ui.painter(), r.center(), "\u{E2A8}", FontId::new(16.0, fam), if on { ACCENT } else if resp.hovered() { TEXT } else { MUTED });
                            if resp.on_hover_text(if on { "Убрать из «Любимого»" } else { "В «Любимое»" }).clicked() { acts.push(Act::Like(vec![id.clone()], !on)); }
                        }
                    });
                    row.col(|ui| { ui.label(RichText::new(fmt_time(t.duration)).color(MUTED)); });
                    let r = row.response();
                    if r.hovered() || r.contains_pointer() { self.hover_row = Some(id.clone()); }
                    if can_drag {
                        if r.drag_started() {
                            let sel: Vec<String> = if self.selected.contains(id) { ids.iter().filter(|x| self.selected.contains(*x)).cloned().collect() } else { vec![id.clone()] };
                            self.drag_ids = Some(sel);
                        }
                        if let (Some(_), Some(p)) = (&self.drag_ids, pointer) {
                            if r.rect.y_range().contains(p.y) {
                                let below = p.y > r.rect.center().y;
                                let to = if below { i + 1 } else { i };
                                self.drop_at = Some(to);
                                line = Some((if below { r.rect.bottom() } else { r.rect.top() }, r.rect.left(), r.rect.right()));
                            }
                        }
                    }
                    if r.double_clicked() { acts.push(Act::Play(ids.to_vec(), i, false)); }
                    else if r.clicked() {
                        if mods.shift { if let Some(a) = self.last_clicked.as_ref().and_then(|l| ids.iter().position(|x| x == l)) { let (x, y) = (a.min(i), a.max(i)); self.selected.extend(ids[x..=y].iter().cloned()); } }
                        else if mods.command { if !self.selected.remove(id) { self.selected.insert(id.clone()); } self.last_clicked = Some(id.clone()); }
                        else { self.selected = HashSet::from([id.clone()]); self.last_clicked = Some(id.clone()); }
                    }
                    if r.secondary_clicked() && !self.selected.contains(id) { self.selected = HashSet::from([id.clone()]); }
                    let sel: Vec<String> = if self.selected.contains(id) { ids.iter().filter(|x| self.selected.contains(*x)).cloned().collect() } else { vec![id.clone()] };
                    let is_liked = liked.contains(id);
                    r.context_menu(|ui| {
                        let lbl = if is_liked { "Убрать из «Любимого»" } else { "В «Любимое»" };
                        if ui.button(lbl).clicked() { acts.push(Act::Like(sel.clone(), !is_liked)); ui.close(); }
                        track_menu(ui, &sel, &t, pid.as_deref(), &pls, acts)
                    });
                });
            });
        if self.drag_ids.is_some() {
            if let Some((y, x0, x1)) = line {
                ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("drop-line"))).line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(2.0, ACCENT));
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if ui.input(|i| i.pointer.any_released()) {
                if let (Some(ids), Some(to)) = (self.drag_ids.take(), self.drop_at.take()) { acts.push(Act::Move(ids, to)); }
                self.drag_ids = None;
                self.drop_at = None;
            }
        }
    }

    /// Постер (2:3) с обрезкой по центру; без постера — заглушка.
    fn poster(&mut self, ui: &egui::Ui, a: &anime::Anime, r: Rect, px: u32) {
        let tex = a.poster.clone().and_then(|p| self.thumbs.poster(&p, px));
        match tex {
            Some(t) => {
                let s = t.size_vec2();
                let (ta, ra) = (s.x / s.y, r.width() / r.height());
                let uv = if ta > ra { let w = ra / ta; Rect::from_min_max(Pos2::new(0.5 - w / 2.0, 0.0), Pos2::new(0.5 + w / 2.0, 1.0)) }
                    else { let h = ta / ra; Rect::from_min_max(Pos2::new(0.0, 0.5 - h / 2.0), Pos2::new(1.0, 0.5 + h / 2.0)) };
                egui::Image::new(&t).uv(uv).corner_radius(10).paint_at(ui, r);
            }
            None => {
                ui.painter().rect_filled(r, 10, BG3);
                ui.painter().rect_stroke(r, 10, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E754}", FontId::proportional(r.width() * 0.28), MUTED);
            }
        }
    }

    fn anime_grid(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let list = self.store.lock().unwrap().lib.anime.clone();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("КОЛЛЕКЦИЯ").size(11.0).color(MUTED));
                ui.label(RichText::new("Аниме").size(34.0).family(bold()).color(TEXT));
                ui.label(RichText::new(if list.is_empty() { "Серии с компьютера — с обложками и названиями с Шикимори".to_string() } else { format!("{} тайтл(ов)", list.len()) }).color(MUTED));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if pill(ui, "\u{E25A}  Папка", true).clicked() { acts.push(Act::ImportAnimeFolder); }
                if pill(ui, "\u{EA20}  Файлы", false).clicked() { acts.push(Act::ImportAnimeFiles); }
            });
        });
        ui.add_space(14.0);
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("\u{E30C} Найти тайтл…").desired_width(360.0).margin(egui::Margin::symmetric(12, 8)));
        ui.add_space(12.0);
        if list.is_empty() {
            ui.add_space(50.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Пока пусто").size(22.0).family(bold()).color(TEXT));
                ui.label(RichText::new("Перетащи сюда папку с сериями (mkv, mp4). Название и номер серии Wavy поймёт из имён файлов,").color(MUTED));
                ui.label(RichText::new("а обложку и русское название возьмёт с Шикимори.").color(MUTED));
            });
            return;
        }
        let q = self.filter.trim().to_lowercase();
        let list: Vec<anime::Anime> = list.into_iter().filter(|a| q.is_empty() || format!("{} {} {}", a.russian, a.name, a.title).to_lowercase().contains(&q)).collect();
        let (cw, ch) = (156.0, 234.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(18.0, 18.0);
                for a in &list {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(cw, ch + 70.0), Sense::click());
                    let pr = Rect::from_min_size(r.min, Vec2::new(cw, ch));
                    self.poster(ui, a, pr, 320);
                    if resp.hovered() { ui.painter().rect_stroke(pr, 10, Stroke::new(2.0, TEXT), egui::StrokeKind::Outside); }
                    let seen = a.watched.iter().filter(|w| a.episodes.contains(w)).count();
                    let badge = format!("{seen}/{}", a.episodes.len());
                    let br = Rect::from_min_size(pr.right_top() + Vec2::new(-52.0, 8.0), Vec2::new(44.0, 20.0));
                    ui.painter().rect_filled(br, 99, Color32::from_black_alpha(190));
                    ui.painter().text(br.center(), Align2::CENTER_CENTER, badge, FontId::proportional(11.5), Color32::WHITE);
                    // название — до двух строк с «…»
                    let mut job = egui::text::LayoutJob::simple(a.display().to_string(), FontId::new(13.5, semi()), TEXT, cw);
                    job.wrap.max_rows = 2;
                    let g = ui.fonts_mut(|f| f.layout_job(job));
                    let th = g.size().y;
                    ui.painter().galley(pr.left_bottom() + Vec2::new(0.0, 8.0), g, TEXT);
                    let meta = if a.matched && a.shiki_id.is_some() { a.meta() } else if a.matched { "нет на Шикимори".into() } else { "ищу на Шикимори…".into() };
                    ui.painter().text(pr.left_bottom() + Vec2::new(0.0, 12.0 + th), Align2::LEFT_TOP, ellipsize(&meta, cw - 10.0, 12.0), FontId::proportional(12.0), MUTED);
                    if resp.clicked() { acts.push(Act::View(View::AnimeTitle(a.id.clone()))); }
                    let id = a.id.clone();
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand).context_menu(|ui| {
                        if ui.button("Смотреть").clicked() { acts.push(Act::PlayAnime(id.clone(), None)); ui.close(); }
                        if ui.button("Другой тайтл на Шикимори…").clicked() { acts.push(Act::Modal(Modal::AnimeMatch { id: id.clone(), query: a.display().to_string(), results: vec![], busy: false, err: None })); acts.push(Act::AnimeSearch(a.display().to_string())); ui.close(); }
                        ui.separator();
                        if ui.button(RichText::new("Удалить из раздела").color(BAD)).clicked() { acts.push(Act::Modal(Modal::DeleteAnime { id: id.clone() })); ui.close(); }
                    });
                }
            });
            ui.add_space(20.0);
        });
    }

    fn anime_page(&mut self, ui: &mut egui::Ui, id: &str, acts: &mut Vec<Act>) {
        let Some(a) = self.store.lock().unwrap().anime(id).cloned() else { acts.push(Act::View(View::Anime)); return };
        if ui.add(egui::Label::new(RichText::new("\u{E138}  Аниме").color(MUTED)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { acts.push(Act::View(View::Anime)); }
        ui.add_space(10.0);
        let eps: Vec<Track> = a.episodes.iter().filter_map(|e| self.track(e)).collect();
        let cont = a.last.as_ref().and_then(|l| eps.iter().find(|t| &t.id == l)).map(|t| t.title.clone());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(150.0, 225.0), Sense::hover());
            self.poster(ui, &a, r, 360);
            ui.add_space(18.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(if a.kind.is_empty() { "АНИМЕ".to_string() } else { anime::kind_ru(&a.kind).to_uppercase() }).size(11.0).color(MUTED));
                ui.add(egui::Label::new(RichText::new(a.display()).size(30.0).family(bold()).color(TEXT)).wrap());
                if !a.name.is_empty() && a.name != a.display() { ui.label(RichText::new(&a.name).color(MUTED)); }
                let mut meta = a.meta();
                let seen = a.watched.iter().filter(|w| a.episodes.contains(w)).count();
                meta = [meta, format!("серий в папке: {} · просмотрено {seen}", eps.len())].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                ui.label(RichText::new(meta).color(MUTED));
                if !a.matched { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Ищу на Шикимори…").color(MUTED)); }); }
                else if a.shiki_id.is_none() { ui.label(RichText::new("На Шикимори не нашлось — выбери вручную").color(MUTED)); }
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    let play = match &cont { Some(t) if a.last_pos > 5.0 || a.watched.len() > 0 => format!("\u{E3D0}  Продолжить · {t}"), _ => "\u{E3D0}  Смотреть".into() };
                    if !eps.is_empty() && pill(ui, &play, true).clicked() { acts.push(Act::PlayAnime(a.id.clone(), None)); }
                    if let Some(u) = &a.url { if pill(ui, "\u{E5DE}  Шикимори", false).clicked() { acts.push(Act::OpenUrl(u.clone())); } }
                    if pill(ui, "\u{E30C}  Другой тайтл", false).on_hover_text("Если Wavy угадал не то аниме или сезон").clicked() {
                        let q = if a.title.is_empty() { a.display().to_string() } else { a.title.clone() };
                        acts.push(Act::Modal(Modal::AnimeMatch { id: a.id.clone(), query: q.clone(), results: vec![], busy: false, err: None }));
                        acts.push(Act::AnimeSearch(q));
                    }
                    if pill(ui, "\u{EA20}  Добавить серии", false).clicked() { acts.push(Act::ImportAnimeFiles); }
                    if pill(ui, "\u{E4A6}  Удалить", false).clicked() { acts.push(Act::Modal(Modal::DeleteAnime { id: a.id.clone() })); }
                });
            });
        });
        ui.add_space(16.0);
        let cur = self.current.as_ref().map(|t| t.id.clone());
        let playing = self.audio.is_playing();
        TableBuilder::new(ui)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(36.0))
            .column(Column::remainder().at_least(200.0).clip(true))
            .column(Column::exact(64.0))
            .max_scroll_height(f32::INFINITY)
            .header(26.0, |mut h| { for t in ["", "СЕРИЯ", "\u{E19A}"] { h.col(|ui| { ui.label(RichText::new(t).size(11.0).color(MUTED)); }); } })
            .body(|body| {
                body.rows(46.0, eps.len(), |mut row| {
                    let t = &eps[row.index()];
                    let is_cur = cur.as_deref() == Some(t.id.as_str());
                    let seen = a.watched.contains(&t.id);
                    row.col(|ui| {
                        let (ic, c) = if is_cur { (if playing { "\u{E340}" } else { "\u{E39E}" }, ACCENT) } else if seen { ("\u{E184}", MUTED) } else { ("", MUTED) };
                        ui.label(RichText::new(ic).color(c));
                    });
                    row.col(|ui| {
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(&t.title).color(if is_cur { ACCENT } else if seen { MUTED } else { TEXT }).size(14.0)).truncate());
                            let file = t.path.as_deref().and_then(|p| Path::new(p).file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.add(egui::Label::new(RichText::new(file).color(MUTED).size(11.5)).truncate());
                        });
                    });
                    row.col(|ui| { ui.label(RichText::new(fmt_time(t.duration)).color(MUTED)); });
                    let r = row.response();
                    if r.double_clicked() { acts.push(Act::PlayAnime(a.id.clone(), Some(t.id.clone()))); }
                    r.context_menu(|ui| {
                        if ui.button("Смотреть").clicked() { acts.push(Act::PlayAnime(a.id.clone(), Some(t.id.clone()))); ui.close(); }
                        if ui.button(if seen { "Отметить непросмотренной" } else { "Отметить просмотренной" }).clicked() { acts.push(Act::MarkWatched(t.id.clone(), !seen)); ui.close(); }
                        if let Some(p) = &t.path { if ui.button("Показать в папке").clicked() { acts.push(Act::Reveal(p.clone())); ui.close(); } }
                    });
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
                            ui.add(egui::Label::new(RichText::new(if self.loading { "загрузка…".to_string() } else if self.audio.buffering() { "буферизация…".to_string() } else { t.artist.clone() }).color(MUTED).size(12.5)).truncate());
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
                    // высота ряда — сразу по большой кнопке: иначе кнопки до неё центрируются по 34 px, а после — по 44
                    ui.set_min_height(44.0);
                    let bw = 34.0 * 4.0 + 44.0 + 4.0 * 14.0;
                    ui.add_space(((ui.available_width() - bw) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 14.0;
                    if icon_btn(ui, "\u{E422}", 34.0, s.shuffle).on_hover_text("Перемешать").clicked() { acts.push(Act::Shuffle); }
                    if icon_btn(ui, "\u{E5A4}", 34.0, false).clicked() { acts.push(Act::Prev); }
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 21.0, if resp.hovered() { Color32::WHITE } else { TEXT });
                    if self.loading || self.audio.buffering() {
                        ui.ctx().request_repaint_after(Duration::from_millis(33));
                        // загрузка: три точки «бегут» по кругу вместо смещённого символа «…»
                        let t = ui.input(|i| i.time) as f32;
                        for k in 0..3 {
                            let a = (k as f32 - 1.0) * 7.0;
                            let lift = ((t * 6.0 - k as f32 * 0.9).sin().max(0.0)) * 3.0;
                            ui.painter().circle_filled(r.center() + Vec2::new(a, -lift), 2.4, Color32::BLACK);
                        }
                    } else {
                        glyph(ui.painter(), r.center(), if self.audio.is_playing() { "\u{E39E}" } else { "\u{E3D0}" }, FontId::new(20.0, egui::FontFamily::Name("fill".into())), Color32::BLACK);
                    }
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
                if icon_btn(ui, "\u{E64C}", 34.0, s.mini).on_hover_text("Мини-плеер поверх окон").clicked() { acts.push(Act::ToggleMini); }
                // звук: скорость, кроссфейд, громкость, эквалайзер
                let fx_on = (s.speed - 1.0).abs() > 0.01 || s.crossfade > 0.0 || s.normalize || s.eq_on;
                let r = icon_btn(ui, "\u{E434}", 34.0, fx_on).on_hover_text("Звук: скорость, кроссфейд, эквалайзер");
                egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| self.sound_ui(ui, &s, acts));
                // таймер сна
                let r = icon_btn(ui, "\u{E330}", 34.0, self.sleep.is_some()).on_hover_text(match self.sleep {
                    Some(Sleep::At(t)) => format!("Таймер сна: через {} мин", (t.saturating_duration_since(Instant::now()).as_secs() + 59) / 60),
                    Some(Sleep::EndOfTrack) => "Таймер сна: после этого трека".into(),
                    None => "Таймер сна".into(),
                });
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(170.0);
                    ui.label(RichText::new("Выключить музыку").color(MUTED).size(12.0));
                    for m in [15u64, 30, 45, 60, 90] {
                        if ui.button(format!("через {m} мин")).clicked() { self.sleep = Some(Sleep::At(Instant::now() + Duration::from_secs(m * 60))); ui.close(); }
                    }
                    if ui.button("после этого трека").clicked() { self.sleep = Some(Sleep::EndOfTrack); ui.close(); }
                    if self.sleep.is_some() && ui.button(RichText::new("отменить таймер").color(BAD)).clicked() { self.sleep = None; ui.close(); }
                });
                if icon_btn(ui, "\u{E660}", 34.0, self.show_lyrics).on_hover_text("Текст песни (T)").clicked() { self.show_lyrics = !self.show_lyrics; if self.show_lyrics { self.show_queue = false; } }
                if icon_btn(ui, "\u{E6AC}", 34.0, self.show_queue).on_hover_text("Очередь (Q)").clicked() { self.show_queue = !self.show_queue; if self.show_queue { self.show_lyrics = false; } }
                if let Some(t) = self.current.clone().filter(|t| t.anime.is_none() && self.store.lock().unwrap().lib.tracks.contains_key(&t.id)) {
                    let on = self.store.lock().unwrap().is_liked(&t.id);
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                    if resp.hovered() { ui.painter().circle_filled(r.center(), 17.0, BG3); }
                    glyph(ui.painter(), r.center(), "\u{E2A8}", FontId::new(17.0, if on { egui::FontFamily::Name("fill".into()) } else { egui::FontFamily::Proportional }), if on { ACCENT } else if resp.hovered() { TEXT } else { MUTED });
                    if resp.on_hover_text(if on { "Убрать из «Любимого» (L)" } else { "В «Любимое» (L)" }).clicked() { acts.push(Act::Like(vec![t.id.clone()], !on)); }
                }
                if self.current.is_some() && icon_btn(ui, "\u{E802}", 34.0, false).on_hover_text("Что это играет? (Shazam) — например, опенинг в аниме").clicked() { acts.push(Act::Shazam); }
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
            let vr = fit(tex.size_vec2());
            p.image(tex.id(), vr, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let clock = self.audio.position();
            if let Some((_, v)) = &self.video { draw_subs(ui, &p, vr, &v.subs_at(clock)); }
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
            let note = if let Some(e) = err.as_ref().filter(|e| e.starts_with(wavy_core::ffmpeg::NEED)) {
                let what = wavy_core::ffmpeg::human(e);
                match self.ff.state() {
                    wavy_core::ffmpeg::State::Downloading(p) => Some((format!("{what}: скачиваю декодер (≈6 МБ, один раз)… {p}%"), MUTED)),
                    wavy_core::ffmpeg::State::Failed(_) => Some((format!("Видео не открылось: {what}"), BAD)),
                    _ => Some((format!("{what}: готовлю декодер…"), MUTED)),
                }
            }
            else if let Some(e) = &err { Some((format!("Видео не открылось: {e}"), BAD)) }
                else if t.has_video() {
                    let pct = self.video_dl.as_ref().filter(|v| v.0 == t.id).and_then(|v| v.1.progress());
                    Some((pct.map(|p| format!("Загружаю видео… {p}%")).unwrap_or_else(|| "Загружаю видео…".into()), MUTED))
                }
                else { None };
            if let Some((n, c)) = note {
                p.text(Pos2::new(area.center().x, y + 54.0), Align2::CENTER_TOP, ellipsize(&n, area.width() - 40.0, 13.0), FontId::proportional(13.0), c);
                if err.as_ref().is_none_or(|e| e.starts_with(wavy_core::ffmpeg::NEED)) { ui.ctx().request_repaint_after(Duration::from_millis(200)); }
            }
        }
        // озвучка / субтитры / Shazam — в оконном режиме всегда, в полноэкранном — когда двигали мышью
        let moved = ui.input(|i| i.pointer.time_since_last_movement()) < 2.5;
        if !self.fullscreen || moved { self.video_controls(ui, area, &t, acts); }
        if t.anime.is_some() { self.anime_overlay(ui, area, acts); }
        // кнопка «во весь экран» и подсказка
        let btn = Rect::from_min_size(area.right_top() + Vec2::new(-46.0, 12.0), Vec2::splat(34.0));
        let b = ui.interact(btn, Id::unique("fs-btn"), Sense::click());
        p.circle_filled(btn.center(), 17.0, Color32::from_black_alpha(if b.hovered() { 200 } else { 120 }));
        p.text(btn.center(), Align2::CENTER_CENTER, if self.fullscreen { "\u{E1CE}" } else { "\u{E1D0}" }, FontId::proportional(17.0), Color32::WHITE);
        if b.on_hover_text(if self.fullscreen { "Выйти из полноэкранного (Esc)" } else { "Во весь экран (двойной клик)" }).clicked() {
            acts.push(Act::Fullscreen(!self.fullscreen));
        } else if resp.double_clicked() {
            // первый клик двойного уже переключил паузу — возвращаем как было
            acts.push(Act::TogglePlay);
            acts.push(Act::Fullscreen(!self.fullscreen));
        } else if resp.clicked() {
            acts.push(Act::TogglePlay);
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        let _ = resp;
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

    /// Кнопки поверх видео: выбор озвучки и субтитров, Shazam.
    fn video_controls(&mut self, ui: &mut egui::Ui, area: Rect, t: &Track, acts: &mut Vec<Act>) {
        let info = self.media_info.as_ref().filter(|m| m.0 == t.id).map(|m| m.1.clone());
        let r = Rect::from_min_size(area.left_top() + Vec2::new(12.0, 12.0), Vec2::new((area.width() - 80.0).max(100.0), 34.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(Layout::left_to_right(Align::Center)));
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 8.0;
        let chip = |text: String| egui::Button::new(RichText::new(text).color(Color32::WHITE).size(13.0)).fill(Color32::from_black_alpha(170)).stroke(Stroke::NONE).corner_radius(99).min_size(Vec2::new(0.0, 30.0));
        if let Some(info) = &info {
            if info.audio.len() > 1 {
                let cur = info.audio.get(self.audio_sel).map(|a| a.label.clone()).unwrap_or_default();
                let resp = ui.add(chip(format!("\u{E44A}  {}", ellipsize(&cur, 180.0, 13.0))));
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(220.0);
                    for (i, a) in info.audio.iter().enumerate() {
                        let txt = if i == self.audio_sel { format!("\u{E182}  {}", a.label) } else { format!("      {}", a.label) };
                        if ui.add(egui::Button::new(txt).frame(false)).clicked() { acts.push(Act::AudioTrack(i)); ui.close(); }
                    }
                });
            }
            if !info.subs.is_empty() {
                let cur = self.sub_sel.and_then(|i| info.subs.get(i)).map(|s| s.label.clone()).unwrap_or_else(|| "выкл".into());
                let resp = ui.add(chip(format!("\u{E1A8}  {}", ellipsize(&cur, 180.0, 13.0))));
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(240.0);
                    let mark = |on: bool, l: &str| if on { format!("\u{E182}  {l}") } else { format!("      {l}") };
                    if ui.add(egui::Button::new(mark(self.sub_sel.is_none(), "Без субтитров")).frame(false)).clicked() { acts.push(Act::Subs(None)); ui.close(); }
                    for (i, s) in info.subs.iter().enumerate() {
                        let l = if s.forced { format!("{} (надписи)", s.label) } else { s.label.clone() };
                        if ui.add(egui::Button::new(mark(self.sub_sel == Some(i), &l)).frame(false)).clicked() { acts.push(Act::Subs(Some(i))); ui.close(); }
                    }
                });
            }
        }
        if ui.add(chip("\u{E802}  Что за песня?".into())).on_hover_text("Shazam: узнать опенинг/эндинг и добавить в библиотеку").clicked() { acts.push(Act::Shazam); }
    }

    /// Серия аниме: загрузить отрезки опенинга/эндинга, пропускать их, предлагать следующую серию.
    fn anime_tick(&mut self, ctx: &egui::Context) {
        let Some(t) = self.current.clone().filter(|t| t.anime.is_some()) else { self.next_ep = None; return };
        if self.skips.as_ref().map(|s| &s.0) != Some(&t.id) {
            self.skips = Some((t.id.clone(), Vec::new()));
            self.next_ep = None;
            let a = self.store.lock().unwrap().anime(t.anime.as_deref().unwrap_or_default()).cloned();
            if let (Some(mal), Some(ep)) = (a.and_then(|a| a.shiki_id), t.episode) {
                let (tx, c, dir, id, len) = (self.tx.clone(), ctx.clone(), wavy_core::data_dir().join("aniskip"), t.id.clone(), t.duration);
                std::thread::spawn(move || {
                    match wavy_core::aniskip::fetch(mal, ep.round() as u32, len, &dir) {
                        Ok(v) => { let _ = tx.send(Bg::Skips(id, v)); }
                        Err(e) => wavy_core::log(&format!("aniskip: {e}")),
                    }
                    c.request_repaint();
                });
            }
        }
        let st = self.settings();
        let pos = self.audio.position();
        let has_next = self.qi + 1 < self.queue.len();
        let skips = self.skips.as_ref().map(|s| s.1.clone()).unwrap_or_default();
        for s in &skips {
            let inside = pos >= s.start && pos < s.end - 3.0;
            // эндинг перед следующей серией — предложим её; иначе (опенинг и т.п.) сами пропускаем, если включено
            if s.is_ed() && has_next && st.anime_next {
                if inside && self.next_ep.is_none() && self.stay_ep.as_deref() != Some(t.id.as_str()) && self.audio.is_playing() {
                    self.next_ep = Some((t.id.clone(), Instant::now() + Duration::from_secs(10)));
                }
            } else if inside && st.anime_autoskip && self.audio.is_playing() {
                let key = format!("{}:{}", t.id, s.start as i64);
                if self.auto_skipped.insert(key) { self.audio.seek(s.end); if let Some((_, v)) = &mut self.video { v.seek(s.end); } self.toast(format!("Пропустил {}", s.label()), false); }
            }
        }
        if let Some((id, at)) = self.next_ep.clone() {
            if id != t.id { self.next_ep = None; }
            else if Instant::now() >= at { self.next_ep = None; self.next(false, ctx); }
            else { ctx.request_repaint_after(Duration::from_millis(250)); }
        }
    }

    /// Отметить на Шикимори, сколько серий тайтла просмотрено.
    fn shiki_sync(&mut self, anime_id: String, ctx: &egui::Context) {
        let mut acc = self.settings().shiki;
        if !acc.logged_in() { return; }
        let Some(a) = self.store.lock().unwrap().anime(&anime_id).cloned() else { return };
        let Some(sid) = a.shiki_id else { return };
        let eps = { let s = self.store.lock().unwrap(); a.watched.iter().filter_map(|w| s.lib.tracks.get(w).and_then(|t| t.episode)).fold(0f32, f32::max) as u32 };
        if eps == 0 { return; }
        let (tx, c) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = wavy_core::shikimori::set_progress(&mut acc, sid, eps, a.episodes_total).map(|_| acc);
            if let Err(e) = &r { wavy_core::log(&format!("шикимори: {e}")); }
            let _ = tx.send(Bg::Shiki(r, false));
            c.request_repaint();
        });
    }

    /// Кнопка «Пропустить опенинг» и карточка «Следующая серия через…» поверх видео.
    fn anime_overlay(&mut self, ui: &mut egui::Ui, area: Rect, acts: &mut Vec<Act>) {
        let pos = self.audio.position();
        let p = ui.painter().clone();
        let corner = area.right_bottom() - Vec2::new(24.0, 90.0);
        if let Some((_, at)) = &self.next_ep {
            let left = at.saturating_duration_since(Instant::now()).as_secs() + 1;
            // сверху справа — внизу субтитры
            let top = area.right_top() + Vec2::new(-24.0, 60.0);
            let card = Rect::from_min_max(Pos2::new(top.x - 300.0, top.y), Pos2::new(top.x, top.y + 80.0));
            p.rect_filled(card, 14, Color32::from_black_alpha(220));
            p.rect_stroke(card, 14, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
            p.text(card.left_top() + Vec2::new(16.0, 14.0), Align2::LEFT_TOP, format!("Следующая серия через {left} с"), FontId::new(15.0, semi()), TEXT);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink2(Vec2::new(14.0, 10.0)).with_min_y(card.top() + 40.0)).layout(Layout::left_to_right(Align::Center)));
            if pill(&mut c, "Смотреть", true).clicked() { self.next_ep = None; acts.push(Act::Next); }
            if pill(&mut c, "Остаться", false).clicked() { self.stay_ep = self.current.as_ref().map(|t| t.id.clone()); self.next_ep = None; }
            return;
        }
        let skip = self.skips.as_ref().and_then(|s| s.1.iter().find(|k| pos >= k.start - 0.5 && pos < k.end - 2.0).cloned());
        let (label, to) = match skip {
            Some(k) => (format!("\u{E5A6}  Пропустить {}", k.label()), k.end),
            // данных нет — обычный опенинг длится 1:30, даём прыжок в начале серии
            None if self.skips.as_ref().is_some_and(|s| s.1.is_empty()) && pos > 1.0 && pos < 240.0 => ("\u{E5A6}  +85 с".to_string(), pos + 85.0),
            None => return,
        };
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(corner - Vec2::new(300.0, 40.0), corner)).layout(Layout::right_to_left(Align::Max)));
        if pill(&mut c, &label, true).clicked() { acts.push(Act::Seek(to)); }
    }

    fn stats_page(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let day = 86_400_000u64;
        let now = wavy_core::now_ms();
        let periods = [("За месяц", now.saturating_sub(30 * day)), ("За год", now.saturating_sub(365 * day)), ("За всё время", 0)];
        let (st, first) = {
            let s = self.store.lock().unwrap();
            (s.stats(periods[self.stats_period as usize].1), s.lib.history.first().map(|h| h.1))
        };
        ui.label(RichText::new("ТВОЯ МУЗЫКА").size(11.0).color(MUTED));
        ui.label(RichText::new("Итоги").size(34.0).family(bold()).color(TEXT));
        ui.horizontal(|ui| {
            for (i, (name, _)) in periods.iter().enumerate() {
                if ui.selectable_label(self.stats_period == i as u8, *name).clicked() { self.stats_period = i as u8; }
            }
        });
        ui.add_space(14.0);
        if st.plays == 0 {
            ui.label(RichText::new("Пока пусто: прослушивание засчитывается, если трек играл дольше 30 секунд.").color(MUTED));
            return;
        }
        ui.horizontal(|ui| {
            let big = |ui: &mut egui::Ui, n: String, label: &str| {
                egui::Frame::new().fill(BG3).corner_radius(14).inner_margin(egui::Margin::symmetric(20, 14)).show(ui, |ui| {
                    ui.vertical(|ui| { ui.label(RichText::new(n).size(30.0).family(bold()).color(TEXT)); ui.label(RichText::new(label).color(MUTED)); });
                });
            };
            big(ui, st.plays.to_string(), "прослушиваний");
            let m = st.minutes.round() as u64;
            big(ui, if m >= 120 { format!("{} ч {} мин", m / 60, m % 60) } else { format!("{m} мин") }, "музыки");
            if let Some((a, _)) = st.top_artists.first() { big(ui, a.clone(), "артист №1"); }
        });
        if let Some(f) = first { ui.label(RichText::new(format!("История ведётся с {}", fmt_date(f))).size(11.5).color(MUTED)); }
        ui.add_space(16.0);
        ui.columns_const(|[l, r]| {
            l.label(RichText::new("Топ треков").size(16.0).family(semi()));
            l.add_space(6.0);
            let ids: Vec<String> = st.top_tracks.iter().map(|x| x.0.clone()).collect();
            for (i, (id, n)) in st.top_tracks.iter().enumerate() {
                let Some(t) = self.track(id) else { continue };
                let resp = l.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}", i + 1)).color(MUTED).size(13.0));
                    self.cover(ui, &t, 40.0);
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(&t.title).color(TEXT)).truncate());
                        ui.label(RichText::new(format!("{} · {n} раз", t.artist)).color(MUTED).size(12.0));
                    });
                }).response.interact(Sense::click());
                if resp.double_clicked() { acts.push(Act::Play(ids.clone(), i, false)); }
                l.add_space(4.0);
            }
            r.label(RichText::new("Топ артистов").size(16.0).family(semi()));
            r.add_space(6.0);
            let max = st.top_artists.first().map(|x| x.1).unwrap_or(1) as f32;
            for (a, n) in &st.top_artists {
                r.label(RichText::new(format!("{a} · {n}")).color(TEXT));
                let (rect, _) = r.allocate_exact_size(Vec2::new(r.available_width().min(260.0), 4.0), Sense::hover());
                r.painter().rect_filled(rect, 2, BG3);
                r.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width() * *n as f32 / max, 4.0)), 2, TEXT);
                r.add_space(6.0);
            }
        });
    }

    fn lyrics_ui(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Текст").family(semi()).size(15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if icon_btn(ui, "\u{E4F6}", 24.0, false).clicked() { self.show_lyrics = false; } });
        });
        ui.separator();
        let Some(t) = self.current.clone() else { ui.label(RichText::new("Ничего не играет").color(MUTED)); return };
        // загрузка текста для нового трека
        if self.lyrics.as_ref().map(|l| &l.0) != Some(&t.id) {
            self.lyrics = Some((t.id.clone(), None));
            let (tx, c, dir) = (self.tx.clone(), ui.ctx().clone(), wavy_core::data_dir().join("lyrics"));
            std::thread::spawn(move || { let id = t.id.clone(); let _ = tx.send(Bg::Lyrics(id, wavy_core::lyrics::fetch(&t, &dir))); c.request_repaint(); });
        }
        let Some((_, slot)) = &self.lyrics else { return };
        match slot {
            None => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Ищу текст…").color(MUTED)); }); }
            Some(Err(e)) => { ui.label(RichText::new(e.as_str()).color(BAD)); }
            Some(Ok(l)) if l.instrumental => { ui.label(RichText::new("♪ Инструментал — без слов").color(MUTED)); }
            Some(Ok(l)) if l.is_empty() => {
                ui.label(RichText::new("Текста не нашлось").color(MUTED));
                ui.label(RichText::new("Тексты берутся из открытой базы LRCLIB. Если у трека странное название — поправь его (правый клик → «Изменить…»).").color(MUTED).size(12.0));
            }
            Some(Ok(l)) if l.synced.is_empty() => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| { ui.label(RichText::new(&l.plain).size(14.0).color(TEXT)); });
            }
            Some(Ok(l)) => {
                let l = l.clone();
                let cur = l.line_at(self.audio.position());
                let jump = cur != self.lyrics_line;
                self.lyrics_line = cur;
                ui.label(RichText::new("клик по строке — перемотать").size(11.0).color(MUTED));
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(ui.available_height() * 0.3);
                    for (i, (time, line)) in l.synced.iter().enumerate() {
                        let on = Some(i) == cur;
                        let text = if line.is_empty() { "♪".to_string() } else { line.clone() };
                        let r = ui.add(egui::Label::new(RichText::new(text).size(if on { 18.0 } else { 15.0 }).family(if on { bold() } else { egui::FontFamily::Proportional })
                            .color(if on { TEXT } else if cur.is_some_and(|c| i < c) { MUTED.gamma_multiply(0.6) } else { MUTED })).wrap().sense(Sense::click()));
                        if r.clicked() { acts.push(Act::Seek(*time)); }
                        if on && jump { r.scroll_to_me(Some(Align::Center)); }
                        ui.add_space(6.0);
                    }
                    ui.add_space(200.0);
                });
                if self.audio.is_playing() { ui.ctx().request_repaint_after(Duration::from_millis(200)); }
            }
        }
    }

    fn queue_ui(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Очередь").family(semi()).size(15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if icon_btn(ui, "\u{E4F6}", 24.0, false).clicked() { self.show_queue = false; } });
        });
        ui.separator();
        ui.label(RichText::new("тяни трек — переставить").size(11.0).color(MUTED));
        let q = self.queue.clone();
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let mut target: Option<(usize, f32, f32, f32)> = None;
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
                }).response.interact(Sense::click_and_drag());
                if r.clicked() { acts.push(Act::JumpQueue(i)); }
                if r.drag_started() { self.drag_queue = Some(i); }
                if let (Some(_), Some(p)) = (self.drag_queue, pointer) {
                    if r.rect.y_range().contains(p.y) {
                        let below = p.y > r.rect.center().y;
                        target = Some((if below { i + 1 } else { i }, if below { r.rect.bottom() } else { r.rect.top() }, r.rect.left(), r.rect.right()));
                    }
                }
            }
        });
        if let Some(from) = self.drag_queue {
            if let Some((_, y, x0, x1)) = target {
                ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("q-drop"))).line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(2.0, ACCENT));
            }
            if ui.input(|i| i.pointer.any_released()) {
                self.drag_queue = None;
                if let Some((to, ..)) = target { acts.push(Act::MoveQueue(from, to)); }
            }
        }
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
                    ui.label(RichText::new("Spotify (плейлист, альбом, трек), YouTube Music, YouTube, SoundCloud, Bandcamp — всё, что умеет yt-dlp.").color(MUTED));
                    ui.add_space(10.0);
                    let r = ui.add(egui::TextEdit::singleline(url).hint_text("https://music.youtube.com/playlist?list=...").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                    if url.is_empty() { r.request_focus(); }
                    if let Some(p) = &cur_pl { ui.checkbox(into, format!("Добавить в текущий плейлист «{}»", pls_name(p))); }
                    if *busy { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Загружаю… большие плейлисты — до минуты").color(MUTED)); }); }
                    if let Some(e) = err { ui.label(RichText::new(e.as_str()).color(BAD)); }
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter)); // Enter запускает импорт, даже если ссылку вставили без фокуса на поле
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
                Modal::WhatsNew { from, to, items } => {
                    ui.label(RichText::new("Что нового в Wavy").size(20.0).family(bold()));
                    ui.label(RichText::new(format!("Сборки {} → {}", *from + 1, to)).color(MUTED));
                    ui.add_space(8.0);
                    match items {
                        None => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Загружаю…").color(MUTED)); }); }
                        Some(Err(e)) => { ui.label(RichText::new(e.as_str()).color(BAD)); }
                        Some(Ok(v)) if v.is_empty() => { ui.label(RichText::new("Только мелкие технические правки").color(MUTED)); }
                        Some(Ok(v)) => {
                            egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                                for (title, body) in v.iter() {
                                    ui.label(RichText::new(format!("• {title}")).family(semi()).color(TEXT));
                                    if !body.is_empty() { ui.label(RichText::new(body).color(MUTED).size(12.5)); }
                                    ui.add_space(8.0);
                                }
                            });
                        }
                    }
                    ui.add_space(6.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, "Круто", true).clicked() { close = true; } });
                }
                Modal::EditTrack { id, title, artist, album } => {
                    ui.label(RichText::new("Изменить трек").size(20.0).family(bold()));
                    ui.add_space(8.0);
                    for (label, v) in [("Название", &mut *title), ("Исполнитель", &mut *artist), ("Альбом", &mut *album)] {
                        ui.label(RichText::new(label).color(MUTED).size(12.0));
                        ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                        ui.add_space(4.0);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if pill(ui, "Своя обложка…", false).clicked() { acts.push(Act::TrackCover(id.clone())); }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if pill(ui, "Сохранить", true).clicked() && !title.trim().is_empty() { acts.push(Act::EditTrack(id.clone(), title.clone(), artist.clone(), album.clone())); close = true; }
                            if pill(ui, "Отмена", false).clicked() { close = true; }
                        });
                    });
                }
                Modal::Dupes { groups } => {
                    let n: usize = groups.iter().map(|g| g.len() - 1).sum();
                    ui.label(RichText::new(format!("Дубли: {n}")).size(20.0).family(bold()));
                    ui.label(RichText::new("Из каждой группы останется первый (самый старый) трек. В плейлистах дубли заменятся им, лайки и прослушивания перейдут к нему.").color(MUTED));
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                        for g in groups.iter() {
                            for (i, id) in g.iter().enumerate() {
                                if let Some(t) = self.store.lock().unwrap().lib.tracks.get(id) {
                                    let src = if t.is_local() { t.path.clone().unwrap_or_default() } else { t.url.clone().unwrap_or_default() };
                                    ui.label(RichText::new(format!("{} {} — {}  ·  {}", if i == 0 { "✓" } else { "×" }, t.artist, t.title, src)).color(if i == 0 { TEXT } else { MUTED }).size(12.5));
                                }
                            }
                            ui.separator();
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, "Оставить по одному", true).clicked() { acts.push(Act::MergeDupes(groups.clone())); close = true; }
                        if pill(ui, "Отмена", false).clicked() { close = true; }
                    });
                }
                Modal::Folder { id, name } => {
                    ui.label(RichText::new(format!("Папка для «{}»", pls_name(id))).size(20.0).family(bold()));
                    let existing: Vec<String> = { let mut v: Vec<String> = self.store.lock().unwrap().lib.playlists.iter().filter_map(|p| p.folder.clone()).collect(); v.sort(); v.dedup(); v };
                    if !existing.is_empty() {
                        ui.horizontal_wrapped(|ui| { for f in &existing { if ui.selectable_label(name == f, f).clicked() { *name = f.clone(); } } });
                    }
                    ui.add(egui::TextEdit::singleline(name).hint_text("Название новой папки").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, "Готово", true).clicked() { acts.push(Act::SetFolder(id.clone(), Some(name.clone()))); close = true; }
                        if pill(ui, "Отмена", false).clicked() { close = true; }
                    });
                }
                Modal::AnimeMatch { id, query, results, busy, err } => {
                    ui.label(RichText::new("Какое это аниме?").size(20.0).family(bold()));
                    ui.label(RichText::new("Поиск на Шикимори — по-русски, по-английски или по-японски (ромадзи).").color(MUTED));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(query).desired_width(360.0).margin(egui::Margin::symmetric(10, 8)));
                        let go = pill(ui, if *busy { "Ищу…" } else { "Найти" }, true).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if go && !query.trim().is_empty() && !*busy { acts.push(Act::AnimeSearch(query.trim().to_string())); }
                    });
                    if let Some(e) = err { ui.label(RichText::new(e.as_str()).color(BAD)); }
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        for r in results.iter() {
                            let resp = ui.horizontal(|ui| {
                                ui.set_min_height(64.0);
                                let (pr, _) = ui.allocate_exact_size(Vec2::new(40.0, 60.0), Sense::hover());
                                match r.poster.as_ref().and_then(|u| self.thumbs.poster(u, 120)) {
                                    Some(t) => { egui::Image::new(&t).corner_radius(4).paint_at(ui, pr); }
                                    None => { ui.painter().rect_filled(pr, 4, BG3); }
                                }
                                ui.vertical(|ui| {
                                    ui.add_space(6.0);
                                    ui.add(egui::Label::new(RichText::new(r.display()).family(semi()).color(TEXT)).truncate());
                                    let meta = [anime::kind_ru(&r.kind).to_string(), if r.year > 0 { r.year.to_string() } else { String::new() }, if r.episodes > 0 { format!("{} эп.", r.episodes) } else { String::new() }, r.name.clone()];
                                    ui.add(egui::Label::new(RichText::new(meta.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")).color(MUTED).size(12.5)).truncate());
                                });
                            }).response.interact(Sense::click());
                            if resp.hovered() { ui.painter().rect_stroke(resp.rect.expand(2.0), 8, Stroke::new(1.0, LINE), egui::StrokeKind::Outside); }
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { acts.push(Act::AnimeApply(id.clone(), r.clone())); close = true; }
                        }
                    });
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, "Отмена", false).clicked() { close = true; } });
                }
                Modal::DeleteAnime { id } => {
                    let name = self.store.lock().unwrap().anime(id).map(|a| a.display().to_string()).unwrap_or_default();
                    ui.label(RichText::new(format!("Убрать «{name}»?")).size(20.0).family(bold()));
                    ui.label(RichText::new("Файлы серий на диске останутся — пропадёт только запись в Wavy.").color(MUTED));
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, "Убрать", true).clicked() { acts.push(Act::DeleteAnime(id.clone())); close = true; }
                        if pill(ui, "Отмена", false).clicked() { close = true; }
                    });
                }
                Modal::Shazam { state, to_openings } => {
                    ui.label(RichText::new("\u{E802}  Что за песня?").size(20.0).family(bold()));
                    ui.add_space(8.0);
                    match state.clone() {
                        ShazamState::Listening => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Слушаю 12 секунд с текущего места и спрашиваю Shazam…").color(MUTED)); }); }
                        ShazamState::Adding => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Ищу трек на YouTube и добавляю…").color(MUTED)); }); }
                        ShazamState::NotFound => {
                            ui.label(RichText::new("Shazam не узнал этот кусок.").color(TEXT));
                            ui.label(RichText::new("Попробуй ещё раз, когда играет сама песня (опенинг/эндинг), а не диалог.").color(MUTED));
                        }
                        ShazamState::Error(e) => { ui.label(RichText::new(e).color(BAD)); }
                        ShazamState::Found(f) => {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(96.0), Sense::hover());
                                match f.cover.as_ref().and_then(|c| self.thumbs.get(c, 200)) {
                                    Some(t) => { egui::Image::new(&t).corner_radius(10).paint_at(ui, r); }
                                    None => { ui.painter().rect_filled(r, 10, BG3); ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E33C}", FontId::proportional(36.0), MUTED); }
                                }
                                ui.add_space(10.0);
                                ui.vertical(|ui| {
                                    ui.add_space(16.0);
                                    ui.add(egui::Label::new(RichText::new(&f.title).size(18.0).family(bold()).color(TEXT)).wrap());
                                    ui.label(RichText::new(&f.artist).color(MUTED));
                                });
                            });
                            ui.add_space(8.0);
                            ui.checkbox(to_openings, format!("Ещё и в плейлист «{OPENINGS}»"));
                        }
                    }
                    ui.add_space(10.0);
                    let st = state.clone();
                    let to_pl = *to_openings;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        match &st {
                            ShazamState::Found(f) => {
                                if pill(ui, "\u{E340}  В библиотеку", true).clicked() { acts.push(Act::ShazamAdd(f.clone(), to_pl)); }
                                if let Some(u) = &f.url { if pill(ui, "Открыть в Shazam", false).clicked() { acts.push(Act::OpenUrl(u.clone())); } }
                            }
                            ShazamState::NotFound | ShazamState::Error(_) => { if pill(ui, "Ещё раз", true).clicked() { acts.push(Act::Shazam); } }
                            _ => {}
                        }
                        if !matches!(st, ShazamState::Adding) && pill(ui, "Закрыть", false).clicked() { close = true; }
                    });
                }
                Modal::Settings { client_id, port, sp_id, sp_secret } => {
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

                        section(ui, "\u{E754}  Аниме");
                        let mut a = s.anime_autoskip;
                        if ui.checkbox(&mut a, "Сам пропускать опенинги (время берётся из AniSkip)").changed() { acts.push(Act::Settings(json!({"animeAutoskip": a}))); }
                        let mut n = s.anime_next;
                        if ui.checkbox(&mut n, "На эндинге предлагать следующую серию").changed() { acts.push(Act::Settings(json!({"animeNext": n}))); }
                        ui.add_space(4.0);
                        ui.label(RichText::new("Шикимори: отмечать просмотренные серии в твоём списке").family(semi()));
                        let acc = s.shiki.clone();
                        if acc.logged_in() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("Вошёл как {}", acc.nickname)).color(OK));
                                if pill(ui, "Выйти", false).clicked() { acts.push(Act::Settings(json!({"shiki": wavy_core::shikimori::Account { client_id: acc.client_id.clone(), client_secret: acc.client_secret.clone(), ..Default::default() }}))); }
                            });
                        } else {
                            ui.label(RichText::new("1) На shikimori.io/oauth/applications создай приложение: название Wavy, Redirect URI — urn:ietf:wg:oauth:2.0:oob, доступ user_rates. 2) Вставь сюда его Client ID и Client Secret. 3) Нажми «Войти», разреши доступ и вставь код со страницы.").color(MUTED).size(12.0));
                            let (mut id, mut sec) = (acc.client_id.clone(), acc.client_secret.clone());
                            ui.horizontal(|ui| {
                                ui.label("Client ID");
                                if ui.add(egui::TextEdit::singleline(&mut id).desired_width(300.0)).changed() { acts.push(Act::Settings(json!({"shiki": {"clientId": id.trim()}}))); }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Client Secret");
                                if ui.add(egui::TextEdit::singleline(&mut sec).password(true).desired_width(280.0)).changed() { acts.push(Act::Settings(json!({"shiki": {"clientSecret": sec.trim()}}))); }
                            });
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!acc.client_id.is_empty(), pill_btn("Войти", false)).clicked() { open_url(&wavy_core::shikimori::authorize_url(&acc.client_id)); }
                                ui.add(egui::TextEdit::singleline(&mut self.shiki_code).hint_text("код со страницы Шикимори").desired_width(200.0));
                                if ui.add_enabled(!self.shiki_code.trim().is_empty() && !acc.client_secret.is_empty(), pill_btn("Подтвердить", true)).clicked() {
                                    let (mut acc, code, tx, c) = (acc.clone(), self.shiki_code.clone(), self.tx.clone(), ui.ctx().clone());
                                    std::thread::spawn(move || { let r = wavy_core::shikimori::login(&mut acc, &code).map(|_| acc); let _ = tx.send(Bg::Shiki(r, true)); c.request_repaint(); });
                                }
                            });
                        }
                        section(ui, "\u{E58C}  Видео");
                        ui.label(RichText::new("H.264 показывается встроенным декодером. Для HEVC (H.265), 10-битного видео, VP9, AV1 и звука Opus/AC3/DTS Wavy один раз скачивает свой маленький ffmpeg (≈6 МБ) — сам, когда такое видео встретится. Если видеокарта умеет этот формат, декодирует она.").color(MUTED).size(12.5));
                        match self.ff.state() {
                            wavy_core::ffmpeg::State::Ready(_) => { ui.label(RichText::new("Декодер ffmpeg: готов").color(OK)); }
                            wavy_core::ffmpeg::State::Downloading(p) => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(format!("Скачиваю декодер… {p}%")).color(MUTED)); }); }
                            wavy_core::ffmpeg::State::Failed(e) => { ui.label(RichText::new(format!("Декодер: {e}")).color(BAD)); }
                            wavy_core::ffmpeg::State::Missing => {
                                if pill(ui, "Скачать декодер сейчас", false).clicked() { let c = ui.ctx().clone(); self.ff.install_background(move || c.request_repaint()); }
                            }
                        }

                        section(ui, "\u{E58C}  Синхронизация с телефоном");
                        ui.label(RichText::new("Плейлисты и треки YouTube / Spotify переносятся файлом wavy-export.json в обе стороны. Файл с телефона можно просто перетащить в окно. Локальные файлы не переносятся.").color(MUTED).size(12.5));
                        ui.horizontal(|ui| {
                            if pill(ui, "Экспорт для телефона", true).clicked() { acts.push(Act::SyncExport); }
                            if pill(ui, "Импорт с телефона", false).clicked() { acts.push(Act::SyncImport(None)); }
                        });

                        section(ui, "\u{E340}  Spotify");
                        ui.label(RichText::new("Ссылки на плейлисты, альбомы и треки Spotify импортируются без настроек: каждый трек находится на YouTube. Без ключа — до 100 треков из плейлиста.").color(MUTED).size(12.5));
                        ui.label(RichText::new("Для больших плейлистов: developer.spotify.com → Create app → скопируй Client ID и Client Secret.").color(MUTED).size(12.5));
                        ui.horizontal(|ui| {
                            ui.label("Client ID");
                            let r = ui.add(egui::TextEdit::singleline(sp_id).desired_width(300.0).margin(egui::Margin::symmetric(8, 6)));
                            if r.changed() { sp_id.retain(|c| c.is_ascii_alphanumeric()); }
                            if r.lost_focus() || (r.changed() && sp_id.len() == 32) { acts.push(Act::Settings(json!({"spotify": {"clientId": sp_id.as_str()}}))); }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Secret   ");
                            let r = ui.add(egui::TextEdit::singleline(sp_secret).password(true).desired_width(300.0).margin(egui::Margin::symmetric(8, 6)));
                            if r.changed() { sp_secret.retain(|c| c.is_ascii_alphanumeric()); }
                            if r.lost_focus() || (r.changed() && sp_secret.len() == 32) { acts.push(Act::Settings(json!({"spotify": {"clientSecret": sp_secret.as_str()}}))); }
                        });

                        section(ui, "\u{E272}  Удобство");
                        if cfg!(windows) {
                            let mut ct = s.close_to_tray;
                            if ui.checkbox(&mut ct, "Крестик сворачивает в трей (музыка продолжает играть)").changed() { acts.push(Act::Settings(json!({"closeToTray": ct}))); }
                        }
                        let mut mini = s.mini;
                        if ui.checkbox(&mut mini, "Мини-плеер поверх всех окон").changed() { acts.push(Act::ToggleMini); }
                        ui.label(RichText::new("Клавиши в окне: Пробел — пауза · ←/→ — ±5 с · Ctrl+←/→ — трек · ↑/↓ — громкость · M — без звука · L — лайк · Q — очередь · S — перемешать · R — повтор · Ctrl+F — поиск").color(MUTED).size(12.0));
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
                            if let Some(b) = update::current_build() { if pill(ui, "Что нового", false).clicked() { acts.push(Act::WhatsNew(b.saturating_sub(5).max(1), b)); } }
                        });
                    });
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, "Готово", true).clicked() { close = true; } });
                }
            }
        });
        if resp.should_close() && !matches!(m, Modal::ImportUrl { busy: true, .. } | Modal::Shazam { state: ShazamState::Adding, .. }) { close = true; }
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
    fn sound_ui(&mut self, ui: &mut egui::Ui, s: &wavy_core::store::Settings, acts: &mut Vec<Act>) {
        ui.set_width(330.0);
        ui.label(RichText::new("Скорость").family(semi()));
        ui.horizontal(|ui| {
            for (name, v) in [("Slowed", 0.85f32), ("Обычно", 1.0), ("Nightcore", 1.25)] {
                if ui.selectable_label((s.speed - v).abs() < 0.01, name).clicked() { acts.push(Act::Settings(json!({"speed": v}))); }
            }
        });
        let mut sp = s.speed;
        if ui.add(egui::Slider::new(&mut sp, 0.5..=2.0).step_by(0.05).custom_formatter(|v, _| format!("×{v:.2}"))).changed() { acts.push(Act::Settings(json!({"speed": sp}))); }
        ui.label(RichText::new("Тон меняется вместе со скоростью — как в nightcore/slowed").color(MUTED).size(11.5));
        ui.separator();
        let mut cf = s.crossfade;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Кроссфейд").family(semi()));
            if ui.add(egui::Slider::new(&mut cf, 0.0..=12.0).step_by(1.0).custom_formatter(|v, _| if v == 0.0 { "выкл".into() } else { format!("{v:.0} с") })).changed() { acts.push(Act::Settings(json!({"crossfade": cf}))); }
        });
        let mut nz = s.normalize;
        if ui.checkbox(&mut nz, "Выравнивать громкость (тихие треки громче, громкие тише)").changed() { acts.push(Act::Settings(json!({"normalize": nz}))); }
        ui.separator();
        let mut on = s.eq_on;
        ui.horizontal(|ui| {
            if ui.checkbox(&mut on, RichText::new("Эквалайзер").family(semi())).changed() { acts.push(Act::Settings(json!({"eqOn": on}))); }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("Пресеты", |ui| {
                    for (name, eq) in fx::PRESETS {
                        if ui.button(*name).clicked() { acts.push(Act::Settings(json!({"eq": eq, "eqOn": true}))); ui.close(); }
                    }
                });
            });
        });
        ui.add_enabled_ui(s.eq_on, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut eq = s.eq;
                let mut changed = false;
                for (i, f) in fx::BANDS.iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.set_width(28.0);
                        changed |= ui.add(egui::Slider::new(&mut eq[i], -12.0..=12.0).vertical().step_by(1.0).show_value(false)).on_hover_text(format!("{:+.0} дБ", eq[i])).changed();
                        ui.label(RichText::new(if *f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") }).size(10.0).color(MUTED));
                    });
                }
                if changed { acts.push(Act::Settings(json!({"eq": eq}))); }
            });
        });
    }

    /// Мини-плеер: маленькое окно поверх всех, тянется мышкой.
    fn mini_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let s = self.settings();
        let pos = s.mini_pos.unwrap_or([60.0, 60.0]);
        let vb = egui::ViewportBuilder::default().with_title("Wavy mini").with_decorations(false).with_always_on_top()
            .with_taskbar(false).with_resizable(false).with_position(pos).with_inner_size([360.0, 84.0]);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("wavy-mini"), vb, |ui, _| {
            let ctx = ui.ctx().clone();
            let rect = ui.max_rect();
            let p = ui.painter().clone();
            p.rect_filled(rect, 0, BG2);
            p.rect_stroke(rect, 0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
            if ctx.input(|i| i.viewport().close_requested()) { acts.push(Act::ToggleMini); }
            // перетаскивание за любое пустое место
            let drag = ui.interact(rect, Id::unique("mini-drag"), Sense::click_and_drag());
            if drag.drag_started() { ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
            if drag.double_clicked() { acts.push(Act::ShowWindow); }
            if let Some(o) = ctx.input(|i| i.viewport().outer_rect) {
                let b = [o.min.x, o.min.y];
                if b != pos && !ctx.input(|i| i.pointer.any_down()) { acts.push(Act::Settings(json!({"miniPos": b}))); }
            }
            let cr = Rect::from_min_size(rect.min + Vec2::new(12.0, 12.0), Vec2::splat(60.0));
            match self.current.clone() {
                Some(t) => {
                    match t.cover.clone().and_then(|c| self.thumbs.get(&c, 120)) {
                        Some(tex) => { egui::Image::new(&tex).corner_radius(8).paint_at(ui, cr); }
                        None => { p.rect_filled(cr, 8, BG3); glyph(&p, cr.center(), "\u{E33C}", FontId::proportional(22.0), MUTED); }
                    }
                    let tx = cr.right() + 12.0;
                    let w = rect.right() - 128.0 - tx;
                    let clip = Rect::from_min_max(Pos2::new(tx, rect.top()), Pos2::new(tx + w, rect.bottom()));
                    let pc = p.with_clip_rect(clip);
                    pc.text(Pos2::new(tx, cr.top() + 12.0), Align2::LEFT_CENTER, &t.title, FontId::new(14.0, semi()), TEXT);
                    pc.text(Pos2::new(tx, cr.top() + 32.0), Align2::LEFT_CENTER, &t.artist, FontId::proportional(12.0), MUTED);
                    // прогресс
                    let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
                    let f = if dur > 0.0 { (self.audio.position() / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
                    let bar = Rect::from_min_size(Pos2::new(tx, cr.bottom() - 6.0), Vec2::new(w, 3.0));
                    p.rect_filled(bar, 2, BG3);
                    p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * f, 3.0)), 2, TEXT);
                }
                None => { p.text(Pos2::new(cr.right() + 12.0, rect.center().y), Align2::LEFT_CENTER, "Ничего не играет", FontId::proportional(13.0), MUTED); }
            }
            // кнопки
            let mut x = rect.right() - 120.0;
            let cy = rect.center().y;
            for (icon, act, big) in [("\u{E5A4}", 0, false), (if self.audio.is_playing() { "\u{E39E}" } else { "\u{E3D0}" }, 1, true), ("\u{E5A6}", 2, false)] {
                let sz = if big { 40.0 } else { 32.0 };
                let r = Rect::from_center_size(Pos2::new(x + sz / 2.0, cy), Vec2::splat(sz));
                let resp = ui.interact(r, Id::unique(("mini-btn", act)), Sense::click());
                if big { p.circle_filled(r.center(), sz / 2.0, if resp.hovered() { Color32::WHITE } else { TEXT }); }
                else if resp.hovered() { p.circle_filled(r.center(), sz / 2.0, BG3); }
                let font = if big { FontId::new(18.0, egui::FontFamily::Name("fill".into())) } else { FontId::proportional(16.0) };
                glyph(&p, r.center(), icon, font, if big { Color32::BLACK } else { TEXT });
                if resp.clicked() { acts.push([Act::Prev, Act::TogglePlay, Act::Next].into_iter().nth(act).unwrap()); }
                x += sz + 2.0;
            }
            // закрыть
            let xr = Rect::from_min_size(Pos2::new(rect.right() - 18.0, rect.top() + 2.0), Vec2::splat(16.0));
            let xresp = ui.interact(xr, Id::unique("mini-x"), Sense::click());
            p.text(xr.center(), Align2::CENTER_CENTER, "×", FontId::proportional(14.0), if xresp.hovered() { TEXT } else { MUTED });
            if xresp.clicked() { acts.push(Act::ToggleMini); }
            if self.audio.is_playing() { ctx.request_repaint_after(Duration::from_millis(500)); }
        });
    }

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

enum AudioSrc { File(PathBuf), Stream(wavy_core::stream::Download) }

/// Ошибка «видео больше нет»: удалено, закрыто, заблокировано в стране или по жалобе.
fn gone_from_youtube(err: &str) -> bool {
    let e = err.to_lowercase();
    ["video unavailable", "not available", "private video", "has been removed", "been terminated", "copyright", "unavailable"].iter().any(|m| e.contains(m))
}

/// Звук YouTube-трека: готовый файл из кэша → поток через rustypipe (играет с первых КБ) → yt-dlp.
fn fetch_audio(yt: &YouTube, t: &Track, cache: &Path) -> Result<AudioSrc, String> {
    let _ = std::fs::create_dir_all(cache);
    let key = t.url.clone().unwrap_or_default();
    let path = cache.join(format!("{}.s.m4a", wavy_core::sha16(key.as_bytes())));
    if wavy_core::stream::is_complete(&path) { return Ok(AudioSrc::File(path)); }
    if let Some(vid) = &t.video_id {
        match yt.direct(vid, false) {
            Ok(d) => {
                let dl = wavy_core::stream::Download::start(d.url, path.clone(), d.size);
                // ~200 КБ хватает декодеру, чтобы начать
                if dl.wait_for(200 * 1024, Duration::from_secs(20)) { return Ok(AudioSrc::Stream(dl)); }
                wavy_core::log(&format!("поток «{}» не пошёл: {:?} — пробую yt-dlp", t.title, dl.error()));
                dl.cancel();
                yt.forget(vid);
            }
            Err(e) => wavy_core::log(&format!("rustypipe «{}»: {e} — пробую yt-dlp", t.title)),
        }
        // yt-dlp потоком: играем, пока он качает (с 2025 года ссылки rustypipe YouTube часто отклоняет — 403)
        let t0 = std::time::Instant::now();
        match yt.stream_audio(t, &path) {
            Ok(dl) => {
                if dl.wait_for(200 * 1024, Duration::from_secs(40)) {
                    wavy_core::log(&format!("«{}»: поток yt-dlp пошёл через {:.1} с", t.title, t0.elapsed().as_secs_f32()));
                    return Ok(AudioSrc::Stream(dl));
                }
                let e = dl.error().unwrap_or_else(|| "долго нет данных".into());
                dl.cancel();
                // видео пропало — дальше качать бессмысленно, пусть ищется замена
                if gone_from_youtube(&e) { return Err(e); }
                wavy_core::log(&format!("поток yt-dlp «{}» не пошёл: {e} — качаю целиком", t.title));
            }
            Err(e) => wavy_core::log(&format!("yt-dlp потоком «{}»: {e}", t.title)),
        }
    }
    yt.download_audio(t, cache).map(AudioSrc::File)
}

/// Ссылка → треки: Spotify (через поиск на YouTube) или всё, что умеет yt-dlp.
/// Возвращает (название плейлиста, источник, треки, примечание для пользователя).
fn fetch_url(url: &str, yt: &YouTube, store: &Arc<Mutex<Store>>, tx: &Sender<Bg>) -> Result<(Option<String>, String, Vec<Track>, String), String> {
    let t2 = tx.clone();
    let status = move |m: &str| { let _ = t2.send(Bg::Status(m.into())); };
    if let Some((kind, _)) = wavy_core::spotify::parse_url(url) {
        let sp = store.lock().unwrap().lib.settings.spotify.clone();
        let r = wavy_core::spotify::import(url, yt, Some((&sp.client_id, &sp.client_secret)), &status)?;
        let mut note = String::new();
        if !r.not_found.is_empty() { note += &format!(", не нашлось на YouTube: {} (список — в wavy.log)", r.not_found.len()); }
        if r.truncated { note += ". Spotify без ключа отдаёт только первые 100 треков — ключ добавляется в Настройках"; }
        Ok(((kind != "track").then_some(r.name), r.source, r.tracks, note))
    } else {
        let r = yt.import_url(url.trim(), &status)?;
        Ok((r.name, r.source, r.tracks, String::new()))
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
    let lbl = if ids.len() > 1 { format!("Сохранить как MP3 ({})…", ids.len()) } else { "Сохранить как MP3…".into() };
    if ui.button(lbl).clicked() { acts.push(Act::Export(ids.clone(), None)); ui.close(); }
    if let Some(p) = pid { if ui.button("Убрать из плейлиста").clicked() { acts.push(Act::RemoveFrom(p.into(), ids.clone())); ui.close(); } }
    if ids.len() == 1 {
        if t.video_id.is_some() && ui.button("Радио по треку").on_hover_text("Похожие песни с YouTube — в очередь").clicked() { acts.push(Act::Radio(t.id.clone())); ui.close(); }
        if ui.button("Изменить…").clicked() { acts.push(Act::Modal(Modal::EditTrack { id: t.id.clone(), title: t.title.clone(), artist: t.artist.clone(), album: t.album.clone() })); ui.close(); }
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
    // с точкой-индикатором иконка чуть приподнята, чтобы пара «иконка + точка» стояла по центру
    glyph(ui.painter(), r.center() - Vec2::new(0.0, if on { 2.0 } else { 0.0 }), icon, FontId::proportional(size * 0.5), c);
    if on { ui.painter().circle_filled(r.center_bottom() - Vec2::new(0.0, 2.0), 2.0, ACCENT); }
    resp
}

/// Иконка точно по центру: по контуру самого значка, а не по высоте строки шрифта
/// (у шрифта иконок значки сидят в строке по-разному — отсюда «чутка оффсетнутые» кнопки).
fn glyph(p: &egui::Painter, center: Pos2, icon: &str, font: FontId, color: Color32) {
    let g = p.layout_no_wrap(icon.to_string(), font, color);
    let b = g.mesh_bounds;
    let at = if b.is_positive() { center - b.center().to_vec2() } else { center - g.rect.center().to_vec2() };
    p.galley(at, g, color);
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

/// Субтитры: белый текст с чёрной обводкой, снизу кадра (надписи \an8 — сверху).
fn draw_subs(ui: &egui::Ui, p: &egui::Painter, vr: Rect, cues: &[wavy_core::media::Cue]) {
    if cues.is_empty() { return; }
    let size = (vr.height() * 0.052).clamp(15.0, 54.0);
    let font = FontId::new(size, semi());
    let wrap = vr.width() * 0.86;
    let outline = (size / 14.0).max(1.5);
    let draw = |text: &str, y: f32, from_bottom: bool| -> f32 {
        let g = ui.fonts_mut(|f| f.layout(text.to_string(), font.clone(), Color32::WHITE, wrap));
        let h = g.size().y;
        let top = if from_bottom { y - h } else { y };
        let pos = Pos2::new(vr.center().x - g.size().x / 2.0, top);
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-0.7, -0.7), (0.7, -0.7), (-0.7, 0.7), (0.7, 0.7)] {
            p.galley_with_override_text_color(pos + Vec2::new(dx, dy) * outline, g.clone(), Color32::BLACK);
        }
        p.galley(pos, g, Color32::WHITE);
        h
    };
    // каждую строку отдельно — так все строки по центру
    let mut y = vr.bottom() - vr.height() * 0.06;
    for c in cues.iter().filter(|c| !c.top).rev() {
        for line in c.text.lines().rev() { y -= draw(line, y, true); }
        y -= size * 0.2;
    }
    let mut y = vr.top() + vr.height() * 0.05;
    for c in cues.iter().filter(|c| c.top) {
        for line in c.text.lines() { y += draw(line, y, false); }
        y += size * 0.2;
    }
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

/// Есть ли в тексте японские/китайские/корейские символы (Inter их не рисует — были бы квадратики).
fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F))
}

/// Системный шрифт с иероглифами: в exe его не вшиваем (+10 МБ), а берём из Windows,
/// и только когда такие символы реально встретились. Корейский — отдельным шрифтом, если есть.
fn load_cjk_fonts() -> Vec<(String, Vec<u8>)> {
    let dirs: Vec<PathBuf> = if cfg!(windows) {
        vec![PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts")]
    } else {
        vec!["/usr/share/fonts/opentype/noto".into(), "/usr/share/fonts/noto-cjk".into(), "/usr/share/fonts/truetype/wqy".into(), "/usr/share/fonts/truetype/droid".into(), "/System/Library/Fonts".into()]
    };
    let ja = ["YuGothM.ttc", "YuGothR.ttc", "meiryo.ttc", "msgothic.ttc", "msyh.ttc", "simsun.ttc", "NotoSansCJK-Regular.ttc", "wqy-zenhei.ttc", "DroidSansFallbackFull.ttf", "Hiragino Sans GB.ttc"];
    let ko = ["malgun.ttf", "gulim.ttc"];
    let find = |names: &[&str]| dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find_map(|p| std::fs::read(&p).ok().map(|b| (p, b)));
    let mut out = Vec::new();
    if let Some((p, b)) = find(&ja) { wavy_core::log(&format!("шрифт для иероглифов: {}", p.display())); out.push(("cjk".to_string(), b)); }
    if let Some((_, b)) = find(&ko) { out.push(("korean".to_string(), b)); }
    out
}

fn font_defs(extra: &[(String, Vec<u8>)]) -> egui::FontDefinitions {
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
    // иероглифы — последними: берутся только для символов, которых нет в Inter
    for (name, bytes) in extra {
        fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes.clone())));
        for fam in fonts.families.values_mut() { fam.push(name.clone()); }
    }
    fonts
}

fn style(ctx: &egui::Context) {
    ctx.set_fonts(font_defs(&[]));
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

/// Своя обложка плейлиста: квадрат по центру, до 600 px, JPEG в папке обложек
/// (новое имя каждый раз — чтобы не показалась старая картинка из кэша миниатюр).
fn playlist_cover(src: &Path, covers: &Path, id: &str) -> Result<PathBuf, String> {
    let img = image::open(src).map_err(|e| format!("не открылась картинка: {e}"))?;
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let img = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let img = if side > 600 { img.resize_exact(600, 600, image::imageops::FilterType::Triangle) } else { img };
    std::fs::create_dir_all(covers).map_err(|e| e.to_string())?;
    let path = covers.join(format!("{id}_{}.jpg", wavy_core::now_ms()));
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&img.to_rgb8()).map_err(|e| e.to_string())?;
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(path)
}

fn open_folder(dir: &Path) {
    #[cfg(windows)] { let _ = std::process::Command::new("explorer").arg(dir).spawn(); }
    #[cfg(target_os = "macos")] { let _ = std::process::Command::new("open").arg(dir).spawn(); }
    #[cfg(all(unix, not(target_os = "macos")))] { let _ = std::process::Command::new("xdg-open").arg(dir).spawn(); }
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

/// Миллисекунды Unix → «21.11.2026».
fn fmt_date(ms: u64) -> String {
    // дни с 1970-01-01 → дата (алгоритм Говарда Хиннанта)
    let z = (ms / 86_400_000) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{d:02}.{m:02}.{y}")
}
