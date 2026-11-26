// Wavy — native version (egui + rodio). No browser engine: minimal memory.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
extern crate wavy_core;

mod media;
mod update;
mod video;
mod player;
mod thumbs;
mod export;
mod tray;
mod instance;
mod hotkeys;
mod recap;
mod fx;
mod theme;
use theme::p as th;

use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wavy_core::{anime, discord::{Discord, DiscordStatus}, fmt_time, local, media::{MediaInfo, SubSrc}, overlay::OverlayServer, shazam::Found, shikimori::ShikiAnime, store::Store, youtube::YouTube, PlayerState, Track};

// ---------- palette ----------
// colors come from the current theme: th().bg, th().text, … (see theme.rs)
const BAD: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);
const OK: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80);

const AUDIO_FILTER: &[&str] = &["mp3", "flac", "wav", "ogg", "oga", "opus", "m4a", "aac", "alac", "aiff", "mp4", "m4v", "mov", "mkv", "m3u", "m3u8"];
/// Playlist where Shazam puts recognized openings.
/// Playlist for songs from Shazam in anime (name in the interface language; looked up by both).
const OPENINGS: [&str; 2] = ["Опенинги", "Openings"];
fn openings() -> &'static str { l!("Опенинги", "Openings") }

#[derive(Clone, Copy, PartialEq)]
enum Sleep { At(Instant), EndOfTrack }

#[derive(Clone, PartialEq)]
enum View { Library, Liked, Stats, Playlist(String), Search, Anime, AnimeTitle(String), Smart(String) }

#[derive(Clone)]
enum ShazamState { Listening, Found(Found), NotFound, Error(String), Adding }

#[derive(Clone)]
enum UpdState { Checking, Downloading(u32), Ready(u32), UpToDate, Error(String) }

enum Bg {
    Status(String),
    OpenPlaylist(String),
    Search(Result<Vec<Track>, String>),
    Ready { token: u64, path: PathBuf },
    /// the track's video is gone from YouTube or became 18+ — found the same track in another video (bool: 18+)
    Replaced(Track, bool),
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
    /// Shikimori sign-in/sync: updated account (tokens) or an error
    Shiki(Result<wavy_core::shikimori::Account, String>, bool),
    SongRequest(String, Result<Track, String>),
    /// Twitch: sign-in code / sign-in finished / token refreshed
    TwitchDevice(Result<wavy_core::twitch::Device, String>),
    TwitchLogin(Result<wavy_core::twitch::Account, String>),
    TwitchToken(wavy_core::twitch::Account),
    /// Spotify import: what wasn't found on YouTube (playlist, tracks)
    NotFound(Option<String>, Vec<wavy_core::spotify::SpTrack>),
    /// search results for the manual video choice
    Picked(Result<Vec<Track>, String>),
    /// Google sign-in: path to the saved cookies
    GoogleCookies(Result<String, String>),
    /// YouTube Music account connected (or not)
    Account(Result<(), String>),
    /// "My wave": tracks, whether it's a top-up
    Wave(Result<Vec<Track>, String>, bool),
    /// Last.fm: sign-in page (token, link) / sign-in finished
    LastfmBegin(Result<(String, String), String>),
    LastfmDone(Result<wavy_core::lastfm::Account, String>),
    CacheSize(u64, usize),
    /// openings and endings: (title, tracks found)
    Themes(String, Result<Vec<Track>, String>),
    /// cookies for 18+ are set, but yt-dlp couldn't use them (browser open, not signed in…)
    CookiesFailed(String),
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
    /// changes between builds: (from, to, downloaded list)
    WhatsNew { from: u32, to: u32, items: Option<Result<Vec<(String, String)>, String>> },
    EditTrack { id: String, title: String, artist: String, album: String },
    Dupes { groups: Vec<Vec<String>> },
    Folder { id: String, name: String },
    /// Spotify tracks not found on YouTube: (playlist to add to; tracks)
    NotFound { pid: Option<String>, items: Vec<wavy_core::spotify::SpTrack> },
    /// manual video choice for a track: search YouTube Music and YouTube
    Pick { query: String, results: Vec<Track>, busy: bool, err: Option<String>, target: PickTarget, back: Vec<wavy_core::spotify::SpTrack> },
}

/// YouTube Music account.
#[derive(Clone, PartialEq)]
enum AccState { Off, Connecting, On, Err(String) }

/// What we're choosing a video for.
#[derive(Clone)]
enum PickTarget {
    /// a library track has the wrong video — replace it (id and playlists stay)
    Replace(String),
    /// a Spotify track wasn't found — add it to the library (and the playlist)
    Add { pid: Option<String>, sp: wavy_core::spotify::SpTrack },
}

enum Act {
    Play(Vec<String>, usize, bool),
    TogglePlay, Next, Prev, Seek(f64), Volume(f32), SaveVolume, Shuffle, Repeat,
    /// addons: enable/disable, restart, rescan the folder
    AddonToggle(String, bool), AddonRestart(String), AddonsRescan,
    View(View), ImportFiles, ImportFolder, ImportPaths(Vec<PathBuf>, Option<String>), ImportUrl(String, Option<String>),
    Search(String), AddTracks(Vec<Track>), PlayNext(Vec<String>), Enqueue(Vec<String>),
    AddTo(String, Vec<String>), RemoveFrom(String, Vec<String>), RemoveTracks(Vec<String>),
    CreatePlaylist(String, Vec<String>), Rename(String, String), DeletePlaylist(String), Sync(String),
    Reveal(String), OpenUrl(String), Settings(serde_json::Value), ToggleOverlay, ToggleLock, JumpQueue(usize),
    Modal(Modal), YtUpdate, SyncExport, SyncImport(Option<PathBuf>), ToggleVideo, Fullscreen(bool), CheckUpdate, Restart,
    ImportAnime(Vec<PathBuf>), ImportAnimeFiles, ImportAnimeFolder, PlayAnime(String, Option<String>), AnimeSearch(String), AnimeApply(String, ShikiAnime),
    DeleteAnime(String), MarkWatched(String, bool), AudioTrack(usize), Subs(Option<usize>), Shazam, ShazamAdd(Found, bool),
    /// tracks → MP3 into a folder; Some(name) — a whole playlist (its own subfolder)
    Export(Vec<String>, Option<String>),
    /// a title's openings and endings → playlist
    AnimeThemes(String),
    /// manual video choice: search / pick (video, what for, the remaining not-found ones)
    PickSearch(String), PickChoose(Track, PickTarget, Vec<wavy_core::spotify::SpTrack>),
    /// YouTube Music account: sign in with Google / cancel / connect with cookies / sign out
    GoogleLogin, GoogleLoginCancel, AccountConnect, AccountLogout,
    /// import from the account: likes / saved playlists
    YtImportLikes, YtImportPlaylists,
    /// "My wave" (true — top up the queue, false — start over)
    Wave(bool),
    /// subtitles from any file (name doesn't match the episode)
    PickSubs,
    /// save the current video frame as PNG
    Screenshot,
    /// (key: "library" or playlist id, method)
    Sort(String, String),
    PlaylistCover(String), PlaylistCoverReset(String),
    Like(Vec<String>, bool),
    /// drag and drop within the current list (playlist / "Liked"): tracks, target (index)
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
    /// What was last sent to the overlay and when: (position, playing or not, when) — the overlay advances time itself; we check it.
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
    resume: Option<(String, f64, bool)>, // (track id, position, autoplay) — continue from the same place
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
    /// window position and size before full screen — to restore them afterwards
    pre_fullscreen: Option<(Pos2, Vec2, bool)>,
    restore_window: Option<(Pos2, Vec2, bool, u8)>,
    _hotkeys: Option<GlobalHotKeyManager>,
    hk_rx: Receiver<u32>,
    hk_keys: hotkeys::Keys,
    /// recording a new combination in settings: for which action
    hk_capture: Option<String>,
    /// Tracks of the current video and what's selected (audio — index among audio tracks, subtitles — index in the list).
    media_info: Option<(String, MediaInfo)>,
    audio_sel: usize,
    sub_sel: Option<usize>,
    /// episode already marked watched (so we don't write to disk every frame)
    watched_mark: Option<String>,
    /// ffmpeg for HEVC/10-bit/VP9/AV1 and Opus/AC3/DTS audio (downloaded on demand)
    ff: wavy_core::ffmpeg::Ffmpeg,
    /// audio didn't open without ffmpeg — reopen the track once it's downloaded
    reload_after_ff: bool,
    exporting: bool,
    /// (track, how many times it has been recovered after a drop)
    cut_retries: Option<(String, u8)>,
    /// track whose play has already been counted (after 30 seconds)
    counted: Option<String>,
    /// dragged tracks and where to drop them; the row under the mouse (for the heart)
    drag_ids: Option<Vec<String>>,
    drop_at: Option<usize>,
    hover_row: Option<String>,
    drag_queue: Option<usize>,
    muted_vol: f32,
    focus_filter: bool,
    quitting: bool,
    sleep: Option<Sleep>,
    /// radio tracks (not in the library, only in the queue)
    radio: Vec<Track>,
    show_lyrics: bool,
    lyrics: Option<(String, Option<Result<wavy_core::lyrics::Lyrics, String>>)>,
    lyrics_line: Option<usize>,
    stats_period: u8,
    skips: Option<(String, Vec<wavy_core::aniskip::Skip>)>,
    auto_skipped: HashSet<String>,
    /// "next episode in…": (episode, when to switch); the episode where the user chose to stay
    next_ep: Option<(String, Instant)>,
    stay_ep: Option<String>,
    shiki_code: String,
    /// Twitch chat: (channel, stop flag), requests, who requested when, ids of requested tracks in the queue
    /// Twitch chat: (channel|sign-in, stop, sending to chat)
    twitch: Option<(String, Arc<std::sync::atomic::AtomicBool>, wavy_core::twitch::Chat)>,
    sr_tx: Sender<wavy_core::twitch::Msg>,
    sr_rx: Receiver<wavy_core::twitch::Msg>,
    skip_vote: wavy_core::twitch::SkipVote,
    /// phone remote: state for it and commands from it
    remote: (Arc<Mutex<serde_json::Value>>, Arc<Mutex<Option<String>>>, Sender<(String, f64)>, Receiver<(String, f64)>),
    /// waiting for the first channel points reward to remember it as the "song request" one
    reward_bind: bool,
    /// which title openings are being looked up for right now
    themes_busy: Option<String>,
    /// command palette (Ctrl+K): query and selected row
    palette: Option<(String, usize)>,
    /// open the "Sound" window (from the command palette)
    sound_open: bool,
    /// YouTube Music account: connected or not (and the error if not)
    acc: AccState,
    /// Google sign-in in progress: cancel flag
    google_login: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// "My wave" is playing (the queue tops itself up) / being picked
    wave_on: bool,
    wave_busy: bool,
    /// Last.fm: who has already been sent "now playing", what's already scrobbled, when playback started (s), sign-in token
    lfm_np: Option<String>,
    lfm_done: Option<String>,
    lfm_started: u64,
    lfm_token: Option<String>,
    /// Twitch sign-in: code for twitch.tv/activate
    twitch_device: Option<wavy_core::twitch::Device>,
    sr_last: std::collections::HashMap<String, Instant>,
    /// chat requests: track id → who requested it
    sr_by: HashMap<String, String>,
    cache_size: Option<(u64, usize)>,
    cache_trimmed: Instant,
    cache_counting: bool,
    wifi: Option<(String, wavy_core::wifisync::Server)>,
    /// track for which the crossfade has already started
    xfaded: Option<String>,
    tray: Option<(tray::Tray, Receiver<tray::Cmd>)>,
    /// commands from later Wavy launches (show the window / quit after an update)
    instance: Option<Receiver<instance::Cmd>>,
    /// the CJK font is already loaded (or we looked and didn't find one)
    cjk_loaded: bool,
    cjk_check: Instant,
    /// addons: commands from the local API, found addons, running processes, API token, last state push
    api_rx: Receiver<(String, serde_json::Value)>,
    addon_list: Vec<wavy_core::addons::Addon>,
    addon_run: wavy_core::addons::Runner,
    api_token: String,
    api_pushed: Instant,
    /// output devices for the Sound window (when they were listed, names)
    devices: Option<(Instant, Vec<String>)>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let ctx = cc.egui_ctx.clone();
        let dir = wavy_core::data_dir();
        let store = Store::open(&dir);
        theme::set(&store.lib.settings.theme, &store.lib.settings.accent);
        style(&ctx);
        let yt_cookies = store.lib.settings.yt_cookies.clone();
        let (tx, rx) = channel();
        let (sr_tx, sr_rx) = channel();
        let (t2, c2) = (tx.clone(), ctx.clone());
        let discord = Discord::start(move |s| { let _ = t2.send(Bg::Discord(s.clone())); c2.request_repaint(); });
        let overlay = OverlayServer::new();
        let s = store.lib.settings.clone();
        wavy_core::i18n::set(&s.lang);
        discord.configure(s.discord.enabled, &s.discord.client_id, s.discord.show_buttons);
        overlay.set_config(&s.overlay.theme, s.overlay.auto_hide_sec, s.overlay.show_queue, s.overlay.show_lyrics);
        let mut toast = None;
        let api_token = wavy_core::addons::token(&wavy_core::addons::dir());
        let (api_tx, api_rx) = channel();
        let c3 = ctx.clone();
        overlay.set_api(api_token.clone(), api_tx, Arc::new(move || c3.request_repaint()));
        if let Err(e) = overlay.start(s.overlay.port) { wavy_core::log(&format!("оверлей: {e}")); toast = Some((format!("Оверлей: {e}"), true, Instant::now())); }

        // global hotkeys (work in games too): pause, tracks, volume, overlay… — configurable
        let (hk_tx, hk_rx) = channel();
        let hotkeys = GlobalHotKeyManager::new().ok();
        let mut hk_keys = hotkeys::Keys::default();
        if let Some(m) = &hotkeys { hk_keys.apply(m, &s.hotkeys); }
        let c3 = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed { let _ = hk_tx.send(e.id); c3.request_repaint(); }
        }));

        let cache = dir.join("cache");
        std::thread::spawn({ let c = cache.clone(); move || trim_cache(&c, 1_500_000_000, 1_000_000_000) });

        let (media_tx, media_rx) = channel();
        let media = media::Media::new(cc, media_tx);
        let ff = wavy_core::ffmpeg::Ffmpeg::new(dir.join("bin"));
        let mut audio = player::Audio::new(&store.lib.settings.audio_device);
        audio.ffmpeg = ff.path();
        if let Some(e) = &audio.error { wavy_core::log(e); toast = Some((e.clone(), true, Instant::now())); }
        let mut app = Self {
            overlay_open: false,
            overlay_init: s.overlay_window.bounds.unwrap_or([24.0, 24.0, 440.0, 124.0]),
            store: Arc::new(Mutex::new(store)),
            yt: Arc::new({ let y = YouTube::new(dir.join("bin")); y.set_cookies(&yt_cookies); y }),
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
            _hotkeys: hotkeys, hk_rx, hk_keys, hk_capture: None,
            media_info: None, audio_sel: 0, sub_sel: None, watched_mark: None,
            ff, reload_after_ff: false, exporting: false, cut_retries: None, counted: None, drag_ids: None, drop_at: None, hover_row: None, drag_queue: None, muted_vol: 0.0, focus_filter: false, quitting: false, sleep: None, radio: Vec::new(), show_lyrics: false, lyrics: None, lyrics_line: None, stats_period: 0, skips: None, auto_skipped: HashSet::new(), next_ep: None, stay_ep: None, shiki_code: String::new(), twitch: None, sr_tx, sr_rx, skip_vote: Default::default(), remote: { let (t, r) = channel(); (Default::default(), Default::default(), t, r) }, reward_bind: false, themes_busy: None, palette: None, sound_open: false, acc: AccState::Off, google_login: None, wave_on: false, wave_busy: false, lfm_np: None, lfm_done: None, lfm_started: 0, lfm_token: None, twitch_device: None, sr_last: Default::default(), sr_by: HashMap::new(), cache_size: None, cache_trimmed: Instant::now() - Duration::from_secs(3600), cache_counting: false, wifi: None, xfaded: None, tray: None, instance: None, cjk_loaded: false, cjk_check: Instant::now() - Duration::from_secs(60), api_rx, addon_list: Vec::new(), addon_run: Default::default(), api_token, api_pushed: Instant::now(), devices: None,
        };
        app.fix_episode_numbers();
        if s.yt_account && !s.yt_cookies.is_empty() { app.connect_account(&cc.egui_ctx); }
        app.overlay_open = s.overlay_window.visible;
        app.tray = tray::new(cc);
        // after an update — show what changed
        if let Some(cur) = update::current_build() {
            match s.last_build { Some(m) if m < cur => app.apply(Act::WhatsNew(m, cur), &ctx), _ => {} }
            if s.last_build != Some(cur) { app.store.lock().unwrap().patch_settings(&json!({"lastBuild": cur})); }
        }
        app.restore_session(&ctx);
        app.start_addons();
        // "Open with Wavy"
        let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).filter(|p| p.exists()).collect();
        if !args.is_empty() { app.apply(Act::ImportPaths(args, None), &ctx); }
        app
    }

    /// Episodes whose number wasn't recognized before (e.g. "Title_[01]_[AniLibria_TV]") — parse them again.
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

    /// ffmpeg is needed: download it (once) and repaint when ready.
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
        // a local file is opened right away, paused; YouTube on pressing play
        if self.current.as_ref().is_some_and(|t| t.is_local()) { self.load_current(ctx); }
    }

    fn settings(&self) -> wavy_core::store::Settings { self.store.lock().unwrap().lib.settings.clone() }
    fn track(&self, id: &str) -> Option<Track> {
        self.store.lock().unwrap().lib.tracks.get(id).cloned().or_else(|| self.results.iter().chain(self.radio.iter()).find(|t| t.id == id).cloned())
    }
    /// Connect the YouTube Music account with cookies from settings (in the background).
    fn connect_account(&mut self, ctx: &egui::Context) {
        self.acc = AccState::Connecting;
        let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || { let _ = tx.send(Bg::Account(yt.account_connect())); c.request_repaint(); });
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
            View::Smart(rule) => {
                let q = self.filter.trim().to_lowercase();
                return s.smart(rule).into_iter().filter(|id| q.is_empty() || s.lib.tracks.get(id).is_some_and(|t| format!("{} {} {}", t.title, t.artist, t.album).to_lowercase().contains(&q))).collect();
            }
        };
        let (key, pl) = match &self.view { View::Playlist(id) => (id.clone(), true), View::Liked => ("liked".to_string(), true), _ => ("library".to_string(), false) };
        let ids = s.sorted(ids, s.lib.settings.sort.get(&key).map(String::as_str).unwrap_or(""), pl);
        let q = self.filter.trim().to_lowercase();
        if q.is_empty() { return ids; }
        ids.into_iter().filter(|id| s.lib.tracks.get(id).is_some_and(|t| format!("{} {} {}", t.title, t.artist, t.album).to_lowercase().contains(&q))).collect()
    }

    // ---------- playback ----------
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
        // starting a track anew is a new play (while resuming after a drop is the same one)
        if self.resume.as_ref().map(|r| &r.0) != Some(&id) { self.counted = None; self.lfm_np = None; self.lfm_done = None; }
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
                // video deleted / private / unavailable in the country / 18+ (not served without sign-in) — look for the same track in another video
                if let Err(e) = &res {
                    let age = age_restricted(e);
                    // 18+ and settings have cookies — try the same video "signed in"
                    if age && yt.has_cookies() {
                        let _ = tx.send(Bg::Status(lf!("«{}»: видео 18+ — пробую с куками браузера…", "“{}”: 18+ video — trying with browser cookies…", t.title)));
                        c.request_repaint();
                        match fetch_with_cookies(&yt, &t, &cache) {
                            Ok(r) => res = Ok(r),
                            Err(e2) => { wavy_core::log(&format!("«{}»: с куками не вышло: {e2}", t.title)); let _ = tx.send(Bg::CookiesFailed(e2)); }
                        }
                    }
                }
                if let Err(e) = &res {
                    let age = age_restricted(e);
                    if (age || gone_from_youtube(e)) && t.video_id.is_some() {
                        wavy_core::log(&format!("«{}»: видео недоступно ({e}) — ищу замену", t.title));
                        let why = if age { l!("видео 18+, YouTube не даёт его без входа", "18+ video, YouTube won't play it without signing in") } else { l!("больше нет на YouTube", "no longer on YouTube") };
                        let _ = tx.send(Bg::Status(lf!("«{}»: {why} — ищу такой же трек в другом видео…", "“{}”: {why} — looking for the same track in another video…", t.title)));
                        c.request_repaint();
                        // the first replacement may also be 18+ (YouTube looks at the title) — try up to three
                        for n in wavy_core::spotify::find_replacements(&yt, &t).unwrap_or_default().into_iter().take(3) {
                            wavy_core::log(&format!("«{}»: пробую замену {:?}", t.title, n.video_id));
                            match fetch_audio(&yt, &n, &cache) {
                                Ok(r2) => { let _ = tx.send(Bg::Replaced(n, age)); res = Ok(r2); break; }
                                Err(e2) => wavy_core::log(&format!("замена {:?} не играет: {e2}", n.video_id)),
                            }
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
                // preload the whole next track — switching will be instant
                if let Some(n) = next { if let Ok(AudioSrc::Stream(dl)) = fetch_audio(&yt, &n, &cache) { dl.wait_for(u64::MAX, Duration::from_secs(600)); } }
            });
        }
    }

    /// Video file tracks: which dub and which subtitles to enable (for anime — same as the previous episode).
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
                        if pos > 1.0 { self.audio.seek(pos); } // will jump when this chunk finishes downloading
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
            // audio in Opus/AC3/DTS… — download the decoder and reopen the track from the same place
            self.reload_after_ff = true;
            self.loading = false;
            self.toast(l!("Скачиваю декодер для этого формата (≈6 МБ, один раз)…", "Downloading a decoder for this format (≈6 MB, once)…"), false);
            return;
        }
        let err = if age_restricted(&err) { l!("видео 18+, YouTube не даёт его без входа, а другой загрузки этого трека не нашлось", "18+ video, YouTube won't play it without signing in, and no other upload of this track was found").to_string() }
            else { wavy_core::ffmpeg::human(&err).to_string() };
        let title = self.current.as_ref().map(|t| t.title.clone()).unwrap_or_default();
        self.toast(lf!("Не получилось воспроизвести «{title}»: {err}", "Couldn't play “{title}”: {err}"), true);
        self.errors_in_row += 1;
        self.loading = false;
        // without an audio device, skipping through the queue is pointless — stop
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

    /// The track "finished" though it's far from the end: the YouTube download broke (link expired, 403, network).
    /// The player used to just start the next song at this point. Now it restarts the download and
    /// continues from the same place — up to twice per track, then moves on to the next one.
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
        if self.settings().now_playing_file {
            let text = st.track.as_ref().map(|t| if t.artist.is_empty() { t.title.clone() } else { format!("{} — {}", t.artist, t.title) }).unwrap_or_default();
            let _ = std::fs::write(wavy_core::data_dir().join("now_playing.txt"), text);
        }
    }

    /// The overlay (OBS) and the system player advance time themselves from the last update. If the track took long
    /// to load or the position drifted (download catching up, seeking into an undownloaded part), their clocks drift from the audio —
    /// so once a second we check and correct. Discord is left alone: it rate-limits updates.
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

    // ---------- background work ----------
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
            Act::Play(ids, i, sh) => { self.wave_on = false; self.play_list(ids, i, sh, ctx) }
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
            Act::View(v) => { self.view = v; self.selected.clear(); self.filter.clear(); self.show_video = false; } // switching sections collapses the video
            Act::ImportFiles | Act::ImportFolder => {
                let pid = self.cur_playlist();
                let dlg = rfd::FileDialog::new();
                let picked = if matches!(a, Act::ImportFiles) { dlg.set_title(l!("Выбери музыку", "Choose music")).add_filter(l!("Аудио и плейлисты", "Audio and playlists"), AUDIO_FILTER).pick_files() } else { dlg.set_title(l!("Выбери папку с музыкой", "Choose a music folder")).pick_folders() };
                if let Some(p) = picked { self.apply(Act::ImportPaths(p, pid), ctx); }
            }
            Act::ImportPaths(paths, pid) => {
                self.toast(l!("Импорт…", "Importing…"), false);
                let (store, covers) = (self.store.clone(), self.covers.clone());
                self.bg(ctx, move |tx| {
                    let res = local::import_paths(&paths, &covers, |i, n| { if n > 30 && i % 25 == 0 { let _ = tx.send(Bg::Status(lf!("Импорт: {i} / {n}", "Importing: {i} / {n}"))); } });
                    let n = res.tracks.len();
                    let mut s = store.lock().unwrap();
                    let ids: Vec<String> = res.tracks.iter().map(|t| t.id.clone()).collect();
                    s.add_tracks(res.tracks);
                    for (name, ids) in res.playlists { s.create_playlist(&name, ids, Some("m3u".into())); }
                    if let Some(pid) = pid { s.add_to_playlist(&pid, &ids); }
                    let _ = tx.send(Bg::Status(lf!("Импортировано треков: {n}", "Tracks imported: {n}")));
                    Ok(())
                });
            }
            Act::ImportUrl(url, pid) => {
                let (store, yt) = (self.store.clone(), self.yt.clone());
                self.bg(ctx, move |tx| {
                    let (name, source, tracks, note, not_found) = fetch_url(&url, &yt, &store, tx)?;
                    let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
                    let n = ids.len();
                    let mut s = store.lock().unwrap();
                    s.add_tracks(tracks);
                    let target = if let Some(pid) = pid { s.add_to_playlist(&pid, &ids); Some(pid) }
                        else { name.map(|name| s.create_playlist(&name, ids, Some(source)).id) };
                    let _ = tx.send(Bg::Status(lf!("Добавлено {n} трек(ов){note}", "Added {n} track(s){note}")));
                    if let Some(t) = &target { let _ = tx.send(Bg::OpenPlaylist(t.clone())); }
                    if !not_found.is_empty() { let _ = tx.send(Bg::NotFound(target, not_found)); }
                    Ok(())
                });
            }
            Act::Search(q) => {
                self.searching = true;
                let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
                std::thread::spawn(move || { let _ = tx.send(Bg::Search(yt.search(&q, 25))); c.request_repaint(); });
            }
            Act::AddTracks(t) => { self.store.lock().unwrap().add_tracks(t); self.toast(l!("Добавлено в библиотеку", "Added to library"), false); }
            Act::PlayNext(ids) => {
                if self.current.is_none() { return self.play_list(ids, 0, false, ctx); }
                let n = ids.len();
                for (k, id) in ids.into_iter().enumerate() { self.queue.insert((self.qi + 1 + k).min(self.queue.len()), id); }
                self.toast(if n > 1 { lf!("{n} треков сыграют следующими", "{n} tracks will play next") } else { l!("Сыграет следующим", "Will play next").into() }, false);
            }
            Act::Enqueue(ids) => {
                if self.current.is_none() { return self.play_list(ids, 0, false, ctx); }
                self.queue.extend(ids);
                self.toast(l!("Добавлено в очередь", "Added to queue"), false);
            }
            Act::AddTo(pid, ids) => {
                self.ensure_in_library(&ids);
                if let Some(n) = self.store.lock().unwrap().add_to_playlist(&pid, &ids) { self.toast = Some((lf!("Добавлено в «{n}»", "Added to “{n}”"), false, Instant::now())); }
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
                if ids.is_empty() { self.view = View::Playlist(p.id); } else { self.toast(lf!("Плейлист «{name}» создан", "Playlist “{name}” created"), false); }
            }
            Act::Rename(id, name) => self.store.lock().unwrap().rename_playlist(&id, &name),
            Act::DeletePlaylist(id) => { self.store.lock().unwrap().delete_playlist(&id); self.view = View::Library; }
            Act::Sync(id) => {
                let (store, yt) = (self.store.clone(), self.yt.clone());
                self.bg(ctx, move |tx| {
                    let src = store.lock().unwrap().lib.playlists.iter().find(|p| p.id == id).and_then(|p| p.source.clone()).filter(|s| s.starts_with("http")).ok_or(l!("У плейлиста нет ссылки", "The playlist has no link"))?;
                    let (_, _, tracks, note, not_found) = fetch_url(&src, &yt, &store, tx)?;
                    let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
                    let n = ids.len();
                    let mut s = store.lock().unwrap();
                    s.add_tracks(tracks);
                    s.set_playlist_tracks(&id, ids);
                    let _ = tx.send(Bg::Status(lf!("Синхронизировано: {n} треков{note}", "Synced: {n} tracks{note}")));
                    if !not_found.is_empty() { let _ = tx.send(Bg::NotFound(Some(id), not_found)); }
                    Ok(())
                });
            }
            Act::Export(ids, name) => {
                if self.exporting { self.toast(l!("Экспорт уже идёт — дождись, пока закончится", "An export is already running — wait until it finishes"), true); return; }
                let tracks: Vec<Track> = ids.iter().filter_map(|i| self.track(i)).collect();
                if tracks.is_empty() { return; }
                let title = match &name { Some(n) => lf!("Куда сохранить плейлист «{n}»? (создам в ней папку)", "Where to save the playlist “{n}”? (I'll create a folder in it)"), None => l!("Куда сохранить MP3", "Where to save the MP3").into() };
                let Some(base) = rfd::FileDialog::new().set_title(title).pick_folder() else { return };
                let dir = match &name { Some(n) => base.join(export::safe_name(n)), None => base };
                self.exporting = true;
                self.toast(lf!("Экспорт MP3: {} трек(ов)…", "MP3 export: {} track(s)…", tracks.len()), false);
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
                let Some(src) = rfd::FileDialog::new().set_title(l!("Картинка для обложки", "Cover image")).add_filter(l!("Картинки", "Images"), &["jpg", "jpeg", "png", "webp"]).pick_file() else { return };
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
                        Err(e) => { let _ = tx.send(Bg::Status(lf!("Не вышло поставить обложку: {e}", "Couldn't set the cover: {e}"))); }
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
                // liking a track not in the library (search, radio) — add it to the library first
                let extra: Vec<Track> = ids.iter().filter(|i| !self.store.lock().unwrap().lib.tracks.contains_key(*i)).filter_map(|i| self.track(i)).collect();
                let mut s = self.store.lock().unwrap();
                if on && !extra.is_empty() { s.add_tracks(extra); }
                s.set_liked(&ids, on);
            }
            Act::EditTrack(id, title, artist, album) => self.store.lock().unwrap().edit_track(&id, &title, &artist, &album, None),
            Act::TrackCover(id) => {
                let Some(src) = rfd::FileDialog::new().set_title(l!("Картинка для обложки", "Cover image")).add_filter(l!("Картинки", "Images"), &["jpg", "jpeg", "png", "webp"]).pick_file() else { return };
                match playlist_cover(&src, &self.covers, &format!("t_{}", wavy_core::sha16(id.as_bytes()))) {
                    Ok(path) => {
                        let Some(t) = self.track(&id) else { return };
                        self.store.lock().unwrap().edit_track(&id, &t.title, &t.artist, &t.album, Some(Some(path.to_string_lossy().into_owned())));
                        if self.current.as_ref().is_some_and(|c| c.id == id) { self.current = self.track(&id); }
                    }
                    Err(e) => self.toast(lf!("Не вышло поставить обложку: {e}", "Couldn't set the cover: {e}"), true),
                }
            }
            Act::MergeDupes(groups) => { let n = self.store.lock().unwrap().merge_duplicates(&groups); self.toast(lf!("Убрано дублей: {n}", "Duplicates removed: {n}"), false); }
            Act::SetFolder(id, f) => self.store.lock().unwrap().set_folder(&id, f),
            Act::Radio(id) => {
                let Some(t) = self.track(&id) else { return };
                let Some(vid) = t.video_id.clone() else { self.toast(l!("Радио работает для треков с YouTube", "Radio works for YouTube tracks"), true); return };
                self.toast(lf!("Собираю радио по «{}»…", "Building a radio from “{}”…", t.title), false);
                let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
                std::thread::spawn(move || { let _ = tx.send(Bg::Radio(id, yt.radio(&vid, 30))); c.request_repaint(); });
            }
            Act::Move(ids, to) => {
                let mut s = self.store.lock().unwrap();
                match &self.view { View::Playlist(p) => s.move_in_playlist(p, &ids, to), View::Liked => s.move_liked(&ids, to), _ => {} }
            }
            Act::MoveQueue(from, to) => {
                if from < self.queue.len() && from != to {
                    // shift position indices so the playing track stays the same (the queue can have repeats)
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
            Act::AnimeThemes(id) => {
                let Some(a) = self.store.lock().unwrap().anime(&id).cloned() else { return };
                let Some(mal) = a.shiki_id else { return };
                self.themes_busy = Some(id.clone());
                let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let r = wavy_core::animethemes::fetch(mal).and_then(|themes| {
                        if themes.is_empty() { return Err(l!("у этого тайтла нет опенингов в базе", "this title has no openings in the database").into()); }
                        // search in parallel, 4 at a time — 10 songs are found in a couple of seconds
                        let mut found: Vec<Option<Track>> = vec![None; themes.len()];
                        for (chunk_i, chunk) in themes.chunks(4).enumerate() {
                            let hs: Vec<_> = chunk.iter().map(|t| { let (yt, t) = (yt.clone(), t.clone()); std::thread::spawn(move || {
                                let mut tr = yt.lookup(&t.query()).ok()?;
                                tr.title = format!("{} · {}", t.label(), t.title);
                                if !t.artist.is_empty() { tr.artist = t.artist.clone(); }
                                Some(tr)
                            }) }).collect();
                            for (i, h) in hs.into_iter().enumerate() { found[chunk_i * 4 + i] = h.join().ok().flatten(); }
                        }
                        let list: Vec<Track> = found.into_iter().flatten().collect();
                        if list.is_empty() { Err(l!("опенинги нашлись, но не на YouTube", "openings were found, but not on YouTube").into()) } else { Ok(list) }
                    });
                    let _ = tx.send(Bg::Themes(id, r));
                    c.request_repaint();
                });
            }
            Act::ToggleMini => { let on = !self.settings().mini; self.store.lock().unwrap().patch_settings(&json!({"mini": on})); }
            Act::ShowWindow => tray::show(ctx),
            Act::Quit => { self.quitting = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            Act::Reveal(p) => reveal(&p),
            Act::AddonToggle(id, on) => {
                let mut list = self.settings().addons;
                list.retain(|x| x != &id);
                if on { list.push(id.clone()); }
                self.store.lock().unwrap().patch_settings(&json!({"addons": list}));
                if on { self.run_addon(&id); } else { self.addon_run.stop(&id); self.addon_run.exited.remove(&id); }
            }
            Act::AddonRestart(id) => self.run_addon(&id),
            Act::AddonsRescan => { self.addon_list = wavy_core::addons::scan(&wavy_core::addons::dir()); self.start_addons(); }
            Act::OpenUrl(u) => open_url(&u),
            Act::Settings(patch) => {
                self.store.lock().unwrap().patch_settings(&patch);
                let s = self.settings();
                wavy_core::i18n::set(&s.lang);
                if theme::set(&s.theme, &s.accent) { apply_theme(ctx); }
                // another output device: reopen the current track there, at the same place
                if patch.get("audioDevice").is_some() {
                    let (pos, playing) = (self.audio.position(), self.audio.is_playing());
                    self.audio.set_device(&s.audio_device);
                    if let Some(t) = self.current.clone() { self.resume = Some((t.id, pos, playing)); self.load_current(ctx); }
                }
                if let Some((t, _)) = &self.tray { t.relabel(); }
                // the cookie source changed — the account reconnects with the new ones
                if patch.get("ytCookies").is_some() && patch.get("ytAccount").is_none() && s.yt_account { if s.yt_cookies.is_empty() { self.acc = AccState::Off; } else { self.connect_account(ctx); } }
                self.discord.configure(s.discord.enabled, &s.discord.client_id, s.discord.show_buttons);
                self.overlay.set_config(&s.overlay.theme, s.overlay.auto_hide_sec, s.overlay.show_queue, s.overlay.show_lyrics);
                self.yt.set_cookies(&s.yt_cookies);
                if let Some(m) = &self._hotkeys { self.hk_keys.apply(m, &s.hotkeys); }
                if let Err(e) = self.overlay.start(s.overlay.port) { self.toast(lf!("Оверлей: {e}", "Overlay: {e}"), true); }
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
                if matches!(m, Modal::Settings { .. }) { self.cache_size = None; self.cache_counting = false; }
                if matches!(m, Modal::Settings { .. }) && self.yt_version.is_none() {
                    let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
                    std::thread::spawn(move || { let _ = tx.send(Bg::YtVersion(yt.version())); c.request_repaint(); });
                }
                self.modal = Some(m);
            }
            Act::SyncExport => {
                let text = wavy_core::sync::export(&self.store.lock().unwrap());
                if let Some(path) = rfd::FileDialog::new().set_title(l!("Экспорт плейлистов для телефона", "Export playlists for the phone")).set_file_name("wavy-export.json").add_filter("Wavy", &["json"]).save_file() {
                    match std::fs::write(&path, text) {
                        Ok(()) => self.toast(l!("Готово: открой этот файл в Wavy на телефоне (Настройки → Синхронизация → Импорт)", "Done: open this file in Wavy on your phone (Settings → Sync → Import)"), false),
                        Err(e) => self.toast(lf!("Не удалось сохранить: {e}", "Couldn't save: {e}"), true),
                    }
                }
            }
            Act::SyncImport(path) => {
                let path = path.or_else(|| rfd::FileDialog::new().set_title(l!("Импорт плейлистов с телефона", "Import playlists from the phone")).add_filter("Wavy", &["json"]).pick_file());
                if let Some(path) = path {
                    let res = std::fs::read_to_string(&path).map_err(|e| e.to_string())
                        .and_then(|t| wavy_core::sync::import(&mut self.store.lock().unwrap(), &t));
                    match res {
                        Ok(r) => self.toast(lf!("Импортировано: {} треков, новых плейлистов {}, обновлено {}{}", "Imported: {} tracks, new playlists {}, updated {}{}", r.tracks, r.new_playlists, r.updated_playlists,
                            if r.skipped_local > 0 { lf!(" (файлы с телефона не переносятся: {})", " (files from the phone aren't transferred: {})", r.skipped_local) } else { String::new() }), false),
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
                    // remember the window: after leaving full screen the system applies the default size
                    self.pre_fullscreen = ctx.input(|i| {
                        let v = i.viewport();
                        Some((v.outer_rect?.min, v.inner_rect?.size(), v.maximized.unwrap_or(false)))
                    });
                    self.show_video = true;
                } else if let Some((pos, size, max)) = self.pre_fullscreen.take() {
                    self.restore_window = Some((pos, size, max, 3)); // apply it for several frames in a row until the window "settles"
                }
                self.fullscreen = on;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
            }
            Act::CheckUpdate => { self.upd_checked = Some(Instant::now()); self.start_update(ctx); }
            Act::Restart => {
                // otherwise "close to tray" would hide the old window and it would keep playing next to the new one
                self.quitting = true;
                self.audio.stop();
                self.save_session();
                update::restart();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Act::ImportAnimeFiles | Act::ImportAnimeFolder => {
                let dlg = rfd::FileDialog::new();
                let picked = if matches!(a, Act::ImportAnimeFiles) { dlg.set_title(l!("Выбери серии", "Choose episodes")).add_filter(l!("Видео", "Video"), anime::EPISODE_EXT).pick_files() } else { dlg.set_title(l!("Выбери папку с аниме", "Choose an anime folder")).pick_folders() };
                if let Some(p) = picked { self.apply(Act::ImportAnime(p), ctx); }
            }
            Act::ImportAnime(paths) => {
                self.toast(l!("Ищу серии…", "Looking for episodes…"), false);
                let (store, covers) = (self.store.clone(), self.covers.clone());
                self.bg(ctx, move |tx| {
                    let groups = anime::scan(&paths, |i, n| { if n > 30 && i % 25 == 0 { let _ = tx.send(Bg::Status(lf!("Серии: {i} / {n}", "Episodes: {i} / {n}"))); } });
                    if groups.is_empty() { return Err(l!("Не нашлось видео (mkv, mp4, mov)", "No videos found (mkv, mp4, mov)").into()); }
                    let eps: usize = groups.iter().map(|g| g.tracks.len()).sum();
                    let mut ids = Vec::new();
                    for g in groups {
                        let n = g.tracks.len();
                        let (id, _) = store.lock().unwrap().add_episodes(&g.key, &g.parsed, g.tracks);
                        ids.push(id.clone());
                        let need = store.lock().unwrap().anime(&id).is_some_and(|a| !a.matched);
                        if !need { continue; }
                        let _ = tx.send(Bg::Status(lf!("Ищу «{}» на Шикимори…", "Looking up “{}” on Shikimori…", g.parsed.title)));
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
                    let _ = tx.send(Bg::Status(lf!("Добавлено серий: {eps}", "Episodes added: {eps}")));
                    let _ = tx.send(Bg::OpenAnime(if ids.len() == 1 { ids.pop() } else { None }));
                    Ok(())
                });
            }
            Act::PlayAnime(id, ep) => {
                let Some(a) = self.store.lock().unwrap().anime(&id).cloned() else { return };
                if a.episodes.is_empty() { return; }
                // continue: the last episode (if finished, the next one), from the same place
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
            Act::GoogleLogin => {
                let Some(browser) = wavy_core::ytaccount::find_browser() else {
                    self.toast(l!("Для входа нужен Chrome, Edge или Brave. Можно и без них: выбери Firefox или cookies.txt в «Куки из»", "Signing in needs Chrome, Edge or Brave. Without them, pick Firefox or cookies.txt in “Cookies from”"), true);
                    return;
                };
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.google_login = Some(cancel.clone());
                let (tx, c, dir) = (self.tx.clone(), ctx.clone(), wavy_core::data_dir());
                std::thread::spawn(move || {
                    let r = (|| {
                        let mut w = wavy_core::ytaccount::GoogleLogin::start(&browser, &dir.join("google-login"))?;
                        let t0 = Instant::now();
                        loop {
                            std::thread::sleep(Duration::from_secs(2));
                            if cancel.load(std::sync::atomic::Ordering::Relaxed) { w.close(); return Err(l!("вход отменён", "sign-in cancelled").to_string()); }
                            if w.closed() { return Err(l!("окно закрыли до входа", "the window was closed before signing in").to_string()); }
                            if t0.elapsed() > Duration::from_secs(900) { w.close(); return Err(l!("вход не завершён за 15 минут", "sign-in wasn't finished within 15 minutes").to_string()); }
                            if let Ok(Some(txt)) = w.poll() {
                                let path = dir.join("youtube-cookies.txt");
                                std::fs::write(&path, txt).map_err(|e| e.to_string())?;
                                w.close();
                                return Ok(path.to_string_lossy().to_string());
                            }
                        }
                    })();
                    let _ = tx.send(Bg::GoogleCookies(r));
                    c.request_repaint();
                });
            }
            Act::GoogleLoginCancel => { if let Some(c) = self.google_login.take() { c.store(true, std::sync::atomic::Ordering::Relaxed); } }
            Act::AccountConnect => self.connect_account(ctx),
            Act::AccountLogout => {
                let s = self.settings();
                let ours = wavy_core::data_dir().join("youtube-cookies.txt");
                let mine = s.yt_cookies == ours.to_string_lossy();
                self.wave_on = false;
                self.acc = AccState::Off;
                let yt = self.yt.clone();
                std::thread::spawn(move || {
                    yt.account_disconnect();
                    if mine { let _ = std::fs::remove_file(&ours); let _ = std::fs::remove_dir_all(wavy_core::data_dir().join("google-login")); }
                });
                self.store.lock().unwrap().patch_settings(&if mine { json!({"ytAccount": false, "ytCookies": ""}) } else { json!({"ytAccount": false}) });
                self.yt.set_cookies(&self.settings().yt_cookies);
                self.toast(l!("Вышел из аккаунта YouTube", "Signed out of YouTube"), false);
            }
            Act::YtImportLikes => {
                let (store, yt) = (self.store.clone(), self.yt.clone());
                self.bg(ctx, move |tx| {
                    let _ = tx.send(Bg::Status(l!("Загружаю лайки YouTube Music…", "Loading YouTube Music likes…").into()));
                    let tracks = yt.liked_tracks(5000)?;
                    let n = tracks.len();
                    let pid = save_ytm_playlist(&store, &format!("ytm:{}", wavy_core::ytaccount::LIKED_ID), l!("Лайки YouTube Music", "YouTube Music likes"), tracks);
                    let _ = tx.send(Bg::Status(lf!("Лайки YouTube Music: {n} треков", "YouTube Music likes: {n} tracks")));
                    let _ = tx.send(Bg::OpenPlaylist(pid));
                    Ok(())
                });
            }
            Act::YtImportPlaylists => {
                let (store, yt) = (self.store.clone(), self.yt.clone());
                self.bg(ctx, move |tx| {
                    let _ = tx.send(Bg::Status(l!("Загружаю плейлисты YouTube Music…", "Loading YouTube Music playlists…").into()));
                    let list = yt.saved_playlists()?;
                    let list: Vec<_> = list.into_iter().filter(|p| p.0 != wavy_core::ytaccount::LIKED_ID).collect();
                    let total = list.len();
                    let mut done = 0;
                    for (i, (id, name, _)) in list.into_iter().enumerate() {
                        let _ = tx.send(Bg::Status(lf!("Плейлисты YouTube Music: {} / {total} — {name}", "YouTube Music playlists: {} / {total} — {name}", i + 1)));
                        match yt.playlist_tracks(&id, 5000) {
                            Ok(tracks) => { save_ytm_playlist(&store, &format!("ytm:{id}"), &name, tracks); done += 1; }
                            Err(e) => wavy_core::log(&format!("плейлист YouTube Music «{name}»: {e}")),
                        }
                    }
                    let _ = tx.send(Bg::Status(lf!("Плейлисты YouTube Music: {done} из {total}", "YouTube Music playlists: {done} of {total}")));
                    Ok(())
                });
            }
            Act::Wave(more) => {
                if self.wave_busy { return; }
                if self.acc != AccState::On { self.toast(l!("Сначала войди в YouTube в настройках", "Sign in to YouTube in Settings first"), true); return; }
                self.wave_busy = true;
                if !more { self.toast(l!("Собираю «Мою волну»…", "Building “My wave”…"), false); }
                // seed tracks for the top-up — the last ones played from the wave
                let seeds: Vec<String> = if more {
                    let from = self.qi.saturating_sub(2);
                    self.queue.get(from..=self.qi.min(self.queue.len().saturating_sub(1))).unwrap_or(&[]).iter().filter_map(|id| self.track(id).and_then(|t| t.video_id)).collect()
                } else { Vec::new() };
                let mut exclude: std::collections::HashSet<String> = self.store.lock().unwrap().lib.tracks.values().filter_map(|t| t.video_id.clone()).collect();
                exclude.extend(self.queue.iter().filter_map(|id| self.track(id).and_then(|t| t.video_id)));
                let (yt, tx, c) = (self.yt.clone(), self.tx.clone(), ctx.clone());
                std::thread::spawn(move || { let _ = tx.send(Bg::Wave(yt.wave(&seeds, &exclude, 40), more)); c.request_repaint(); });
            }
            Act::PickSearch(q) => {
                if let Some(Modal::Pick { busy, err, .. }) = &mut self.modal { *busy = true; *err = None; }
                let (tx, c, yt) = (self.tx.clone(), ctx.clone(), self.yt.clone());
                std::thread::spawn(move || { let _ = tx.send(Bg::Picked(wavy_core::spotify::candidates(&yt, &q))); c.request_repaint(); });
            }
            Act::PickChoose(c, target, back) => {
                let mut s = self.store.lock().unwrap();
                match target {
                    PickTarget::Replace(id) => {
                        if let Some(old) = s.lib.tracks.get(&id).cloned() {
                            let ytcover = old.cover.as_deref().is_none_or(|u| u.contains("ytimg.com") || u.contains("googleusercontent"));
                            let t = Track { video_id: c.video_id.clone(), url: c.url.clone(), cover: if ytcover { c.cover.clone() } else { old.cover.clone() },
                                duration: if c.duration > 0.0 { c.duration } else { old.duration }, kind: "youtube".into(), ..old };
                            s.add_tracks(vec![t.clone()]);
                            drop(s);
                            if self.current.as_ref().is_some_and(|x| x.id == t.id) { self.current = Some(t.clone()); }
                            self.toast(lf!("«{}»: видео заменено — со следующего запуска трека", "“{}”: video replaced — takes effect next time the track plays", t.title), false);
                        }
                    }
                    PickTarget::Add { pid, sp } => {
                        let t = wavy_core::spotify::from_candidate(&c, &sp);
                        let title = t.title.clone();
                        let id = t.id.clone();
                        s.add_tracks(vec![t]);
                        if let Some(p) = &pid { s.add_to_playlist(p, &[id]); }
                        drop(s);
                        self.toast(lf!("Добавлено: {title}", "Added: {title}"), false);
                        if !back.is_empty() { self.modal = Some(Modal::NotFound { pid, items: back }); }
                    }
                }
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
                    let mut a = store.lock().unwrap().anime(&id).cloned().ok_or(l!("Тайтл не найден", "Title not found"))?;
                    wavy_core::shikimori::apply(&mut a, &sh, &covers);
                    let mut s = store.lock().unwrap();
                    if let Some(x) = s.anime_mut(&id) { *x = a; }
                    s.retitle_episodes(&id);
                    s.save();
                    let _ = tx.send(Bg::Status(lf!("Теперь это «{name}»", "Now it's “{name}”")));
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
            Act::Screenshot => {
                let Some(v) = self.video.as_ref() else { self.toast(l!("Скриншот — только для видео", "Screenshots are for videos only"), true); return };
                let Some(([w, h], px)) = v.1.frame() else { return };
                let px = px.to_vec();
                let pos = self.audio.position() as u64;
                let title = self.current.as_ref().map(|t| t.title.clone()).unwrap_or_default();
                let anime = self.current.as_ref().and_then(|t| t.anime.clone()).and_then(|a| self.store.lock().unwrap().anime(&a).map(|a| a.display().to_string()));
                let name = format!("{} {} {}m{:02}s.png", export::safe_name(&anime.unwrap_or_default()), export::safe_name(&title), pos / 60, pos % 60).trim().to_string();
                let mut path = wavy_core::pictures_dir().join(&name);
                let mut n = 2;
                while path.exists() { path = wavy_core::pictures_dir().join(name.replace(".png", &format!(" ({n}).png"))); n += 1; }
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let r = image::RgbaImage::from_raw(w as u32, h as u32, px).ok_or(l!("кадр повреждён", "the frame is corrupted").to_string())
                        .and_then(|img| img.save(&path).map_err(|e| e.to_string()));
                    let _ = tx.send(Bg::Status(match r { Ok(()) => lf!("Кадр сохранён: {}", "Frame saved: {}", path.display()), Err(e) => lf!("Скриншот не сохранился: {e}", "Screenshot wasn't saved: {e}") }));
                    c.request_repaint();
                });
            }
            Act::PickSubs => {
                let Some(p) = rfd::FileDialog::new().set_title(l!("Файл субтитров", "Subtitle file")).add_filter(l!("Субтитры", "Subtitles"), wavy_core::anime::SUB_EXT).pick_file() else { return };
                let Some((_, info)) = self.media_info.as_mut() else { return };
                let label = lf!("Файл · {}", "File · {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                let i = match info.subs.iter().position(|s| s.src == wavy_core::media::SubSrc::External(p.clone())) {
                    Some(i) => i,
                    None => { info.subs.push(wavy_core::media::SubTrack { src: wavy_core::media::SubSrc::External(p), label, lang: String::new(), default: false, forced: false }); info.subs.len() - 1 }
                };
                self.apply(Act::Subs(Some(i)), ctx);
            }
            Act::Subs(sel) => {
                self.sub_sel = sel;
                self.video = None; // will reopen with the chosen subtitles at the same position
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
                if !path.exists() { self.toast(l!("Shazam работает с файлами на компьютере (и уже загруженными треками)", "Shazam works with files on your computer (and already downloaded tracks)"), true); return; }
                let (pos, track, ff) = (self.audio.position(), self.audio_sel, self.ff.path());
                self.modal = Some(Modal::Shazam { state: ShazamState::Listening, to_openings: t.anime.is_some() });
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    // take a little before the current spot and forward — people usually press it when the opening has already started
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
                        .ok_or_else(|| lf!("На YouTube не нашлось «{q}»", "“{q}” not found on YouTube"))?;
                    let mut t = best;
                    t.title = f.title.clone();
                    if !f.artist.is_empty() { t.artist = f.artist.clone(); }
                    let id = t.id.clone();
                    let mut s = store.lock().unwrap();
                    s.add_tracks(vec![t]);
                    let mut note = String::new();
                    if to_pl {
                        let pid = match s.lib.playlists.iter().find(|p| OPENINGS.contains(&p.name.as_str())) { Some(p) => p.id.clone(), None => s.create_playlist(openings(), vec![], None).id };
                        s.add_to_playlist(&pid, &[id]);
                        note = lf!(" и в «{}»", " and to “{}”", openings());
                    }
                    let _ = tx.send(Bg::Status(lf!("{} — {}: добавлено в библиотеку{note}", "{} — {}: added to library{note}", f.artist, f.title)));
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

    /// Background thread messages, end of track, hotkeys, drag & drop.
    fn tick(&mut self, ctx: &egui::Context) {
        self.thumbs.poll(ctx);
        let st = self.settings();
        self.audio.fx.set(fx::Settings { speed: st.speed, eq_on: st.eq_on, eq: st.eq, normalize: st.normalize });
        if self.audio.tick() { ctx.request_repaint_after(Duration::from_millis(40)); }
        // sleep timer
        if let Some(Sleep::At(t)) = self.sleep {
            if Instant::now() >= t { self.sleep = None; if self.audio.is_playing() { self.audio.pause(); self.push_state(); } self.toast(l!("Таймер сна: спокойной ночи", "Sleep timer: good night"), false); }
            else { ctx.request_repaint_after(Duration::from_secs(1)); }
        }
        // crossfade: N seconds before the end start the next track while this one fades out
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
                Bg::Lyrics(id, r) => if let Some((cur, slot)) = &mut self.lyrics { if *cur == id {
                    let synced = r.as_ref().ok().filter(|l| !l.synced.is_empty()).map(|l| json!(l.synced));
                    *slot = Some(r);
                    self.lyrics_line = None;
                    self.overlay.set_lyrics(synced.unwrap_or(serde_json::Value::Null));
                } },
                Bg::Radio(id, r) => match r {
                    Ok(list) if !list.is_empty() => {
                        let ids: Vec<String> = list.iter().map(|t| t.id.clone()).collect();
                        self.radio.retain(|t| !ids.contains(&t.id));
                        self.radio.extend(list);
                        let n = ids.len();
                        if self.current.as_ref().is_some_and(|c| c.id == id) {
                            // this track is playing — the radio goes next in the queue instead of the rest
                            self.queue.truncate(self.qi + 1);
                            self.queue.extend(ids);
                        } else {
                            let mut q = vec![id];
                            q.extend(ids);
                            self.play_list(q, 0, false, ctx);
                        }
                        self.toast(lf!("Радио: {n} похожих треков в очереди", "Radio: {n} similar tracks in the queue"), false);
                    }
                    Ok(_) => self.toast(l!("Похожих треков не нашлось", "No similar tracks found"), true),
                    Err(e) => self.toast(lf!("Радио не собралось: {e}", "Radio failed: {e}"), true),
                },
                Bg::Skips(id, v) => if let Some((cur, slot)) = &mut self.skips { if *cur == id { *slot = v; } },
                Bg::Shiki(r, login) => match r {
                    Ok(acc) => {
                        if login { self.toast(lf!("Шикимори: вошёл как {}", "Shikimori: signed in as {}", acc.nickname), false); self.shiki_code.clear(); }
                        self.store.lock().unwrap().patch_settings(&json!({ "shiki": acc }));
                    }
                    Err(e) => self.toast(e, true),
                },
                Bg::SongRequest(user, r) => self.song_request(user, r, ctx),
                Bg::Themes(id, r) => {
                    self.themes_busy = None;
                    match r {
                        Ok(list) => {
                            let name = self.store.lock().unwrap().anime(&id).map(|a| format!("{} — OP/ED", a.display())).unwrap_or_else(|| "OP/ED".into());
                            let ids: Vec<String> = list.iter().map(|t| t.id.clone()).collect();
                            let mut s = self.store.lock().unwrap();
                            s.add_tracks(list);
                            let pid = match s.lib.playlists.iter().find(|p| p.name == name).map(|p| p.id.clone()) {
                                Some(pid) => { s.add_to_playlist(&pid, &ids); pid }
                                None => s.create_playlist(&name, ids.clone(), None).id,
                            };
                            drop(s);
                            self.toast(lf!("«{name}»: {} песен", "“{name}”: {} songs", ids.len()), false);
                            self.apply(Act::View(View::Playlist(pid)), ctx);
                        }
                        Err(e) => self.toast(lf!("Опенинги: {e}", "Openings: {e}"), true),
                    }
                }
                Bg::TwitchDevice(r) => match r {
                    Ok(d) => { open_url(&d.uri); self.twitch_device = Some(d.clone()); self.twitch_poll(d, ctx); }
                    Err(e) => self.toast(e, true),
                },
                Bg::TwitchLogin(r) => {
                    self.twitch_device = None;
                    match r {
                        Ok(a) => { self.toast(lf!("Twitch: вошёл как {}", "Twitch: signed in as {}", a.login), false); self.store.lock().unwrap().patch_settings(&json!({"twitchAccount": a})); }
                        Err(e) => self.toast(e, true),
                    }
                }
                Bg::LastfmBegin(r) => match r {
                    Ok((token, url)) => { open_url(&url); self.lfm_token = Some(token); }
                    Err(e) => self.toast(e, true),
                },
                Bg::LastfmDone(r) => match r {
                    Ok(a) => { self.lfm_token = None; self.toast(lf!("Last.fm: вошёл как {}", "Last.fm: signed in as {}", a.user), false); self.store.lock().unwrap().patch_settings(&json!({"lastfm": a})); }
                    Err(e) => self.toast(e, true),
                },
                Bg::TwitchToken(a) => { self.store.lock().unwrap().patch_settings(&json!({"twitchAccount": a})); }
                Bg::CacheSize(b, n) => self.cache_size = Some((b, n)),
                Bg::Changes(r) => if let Some(Modal::WhatsNew { items, .. }) = &mut self.modal { *items = Some(r); },
                Bg::Exported(r) => {
                    self.exporting = false;
                    match r {
                        Ok(rep) => {
                            let msg = if rep.failed.is_empty() { lf!("Готово: {} MP3 в папке «{}»", "Done: {} MP3 in the folder “{}”", rep.done, rep.dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()) }
                                else { lf!("Сохранено {} MP3, не вышло {}: {} — {}", "Saved {} MP3, failed {}: {} — {}", rep.done, rep.failed.len(), rep.failed[0].0, rep.failed[0].1) };
                            self.toast(msg, !rep.failed.is_empty());
                            if rep.done > 0 { open_folder(&rep.dir); }
                        }
                        Err(e) => self.toast(lf!("Экспорт не вышел: {e}", "Export failed: {e}"), true),
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
                Bg::CookiesFailed(e) => self.toast(lf!("Куки браузера не сработали: {}", "Browser cookies didn't work: {}", cookie_hint(&e)), true),
                Bg::Replaced(t, age) => {
                    // remember the new video in the library (same id — playlists untouched)
                    let mut s = self.store.lock().unwrap();
                    if s.lib.tracks.contains_key(&t.id) { s.add_tracks(vec![t.clone()]); }
                    drop(s);
                    if self.current.as_ref().is_some_and(|c| c.id == t.id) { self.current = Some(t.clone()); }
                    self.video_error = None;
                    self.toast(if age { lf!("«{}»: видео 18+ без входа не играет — нашёл тот же трек в другом видео", "“{}”: 18+ video won't play without signing in — found the same track in another video", t.title) }
                        else { lf!("«{}»: старое видео пропало с YouTube, нашёл замену", "“{}”: the old video is gone from YouTube, found a replacement", t.title) }, false);
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
                    if let UpdState::Ready(n) = &st { self.toast(lf!("Новая версия (сборка {n}) установлена — перезапусти плеер", "New version (build {n}) installed — restart the player"), false); }
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
                Bg::GoogleCookies(r) => {
                    self.google_login = None;
                    match r {
                        Ok(path) => {
                            self.apply(Act::Settings(json!({"ytCookies": path, "ytAccount": true})), ctx);
                            self.apply(Act::AccountConnect, ctx);
                        }
                        Err(e) => self.toast(lf!("Вход в Google: {e}", "Google sign-in: {e}"), true),
                    }
                }
                Bg::Account(r) => match r {
                    Ok(()) => { if self.acc == AccState::Connecting { self.toast(l!("YouTube Music: аккаунт подключён", "YouTube Music: account connected"), false); } self.acc = AccState::On; }
                    Err(e) => { self.acc = AccState::Err(e.clone()); self.toast(lf!("YouTube Music: {e}", "YouTube Music: {e}"), true); }
                },
                Bg::Wave(r, more) => {
                    self.wave_busy = false;
                    match r {
                        Ok(list) if !list.is_empty() => {
                            let ids: Vec<String> = list.iter().map(|t| t.id.clone()).collect();
                            self.radio.retain(|t| !ids.contains(&t.id));
                            self.radio.extend(list);
                            let n = ids.len();
                            if more { self.queue.extend(ids); } else { self.play_list(ids, 0, false, ctx); self.toast(lf!("Моя волна: {n} треков, дальше подберётся сама", "My wave: {n} tracks, more will come by itself"), false); }
                            self.wave_on = true;
                        }
                        // error — the wave stops (otherwise it would ask for a top-up every frame)
                        Ok(_) => { self.wave_on = false; self.toast(l!("Волна не нашла новых треков", "The wave found no new tracks"), true); }
                        Err(e) => {
                            self.wave_on = false;
                            // sign-in expired (YouTube rotates cookies) — reconnect so the next attempt works
                            let low = e.to_lowercase();
                            if ["login", "auth", "401", "403", "cookie"].iter().any(|w| low.contains(w)) { self.connect_account(ctx); }
                            self.toast(lf!("Моя волна: {e}", "My wave: {e}"), true);
                        }
                    }
                }
                Bg::NotFound(pid, items) => if self.modal.is_none() { self.modal = Some(Modal::NotFound { pid, items }); },
                Bg::Picked(r) => {
                    if let Some(Modal::Pick { results, busy, err, .. }) = &mut self.modal {
                        *busy = false;
                        match r { Ok(v) => { if v.is_empty() { *err = Some(l!("Ничего не нашлось — попробуй написать иначе", "Nothing found — try different wording").into()); } *results = v; } Err(e) => *err = Some(e) }
                    }
                }
                Bg::ShikiResults(r) => {
                    if let Some(Modal::AnimeMatch { results, busy, err, .. }) = &mut self.modal {
                        *busy = false;
                        match r { Ok(v) => { if v.is_empty() { *err = Some(l!("Ничего не нашлось — попробуй другое написание (можно по-русски или по-английски)", "Nothing found — try a different spelling").into()); } *results = v; } Err(e) => *err = Some(e) }
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
                self.toast(l!("Таймер сна: трек доиграл, спокойной ночи", "Sleep timer: the track finished, good night"), false);
            } else { self.next(true, ctx); }
        }
        let state = (self.current.as_ref().map(|t| t.id.clone()), self.audio.is_playing());
        if state != self.last_pushed && !self.loading { self.push_state(); }
        self.sync_clock();
        if let Some(t) = self.current.as_ref().filter(|t| t.anime.is_none() && self.counted.as_deref() != Some(t.id.as_str())) {
            if self.audio.is_playing() && self.audio.position() > 30.0 { let id = t.id.clone(); self.store.lock().unwrap().count_play(&id); self.counted = Some(id); }
        }
        self.lastfm_tick();
        if self.wave_on && !self.wave_busy && self.qi + 3 >= self.queue.len() { self.apply(Act::Wave(true), ctx); }

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
            let Some(action) = self.hk_keys.ids.get(&id).cloned() else { continue };
            let vol = self.settings().volume;
            let act = match action.as_str() {
                "toggle" => Act::TogglePlay, "next" => Act::Next, "prev" => Act::Prev,
                "volUp" => { self.apply(Act::Volume((vol + 0.05).min(1.0)), ctx); Act::SaveVolume }
                "volDown" => { self.apply(Act::Volume((vol - 0.05).max(0.0)), ctx); Act::SaveVolume }
                "like" => match self.current.as_ref() { Some(t) => { let on = self.store.lock().unwrap().is_liked(&t.id); Act::Like(vec![t.id.clone()], !on) } None => continue },
                "skip" => {
                    let pos = self.audio.position();
                    let to = self.skips.as_ref().and_then(|s| s.1.iter().find(|k| pos >= k.start - 0.5 && pos < k.end - 2.0).map(|k| k.end)).unwrap_or(pos + 85.0);
                    Act::Seek(to)
                }
                "mini" => Act::ToggleMini, "overlay" => Act::ToggleOverlay, "lock" => Act::ToggleLock,
                _ => continue,
            };
            self.apply(act, ctx);
        }
        let mut dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        // wavy-export.json from the phone is sync, not music
        let exports: Vec<PathBuf> = dropped.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))).cloned().collect();
        dropped.retain(|p| !exports.contains(p));
        for e in exports { self.apply(Act::SyncImport(Some(e)), ctx); }
        if !dropped.is_empty() {
            if matches!(self.view, View::Anime | View::AnimeTitle(_)) { self.apply(Act::ImportAnime(dropped), ctx); }
            else { let pid = self.cur_playlist(); self.apply(Act::ImportPaths(dropped, pid), ctx); }
        }
        // episode finished (last 10 % or credits) — mark it
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
        self.stream_tick(ctx);
        if let Some((_, _, at)) = &self.toast { if at.elapsed() > Duration::from_secs(4) { self.toast = None; } }
        self.audio.poll();
        if self.audio.buffering() { ctx.request_repaint_after(Duration::from_millis(200)); }
        // video download: show percent, open when ready
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
        // auto-update: 5 s after launch and then every 6 hours
        if self.settings().auto_update && update::current_build().is_some() {
            let due = match self.upd_checked { None => self.started.elapsed() > Duration::from_secs(5), Some(t) => t.elapsed() > Duration::from_secs(6 * 3600) };
            let busy = matches!(self.upd, Some(UpdState::Checking | UpdState::Downloading(_) | UpdState::Ready(_)));
            if due && !busy { self.upd_checked = Some(Instant::now()); self.start_update(ctx); }
            // the window may idle without repaints — ask to be woken for the next check
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

    /// Japanese/Chinese/Korean titles: every couple of seconds check whether any appeared
    /// (library, playlists, search, anime), and if so load the system font once.
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

    /// ffmpeg decoder: download it if needed and retry what didn't open without it.
    fn ffmpeg_tick(&mut self, ctx: &egui::Context) {
        use wavy_core::ffmpeg::{State, NEED};
        let need_video = self.video_error.as_ref().is_some_and(|e| e.1.starts_with(NEED));
        if !(need_video || self.reload_after_ff) { return; }
        match self.ff.state() {
            State::Missing => self.need_ffmpeg(ctx),
            State::Downloading(_) => ctx.request_repaint_after(Duration::from_millis(300)),
            State::Ready(p) => {
                self.audio.ffmpeg = Some(p);
                if need_video { self.video_error = None; self.video = None; } // video_tick will reopen it
                if std::mem::take(&mut self.reload_after_ff) {
                    let pos = self.audio.position();
                    if let (Some(t), None) = (self.current.clone(), &self.resume) { self.resume = Some((t.id, pos, true)); }
                    self.load_current(ctx);
                }
            }
            State::Failed(e) => {
                if need_video { if let Some(v) = &mut self.video_error { v.1 = lf!("{} — не скачался декодер: {e}", "{} — the decoder didn't download: {e}", wavy_core::ffmpeg::human(&v.1)); } }
                if std::mem::take(&mut self.reload_after_ff) { self.toast(lf!("Не скачался декодер: {e}", "The decoder didn't download: {e}"), true); }
            }
        }
    }

    /// Video for the current track: open it while the viewer is open, unload it when closed (memory).
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
                    // fast path: direct URL + chunked download with percent
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
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] { [0.0; 4] } // transparency for the overlay window

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) { self.save_session(); self.addon_run.stop_all(); }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.tick(&ctx);
        let mut acts = Vec::new();
        let cmds: Vec<instance::Cmd> = self.instance.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for c in cmds {
            match c {
                instance::Cmd::Show => acts.push(Act::ShowWindow),
                instance::Cmd::Quit => { wavy_core::log("закрываюсь: запустилась новая версия"); self.audio.stop(); self.save_session(); acts.push(Act::Quit); }
            }
        }
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
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) { self.palette = if self.palette.is_some() { None } else { Some((String::new(), 0)) }; }
        if self.palette.is_some() { self.palette_ui(&ctx, &mut acts); }
        self.keys(&ctx, &mut acts);

        if self.fullscreen {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ui, |ui| self.video_view(ui, &mut acts));
            self.modal_ui(&ctx, &mut acts);
            self.toast_ui(&ctx);
            if self.overlay_open { self.overlay_window(&ctx, &mut acts); }
            for a in acts { self.apply(a, &ctx); }
            return;
        }
        egui::Panel::bottom("player").exact_size(84.0).frame(egui::Frame::new().fill(th().bg2).stroke(Stroke::new(1.0, th().line)).inner_margin(egui::Margin::symmetric(18, 10)))
            .show(ui, |ui| self.player_bar(ui, &mut acts));
        egui::Panel::left("side").exact_size(232.0).resizable(false).frame(egui::Frame::new().fill(th().bg2).inner_margin(egui::Margin::symmetric(12, 16)))
            .show(ui, |ui| self.sidebar(ui, &mut acts));
        if self.show_lyrics {
            egui::Panel::right("lyrics").exact_size(340.0).resizable(false).frame(egui::Frame::new().fill(th().bg2).inner_margin(14))
                .show(ui, |ui| self.lyrics_ui(ui, &mut acts));
        } else if self.show_queue {
            egui::Panel::right("queue").exact_size(290.0).resizable(false).frame(egui::Frame::new().fill(th().bg2).inner_margin(12))
                .show(ui, |ui| self.queue_ui(ui, &mut acts));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(th().bg).inner_margin(egui::Margin { left: 28, right: 20, top: 24, bottom: 0 }))
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
            if i.key_pressed(egui::Key::P) && self.show_video { acts.push(Act::Screenshot); }
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
            ui.painter().rect_filled(r, 8, th().accent);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E340}", FontId::new(15.0, egui::FontFamily::Name("fill".into())), th().on_accent);
            ui.add_space(4.0);
            ui.label(RichText::new("wavy").size(23.0).family(bold()).color(th().text));
        });
        ui.add_space(14.0);
        if nav(ui, l!("\u{E340}  Библиотека", "\u{E340}  Library"), self.view == View::Library, None).clicked() { acts.push(Act::View(View::Library)); }
        let n_liked = { let s = self.store.lock().unwrap(); s.lib.liked.iter().filter(|t| s.lib.tracks.contains_key(*t)).count() };
        if nav(ui, l!("\u{E2A8}  Любимое", "\u{E2A8}  Liked"), self.view == View::Liked, (n_liked > 0).then_some(n_liked)).clicked() { acts.push(Act::View(View::Liked)); }
        if nav(ui, l!("\u{E150}  Итоги", "\u{E150}  Recap"), self.view == View::Stats, None).clicked() { acts.push(Act::View(View::Stats)); }
        if nav(ui, l!("\u{E30C}  Поиск на YouTube", "\u{E30C}  Search YouTube"), self.view == View::Search, None).clicked() { acts.push(Act::View(View::Search)); }
        let n_anime = self.store.lock().unwrap().lib.anime.len();
        if nav(ui, l!("\u{E754}  Аниме", "\u{E754}  Anime"), matches!(self.view, View::Anime | View::AnimeTitle(_)), (n_anime > 0).then_some(n_anime)).clicked() { acts.push(Act::View(View::Anime)); }
        if self.acc == AccState::On {
            let label = if self.wave_busy { l!("\u{E802}  Моя волна…", "\u{E802}  My wave…") } else { l!("\u{E802}  Моя волна", "\u{E802}  My wave") };
            if nav(ui, label, self.wave_on, None).on_hover_text(l!("Бесконечное радио YouTube Music по твоим лайкам и истории", "Endless YouTube Music radio from your likes and history")).clicked() { acts.push(Act::Wave(false)); }
        }
        // smart playlists — built automatically from listening history
        let smart_open = matches!(self.view, View::Smart(_));
        egui::CollapsingHeader::new(RichText::new(l!("\u{E5FE}  Умные плейлисты", "\u{E5FE}  Smart playlists")).color(if smart_open { th().text } else { th().muted })).id_salt("smart").default_open(smart_open).show(ui, |ui| {
            for (rule, name, hint) in wavy_core::store::SMART {
                if nav(ui, wavy_core::i18n::tr(name), self.view == View::Smart(rule.to_string()), None).on_hover_text(wavy_core::i18n::tr(hint)).clicked() { acts.push(Act::View(View::Smart(rule.to_string()))); }
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(l!("ПЛЕЙЛИСТЫ", "PLAYLISTS")).size(11.0).color(th().muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_btn(ui, "+", 18.0, false).on_hover_text(l!("Новый плейлист", "New playlist")).clicked() { acts.push(Act::Modal(Modal::NewPlaylist { name: String::new(), ids: vec![] })); }
            });
        });
        let pls = self.store.lock().unwrap().lib.playlists.clone();
        let foot = 92.0;
        egui::ScrollArea::vertical().max_height(ui.available_height() - foot).auto_shrink([false, true]).show(ui, |ui| {
            if pls.is_empty() { ui.label(RichText::new(l!("Пока пусто", "Nothing here yet")).color(th().muted)); }
            let mut folders: Vec<String> = pls.iter().filter_map(|p| p.folder.clone()).collect();
            folders.sort_by_key(|f| f.to_lowercase());
            folders.dedup();
            let item = |ui: &mut egui::Ui, p: &wavy_core::store::Playlist, acts: &mut Vec<Act>, view: &View| {
                let icon = if p.is_remote() { "\u{E4FC}" } else { "\u{E6AA}" };
                let r = nav(ui, &format!("{icon}  {}", p.name), *view == View::Playlist(p.id.clone()), Some(p.track_ids.len()));
                if r.clicked() { acts.push(Act::View(View::Playlist(p.id.clone()))); }
                r.context_menu(|ui| {
                    if ui.button(l!("В папку…", "Move to folder…")).clicked() { acts.push(Act::Modal(Modal::Folder { id: p.id.clone(), name: p.folder.clone().unwrap_or_default() })); ui.close(); }
                    if p.folder.is_some() && ui.button(l!("Убрать из папки", "Remove from folder")).clicked() { acts.push(Act::SetFolder(p.id.clone(), None)); ui.close(); }
                    if ui.button(l!("Переименовать…", "Rename…")).clicked() { acts.push(Act::Modal(Modal::Rename { id: p.id.clone(), name: p.name.clone() })); ui.close(); }
                    if ui.button(RichText::new(l!("Удалить…", "Delete…")).color(BAD)).clicked() { acts.push(Act::Modal(Modal::Delete { id: p.id.clone() })); ui.close(); }
                });
            };
            for f in &folders {
                egui::CollapsingHeader::new(RichText::new(format!("\u{E24A}  {f}")).color(th().muted)).id_salt(("folder", f)).default_open(true).show(ui, |ui| {
                    for p in pls.iter().filter(|p| p.folder.as_ref() == Some(f)) { item(ui, p, acts, &self.view); }
                });
            }
            for p in pls.iter().filter(|p| p.folder.is_none()) { item(ui, p, acts, &self.view); }
        });
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            let d = if self.discord_status.connected { "\u{E61A}" } else { "" };
            if nav(ui, &lf!("\u{E272}  Настройки  {d}", "\u{E272}  Settings  {d}"), false, None).clicked() {
                let s = self.settings();
                acts.push(Act::Modal(Modal::Settings { client_id: s.discord.client_id, port: s.overlay.port.to_string(), sp_id: s.spotify.client_id, sp_secret: s.spotify.client_secret }));
            }
            let s = self.settings();
            let st = if !self.overlay_open { l!("выкл", "off") } else if s.overlay_window.locked { l!("вкл", "on") } else { l!("двигай", "move") };
            if nav(ui, &lf!("\u{E466}  Оверлей · {st}", "\u{E466}  Overlay · {st}"), self.overlay_open, None).clicked() {
                acts.push(Act::Modal(Modal::Settings { client_id: s.discord.client_id.clone(), port: s.overlay.port.to_string(), sp_id: s.spotify.client_id.clone(), sp_secret: s.spotify.client_secret.clone() }));
            }
            ui.separator();
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, ids: &[String], acts: &mut Vec<Act>) {
        let (eyebrow, title, pl) = {
            let s = self.store.lock().unwrap();
            match &self.view {
                View::Library => (l!("КОЛЛЕКЦИЯ", "COLLECTION"), l!("Библиотека", "Library").to_string(), None),
                View::Liked => (l!("КОЛЛЕКЦИЯ", "COLLECTION"), l!("Любимое", "Liked").to_string(), None),
                View::Smart(r) => (l!("УМНЫЙ ПЛЕЙЛИСТ", "SMART PLAYLIST"), wavy_core::store::SMART.iter().find(|x| x.0 == r).map(|x| wavy_core::i18n::tr(x.1).to_string()).unwrap_or_default(), None),
                View::Stats => (l!("ТВОЯ МУЗЫКА", "YOUR MUSIC"), l!("Итоги", "Recap").to_string(), None),
                View::Search => ("YOUTUBE", l!("Поиск", "Search").to_string(), None),
                View::Anime | View::AnimeTitle(_) => (l!("АНИМЕ", "ANIME"), l!("Аниме", "Anime").to_string(), None),
                View::Playlist(id) => {
                    let p = s.lib.playlists.iter().find(|p| &p.id == id).cloned();
                    (if p.as_ref().is_some_and(|p| p.is_remote()) { l!("ПЛЕЙЛИСТ · ИМПОРТ", "PLAYLIST · IMPORT") } else { l!("ПЛЕЙЛИСТ", "PLAYLIST") }, p.as_ref().map(|p| p.name.clone()).unwrap_or_default(), p)
                }
            }
        };
        let total: f64 = ids.iter().filter_map(|i| self.track(i)).map(|t| t.duration).sum();
        let own = pl.as_ref().and_then(|p| p.cover.clone());
        let cover = own.clone().or_else(|| ids.iter().filter_map(|i| self.track(i)).find_map(|t| t.cover.clone().map(|c| if t.video_id.is_some() { format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", t.video_id.unwrap()) } else { c })));
        ui.horizontal(|ui| {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(120.0), if pl.is_some() { Sense::click() } else { Sense::hover() });
            if let Some(p) = &pl {
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(l!("Поставить свою картинку", "Set your own image"));
                if resp.clicked() { acts.push(Act::PlaylistCover(p.id.clone())); }
                resp.context_menu(|ui| {
                    if ui.button(l!("Поставить свою картинку…", "Set your own image…")).clicked() { acts.push(Act::PlaylistCover(p.id.clone())); ui.close(); }
                    if own.is_some() && ui.button(l!("Вернуть обложку из треков", "Use the cover from the tracks")).clicked() { acts.push(Act::PlaylistCoverReset(p.id.clone())); ui.close(); }
                });
            }
            let hovered = pl.is_some() && ui.rect_contains_pointer(r);
            match cover.filter(|_| pl.is_some()).and_then(|c| self.thumbs.get(&c, 240)) {
                Some(t) => { ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE); }
                None => {
                    ui.painter().rect_filled(r, 14, th().bg3);
                    ui.painter().rect_stroke(r, 14, Stroke::new(1.0, th().line), egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, match self.view { View::Search => "\u{E30C}", View::Liked => "\u{E2A8}", _ => "\u{E340}" }, FontId::new(44.0, if self.view == View::Liked { egui::FontFamily::Name("fill".into()) } else { egui::FontFamily::Proportional }), th().accent);
                }
            }
            if hovered {
                ui.painter().rect_filled(r, 0, Color32::from_black_alpha(150));
                glyph(ui.painter(), r.center() - Vec2::new(0.0, 10.0), "\u{E3B4}", FontId::proportional(26.0), th().text);
                ui.painter().text(r.center() + Vec2::new(0.0, 18.0), Align2::CENTER_CENTER, l!("Своя обложка", "Custom cover"), FontId::proportional(12.0), th().text);
            }
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(eyebrow).size(11.0).color(th().muted));
                ui.label(RichText::new(&title).size(34.0).family(bold()).color(th().text));
                if self.view == View::Search { ui.label(RichText::new(l!("Ищи треки и добавляй в библиотеку или в плейлист", "Search for tracks and add them to the library or a playlist")).color(th().muted)); }
                else { ui.label(RichText::new(lf!("{} трек(ов) · {}", "{} track(s) · {}", ids.len(), fmt_time(total))).color(th().muted)); }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if !ids.is_empty() && self.view != View::Search {
                        if pill(ui, l!("\u{E3D0}  Слушать", "\u{E3D0}  Play"), true).clicked() { acts.push(Act::Play(ids.to_vec(), 0, false)); }
                        if pill(ui, l!("\u{E422}  Вперемешку", "\u{E422}  Shuffle"), false).clicked() { acts.push(Act::Play(ids.to_vec(), 0, true)); }
                    }
                    if let View::Smart(r) = &self.view {
                        if !ids.is_empty() && pill(ui, l!("\u{E248}  Сохранить копию", "\u{E248}  Save a copy"), false).on_hover_text(l!("Обычный плейлист с этими треками — он уже не будет меняться сам", "A regular playlist with these tracks — it won't change by itself anymore")).clicked() {
                            let name = format!("{} · {}", wavy_core::store::SMART.iter().find(|x| x.0 == r).map(|x| wavy_core::i18n::tr(x.1)).unwrap_or(""), fmt_date(wavy_core::now_ms()));
                            acts.push(Act::CreatePlaylist(name, ids.to_vec()));
                        }
                        if let Some((_, _, hint)) = wavy_core::store::SMART.iter().find(|x| x.0 == r) { ui.label(RichText::new(wavy_core::i18n::tr(hint)).color(th().muted)); }
                    }
                    if self.view == View::Library && !ids.is_empty() && pill(ui, l!("\u{E1CA}  Дубли", "\u{E1CA}  Duplicates"), false).on_hover_text(l!("Найти одинаковые треки", "Find identical tracks")).clicked() {
                        let groups = self.store.lock().unwrap().duplicates();
                        if groups.is_empty() { self.toast(l!("Дублей нет — всё чисто", "No duplicates — all clean"), false); } else { acts.push(Act::Modal(Modal::Dupes { groups })); }
                    }
                    if let Some(p) = &pl {
                        if p.is_remote() && pill(ui, l!("\u{E094}  Синхронизировать", "\u{E094}  Sync"), false).clicked() { acts.push(Act::Sync(p.id.clone())); }
                        if !ids.is_empty() && pill(ui, l!("\u{E20C}  Скачать MP3", "\u{E20C}  Download MP3"), false).on_hover_text(l!("Сохранить весь плейлист в MP3 в одну папку", "Save the whole playlist as MP3 into one folder")).clicked() { acts.push(Act::Export(ids.to_vec(), Some(p.name.clone()))); }
                        if pill(ui, l!("\u{E3B4}  Переименовать", "\u{E3B4}  Rename"), false).clicked() { acts.push(Act::Modal(Modal::Rename { id: p.id.clone(), name: p.name.clone() })); }
                        if pill(ui, l!("\u{E4A6}  Удалить", "\u{E4A6}  Delete"), false).clicked() { acts.push(Act::Modal(Modal::Delete { id: p.id.clone() })); }
                    }
                });
            });
        });
        ui.add_space(14.0);
    }

    fn main_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        if let Some(UpdState::Ready(n)) = self.upd.clone() {
            egui::Frame::new().fill(th().bg3).stroke(Stroke::new(1.0, th().line)).corner_radius(12).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(lf!("\u{E036}  Новая версия Wavy (сборка {n}) уже скачана", "\u{E036}  A new version of Wavy (build {n}) is downloaded")).color(th().text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, l!("Перезапустить", "Restart"), true).clicked() { acts.push(Act::Restart); } });
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
                let r = ui.add(egui::TextEdit::singleline(&mut self.search_q).hint_text(l!("Найти трек на YouTube… или вставь ссылку", "Find a track on YouTube… or paste a link")).desired_width(480.0).margin(egui::Margin::symmetric(12, 8)));
                let go = pill(ui, if self.searching { l!("Ищу…", "Searching…") } else { l!("Искать", "Search") }, true).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
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
            let fr = ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(l!("\u{E30C} Фильтр (Ctrl+F)…", "\u{E30C} Filter (Ctrl+F)…")).desired_width(fw).margin(egui::Margin::symmetric(12, 8)));
            if std::mem::take(&mut self.focus_filter) { fr.request_focus(); }
            if fr.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) { self.filter.clear(); fr.surrender_focus(); }
            // sorting (separate for the library and for each playlist)
            let (key, is_pl) = match &self.view { View::Playlist(id) => (id.clone(), true), View::Liked => ("liked".to_string(), true), _ => ("library".to_string(), false) };
            let cur = self.settings().sort.get(&key).cloned().unwrap_or_default();
            let opts: &[(&str, &str)] = if is_pl {
                &[("", if key == "liked" { l!("Как лайкал", "Order liked") } else { l!("Как в плейлисте", "Playlist order") }), ("plays", l!("Часто слушаю", "Most played")), ("newest", l!("Сначала новые", "Newest first")), ("title", l!("По названию", "By title")), ("artist", l!("По исполнителю", "By artist")), ("album", l!("По альбому", "By album"))]
            } else {
                &[("", l!("Сначала новые", "Newest first")), ("plays", l!("Часто слушаю", "Most played")), ("oldest", l!("Сначала старые", "Oldest first")), ("title", l!("По названию", "By title")), ("artist", l!("По исполнителю", "By artist")), ("album", l!("По альбому", "By album"))]
            };
            let label = opts.iter().find(|o| o.0 == cur).map(|o| o.1).unwrap_or(opts[0].1);
            ui.add_space(6.0);
            let text = format!("\u{E444}  {label}");
            let r = ui.add(pill_btn(&text, false)).on_hover_text(l!("Сортировка", "Sorting"));
            egui::Popup::menu(&r).show(|ui| {
                ui.set_min_width(180.0);
                for (k, name) in opts {
                    let on = *k == cur.as_str() || (k.is_empty() && !opts.iter().any(|o| o.0 == cur));
                    if ui.selectable_label(on, *name).clicked() { acts.push(Act::Sort(key.clone(), k.to_string())); ui.close(); }
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // narrow window (lyrics/queue open) — icons only
                let narrow = ui.available_width() < 330.0;
                let t = |icon: &str, text: &str| if narrow { icon.to_string() } else { format!("{icon}  {text}") };
                if pill(ui, &t("\u{E2E2}", l!("По ссылке", "By link")), false).on_hover_text(l!("Добавить по ссылке", "Add by link")).clicked() { acts.push(Act::Modal(Modal::ImportUrl { url: String::new(), into: self.cur_playlist().is_some(), busy: false, err: None })); }
                if pill(ui, &t("\u{E25A}", l!("Папка", "Folder")), false).on_hover_text(l!("Добавить папку", "Add a folder")).clicked() { acts.push(Act::ImportFolder); }
                if pill(ui, &t("\u{EA20}", l!("Файлы", "Files")), false).on_hover_text(l!("Добавить файлы", "Add files")).clicked() { acts.push(Act::ImportFiles); }
            });
        });
        ui.add_space(8.0);
        if ids.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                let filt = !self.filter.is_empty();
                ui.label(RichText::new(if filt { l!("Ничего не нашлось", "Nothing found") } else if self.cur_playlist().is_some() { l!("Плейлист пустой", "The playlist is empty") } else { l!("Тут пока тихо", "It's quiet here for now") }).size(22.0).family(bold()).color(th().text));
                ui.label(RichText::new(l!("Перетащи файлы/папки в окно или импортируй плейлист с YouTube Music по ссылке", "Drag files/folders into the window or import a YouTube Music playlist by link")).color(th().muted));
                ui.add_space(12.0);
                if !filt && pill(ui, l!("\u{E3D0}  Плейлист с YouTube", "\u{E3D0}  Playlist from YouTube"), true).clicked() { acts.push(Act::Modal(Modal::ImportUrl { url: String::new(), into: self.cur_playlist().is_some(), busy: false, err: None })); }
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
        // dragging works in playlists and "Liked" when the order is custom (no sort and no filter)
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
                for t in ["#", l!("НАЗВАНИЕ", "TITLE"), l!("АЛЬБОМ", "ALBUM"), "", "\u{E19A}"] { h.col(|ui| { ui.label(RichText::new(t).size(11.0).color(th().muted)); }); }
            })
            .body(|body| {
                body.rows(52.0, ids.len(), |mut row| {
                    let i = row.index();
                    let id = &ids[i];
                    let Some(t) = self.store.lock().unwrap().lib.tracks.get(id).cloned() else { return };
                    let is_cur = cur.as_deref() == Some(id.as_str());
                    row.set_selected(self.selected.contains(id));
                    row.col(|ui| {
                        if is_cur { ui.label(RichText::new(if playing { "\u{E340}" } else { "\u{E39E}" }).color(th().accent)); }
                        else { ui.label(RichText::new((i + 1).to_string()).color(th().muted)); }
                    });
                    row.col(|ui| {
                        self.cover(ui, &t, 40.0);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.add_space(6.0);
                            ui.add(egui::Label::new(RichText::new(&t.title).color(if is_cur { th().accent } else { th().text }).size(14.0)).truncate());
                            ui.add(egui::Label::new(RichText::new(wavy_core::i18n::tr(&t.artist)).color(th().muted).size(12.5)).truncate());
                        });
                    });
                    row.col(|ui| {
                        let album = if !t.album.is_empty() { t.album.clone() } else if t.kind == "youtube" { "YouTube".into() } else if t.kind == "stream" { l!("Стрим", "Stream").into() } else { String::new() };
                        ui.add(egui::Label::new(RichText::new(album).color(th().muted).size(13.0)).truncate());
                    });
                    row.col(|ui| {
                        let on = liked.contains(id);
                        if on || hover_prev.as_deref() == Some(id.as_str()) {
                            let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                            let fam = if on { egui::FontFamily::Name("fill".into()) } else { egui::FontFamily::Proportional };
                            glyph(ui.painter(), r.center(), "\u{E2A8}", FontId::new(16.0, fam), if on { th().accent } else if resp.hovered() { th().text } else { th().muted });
                            if resp.on_hover_text(if on { l!("Убрать из «Любимого»", "Remove from “Liked”") } else { l!("В «Любимое»", "Add to “Liked”") }).clicked() { acts.push(Act::Like(vec![id.clone()], !on)); }
                        }
                    });
                    row.col(|ui| { ui.label(RichText::new(fmt_time(t.duration)).color(th().muted)); });
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
                        let lbl = if is_liked { l!("Убрать из «Любимого»", "Remove from “Liked”") } else { l!("В «Любимое»", "Add to “Liked”") };
                        if ui.button(lbl).clicked() { acts.push(Act::Like(sel.clone(), !is_liked)); ui.close(); }
                        track_menu(ui, &sel, &t, pid.as_deref(), &pls, acts)
                    });
                });
            });
        if self.drag_ids.is_some() {
            if let Some((y, x0, x1)) = line {
                ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("drop-line"))).line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(2.0, th().accent));
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if ui.input(|i| i.pointer.any_released()) {
                if let (Some(ids), Some(to)) = (self.drag_ids.take(), self.drop_at.take()) { acts.push(Act::Move(ids, to)); }
                self.drag_ids = None;
                self.drop_at = None;
            }
        }
    }

    /// Poster (2:3) center-cropped; without a poster — a placeholder.
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
                ui.painter().rect_filled(r, 10, th().bg3);
                ui.painter().rect_stroke(r, 10, Stroke::new(1.0, th().line), egui::StrokeKind::Inside);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E754}", FontId::proportional(r.width() * 0.28), th().muted);
            }
        }
    }

    fn anime_grid(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let list = self.store.lock().unwrap().lib.anime.clone();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(l!("КОЛЛЕКЦИЯ", "COLLECTION")).size(11.0).color(th().muted));
                ui.label(RichText::new(l!("Аниме", "Anime")).size(34.0).family(bold()).color(th().text));
                ui.label(RichText::new(if list.is_empty() { l!("Серии с компьютера — с обложками и названиями с Шикимори", "Episodes from your computer — with covers and titles from Shikimori").to_string() } else { lf!("{} тайтл(ов)", "{} title(s)", list.len()) }).color(th().muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if pill(ui, l!("\u{E25A}  Папка", "\u{E25A}  Folder"), true).clicked() { acts.push(Act::ImportAnimeFolder); }
                if pill(ui, l!("\u{EA20}  Файлы", "\u{EA20}  Files"), false).clicked() { acts.push(Act::ImportAnimeFiles); }
            });
        });
        ui.add_space(14.0);
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(l!("\u{E30C} Найти тайтл…", "\u{E30C} Find a title…")).desired_width(360.0).margin(egui::Margin::symmetric(12, 8)));
        ui.add_space(12.0);
        if list.is_empty() {
            ui.add_space(50.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(l!("Пока пусто", "Nothing here yet")).size(22.0).family(bold()).color(th().text));
                ui.label(RichText::new(l!("Перетащи сюда папку с сериями (mkv, mp4). Название и номер серии Wavy поймёт из имён файлов,", "Drag a folder with episodes here (mkv, mp4). Wavy will figure out the title and episode number from the file names,")).color(th().muted));
                ui.label(RichText::new(l!("а обложку и русское название возьмёт с Шикимори.", "and take the cover and the title from Shikimori.")).color(th().muted));
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
                    if resp.hovered() { ui.painter().rect_stroke(pr, 10, Stroke::new(2.0, th().text), egui::StrokeKind::Outside); }
                    let seen = a.watched.iter().filter(|w| a.episodes.contains(w)).count();
                    let badge = format!("{seen}/{}", a.episodes.len());
                    let br = Rect::from_min_size(pr.right_top() + Vec2::new(-52.0, 8.0), Vec2::new(44.0, 20.0));
                    ui.painter().rect_filled(br, 99, Color32::from_black_alpha(190));
                    ui.painter().text(br.center(), Align2::CENTER_CENTER, badge, FontId::proportional(11.5), Color32::WHITE);
                    // title — up to two lines with "…"
                    let mut job = egui::text::LayoutJob::simple(a.display().to_string(), FontId::new(13.5, semi()), th().text, cw);
                    job.wrap.max_rows = 2;
                    let g = ui.fonts_mut(|f| f.layout_job(job));
                    let title_h = g.size().y;
                    ui.painter().galley(pr.left_bottom() + Vec2::new(0.0, 8.0), g, th().text);
                    let meta = if a.matched && a.shiki_id.is_some() { a.meta() } else if a.matched { l!("нет на Шикимори", "not on Shikimori").into() } else { l!("ищу на Шикимори…", "searching Shikimori…").into() };
                    ui.painter().text(pr.left_bottom() + Vec2::new(0.0, 12.0 + title_h), Align2::LEFT_TOP, ellipsize(&meta, cw - 10.0, 12.0), FontId::proportional(12.0), th().muted);
                    if resp.clicked() { acts.push(Act::View(View::AnimeTitle(a.id.clone()))); }
                    let id = a.id.clone();
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand).context_menu(|ui| {
                        if ui.button(l!("Смотреть", "Watch")).clicked() { acts.push(Act::PlayAnime(id.clone(), None)); ui.close(); }
                        if ui.button(l!("Другой тайтл на Шикимори…", "Another title on Shikimori…")).clicked() { acts.push(Act::Modal(Modal::AnimeMatch { id: id.clone(), query: a.display().to_string(), results: vec![], busy: false, err: None })); acts.push(Act::AnimeSearch(a.display().to_string())); ui.close(); }
                        ui.separator();
                        if ui.button(RichText::new(l!("Удалить из раздела", "Remove from the section")).color(BAD)).clicked() { acts.push(Act::Modal(Modal::DeleteAnime { id: id.clone() })); ui.close(); }
                    });
                }
            });
            ui.add_space(20.0);
        });
    }

    fn anime_page(&mut self, ui: &mut egui::Ui, id: &str, acts: &mut Vec<Act>) {
        let Some(a) = self.store.lock().unwrap().anime(id).cloned() else { acts.push(Act::View(View::Anime)); return };
        if ui.add(egui::Label::new(RichText::new(l!("\u{E138}  Аниме", "\u{E138}  Anime")).color(th().muted)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { acts.push(Act::View(View::Anime)); }
        ui.add_space(10.0);
        let eps: Vec<Track> = a.episodes.iter().filter_map(|e| self.track(e)).collect();
        let cont = a.last.as_ref().and_then(|l| eps.iter().find(|t| &t.id == l)).map(|t| t.title.clone());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(150.0, 225.0), Sense::hover());
            self.poster(ui, &a, r, 360);
            ui.add_space(18.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(if a.kind.is_empty() { l!("АНИМЕ", "ANIME").to_string() } else { anime::kind_ru(&a.kind).to_uppercase() }).size(11.0).color(th().muted));
                ui.add(egui::Label::new(RichText::new(a.display()).size(30.0).family(bold()).color(th().text)).wrap());
                if !a.name.is_empty() && a.name != a.display() { ui.label(RichText::new(&a.name).color(th().muted)); }
                let mut meta = a.meta();
                let seen = a.watched.iter().filter(|w| a.episodes.contains(w)).count();
                meta = [meta, lf!("серий в папке: {} · просмотрено {seen}", "episodes in the folder: {} · watched {seen}", eps.len())].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                ui.label(RichText::new(meta).color(th().muted));
                if !a.matched { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Ищу на Шикимори…", "Searching Shikimori…")).color(th().muted)); }); }
                else if a.shiki_id.is_none() { ui.label(RichText::new(l!("На Шикимори не нашлось — выбери вручную", "Not found on Shikimori — pick it manually")).color(th().muted)); }
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    let play = match &cont { Some(t) if a.last_pos > 5.0 || a.watched.len() > 0 => lf!("\u{E3D0}  Продолжить · {t}", "\u{E3D0}  Continue · {t}"), _ => l!("\u{E3D0}  Смотреть", "\u{E3D0}  Watch").into() };
                    if !eps.is_empty() && pill(ui, &play, true).clicked() { acts.push(Act::PlayAnime(a.id.clone(), None)); }
                    if let Some(u) = &a.url { if pill(ui, l!("\u{E5DE}  Шикимори", "\u{E5DE}  Shikimori"), false).clicked() { acts.push(Act::OpenUrl(u.clone())); } }
                    if pill(ui, l!("\u{E30C}  Другой тайтл", "\u{E30C}  Another title"), false).on_hover_text(l!("Если Wavy угадал не то аниме или сезон", "If Wavy guessed the wrong anime or season")).clicked() {
                        let q = if a.title.is_empty() { a.display().to_string() } else { a.title.clone() };
                        acts.push(Act::Modal(Modal::AnimeMatch { id: a.id.clone(), query: q.clone(), results: vec![], busy: false, err: None }));
                        acts.push(Act::AnimeSearch(q));
                    }
                    if a.shiki_id.is_some() {
                        let busy = self.themes_busy.as_deref() == Some(a.id.as_str());
                        if ui.add_enabled(!busy, pill_btn(if busy { l!("\u{E340}  Ищу опенинги…", "\u{E340}  Looking for openings…") } else { l!("\u{E340}  Опенинги и эндинги", "\u{E340}  Openings and endings") }, false)).on_hover_text(l!("Все OP и ED этого тайтла — в отдельный плейлист (треки с YouTube)", "All OPs and EDs of this title — into a separate playlist (tracks from YouTube)")).clicked() { acts.push(Act::AnimeThemes(a.id.clone())); }
                    }
                    if pill(ui, l!("\u{EA20}  Добавить серии", "\u{EA20}  Add episodes"), false).clicked() { acts.push(Act::ImportAnimeFiles); }
                    if pill(ui, l!("\u{E4A6}  Удалить", "\u{E4A6}  Delete"), false).clicked() { acts.push(Act::Modal(Modal::DeleteAnime { id: a.id.clone() })); }
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
            .header(26.0, |mut h| { for t in ["", l!("СЕРИЯ", "EPISODE"), "\u{E19A}"] { h.col(|ui| { ui.label(RichText::new(t).size(11.0).color(th().muted)); }); } })
            .body(|body| {
                body.rows(46.0, eps.len(), |mut row| {
                    let t = &eps[row.index()];
                    let is_cur = cur.as_deref() == Some(t.id.as_str());
                    let seen = a.watched.contains(&t.id);
                    row.col(|ui| {
                        let (ic, c) = if is_cur { (if playing { "\u{E340}" } else { "\u{E39E}" }, th().accent) } else if seen { ("\u{E184}", th().muted) } else { ("", th().muted) };
                        ui.label(RichText::new(ic).color(c));
                    });
                    row.col(|ui| {
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(&t.title).color(if is_cur { th().accent } else if seen { th().muted } else { th().text }).size(14.0)).truncate());
                            let file = t.path.as_deref().and_then(|p| Path::new(p).file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.add(egui::Label::new(RichText::new(file).color(th().muted).size(11.5)).truncate());
                        });
                    });
                    row.col(|ui| { ui.label(RichText::new(fmt_time(t.duration)).color(th().muted)); });
                    let r = row.response();
                    if r.double_clicked() { acts.push(Act::PlayAnime(a.id.clone(), Some(t.id.clone()))); }
                    r.context_menu(|ui| {
                        if ui.button(l!("Смотреть", "Watch")).clicked() { acts.push(Act::PlayAnime(a.id.clone(), Some(t.id.clone()))); ui.close(); }
                        if ui.button(if seen { l!("Отметить непросмотренной", "Mark as unwatched") } else { l!("Отметить просмотренной", "Mark as watched") }).clicked() { acts.push(Act::MarkWatched(t.id.clone(), !seen)); ui.close(); }
                        if let Some(p) = &t.path { if ui.button(l!("Показать в папке", "Show in folder")).clicked() { acts.push(Act::Reveal(p.clone())); ui.close(); } }
                    });
                });
            });
    }

    fn search_results(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        if self.results.is_empty() {
            ui.add_space(50.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(if self.searching { l!("Ищу…", "Searching…") } else { l!("Найди что-нибудь", "Find something") }).size(22.0).family(bold()).color(th().text));
                ui.label(RichText::new(l!("Например: «lofi hip hop», «Daft Punk», или вставь ссылку на плейлист", "For example: “lofi hip hop”, “Daft Punk”, or paste a playlist link")).color(th().muted));
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
                        ui.add(egui::Label::new(RichText::new(&t.title).color(if cur.as_deref() == Some(&t.id) { th().accent } else { th().text })).truncate());
                        ui.add(egui::Label::new(RichText::new(wavy_core::i18n::tr(&t.artist)).color(th().muted).size(12.5)).truncate());
                    });
                    ui.label(RichText::new(fmt_time(t.duration)).color(th().muted));
                    ui.add_space(10.0);
                    if pill(ui, "\u{E3D0}", false).clicked() { acts.push(Act::Play(ids.clone(), i, false)); }
                    if ui.add_enabled(!in_lib, egui::Button::new(if in_lib { l!("В библиотеке", "In library") } else { l!("+ В библиотеку", "+ To library") }).corner_radius(99)).clicked() { acts.push(Act::AddTracks(vec![t.clone()])); }
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
                ui.painter().rect_filled(r, 6, th().bg3);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E33C}", FontId::proportional(size * 0.4), th().muted);
            }
        }
        if !t.is_local() { ui.painter().rect_filled(Rect::from_min_size(r.right_bottom() - Vec2::splat(10.0), Vec2::splat(10.0)), 3, Color32::from_rgb(0xff, 0, 0x33)); }
    }

    fn player_bar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let w = ui.available_width();
        let s = self.settings();
        ui.columns_const(|[left, mid, right]| {
            // now playing — a click opens/closes the video view (or the large cover)
            let np = left.horizontal(|ui| {
                ui.set_max_width(w * 0.3);
                match self.current.clone() {
                    Some(t) => {
                        self.cover(ui, &t, 56.0);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.add_space(8.0);
                            ui.add(egui::Label::new(RichText::new(&t.title).family(semi()).color(th().text)).truncate());
                            ui.add(egui::Label::new(RichText::new(if self.loading { l!("загрузка…", "loading…").to_string() } else if self.audio.buffering() { l!("буферизация…", "buffering…").to_string() } else { wavy_core::i18n::tr(&t.artist).to_string() }).color(th().muted).size(12.5)).truncate());
                        });
                    }
                    None => { ui.vertical(|ui| { ui.add_space(10.0); ui.label(RichText::new(l!("Ничего не играет", "Nothing is playing")).family(semi())); ui.label(RichText::new(l!("Закинь музыку — и погнали", "Drop some music in — let's go")).color(th().muted).size(12.5)); }); }
                }
            }).response;
            if self.current.is_some() {
                let has_video = self.current.as_ref().is_some_and(|t| t.has_video());
                let r = left.interact(np.rect.expand(4.0), Id::unique("now-playing"), Sense::click());
                if r.hovered() || self.show_video {
                    left.painter().rect_stroke(np.rect.expand(4.0), 10, Stroke::new(1.0, if self.show_video { th().text } else { th().line }), egui::StrokeKind::Outside);
                }
                let hint = match (self.show_video, has_video) { (true, _) => l!("Скрыть", "Hide"), (false, true) => l!("Смотреть видео", "Watch video"), (false, false) => l!("Показать обложку", "Show cover") };
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hint).clicked() { acts.push(Act::ToggleVideo); }
            }
            // buttons + seek bar
            mid.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    // row height follows the big button from the start: otherwise buttons before it center on 34 px and after it on 44
                    ui.set_min_height(44.0);
                    let bw = 34.0 * 4.0 + 44.0 + 4.0 * 14.0;
                    ui.add_space(((ui.available_width() - bw) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 14.0;
                    if icon_btn(ui, "\u{E422}", 34.0, s.shuffle).on_hover_text(l!("Перемешать", "Shuffle")).clicked() { acts.push(Act::Shuffle); }
                    if icon_btn(ui, "\u{E5A4}", 34.0, false).clicked() { acts.push(Act::Prev); }
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 21.0, if resp.hovered() { th().accent } else { th().text });
                    if self.loading || self.audio.buffering() {
                        ui.ctx().request_repaint_after(Duration::from_millis(33));
                        // loading: three dots "run" in a circle instead of an off-center "…" glyph
                        let t = ui.input(|i| i.time) as f32;
                        for k in 0..3 {
                            let a = (k as f32 - 1.0) * 7.0;
                            let lift = ((t * 6.0 - k as f32 * 0.9).sin().max(0.0)) * 3.0;
                            ui.painter().circle_filled(r.center() + Vec2::new(a, -lift), 2.4, th().bg);
                        }
                    } else {
                        glyph(ui.painter(), r.center(), if self.audio.is_playing() { "\u{E39E}" } else { "\u{E3D0}" }, FontId::new(20.0, egui::FontFamily::Name("fill".into())), th().bg);
                    }
                    if resp.clicked() { acts.push(Act::TogglePlay); }
                    if icon_btn(ui, "\u{E5A6}", 34.0, false).clicked() { acts.push(Act::Next); }
                    let rep = match s.repeat.as_str() { "one" => "\u{E3F8}", _ => "\u{E3F6}" };
                    if icon_btn(ui, rep, 34.0, s.repeat != "off").on_hover_text(l!("Повтор", "Repeat")).clicked() { acts.push(Act::Repeat); }
                });
                ui.horizontal(|ui| {
                    let dur = if self.audio.duration > 0.0 { self.audio.duration } else { self.current.as_ref().map(|t| t.duration).unwrap_or(0.0) };
                    let pos = self.seek_drag.unwrap_or(self.audio.position()).min(dur.max(0.0));
                    ui.label(RichText::new(fmt_time(pos)).size(12.0).color(th().muted));
                    let sw = (ui.available_width() - 48.0).max(60.0);
                    let (r, new) = bar(ui, if dur > 0.0 { (pos / dur) as f32 } else { 0.0 }, sw);
                    if let Some(f) = new { if r.dragged() { self.seek_drag = Some(f as f64 * dur); } }
                    if r.drag_stopped() || r.clicked() {
                        if let Some(v) = new.map(|f| f as f64 * dur).or(self.seek_drag) { acts.push(Act::Seek(v)); }
                        self.seek_drag = None;
                    }
                    ui.label(RichText::new(fmt_time(dur)).size(12.0).color(th().muted));
                });
            });
            // volume, queue
            right.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(4.0);
                let (r, new) = bar(ui, s.volume, 110.0);
                let v = new.unwrap_or(s.volume);
                if let Some(f) = new { acts.push(Act::Volume(f)); }
                if r.drag_stopped() || r.clicked() { acts.push(Act::SaveVolume); }
                ui.label(RichText::new(if v == 0.0 { "\u{E45C}" } else { "\u{E44A}" }).color(th().muted));
                if icon_btn(ui, "\u{E64C}", 34.0, s.mini).on_hover_text(l!("Мини-плеер поверх окон", "Mini player on top of windows")).clicked() { acts.push(Act::ToggleMini); }
                // sound: speed, crossfade, volume, equalizer
                let fx_on = (s.speed - 1.0).abs() > 0.01 || s.crossfade > 0.0 || s.normalize || s.eq_on;
                let r = icon_btn(ui, "\u{E434}", 34.0, fx_on).on_hover_text(l!("Звук: скорость, кроссфейд, эквалайзер", "Sound: speed, crossfade, equalizer"));
                // clicking the button opens/closes it; from the Ctrl+K palette it opens
                let force = if std::mem::take(&mut self.sound_open) { Some(egui::SetOpenCommand::Bool(true)) } else { r.clicked().then_some(egui::SetOpenCommand::Toggle) };
                egui::Popup::from_toggle_button_response(&r).open_memory(force).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| self.sound_ui(ui, &s, acts));
                // sleep timer
                let r = icon_btn(ui, "\u{E330}", 34.0, self.sleep.is_some()).on_hover_text(match self.sleep {
                    Some(Sleep::At(t)) => lf!("Таймер сна: через {} мин", "Sleep timer: in {} min", (t.saturating_duration_since(Instant::now()).as_secs() + 59) / 60),
                    Some(Sleep::EndOfTrack) => l!("Таймер сна: после этого трека", "Sleep timer: after this track").into(),
                    None => l!("Таймер сна", "Sleep timer").into(),
                });
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(170.0);
                    ui.label(RichText::new(l!("Выключить музыку", "Turn off the music")).color(th().muted).size(12.0));
                    for m in [15u64, 30, 45, 60, 90] {
                        if ui.button(lf!("через {m} мин", "in {m} min")).clicked() { self.sleep = Some(Sleep::At(Instant::now() + Duration::from_secs(m * 60))); ui.close(); }
                    }
                    if ui.button(l!("после этого трека", "after this track")).clicked() { self.sleep = Some(Sleep::EndOfTrack); ui.close(); }
                    if self.sleep.is_some() && ui.button(RichText::new(l!("отменить таймер", "cancel the timer")).color(BAD)).clicked() { self.sleep = None; ui.close(); }
                });
                if icon_btn(ui, "\u{E660}", 34.0, self.show_lyrics).on_hover_text(l!("Текст песни (T)", "Lyrics (T)")).clicked() { self.show_lyrics = !self.show_lyrics; if self.show_lyrics { self.show_queue = false; } }
                if icon_btn(ui, "\u{E6AC}", 34.0, self.show_queue).on_hover_text(l!("Очередь (Q)", "Queue (Q)")).clicked() { self.show_queue = !self.show_queue; if self.show_queue { self.show_lyrics = false; } }
                if let Some(t) = self.current.clone().filter(|t| t.anime.is_none() && self.store.lock().unwrap().lib.tracks.contains_key(&t.id)) {
                    let on = self.store.lock().unwrap().is_liked(&t.id);
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                    if resp.hovered() { ui.painter().circle_filled(r.center(), 17.0, th().bg3); }
                    glyph(ui.painter(), r.center(), "\u{E2A8}", FontId::new(17.0, if on { egui::FontFamily::Name("fill".into()) } else { egui::FontFamily::Proportional }), if on { th().accent } else if resp.hovered() { th().text } else { th().muted });
                    if resp.on_hover_text(if on { l!("Убрать из «Любимого» (L)", "Remove from “Liked” (L)") } else { l!("В «Любимое» (L)", "Add to “Liked” (L)") }).clicked() { acts.push(Act::Like(vec![t.id.clone()], !on)); }
                }
                if self.current.is_some() && icon_btn(ui, "\u{E802}", 34.0, false).on_hover_text(l!("Что это играет? (Shazam) — например, опенинг в аниме", "What's playing? (Shazam) — e.g. an anime opening")).clicked() { acts.push(Act::Shazam); }
            });
        });
    }

    /// Viewer: video (a YouTube clip or a local file) or the large cover.
    fn video_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click());
        let p = ui.painter().clone();
        let area = if self.fullscreen { rect } else { rect.shrink2(Vec2::new(0.0, 4.0)).with_max_y(rect.bottom() - 16.0) };
        if !self.fullscreen { p.rect_filled(area, 14, Color32::BLACK); }
        let Some(t) = self.current.clone() else {
            p.text(area.center(), Align2::CENTER_CENTER, l!("Ничего не играет", "Nothing is playing"), FontId::new(18.0, semi()), th().muted);
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
            // large cover (while the video loads, on error, or if there's no video)
            let side = (area.height() * 0.62).min(area.width() * 0.6).max(80.0);
            let cr = Rect::from_center_size(area.center() - Vec2::new(0.0, 36.0), Vec2::splat(side));
            match t.cover.clone().and_then(|c| self.thumbs.get(&c, 512)) {
                Some(tx) => { egui::Image::new(&tx).corner_radius(14).paint_at(ui, cr); }
                None => { p.rect_filled(cr, 14, th().bg3); p.text(cr.center(), Align2::CENTER_CENTER, "\u{E340}", FontId::proportional(side * 0.35), th().muted); }
            }
            let y = cr.bottom() + 16.0;
            p.text(Pos2::new(area.center().x, y), Align2::CENTER_TOP, ellipsize(&t.title, area.width() - 40.0, 20.0), FontId::new(20.0, bold()), th().text);
            p.text(Pos2::new(area.center().x, y + 28.0), Align2::CENTER_TOP, ellipsize(wavy_core::i18n::tr(&t.artist), area.width() - 40.0, 14.0), FontId::proportional(14.0), th().muted);
            let note = if let Some(e) = err.as_ref().filter(|e| e.starts_with(wavy_core::ffmpeg::NEED)) {
                let what = wavy_core::ffmpeg::human(e);
                match self.ff.state() {
                    wavy_core::ffmpeg::State::Downloading(p) => Some((lf!("{what}: скачиваю декодер (≈6 МБ, один раз)… {p}%", "{what}: downloading the decoder (≈6 MB, once)… {p}%"), th().muted)),
                    wavy_core::ffmpeg::State::Failed(_) => Some((lf!("Видео не открылось: {what}", "Video didn't open: {what}"), BAD)),
                    _ => Some((lf!("{what}: готовлю декодер…", "{what}: preparing the decoder…"), th().muted)),
                }
            }
            else if let Some(e) = &err { Some((lf!("Видео не открылось: {e}", "Video didn't open: {e}"), BAD)) }
                else if t.has_video() {
                    let pct = self.video_dl.as_ref().filter(|v| v.0 == t.id).and_then(|v| v.1.progress());
                    Some((pct.map(|p| lf!("Загружаю видео… {p}%", "Loading video… {p}%")).unwrap_or_else(|| l!("Загружаю видео…", "Loading video…").into()), th().muted))
                }
                else { None };
            if let Some((n, c)) = note {
                p.text(Pos2::new(area.center().x, y + 54.0), Align2::CENTER_TOP, ellipsize(&n, area.width() - 40.0, 13.0), FontId::proportional(13.0), c);
                if err.as_ref().is_none_or(|e| e.starts_with(wavy_core::ffmpeg::NEED)) { ui.ctx().request_repaint_after(Duration::from_millis(200)); }
            }
        }
        // dub / subtitles / Shazam — always in windowed mode, in full screen only after the mouse moved
        let moved = ui.input(|i| i.pointer.time_since_last_movement()) < 2.5;
        if !self.fullscreen || moved { self.video_controls(ui, area, &t, acts); }
        if t.anime.is_some() { self.anime_overlay(ui, area, acts); }
        // "full screen" button and hint
        let btn = Rect::from_min_size(area.right_top() + Vec2::new(-46.0, 12.0), Vec2::splat(34.0));
        let b = ui.interact(btn, Id::unique("fs-btn"), Sense::click());
        p.circle_filled(btn.center(), 17.0, Color32::from_black_alpha(if b.hovered() { 200 } else { 120 }));
        p.text(btn.center(), Align2::CENTER_CENTER, if self.fullscreen { "\u{E1CE}" } else { "\u{E1D0}" }, FontId::proportional(17.0), Color32::WHITE);
        if b.on_hover_text(if self.fullscreen { l!("Выйти из полноэкранного (Esc)", "Exit fullscreen (Esc)") } else { l!("Во весь экран (двойной клик)", "Fullscreen (double click)") }).clicked() {
            acts.push(Act::Fullscreen(!self.fullscreen));
        } else if resp.double_clicked() {
            // the first click of the double-click already toggled pause — restore it
            acts.push(Act::TogglePlay);
            acts.push(Act::Fullscreen(!self.fullscreen));
        } else if resp.clicked() {
            acts.push(Act::TogglePlay);
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        let _ = resp;
        if self.fullscreen {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) { acts.push(Act::Fullscreen(false)); }
            // title and progress — only after the mouse moved
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

    /// Buttons over the video: dub and subtitle choice, Shazam.
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
            {
                let cur = self.sub_sel.and_then(|i| info.subs.get(i)).map(|s| s.label.clone()).unwrap_or_else(|| l!("выкл", "off").into());
                let resp = ui.add(chip(format!("\u{E1A8}  {}", ellipsize(&cur, 180.0, 13.0))));
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(240.0);
                    let mark = |on: bool, l: &str| if on { format!("\u{E182}  {l}") } else { format!("      {l}") };
                    if ui.add(egui::Button::new(mark(self.sub_sel.is_none(), l!("Без субтитров", "No subtitles"))).frame(false)).clicked() { acts.push(Act::Subs(None)); ui.close(); }
                    for (i, s) in info.subs.iter().enumerate() {
                        let l = if s.forced { lf!("{} (надписи)", "{} (signs)", s.label) } else { s.label.clone() };
                        if ui.add(egui::Button::new(mark(self.sub_sel == Some(i), &l)).frame(false)).clicked() { acts.push(Act::Subs(Some(i))); ui.close(); }
                    }
                    ui.separator();
                    if ui.add(egui::Button::new(l!("      Открыть файл субтитров…", "      Open a subtitle file…")).frame(false)).on_hover_text(l!(".ass, .srt или .vtt с любым именем", ".ass, .srt or .vtt with any name")).clicked() { acts.push(Act::PickSubs); ui.close(); }
                });
            }
        }
        if ui.add(chip(l!("\u{E10E}  Кадр", "\u{E10E}  Frame").into())).on_hover_text(l!("Сохранить кадр в «Изображения/Wavy» (клавиша P)", "Save the frame to “Pictures/Wavy” (P key)")).clicked() { acts.push(Act::Screenshot); }
        if ui.add(chip(l!("\u{E802}  Что за песня?", "\u{E802}  What's this song?").into())).on_hover_text(l!("Shazam: узнать опенинг/эндинг и добавить в библиотеку", "Shazam: recognize the opening/ending and add it to the library")).clicked() { acts.push(Act::Shazam); }
    }

    /// Anime episode: load opening/ending segments, skip them, offer the next episode.
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
            // ending before the next episode — offer it; otherwise (opening etc.) skip automatically if enabled
            if s.is_ed() && has_next && st.anime_next {
                if inside && self.next_ep.is_none() && self.stay_ep.as_deref() != Some(t.id.as_str()) && self.audio.is_playing() {
                    self.next_ep = Some((t.id.clone(), Instant::now() + Duration::from_secs(10)));
                }
            } else if inside && st.anime_autoskip && self.audio.is_playing() {
                let key = format!("{}:{}", t.id, s.start as i64);
                if self.auto_skipped.insert(key) { self.audio.seek(s.end); if let Some((_, v)) = &mut self.video { v.seek(s.end); } self.toast(lf!("Пропустил {}", "Skipped {}", s.label()), false); }
            }
        }
        if let Some((id, at)) = self.next_ep.clone() {
            if id != t.id { self.next_ep = None; }
            else if Instant::now() >= at { self.next_ep = None; self.next(false, ctx); }
            else { ctx.request_repaint_after(Duration::from_millis(250)); }
        }
    }

    /// Twitch chat (requests) and the cache limit.
    fn stream_tick(&mut self, ctx: &egui::Context) {
        self.overlay_extra(ctx);
        self.backup_tick(false);
        let st = self.settings();
        let acc = Some(st.twitch_account.clone()).filter(|a| a.logged_in());
        let want = (st.twitch_sr && !st.twitch_channel.trim().is_empty())
            .then(|| format!("{}|{}", wavy_core::twitch::clean_channel(&st.twitch_channel), acc.as_ref().map(|a| a.login.as_str()).unwrap_or("")));
        if self.twitch.as_ref().map(|t| &t.0) != want.as_ref() {
            if let Some((_, stop, _)) = self.twitch.take() { stop.store(true, std::sync::atomic::Ordering::Relaxed); }
            if let Some(key) = want {
                let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let (tx, c) = (self.sr_tx.clone(), ctx.clone());
                let (btx, c2) = (self.tx.clone(), ctx.clone());
                let chat = wavy_core::twitch::listen(&st.twitch_channel, acc, stop.clone(),
                    move |m| { let _ = tx.send(m); c.request_repaint(); },
                    move |a| { let _ = btx.send(Bg::TwitchToken(a)); c2.request_repaint(); });
                self.twitch = Some((key, stop, chat));
            }
        }
        let msgs: Vec<wavy_core::twitch::Msg> = self.sr_rx.try_iter().collect();
        for m in msgs { self.chat_message(m, ctx); }
        // Wi-Fi sync with the phone
        let want = (st.wifi_sync && !st.wifi_pin.is_empty()).then(|| st.wifi_pin.clone());
        if self.wifi.as_ref().map(|w| &w.0) != want.as_ref() {
            self.wifi = None;
            if let Some(pin) = want {
                let (tx, c) = (self.tx.clone(), ctx.clone());
                let c2 = ctx.clone();
                let remote = wavy_core::wifisync::Remote { state: self.remote.0.clone(), cover: self.remote.1.clone(), cmds: self.remote.2.clone(), wake: Arc::new(move || c2.request_repaint()) };
                match wavy_core::wifisync::start(self.store.clone(), pin.clone(), remote, move |m| { let _ = tx.send(Bg::Status(m)); c.request_repaint(); }) {
                    Ok(srv) => self.wifi = Some((pin, srv)),
                    Err(e) => { self.toast(lf!("Синхронизация по Wi-Fi: {e}", "Wi-Fi sync: {e}"), true); self.store.lock().unwrap().patch_settings(&json!({"wifiSync": false})); }
                }
            }
        }
        // cache limit: check every 10 minutes
        if st.cache_limit_gb > 0 && self.cache_trimmed.elapsed() > Duration::from_secs(600) {
            self.cache_trimmed = Instant::now();
            let keep: Vec<PathBuf> = [self.queue.get(self.qi), self.queue.get(self.qi + 1)].into_iter().flatten().filter_map(|id| self.track(id))
                .filter_map(|t| t.url.map(|u| self.cache.join(wavy_core::sha16(u.as_bytes())))).collect();
            let (dir, limit) = (self.cache.clone(), st.cache_limit_gb as u64 * 1_073_741_824);
            std::thread::spawn(move || { let f = wavy_core::stream::trim_dir(&dir, limit, &keep); if f > 0 { wavy_core::log(&format!("кэш: удалено {} МБ старого", f / 1_048_576)); } });
        }
    }

    /// Command palette (Ctrl+K): type "equalizer", "mini", a track or playlist name — Enter.
    fn palette_ui(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        enum Cmd { Act(Act), Lyrics, Queue, Settings, Sound }
        let (mut q, mut sel) = self.palette.clone().unwrap_or_default();
        let ql = q.trim().to_lowercase();
        let words: Vec<&str> = ql.split_whitespace().collect();
        let fits = |label: &str, keys: &str| { let l = format!("{label} {keys}").to_lowercase(); words.iter().all(|w| l.contains(w)) };
        // (icon, name, caption, what to do)
        let mut items: Vec<(&str, String, String, Cmd)> = Vec::new();
        let mut add = |icon: &'static str, label: &str, hint: &str, keys: &str, c: Cmd| { if fits(label, keys) { items.push((icon, label.to_string(), hint.to_string(), c)); } };
        let playing = self.audio.is_playing();
        add("\u{E3D0}", if playing { l!("Пауза", "Pause") } else { l!("Играть", "Play") }, l!("Пробел", "Space"), "pause play пауза играть стоп", Cmd::Act(Act::TogglePlay));
        add("\u{E5A6}", wavy_core::i18n::tr("Следующий трек"), "Ctrl+→", "next skip дальше", Cmd::Act(Act::Next));
        add("\u{E5A4}", wavy_core::i18n::tr("Предыдущий трек"), "Ctrl+←", "prev назад", Cmd::Act(Act::Prev));
        add("\u{E422}", l!("Перемешать", "Shuffle"), "S", "shuffle", Cmd::Act(Act::Shuffle));
        add("\u{E40E}", l!("Повтор", "Repeat"), "R", "repeat", Cmd::Act(Act::Repeat));
        add("\u{E1D8}", l!("Звук: эквалайзер, скорость, кроссфейд", "Sound: equalizer, speed, crossfade"), "", "eq эквалайзер nightcore скорость громкость нормализация", Cmd::Sound);
        add("\u{E8F4}", l!("Текст песни", "Lyrics"), "T", "lyrics слова", Cmd::Lyrics);
        add("\u{E106}", l!("Очередь", "Queue"), "Q", "queue", Cmd::Queue);
        add("\u{E594}", wavy_core::i18n::tr("Мини-плеер"), "", "mini поверх", Cmd::Act(Act::ToggleMini));
        add("\u{E466}", wavy_core::i18n::tr("Оверлей поверх игры"), "Ctrl+Shift+O", "overlay obs", Cmd::Act(Act::ToggleOverlay));
        add("\u{E340}", l!("Библиотека", "Library"), "", "library треки", Cmd::Act(Act::View(View::Library)));
        add("\u{E2A8}", l!("Любимое", "Liked"), "", "liked лайки", Cmd::Act(Act::View(View::Liked)));
        add("\u{E150}", l!("Итоги", "Recap"), "", "stats статистика топ", Cmd::Act(Act::View(View::Stats)));
        add("\u{E754}", l!("Аниме", "Anime"), "", "anime серии", Cmd::Act(Act::View(View::Anime)));
        add("\u{EA20}", l!("Добавить файлы", "Add files"), "", "import импорт музыка", Cmd::Act(Act::ImportFiles));
        add("\u{E256}", l!("Добавить папку", "Add a folder"), "", "import импорт folder", Cmd::Act(Act::ImportFolder));
        add("\u{E2E2}", l!("Импорт по ссылке", "Import by link"), "Ctrl+V", "youtube spotify url link плейлист", Cmd::Act(Act::Modal(Modal::ImportUrl { url: String::new(), into: false, busy: false, err: None })));
        if self.acc == AccState::On { add("\u{E802}", l!("Моя волна", "My wave"), "", "wave волна радио рекомендации", Cmd::Act(Act::Wave(false))); }
        add("\u{E272}", l!("Настройки", "Settings"), "", "settings discord twitch last.fm хоткеи бэкап", Cmd::Settings);
        add("\u{E036}", l!("Проверить обновления", "Check for updates"), "", "update версия", Cmd::Act(Act::CheckUpdate));
        for (rule, name, hint) in wavy_core::store::SMART { add("\u{E5FE}", wavy_core::i18n::tr(name), wavy_core::i18n::tr(hint), "умный smart", Cmd::Act(Act::View(View::Smart(rule.to_string())))); }
        if !words.is_empty() {
            let s = self.store.lock().unwrap();
            for p in &s.lib.playlists { if fits(&p.name, l!("плейлист", "playlist")) { items.push(("\u{E6AA}", p.name.clone(), lf!("плейлист · {}", "playlist · {}", p.track_ids.len()), Cmd::Act(Act::View(View::Playlist(p.id.clone()))))); } }
            for a in &s.lib.anime { if fits(a.display(), &a.name) { items.push(("\u{E754}", a.display().to_string(), l!("аниме", "anime").into(), Cmd::Act(Act::View(View::AnimeTitle(a.id.clone()))))); } }
            if ql.chars().count() >= 2 {
                let mut n = 0;
                for t in s.lib.tracks.values().filter(|t| t.anime.is_none()) {
                    if n >= 8 { break; }
                    if fits(&t.title, &t.artist) { items.push(("\u{E3D0}", t.title.clone(), t.artist.clone(), Cmd::Act(Act::Play(vec![t.id.clone()], 0, false)))); n += 1; }
                }
            }
            items.push(("\u{E30C}", lf!("Найти на YouTube: {}", "Search YouTube: {}", q.trim()), String::new(), Cmd::Act(Act::Search(q.trim().to_string()))));
        }
        sel = sel.min(items.len().saturating_sub(1));
        let mut chosen: Option<usize> = None;
        let mut close = false;
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) { sel = (sel + 1).min(items.len().saturating_sub(1)); }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) { sel = sel.saturating_sub(1); }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) && !items.is_empty() { chosen = Some(sel); }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) { close = true; }
        });
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("palette")).order(egui::Order::Foreground).fixed_pos(Pos2::new(screen.center().x - 280.0, screen.top() + 90.0)).show(ctx, |ui| {
            egui::Frame::new().fill(th().bg2).stroke(Stroke::new(1.0, th().line)).corner_radius(16).inner_margin(12).shadow(egui::epaint::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(160) }).show(ui, |ui| {
                ui.set_width(560.0);
                let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text(l!("Команда, трек, плейлист…  (Esc — закрыть)", "Command, track, playlist…  (Esc — close)")).desired_width(f32::INFINITY).font(FontId::new(17.0, egui::FontFamily::Proportional)).frame(egui::Frame::NONE));
                r.request_focus();
                if r.changed() { sel = 0; }
                ui.separator();
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    if items.is_empty() { ui.label(RichText::new(l!("Ничего не нашлось", "Nothing found")).color(th().muted)); }
                    for (i, (icon, label, hint, _)) in items.iter().enumerate() {
                        let on = i == sel;
                        let resp = egui::Frame::new().fill(if on { th().bg3 } else { Color32::TRANSPARENT }).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(*icon).color(if on { th().text } else { th().muted }));
                                ui.add(egui::Label::new(RichText::new(label).color(th().text)).truncate());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| { ui.label(RichText::new(hint).color(th().muted).size(12.0)); });
                            });
                        }).response.interact(Sense::click());
                        if on { resp.scroll_to_me(None); }
                        if resp.clicked() { chosen = Some(i); }
                        if resp.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) { sel = i; }
                    }
                });
            });
        });
        if let Some(i) = chosen {
            close = true;
            match items.swap_remove(i).3 {
                Cmd::Act(a) => {
                    if let Act::Search(_) = a { acts.push(Act::View(View::Search)); }
                    acts.push(a);
                }
                Cmd::Lyrics => { self.show_lyrics = true; self.show_queue = false; }
                Cmd::Queue => { self.show_queue = true; self.show_lyrics = false; }
                Cmd::Settings => { let s = self.settings(); acts.push(Act::Modal(Modal::Settings { client_id: s.discord.client_id, port: s.overlay.port.to_string(), sp_id: s.spotify.client_id, sp_secret: s.spotify.client_secret })); }
                Cmd::Sound => { self.sound_open = true; }
            }
        }
        self.palette = if close { None } else { Some((q, sel)) };
    }

    /// Library auto-backup once a day (or now, if `force`).
    fn backup_tick(&mut self, force: bool) {
        let st = self.settings();
        let now = wavy_core::now_ms();
        if !force && now.saturating_sub(st.last_backup) < 86_400_000 { return; }
        if !force && self.started.elapsed() < Duration::from_secs(60) { return; } // don't slow down startup
        self.store.lock().unwrap().patch_settings(&json!({"lastBackup": now}));
        let dir = if st.backup_dir.is_empty() { wavy_core::data_dir().join("backups") } else { PathBuf::from(&st.backup_dir) };
        let d = fmt_date(now); // 21.11.2026 → 2026-11-21 (so backups sort by date)
        let p: Vec<&str> = d.split('.').collect();
        let name = format!("wavy-backup-{}-{}-{}.json", p[2], p[1], p[0]);
        let (store, tx) = (self.store.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let r = store.lock().unwrap().backup(&dir, &name, 14);
            match r {
                Ok(p) => { wavy_core::log(&format!("бэкап: {}", p.display())); if force { let _ = tx.send(Bg::Status(lf!("Копия сохранена: {}", "Backup saved: {}", p.display()))); } }
                Err(e) => { wavy_core::log(&format!("бэкап не удался: {e}")); let _ = tx.send(Bg::Status(lf!("Резервная копия не сохранилась: {e}", "Backup wasn't saved: {e}"))); }
            }
        });
    }

    /// Last.fm: "now playing" when a track starts, scrobble after half (or 4 minutes).
    fn lastfm_tick(&mut self) {
        let acc = self.settings().lastfm;
        if !acc.ready() || !self.audio.is_playing() { return; }
        let Some(t) = self.current.clone().filter(|t| t.anime.is_none()) else { return };
        let pos = self.audio.position();
        let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
        let send = |acc: wavy_core::lastfm::Account, t: Track, dur: f64, scrobble: Option<u64>| {
            std::thread::spawn(move || {
                let p = wavy_core::lastfm::Play { artist: &t.artist, title: &t.title, album: &t.album, duration: dur };
                let r = match scrobble { Some(ts) => wavy_core::lastfm::scrobble(&acc, &p, ts), None => wavy_core::lastfm::now_playing(&acc, &p) };
                if let Err(e) = r { wavy_core::log(&format!("last.fm «{}»: {e}", t.title)); }
            });
        };
        if self.lfm_np.as_deref() != Some(t.id.as_str()) {
            self.lfm_np = Some(t.id.clone());
            self.lfm_started = wavy_core::now_ms() / 1000 - pos as u64;
            send(acc.clone(), t.clone(), dur, None);
        }
        if self.lfm_done.as_deref() != Some(t.id.as_str()) && wavy_core::lastfm::due(pos, dur) {
            self.lfm_done = Some(t.id.clone());
            send(acc, t, dur, Some(self.lfm_started));
        }
    }

    /// Write to Twitch chat (if signed in).
    fn say(&self, text: &str) {
        if let Some((_, _, chat)) = &self.twitch { chat.say(text); }
    }

    fn track_name(t: &Track) -> String { if t.artist.is_empty() { t.title.clone() } else { format!("{} — {}", t.artist, t.title) } }

    /// A chat message: request, !skip, !song, !queue, channel points reward.
    fn chat_message(&mut self, m: wavy_core::twitch::Msg, ctx: &egui::Context) {
        let st = self.settings();
        // reward binding: the first redeemed reward with text becomes the "song request" one
        if let Some(r) = m.reward.as_ref().filter(|_| self.reward_bind) {
            self.reward_bind = false;
            self.store.lock().unwrap().patch_settings(&json!({"twitchReward": r}));
            self.toast(l!("Награда привязана: теперь она заказывает трек", "Reward linked: now it requests a track"), false);
            return;
        }
        let by_reward = m.reward.is_some() && m.reward.as_deref() == Some(st.twitch_reward.as_str()) && !st.twitch_reward.is_empty();
        let query = if by_reward { Some(m.text.clone()) } else { m.command("!sr") };
        if let Some(q) = query {
            if q.is_empty() { self.say(&lf!("@{} напиши: !sr ссылка или название", "@{} type: !sr link or title", m.user)); return; }
            // at most once per 30 s per person (channel points have no limit — they cost points anyway)
            if !by_reward && !m.is_mod && self.sr_last.get(&m.user).is_some_and(|t| t.elapsed() < Duration::from_secs(30)) { return; }
            self.sr_last.insert(m.user.clone(), Instant::now());
            let (yt, tx, c, user) = (self.yt.clone(), self.tx.clone(), ctx.clone(), m.user.clone());
            std::thread::spawn(move || { let _ = tx.send(Bg::SongRequest(user, yt.lookup(&q))); c.request_repaint(); });
            return;
        }
        let cur = self.current.clone();
        if m.command("!skip").is_some() {
            let Some(t) = cur else { return };
            if m.is_mod { self.say(&lf!("Пропускаю «{}»", "Skipping “{}”", t.title)); self.apply(Act::Next, ctx); return; }
            let need = st.twitch_skip_votes;
            if need == 0 { return; }
            if self.skip_vote.vote(&t.id, &m.user, need) {
                self.say(&lf!("Чат решил: пропускаю «{}»", "Chat decided: skipping “{}”", t.title));
                self.toast(lf!("Чат пропустил «{}»", "Chat skipped “{}”", t.title), false);
                self.apply(Act::Next, ctx);
            } else {
                self.say(&format!("!skip {}/{need}", self.skip_vote.count(&t.id)));
            }
            return;
        }
        if m.command("!song").is_some() {
            match cur {
                Some(t) => {
                    let link = t.video_id.as_ref().map(|v| format!(" · youtu.be/{v}")).unwrap_or_default();
                    self.say(&lf!("Сейчас играет: {}{link}", "Now playing: {}{link}", Self::track_name(&t)));
                }
                None => self.say(l!("Сейчас ничего не играет", "Nothing is playing right now")),
            }
            return;
        }
        if m.command("!queue").is_some() {
            let next: Vec<String> = self.queue.iter().skip(self.qi + 1).take(3).filter_map(|id| self.track(id)).map(|t| Self::track_name(&t)).collect();
            self.say(&if next.is_empty() { l!("Очередь пуста — закажи: !sr название", "The queue is empty — request one: !sr title").to_string() } else { lf!("Дальше: {}", "Up next: {}", next.join(" | ")) });
        }
    }

    /// Wait for the user to enter the code at twitch.tv/activate.
    fn twitch_poll(&self, d: wavy_core::twitch::Device, ctx: &egui::Context) {
        let (tx, c, id) = (self.tx.clone(), ctx.clone(), self.settings().twitch_account.client_id);
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let r = loop {
                std::thread::sleep(Duration::from_secs(d.interval.max(2)));
                match wavy_core::twitch::device_poll(&id, &d) {
                    Ok(Some(a)) => break Ok(a),
                    Ok(None) if t0.elapsed() < Duration::from_secs(600) => continue,
                    Ok(None) => break Err(l!("Twitch: код не ввели за 10 минут", "Twitch: the code wasn't entered within 10 minutes").to_string()),
                    Err(e) => break Err(e),
                }
            };
            let _ = tx.send(Bg::TwitchLogin(r));
            c.request_repaint();
        });
    }

    fn song_request(&mut self, user: String, r: Result<Track, String>, ctx: &egui::Context) {
        let t = match r { Ok(t) => t, Err(e) => { wavy_core::log(&format!("заказ от {user}: {e}")); self.say(&format!("@{user} не нашёл такой трек")); return; } };
        let max = self.settings().twitch_max_min as f64 * 60.0;
        if max > 0.0 && t.duration > max { self.toast(lf!("{user} заказал слишком длинный трек ({})", "{user} requested a track that's too long ({})", fmt_time(t.duration)), true); self.say(&lf!("@{user} трек длиннее {} мин — не беру", "@{user} the track is longer than {} min — not taking it", max as u32 / 60)); return; }
        if self.queue.iter().skip(self.qi).any(|q| *q == t.id) { self.say(&lf!("@{user} «{}» уже в очереди", "@{user} “{}” is already in the queue", t.title)); return; }
        self.toast(lf!("\u{E340}  {user} заказал: {} — {}", "\u{E340}  {user} requested: {} — {}", t.artist, t.title), false);
        let id = t.id.clone();
        self.radio.retain(|x| x.id != id);
        self.radio.push(t);
        self.sr_by.insert(id.clone(), user.clone());
        if self.current.is_none() || self.queue.is_empty() {
            if let Some(t) = self.track(&id) { self.say(&lf!("@{user} включаю: {}", "@{user} playing: {}", Self::track_name(&t))); }
            self.play_list(vec![id], 0, false, ctx);
            return;
        }
        // after the current one and those already requested — in order
        let mut at = self.qi + 1;
        while at < self.queue.len() && self.sr_by.contains_key(&self.queue[at]) { at += 1; }
        self.queue.insert(at, id);
        let ahead = at - self.qi - 1;
        let name = self.track(&self.queue[at]).map(|t| Self::track_name(&t)).unwrap_or_default();
        self.say(&lf!("@{user} добавил: {name}{}", "@{user} added: {name}{}", if ahead == 0 { l!(" — играет следующим", " — plays next").to_string() } else { lf!(" — через {ahead}", " — in {ahead}") }));
    }

    /// Mark on Shikimori how many episodes of the title were watched.
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

    /// "Skip opening" button and the "Next episode in…" card over the video.
    fn anime_overlay(&mut self, ui: &mut egui::Ui, area: Rect, acts: &mut Vec<Act>) {
        let pos = self.audio.position();
        let p = ui.painter().clone();
        let corner = area.right_bottom() - Vec2::new(24.0, 90.0);
        if let Some((_, at)) = &self.next_ep {
            let left = at.saturating_duration_since(Instant::now()).as_secs() + 1;
            // top right — subtitles are at the bottom
            let top = area.right_top() + Vec2::new(-24.0, 60.0);
            let card = Rect::from_min_max(Pos2::new(top.x - 300.0, top.y), Pos2::new(top.x, top.y + 80.0));
            p.rect_filled(card, 14, Color32::from_black_alpha(220));
            p.rect_stroke(card, 14, Stroke::new(1.0, th().line), egui::StrokeKind::Inside);
            p.text(card.left_top() + Vec2::new(16.0, 14.0), Align2::LEFT_TOP, lf!("Следующая серия через {left} с", "Next episode in {left} s"), FontId::new(15.0, semi()), th().text);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink2(Vec2::new(14.0, 10.0)).with_min_y(card.top() + 40.0)).layout(Layout::left_to_right(Align::Center)));
            if pill(&mut c, l!("Смотреть", "Watch"), true).clicked() { self.next_ep = None; acts.push(Act::Next); }
            if pill(&mut c, l!("Остаться", "Stay"), false).clicked() { self.stay_ep = self.current.as_ref().map(|t| t.id.clone()); self.next_ep = None; }
            return;
        }
        let skip = self.skips.as_ref().and_then(|s| s.1.iter().find(|k| pos >= k.start - 0.5 && pos < k.end - 2.0).cloned());
        let (label, to) = match skip {
            Some(k) => (lf!("\u{E5A6}  Пропустить {}", "\u{E5A6}  Skip {}", k.label()), k.end),
            // no data — a typical opening lasts 1:30, offer a jump at the start of the episode
            None if self.skips.as_ref().is_some_and(|s| s.1.is_empty()) && pos > 1.0 && pos < 240.0 => (l!("\u{E5A6}  +85 с", "\u{E5A6}  +85 s").to_string(), pos + 85.0),
            None => return,
        };
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(corner - Vec2::new(300.0, 40.0), corner)).layout(Layout::right_to_left(Align::Max)));
        if pill(&mut c, &label, true).clicked() { acts.push(Act::Seek(to)); }
    }

    fn stats_page(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let day = 86_400_000u64;
        let now = wavy_core::now_ms();
        let periods = [(l!("За месяц", "Last month"), now.saturating_sub(30 * day)), (l!("За год", "Last year"), now.saturating_sub(365 * day)), (l!("За всё время", "All time"), 0)];
        let (st, first) = {
            let s = self.store.lock().unwrap();
            (s.stats(periods[self.stats_period as usize].1), s.lib.history.first().map(|h| h.1))
        };
        ui.label(RichText::new(l!("ТВОЯ МУЗЫКА", "YOUR MUSIC")).size(11.0).color(th().muted));
        ui.label(RichText::new(l!("Итоги", "Recap")).size(34.0).family(bold()).color(th().text));
        ui.horizontal(|ui| {
            for (i, (name, _)) in periods.iter().enumerate() {
                if ui.selectable_label(self.stats_period == i as u8, *name).clicked() { self.stats_period = i as u8; }
            }
        });
        ui.add_space(14.0);
        if st.plays == 0 {
            ui.label(RichText::new(l!("Пока пусто: прослушивание засчитывается, если трек играл дольше 30 секунд.", "Nothing yet: a play counts if the track played longer than 30 seconds.")).color(th().muted));
            return;
        }
        ui.horizontal(|ui| {
            let big = |ui: &mut egui::Ui, n: String, label: &str| {
                egui::Frame::new().fill(th().bg3).corner_radius(14).inner_margin(egui::Margin::symmetric(20, 14)).show(ui, |ui| {
                    ui.vertical(|ui| { ui.label(RichText::new(n).size(30.0).family(bold()).color(th().text)); ui.label(RichText::new(label).color(th().muted)); });
                });
            };
            big(ui, st.plays.to_string(), l!("прослушиваний", "plays"));
            let m = st.minutes.round() as u64;
            big(ui, if m >= 120 { lf!("{} ч {} мин", "{} h {} min", m / 60, m % 60) } else { lf!("{m} мин", "{m} min") }, l!("музыки", "of music"));
            if let Some((a, _)) = st.top_artists.first() { big(ui, a.clone(), l!("артист №1", "top artist")); }
        });
        if pill(ui, l!("\u{E10E}  Картинка для соцсетей", "\u{E10E}  Image for social media"), false).on_hover_text(l!("PNG 1080×1350 в «Изображения/Wavy»", "PNG 1080×1350 in “Pictures/Wavy”")).clicked() {
            let m = st.minutes.round() as u64;
            let year = fmt_date(now).rsplit('.').next().unwrap_or("").to_string();
            let tracks: Vec<(String, String, Option<String>)> = st.top_tracks.iter().take(5).filter_map(|(id, n)| self.track(id).map(|t| (t.title.clone(), lf!("{} · {n} раз", "{} · {n} times", t.artist), t.cover.clone()))).collect();
            let r = ([l!("Месяц", "Month").to_string(), year, l!("Всё время", "All time").to_string()][self.stats_period as usize].clone(),
                if m >= 120 { lf!("{} ч", "{} h", m / 60) } else { lf!("{m} мин", "{m} min") }, st.plays, st.top_artists.first().map(|a| a.0.clone()), tracks, st.top_artists.clone());
            let (tx, c) = (self.tx.clone(), ui.ctx().clone());
            std::thread::spawn(move || {
                let tmp = std::env::temp_dir().join("wavy-recap");
                let _ = std::fs::create_dir_all(&tmp);
                // covers from YouTube — download temporarily
                let tracks = r.4.into_iter().enumerate().map(|(i, (title, sub, cover))| {
                    let cover = cover.and_then(|c| if c.starts_with("http") {
                        let p = tmp.join(format!("{i}.jpg"));
                        wavy_core::http().get(&c).call().ok().and_then(|mut resp| resp.body_mut().read_to_vec().ok()).and_then(|b| std::fs::write(&p, b).ok()).map(|_| p)
                    } else { Some(PathBuf::from(c)) });
                    recap::Entry { title, sub, cover }
                }).collect();
                let img = recap::render(&recap::Recap { period: r.0.clone(), hours: r.1, plays: r.2, top_artist: r.3, tracks, artists: r.5 }, load_cjk_fonts().into_iter().next().map(|f| f.1));
                let path = wavy_core::pictures_dir().join(lf!("Итоги Wavy · {} · {}.png", "Wavy Recap · {} · {}.png", r.0, fmt_date(wavy_core::now_ms())));
                let msg = match img.save(&path) { Ok(()) => { open_folder(&path); lf!("Картинка сохранена: {}", "Image saved: {}", path.display()) } Err(e) => lf!("Не сохранилось: {e}", "Not saved: {e}") };
                let _ = tx.send(Bg::Status(msg));
                c.request_repaint();
            });
        }
        if let Some(f) = first { ui.label(RichText::new(lf!("История ведётся с {}", "History is kept since {}", fmt_date(f))).size(11.5).color(th().muted)); }
        ui.add_space(16.0);
        ui.columns_const(|[l, r]| {
            l.label(RichText::new(l!("Топ треков", "Top tracks")).size(16.0).family(semi()));
            l.add_space(6.0);
            let ids: Vec<String> = st.top_tracks.iter().map(|x| x.0.clone()).collect();
            for (i, (id, n)) in st.top_tracks.iter().enumerate() {
                let Some(t) = self.track(id) else { continue };
                let resp = l.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}", i + 1)).color(th().muted).size(13.0));
                    self.cover(ui, &t, 40.0);
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(&t.title).color(th().text)).truncate());
                        ui.label(RichText::new(lf!("{} · {n} раз", "{} · {n} times", t.artist)).color(th().muted).size(12.0));
                    });
                }).response.interact(Sense::click());
                if resp.double_clicked() { acts.push(Act::Play(ids.clone(), i, false)); }
                l.add_space(4.0);
            }
            r.label(RichText::new(l!("Топ артистов", "Top artists")).size(16.0).family(semi()));
            r.add_space(6.0);
            let max = st.top_artists.first().map(|x| x.1).unwrap_or(1) as f32;
            for (a, n) in &st.top_artists {
                r.label(RichText::new(format!("{a} · {n}")).color(th().text));
                let (rect, _) = r.allocate_exact_size(Vec2::new(r.available_width().min(260.0), 4.0), Sense::hover());
                r.painter().rect_filled(rect, 2, th().bg3);
                r.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width() * *n as f32 / max, 4.0)), 2, th().text);
                r.add_space(6.0);
            }
        });
    }

    /// Load the current track's lyrics (for the "Lyrics" panel and the overlay) if not loaded yet.
    fn ensure_lyrics(&mut self, ctx: &egui::Context) {
        let Some(t) = self.current.clone() else { return };
        if self.lyrics.as_ref().map(|l| &l.0) == Some(&t.id) { return; }
        self.lyrics = Some((t.id.clone(), None));
        self.overlay.set_lyrics(serde_json::Value::Null);
        if t.anime.is_some() { return; }
        let (tx, c, dir) = (self.tx.clone(), ctx.clone(), wavy_core::data_dir().join("lyrics"));
        std::thread::spawn(move || { let id = t.id.clone(); let _ = tx.send(Bg::Lyrics(id, wavy_core::lyrics::fetch(&t, &dir))); c.request_repaint(); });
    }

    /// Overlay: what's next in the queue and the lyrics.
    fn overlay_extra(&mut self, ctx: &egui::Context) {
        let ov = self.settings().overlay;
        let queue: Vec<serde_json::Value> = if ov.show_queue {
            self.queue.iter().skip(self.qi + 1).take(3).filter_map(|id| self.track(id).map(|t| {
                let by = self.sr_by.get(id).cloned();
                json!({"title": t.title, "artist": t.artist, "by": by})
            })).collect()
        } else { vec![] };
        self.overlay.set_queue(json!(queue));
        if ov.show_lyrics { self.ensure_lyrics(ctx); }
        if self.wifi.is_some() { self.remote_tick(ctx); }
        self.api_tick(ctx);
    }

    /// Scan the addons folder and start the enabled ones that aren't running.
    fn start_addons(&mut self) {
        self.addon_list = wavy_core::addons::scan(&wavy_core::addons::dir());
        for id in self.settings().addons { if !self.addon_run.is_running(&id) { self.run_addon(&id); } }
    }

    fn run_addon(&mut self, id: &str) {
        let Some(a) = self.addon_list.iter().find(|a| a.id == id).cloned() else { return };
        let api = self.overlay.api_url().unwrap_or_default();
        if let Err(e) = self.addon_run.start(&a, &api, &self.api_token) { self.addon_run.exited.insert(a.id.clone(), e); }
    }

    /// Addon API: run commands from addons, publish the player state a few times a second, notice exited addons.
    fn api_tick(&mut self, ctx: &egui::Context) {
        let cmds: Vec<(String, serde_json::Value)> = self.api_rx.try_iter().collect();
        let cmds_empty = cmds.is_empty();
        for (cmd, v) in cmds {
            let num = v["value"].as_f64().unwrap_or(0.0);
            let act = match cmd.as_str() {
                "toggle" => Act::TogglePlay, "next" => Act::Next, "prev" => Act::Prev,
                "play" => if self.audio.is_playing() { continue } else { Act::TogglePlay },
                "pause" => if self.audio.is_playing() { Act::TogglePlay } else { continue },
                "seek" => Act::Seek(num.max(0.0)),
                "volume" => { self.apply(Act::Volume(num.clamp(0.0, 1.0) as f32), ctx); Act::SaveVolume }
                "jump" if (num.max(0.0) as usize) < self.queue.len() => Act::JumpQueue(num.max(0.0) as usize),
                "like" => match self.current.as_ref() {
                    Some(t) => { let on = v["value"].as_bool().unwrap_or(!self.store.lock().unwrap().is_liked(&t.id)); Act::Like(vec![t.id.clone()], on) }
                    None => continue,
                },
                "add" => match v["url"].as_str().filter(|u| u.starts_with("http")) { Some(u) => Act::ImportUrl(u.to_string(), None), None => continue },
                _ => continue,
            };
            self.apply(act, ctx);
        }
        // after a command, publish right away and once more shortly after, when the player has settled
        // (an idle window doesn't redraw on its own)
        let had_cmds = !cmds_empty;
        if had_cmds { ctx.request_repaint_after(Duration::from_millis(350)); }
        if !had_cmds && self.api_pushed.elapsed() < Duration::from_millis(300) { return; }
        self.api_pushed = Instant::now();
        let port = self.overlay.port.lock().unwrap().unwrap_or(0);
        let cover = |t: &Track| t.cover.as_ref().map(|c| if c.starts_with("http") { c.clone() } else { format!("http://127.0.0.1:{port}/cover/{}", t.id) });
        let t = self.current.clone();
        let duration = if self.audio.duration > 0.0 { self.audio.duration } else { t.as_ref().map(|t| t.duration).unwrap_or(0.0) };
        let s = self.settings();
        let liked = t.as_ref().is_some_and(|t| self.store.lock().unwrap().is_liked(&t.id));
        let queue: Vec<serde_json::Value> = self.queue.iter().enumerate().skip(self.qi).take(50)
            .filter_map(|(i, id)| self.track(id).map(|q| json!({"index": i, "id": q.id, "title": q.title, "artist": q.artist, "duration": q.duration})))
            .collect();
        self.overlay.set_api_state(json!({
            "track": t.as_ref().map(|t| json!({"id": t.id, "title": t.title, "artist": t.artist, "album": t.album, "type": t.kind,
                "duration": t.duration, "url": t.url, "cover": cover(t)})),
            "playing": self.audio.is_playing(), "position": self.audio.position(), "duration": duration, "liked": liked,
            "volume": s.volume, "shuffle": s.shuffle, "repeat": s.repeat, "queueIndex": self.qi, "queue": queue,
        }));
        self.addon_run.reap();
    }

    /// Phone remote: run its commands and update the state it sees.
    fn remote_tick(&mut self, ctx: &egui::Context) {
        let cmds: Vec<(String, f64)> = self.remote.3.try_iter().collect();
        for (cmd, v) in cmds {
            let act = match cmd.as_str() {
                "toggle" => Act::TogglePlay, "next" => Act::Next, "prev" => Act::Prev,
                "volume" => { self.apply(Act::Volume(v.clamp(0.0, 1.0) as f32), ctx); Act::SaveVolume }
                "seek" => Act::Seek(v.max(0.0)),
                "jump" if (v.max(0.0) as usize) < self.queue.len() => Act::JumpQueue(v.max(0.0) as usize),
                _ => continue,
            };
            self.apply(act, ctx);
        }
        let t = self.current.clone();
        *self.remote.1.lock().unwrap() = t.as_ref().and_then(|t| t.cover.clone()).filter(|c| !c.starts_with("http"));
        let queue: Vec<serde_json::Value> = self.queue.iter().enumerate().skip(self.qi.saturating_sub(1)).take(30)
            .filter_map(|(i, id)| self.track(id).map(|t| json!({"i": i, "title": t.title, "artist": t.artist, "current": i == self.qi}))).collect();
        let duration = if self.audio.duration > 0.0 { self.audio.duration } else { t.as_ref().map(|t| t.duration).unwrap_or(0.0) };
        *self.remote.0.lock().unwrap() = json!({
            "track": t.as_ref().map(|t| json!({"id": t.id, "title": t.title, "artist": t.artist,
                "cover": t.cover.as_ref().map(|c| if c.starts_with("http") { c.clone() } else { "local".into() })})),
            "playing": self.audio.is_playing(), "position": self.audio.position(), "duration": duration,
            "volume": self.settings().volume, "queue": queue,
        });
    }

    fn lyrics_ui(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(l!("Текст", "Lyrics")).family(semi()).size(15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if icon_btn(ui, "\u{E4F6}", 24.0, false).clicked() { self.show_lyrics = false; } });
        });
        ui.separator();
        if self.current.is_none() { ui.label(RichText::new(l!("Ничего не играет", "Nothing is playing")).color(th().muted)); return };
        self.ensure_lyrics(ui.ctx());
        let Some((_, slot)) = &self.lyrics else { return };
        match slot {
            None => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Ищу текст…", "Looking for lyrics…")).color(th().muted)); }); }
            Some(Err(e)) => { ui.label(RichText::new(e.as_str()).color(BAD)); }
            Some(Ok(l)) if l.instrumental => { ui.label(RichText::new(l!("♪ Инструментал — без слов", "♪ Instrumental — no words")).color(th().muted)); }
            Some(Ok(l)) if l.is_empty() => {
                ui.label(RichText::new(l!("Текста не нашлось", "No lyrics found")).color(th().muted));
                ui.label(RichText::new(l!("Тексты берутся из открытой базы LRCLIB. Если у трека странное название — поправь его (правый клик → «Изменить…»).", "Lyrics come from the open LRCLIB database. If the track has a weird title, fix it (right click → “Edit…”).")).color(th().muted).size(12.0));
            }
            Some(Ok(l)) if l.synced.is_empty() => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| { ui.label(RichText::new(&l.plain).size(14.0).color(th().text)); });
            }
            Some(Ok(l)) => {
                let l = l.clone();
                let cur = l.line_at(self.audio.position());
                let jump = cur != self.lyrics_line;
                self.lyrics_line = cur;
                ui.label(RichText::new(l!("клик по строке — перемотать", "click a line to seek")).size(11.0).color(th().muted));
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(ui.available_height() * 0.3);
                    for (i, (time, line)) in l.synced.iter().enumerate() {
                        let on = Some(i) == cur;
                        let text = if line.is_empty() { "♪".to_string() } else { line.clone() };
                        let r = ui.add(egui::Label::new(RichText::new(text).size(if on { 18.0 } else { 15.0 }).family(if on { bold() } else { egui::FontFamily::Proportional })
                            .color(if on { th().text } else if cur.is_some_and(|c| i < c) { th().muted.gamma_multiply(0.6) } else { th().muted })).wrap().sense(Sense::click()));
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
            ui.label(RichText::new(l!("Очередь", "Queue")).family(semi()).size(15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if icon_btn(ui, "\u{E4F6}", 24.0, false).clicked() { self.show_queue = false; } });
        });
        ui.separator();
        ui.label(RichText::new(l!("тяни трек — переставить", "drag a track to reorder")).size(11.0).color(th().muted));
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
                        let c = if i == self.qi { th().accent } else if i < self.qi { th().muted.gamma_multiply(0.6) } else { th().text };
                        ui.add(egui::Label::new(RichText::new(&t.title).color(c)).truncate());
                        ui.add(egui::Label::new(RichText::new(wavy_core::i18n::tr(&t.artist)).color(th().muted).size(12.0)).truncate());
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
                ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("q-drop"))).line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(2.0, th().accent));
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
        let store_ = self.store.clone();
        let pls_name = move |id: &str| store_.lock().unwrap().lib.playlists.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
        let cur_pl = self.cur_playlist();
        let s = self.settings();
        let resp = egui::Modal::new(Id::unique("modal").with(std::mem::discriminant(&m)).with(if let Modal::NotFound { items, .. } = &m { items.len() } else { 0 })).frame(egui::Frame::popup(&ctx.global_style()).fill(th().bg2).inner_margin(22).corner_radius(16)).show(ctx, |ui| {
            ui.set_width(500.0);
            match &mut m {
                Modal::ImportUrl { url, into, busy, err } => {
                    ui.label(RichText::new(l!("Импорт по ссылке", "Import by link")).size(20.0).family(bold()));
                    ui.label(RichText::new(l!("Spotify (плейлист, альбом, трек), YouTube Music, YouTube, SoundCloud, Bandcamp — всё, что умеет yt-dlp.", "Spotify (playlist, album, track), YouTube Music, YouTube, SoundCloud, Bandcamp — anything yt-dlp supports.")).color(th().muted));
                    ui.add_space(10.0);
                    let r = ui.add(egui::TextEdit::singleline(url).hint_text("https://music.youtube.com/playlist?list=...").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                    if url.is_empty() { r.request_focus(); }
                    if let Some(p) = &cur_pl { ui.checkbox(into, lf!("Добавить в текущий плейлист «{}»", "Add to the current playlist “{}”", pls_name(p))); }
                    if *busy { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Загружаю… большие плейлисты — до минуты", "Loading… big playlists take up to a minute")).color(th().muted)); }); }
                    if let Some(e) = err { ui.label(RichText::new(e.as_str()).color(BAD)); }
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter)); // Enter starts the import even if the link was pasted without focus on the field
                        if ui.add_enabled(!*busy, pill_btn(l!("Импортировать", "Import"), true)).clicked() || (enter && !*busy) {
                            if url.trim().starts_with("http") { *busy = true; *err = None; acts.push(Act::ImportUrl(url.clone(), if *into { cur_pl.clone() } else { None })); }
                            else { *err = Some(l!("Нужна ссылка, начинающаяся с https://", "A link starting with https:// is needed").into()); }
                        }
                        if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; }
                    });
                }
                Modal::NewPlaylist { name, ids } => {
                    ui.label(RichText::new(l!("Новый плейлист", "New playlist")).size(20.0).family(bold()));
                    ui.add_space(8.0);
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text(l!("Название", "Name")).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                    r.request_focus();
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (pill(ui, l!("Создать", "Create"), true).clicked() || enter) && !name.trim().is_empty() { acts.push(Act::CreatePlaylist(name.trim().into(), ids.clone())); close = true; }
                        if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; }
                    });
                }
                Modal::Rename { id, name } => {
                    ui.label(RichText::new(l!("Переименовать плейлист", "Rename playlist")).size(20.0).family(bold()));
                    ui.add_space(8.0);
                    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8))).request_focus();
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (pill(ui, l!("Сохранить", "Save"), true).clicked() || enter) && !name.trim().is_empty() { acts.push(Act::Rename(id.clone(), name.trim().into())); close = true; }
                        if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; }
                    });
                }
                Modal::Delete { id } => {
                    ui.label(RichText::new(lf!("Удалить «{}»?", "Delete “{}”?", pls_name(id))).size(20.0).family(bold()));
                    ui.label(RichText::new(l!("Треки останутся в библиотеке.", "The tracks will stay in the library.")).color(th().muted));
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, l!("Удалить", "Delete"), true).clicked() { acts.push(Act::DeletePlaylist(id.clone())); close = true; }
                        if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; }
                    });
                }
                Modal::WhatsNew { from, to, items } => {
                    ui.label(RichText::new(l!("Что нового в Wavy", "What's new in Wavy")).size(20.0).family(bold()));
                    ui.label(RichText::new(lf!("Сборки {} → {}", "Builds {} → {}", *from + 1, to)).color(th().muted));
                    ui.add_space(8.0);
                    match items {
                        None => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Загружаю…", "Loading…")).color(th().muted)); }); }
                        Some(Err(e)) => { ui.label(RichText::new(e.as_str()).color(BAD)); }
                        Some(Ok(v)) if v.is_empty() => { ui.label(RichText::new(l!("Только мелкие технические правки", "Only small technical fixes")).color(th().muted)); }
                        Some(Ok(v)) => {
                            egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                                for (title, body) in v.iter() {
                                    ui.label(RichText::new(format!("• {title}")).family(semi()).color(th().text));
                                    if !body.is_empty() { ui.label(RichText::new(body).color(th().muted).size(12.5)); }
                                    ui.add_space(8.0);
                                }
                            });
                        }
                    }
                    ui.add_space(6.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, l!("Круто", "Cool"), true).clicked() { close = true; } });
                }
                Modal::EditTrack { id, title, artist, album } => {
                    ui.label(RichText::new(l!("Изменить трек", "Edit track")).size(20.0).family(bold()));
                    ui.add_space(8.0);
                    for (label, v) in [(l!("Название", "Name"), &mut *title), (l!("Исполнитель", "Artist"), &mut *artist), (l!("Альбом", "Album"), &mut *album)] {
                        ui.label(RichText::new(label).color(th().muted).size(12.0));
                        ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                        ui.add_space(4.0);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if pill(ui, l!("Своя обложка…", "Custom cover…"), false).clicked() { acts.push(Act::TrackCover(id.clone())); }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if pill(ui, l!("Сохранить", "Save"), true).clicked() && !title.trim().is_empty() { acts.push(Act::EditTrack(id.clone(), title.clone(), artist.clone(), album.clone())); close = true; }
                            if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; }
                        });
                    });
                }
                Modal::Dupes { groups } => {
                    let n: usize = groups.iter().map(|g| g.len() - 1).sum();
                    ui.label(RichText::new(lf!("Дубли: {n}", "Duplicates: {n}")).size(20.0).family(bold()));
                    ui.label(RichText::new(l!("Из каждой группы останется первый (самый старый) трек. В плейлистах дубли заменятся им, лайки и прослушивания перейдут к нему.", "The first (oldest) track of each group stays. In playlists duplicates will be replaced by it, likes and plays move to it.")).color(th().muted));
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                        for g in groups.iter() {
                            for (i, id) in g.iter().enumerate() {
                                if let Some(t) = self.store.lock().unwrap().lib.tracks.get(id) {
                                    let src = if t.is_local() { t.path.clone().unwrap_or_default() } else { t.url.clone().unwrap_or_default() };
                                    ui.label(RichText::new(format!("{} {} — {}  ·  {}", if i == 0 { "✓" } else { "×" }, t.artist, t.title, src)).color(if i == 0 { th().text } else { th().muted }).size(12.5));
                                }
                            }
                            ui.separator();
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, l!("Оставить по одному", "Keep one of each"), true).clicked() { acts.push(Act::MergeDupes(groups.clone())); close = true; }
                        if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; }
                    });
                }
                Modal::Folder { id, name } => {
                    ui.label(RichText::new(lf!("Папка для «{}»", "Folder for “{}”", pls_name(id))).size(20.0).family(bold()));
                    let existing: Vec<String> = { let mut v: Vec<String> = self.store.lock().unwrap().lib.playlists.iter().filter_map(|p| p.folder.clone()).collect(); v.sort(); v.dedup(); v };
                    if !existing.is_empty() {
                        ui.horizontal_wrapped(|ui| { for f in &existing { if ui.selectable_label(name == f, f).clicked() { *name = f.clone(); } } });
                    }
                    ui.add(egui::TextEdit::singleline(name).hint_text(l!("Название новой папки", "New folder name")).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, l!("Готово", "Done"), true).clicked() { acts.push(Act::SetFolder(id.clone(), Some(name.clone()))); close = true; }
                        if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; }
                    });
                }
                Modal::AnimeMatch { id, query, results, busy, err } => {
                    ui.label(RichText::new(l!("Какое это аниме?", "Which anime is this?")).size(20.0).family(bold()));
                    ui.label(RichText::new(l!("Поиск на Шикимори — по-русски, по-английски или по-японски (ромадзи).", "Search Shikimori — in Russian, English or Japanese (romaji).")).color(th().muted));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(query).desired_width(360.0).margin(egui::Margin::symmetric(10, 8)));
                        let go = pill(ui, if *busy { l!("Ищу…", "Searching…") } else { l!("Найти", "Find") }, true).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
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
                                    None => { ui.painter().rect_filled(pr, 4, th().bg3); }
                                }
                                ui.vertical(|ui| {
                                    ui.add_space(6.0);
                                    ui.add(egui::Label::new(RichText::new(r.display()).family(semi()).color(th().text)).truncate());
                                    let meta = [anime::kind_ru(&r.kind).to_string(), if r.year > 0 { r.year.to_string() } else { String::new() }, if r.episodes > 0 { lf!("{} эп.", "{} ep.", r.episodes) } else { String::new() }, r.name.clone()];
                                    ui.add(egui::Label::new(RichText::new(meta.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")).color(th().muted).size(12.5)).truncate());
                                });
                            }).response.interact(Sense::click());
                            if resp.hovered() { ui.painter().rect_stroke(resp.rect.expand(2.0), 8, Stroke::new(1.0, th().line), egui::StrokeKind::Outside); }
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { acts.push(Act::AnimeApply(id.clone(), r.clone())); close = true; }
                        }
                    });
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; } });
                }
                Modal::NotFound { pid, items } => {
                    ui.label(RichText::new(lf!("Не нашлось на YouTube: {}", "Not found on YouTube: {}", items.len())).size(20.0).family(bold()));
                    ui.label(RichText::new(l!("Автопоиск не уверен в этих треках. Нажми «Найти» и выбери нужное видео сам — трек встанет в плейлист.", "The automatic search wasn't sure about these tracks. Click “Find” and pick the right video yourself — the track goes into the playlist.")).color(th().muted));
                    ui.add_space(8.0);
                    let mut pick = None;
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        for (i, t) in items.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(370.0);
                                    ui.add(egui::Label::new(RichText::new(&t.title).family(semi()).color(th().text)).truncate());
                                    let meta = [t.artists.clone(), t.album.clone(), if t.duration > 0.0 { fmt_time(t.duration) } else { String::new() }];
                                    ui.add(egui::Label::new(RichText::new(meta.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")).color(th().muted).size(12.5)).truncate());
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, l!("Найти…", "Find…"), false).clicked() { pick = Some(i); } });
                            });
                            ui.add_space(4.0);
                        }
                    });
                    if let Some(i) = pick {
                        let mut rest = items.clone();
                        let sp = rest.remove(i);
                        let q = format!("{} {}", sp.artists.split(',').next().unwrap_or("").trim(), sp.title).trim().to_string();
                        acts.push(Act::Modal(Modal::Pick { query: q.clone(), results: Vec::new(), busy: true, err: None, target: PickTarget::Add { pid: pid.clone(), sp }, back: rest }));
                        acts.push(Act::PickSearch(q));
                        close = true;
                    }
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, l!("Закрыть", "Close"), true).clicked() { close = true; } });
                }
                Modal::Pick { query, results, busy, err, target, back } => {
                    let (title, cur_vid) = match &*target {
                        PickTarget::Replace(id) => { let s = self.store.lock().unwrap(); let t = s.lib.tracks.get(id); (t.map(|t| t.title.clone()).unwrap_or_default(), t.and_then(|t| t.video_id.clone())) }
                        PickTarget::Add { sp, .. } => (sp.title.clone(), None),
                    };
                    ui.label(RichText::new(l!("Выбери видео", "Pick a video")).size(20.0).family(bold()));
                    ui.add(egui::Label::new(RichText::new(match &*target {
                        PickTarget::Replace(_) => lf!("для «{title}» — трек останется в тех же плейлистах", "for “{title}” — the track stays in the same playlists"),
                        PickTarget::Add { sp, .. } => format!("{} — {}{}", sp.artists, sp.title, if sp.duration > 0.0 { format!(" · {}", fmt_time(sp.duration)) } else { String::new() }),
                    }).color(th().muted)).truncate());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(query).desired_width(360.0).margin(egui::Margin::symmetric(10, 8)));
                        let go = pill(ui, if *busy { l!("Ищу…", "Searching…") } else { l!("Найти", "Find") }, true).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if go && !query.trim().is_empty() && !*busy { acts.push(Act::PickSearch(query.trim().to_string())); }
                    });
                    if let Some(e) = err { ui.label(RichText::new(e.as_str()).color(BAD)); }
                    if *busy && results.is_empty() { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Ищу в YouTube Music и YouTube…", "Searching YouTube Music and YouTube…")).color(th().muted)); }); }
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        for r in results.iter() {
                            let resp = ui.horizontal(|ui| {
                                ui.set_min_height(52.0);
                                let (pr, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                                match r.cover.as_ref().and_then(|u| self.thumbs.poster(u, 96)) {
                                    Some(t) => { egui::Image::new(&t).corner_radius(6).maintain_aspect_ratio(false).paint_at(ui, pr); }
                                    None => { ui.painter().rect_filled(pr, 6, th().bg3); }
                                }
                                ui.vertical(|ui| {
                                    ui.set_width(380.0);
                                    ui.add_space(4.0);
                                    ui.add(egui::Label::new(RichText::new(&r.title).family(semi()).color(th().text)).truncate());
                                    let src = if r.url.as_deref().is_some_and(|u| u.contains("music.youtube")) { "YouTube Music" } else { "YouTube" };
                                    let now = if r.video_id.is_some() && r.video_id == cur_vid { l!(" · сейчас", " · current") } else { "" };
                                    let meta = [if r.artist == "YouTube" { String::new() } else { r.artist.clone() }, r.album.clone(), if r.duration > 0.0 { fmt_time(r.duration) } else { String::new() }, format!("{src}{now}")];
                                    ui.add(egui::Label::new(RichText::new(meta.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")).color(th().muted).size(12.5)).truncate());
                                });
                            }).response.interact(Sense::click());
                            if resp.hovered() { ui.painter().rect_stroke(resp.rect.expand(2.0), 8, Stroke::new(1.0, th().line), egui::StrokeKind::Outside); }
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { acts.push(Act::PickChoose(r.clone(), target.clone(), back.clone())); close = true; }
                        }
                    });
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, if back.is_empty() { l!("Отмена", "Cancel") } else { l!("Назад к списку", "Back to the list") }, false).clicked() {
                            if let PickTarget::Add { pid, sp } = &*target { let mut items = back.clone(); items.insert(0, sp.clone()); acts.push(Act::Modal(Modal::NotFound { pid: pid.clone(), items })); }
                            close = true;
                        }
                    });
                }
                Modal::DeleteAnime { id } => {
                    let name = self.store.lock().unwrap().anime(id).map(|a| a.display().to_string()).unwrap_or_default();
                    ui.label(RichText::new(lf!("Убрать «{name}»?", "Remove “{name}”?")).size(20.0).family(bold()));
                    ui.label(RichText::new(l!("Файлы серий на диске останутся — пропадёт только запись в Wavy.", "The episode files on disk stay — only the entry in Wavy goes away.")).color(th().muted));
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, l!("Убрать", "Remove"), true).clicked() { acts.push(Act::DeleteAnime(id.clone())); close = true; }
                        if pill(ui, l!("Отмена", "Cancel"), false).clicked() { close = true; }
                    });
                }
                Modal::Shazam { state, to_openings } => {
                    ui.label(RichText::new(l!("\u{E802}  Что за песня?", "\u{E802}  What's this song?")).size(20.0).family(bold()));
                    ui.add_space(8.0);
                    match state.clone() {
                        ShazamState::Listening => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Слушаю 12 секунд с текущего места и спрашиваю Shazam…", "Listening to 12 seconds from the current spot and asking Shazam…")).color(th().muted)); }); }
                        ShazamState::Adding => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Ищу трек на YouTube и добавляю…", "Looking for the track on YouTube and adding it…")).color(th().muted)); }); }
                        ShazamState::NotFound => {
                            ui.label(RichText::new(l!("Shazam не узнал этот кусок.", "Shazam didn't recognize this part.")).color(th().text));
                            ui.label(RichText::new(l!("Попробуй ещё раз, когда играет сама песня (опенинг/эндинг), а не диалог.", "Try again while the song itself plays (opening/ending), not dialogue.")).color(th().muted));
                        }
                        ShazamState::Error(e) => { ui.label(RichText::new(e).color(BAD)); }
                        ShazamState::Found(f) => {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(96.0), Sense::hover());
                                match f.cover.as_ref().and_then(|c| self.thumbs.get(c, 200)) {
                                    Some(t) => { egui::Image::new(&t).corner_radius(10).paint_at(ui, r); }
                                    None => { ui.painter().rect_filled(r, 10, th().bg3); ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E33C}", FontId::proportional(36.0), th().muted); }
                                }
                                ui.add_space(10.0);
                                ui.vertical(|ui| {
                                    ui.add_space(16.0);
                                    ui.add(egui::Label::new(RichText::new(&f.title).size(18.0).family(bold()).color(th().text)).wrap());
                                    ui.label(RichText::new(&f.artist).color(th().muted));
                                });
                            });
                            ui.add_space(8.0);
                            ui.checkbox(to_openings, lf!("Ещё и в плейлист «{}»", "Also to the playlist “{}”", openings()));
                        }
                    }
                    ui.add_space(10.0);
                    let st = state.clone();
                    let to_pl = *to_openings;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        match &st {
                            ShazamState::Found(f) => {
                                if pill(ui, l!("\u{E340}  В библиотеку", "\u{E340}  To library"), true).clicked() { acts.push(Act::ShazamAdd(f.clone(), to_pl)); }
                                if let Some(u) = &f.url { if pill(ui, l!("Открыть в Shazam", "Open in Shazam"), false).clicked() { acts.push(Act::OpenUrl(u.clone())); } }
                            }
                            ShazamState::NotFound | ShazamState::Error(_) => { if pill(ui, l!("Ещё раз", "Try again"), true).clicked() { acts.push(Act::Shazam); } }
                            _ => {}
                        }
                        if !matches!(st, ShazamState::Adding) && pill(ui, l!("Закрыть", "Close"), false).clicked() { close = true; }
                    });
                }
                Modal::Settings { client_id, port, sp_id, sp_secret } => {
                    ui.label(RichText::new(l!("Настройки", "Settings")).size(20.0).family(bold()));
                    egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                        section(ui, "Язык · Language");
                        ui.horizontal(|ui| {
                            for (code, name) in wavy_core::i18n::LANGS {
                                if ui.selectable_label(s.lang == *code || (s.lang.is_empty() && *code == "ru"), *name).clicked() && s.lang != *code { acts.push(Act::Settings(json!({"lang": code}))); }
                            }
                        });
                        section(ui, l!("Тема", "Theme"));
                        ui.horizontal_wrapped(|ui| {
                            for (id, name, pal) in theme::THEMES {
                                let on = s.theme == *id || (*id == "dark" && !theme::THEMES.iter().any(|t| t.0 == s.theme));
                                let label = RichText::new(format!("\u{25CF}  {}", wavy_core::i18n::tr(name)));
                                // a dot in the theme's own accent and background
                                let r = ui.add(egui::Button::selectable(on, label.color(pal.accent)).fill(if on { th().bg3 } else { pal.bg }).stroke(Stroke::new(1.0, if on { th().accent } else { th().line })).corner_radius(99));
                                if r.clicked() && !on { acts.push(Act::Settings(json!({"theme": id}))); }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(l!("Свой цвет акцента", "Custom accent color")).color(th().text));
                            let mut c = theme::parse_hex(&s.accent).unwrap_or(th().accent);
                            if ui.color_edit_button_srgba(&mut c).changed() { acts.push(Act::Settings(json!({"accent": theme::to_hex(c)}))); }
                            // settings merge, so "back to the theme's color" writes an explicit empty value
                            if !s.accent.is_empty() && ui.link(l!("сбросить", "reset")).clicked() { acts.push(Act::Settings(json!({"accent": ""}))); }
                        });
                        section(ui, "\u{E61A}  Discord Rich Presence");
                        let mut en = s.discord.enabled;
                        if ui.checkbox(&mut en, l!("Показывать, что я слушаю, в профиле Discord", "Show what I'm listening to in my Discord profile")).changed() { acts.push(Act::Settings(json!({"discord": {"enabled": en}}))); }
                        ui.label(RichText::new("APPLICATION ID").size(11.0).color(th().muted));
                        let r = ui.add(egui::TextEdit::singleline(client_id).hint_text(l!("например 1234567890123456789", "e.g. 1234567890123456789")).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 7)));
                        if r.changed() { client_id.retain(|c| c.is_ascii_digit()); } // pasted with spaces/junk — keep the digits
                        // save when the ID looks complete (Discord IDs are 17–20 digits) or when leaving the field
                        let full = (17..=20).contains(&client_id.len());
                        if (r.changed() && full) || r.lost_focus() { acts.push(Act::Settings(json!({"discord": {"clientId": client_id.as_str()}}))); }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(l!("Создай приложение на", "Create an application at")).color(th().muted).size(12.5));
                            if ui.link("discord.com/developers").clicked() { acts.push(Act::OpenUrl("https://discord.com/developers/applications".into())); }
                            ui.label(RichText::new(l!("(его название будет после «Слушает»), скопируй Application ID.", "(its name will show after “Listening to”), copy the Application ID.")).color(th().muted).size(12.5));
                        });
                        let mut b = s.discord.show_buttons;
                        if ui.checkbox(&mut b, l!("Кнопка «Открыть трек» (для YouTube-треков)", "“Open track” button (for YouTube tracks)")).changed() { acts.push(Act::Settings(json!({"discord": {"showButtons": b}}))); }
                        let (txt, c) = if !s.discord.enabled { (l!("Выключено", "Off").to_string(), th().muted) } else if s.discord.client_id.is_empty() { (l!("Укажи Application ID", "Enter the Application ID").into(), th().muted) }
                            else if self.discord_status.connected { (l!("\u{E182} Подключено", "\u{E182} Connected").into(), OK) } else { (self.discord_status.error.clone().map(|e| format!("\u{E4F6} {e}")).unwrap_or(l!("Подключаюсь…", "Connecting…").into()), BAD) };
                        ui.label(RichText::new(txt).color(c));

                        section(ui, l!("\u{E466}  Оверлей", "\u{E466}  Overlay"));
                        ui.label(RichText::new(l!("ДЛЯ OBS — BROWSER SOURCE", "FOR OBS — BROWSER SOURCE")).size(11.0).color(th().muted));
                        let url = self.overlay.url().unwrap_or_else(|| format!("http://127.0.0.1:{port}/overlay"));
                        ui.horizontal(|ui| {
                            let mut u = url.clone();
                            ui.add(egui::TextEdit::singleline(&mut u).desired_width(340.0).margin(egui::Margin::symmetric(10, 7)));
                            if pill(ui, l!("Копировать", "Copy"), false).clicked() { ui.ctx().copy_text(url.clone()); acts.push(Act::Settings(json!({}))); self.toast = Some((l!("Ссылка скопирована — вставь в OBS → Browser Source", "Link copied — paste it into OBS → Browser Source").into(), false, Instant::now())); }
                        });
                        ui.label(RichText::new(l!("Ширина 500, высота 160 (вертикальный — 260×320; с очередью или текстом — высота 260). Стиль можно задать и в ссылке: ?theme=compact, vertical, minimal, text, vinyl", "Width 500, height 160 (vertical — 260×320; with queue or lyrics — height 260). The style can also be set in the link: ?theme=compact, vertical, minimal, text, vinyl")).color(th().muted).size(12.0));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(l!("Стиль OBS:", "OBS style:"));
                            let mut ov_theme = s.overlay.theme.clone();
                            let themes: [(&str, &str); 6] = [("card", l!("Карточка", "Card")), ("compact", l!("Компакт", "Compact")), ("vertical", l!("Вертикальный", "Vertical")), ("minimal", l!("Минимал", "Minimal")), ("text", l!("Только текст", "Text only")), ("vinyl", l!("Винил", "Vinyl"))];
                            egui::ComboBox::from_id_salt("theme").selected_text(themes.iter().find(|t| t.0 == ov_theme).map(|t| t.1).unwrap_or(l!("Карточка", "Card"))).show_ui(ui, |ui| {
                                for (k, name) in themes { ui.selectable_value(&mut ov_theme, k.into(), name); }
                            });
                            if ov_theme != s.overlay.theme { acts.push(Act::Settings(json!({"overlay": {"theme": ov_theme}}))); }
                        });
                        ui.horizontal(|ui| {
                            let mut q = s.overlay.show_queue;
                            if ui.checkbox(&mut q, l!("Показывать, что дальше в очереди", "Show what's next in the queue")).changed() { acts.push(Act::Settings(json!({"overlay": {"showQueue": q}}))); }
                            let mut l = s.overlay.show_lyrics;
                            if ui.checkbox(&mut l, l!("Текст песни (строка в такт)", "Lyrics (line in time)")).changed() { acts.push(Act::Settings(json!({"overlay": {"showLyrics": l}}))); }
                            ui.add_space(12.0);
                            ui.label(l!("Скрывать через, сек:", "Hide after, sec:"));
                            let mut h = s.overlay.auto_hide_sec;
                            if ui.add(egui::DragValue::new(&mut h).range(0..=600)).changed() { acts.push(Act::Settings(json!({"overlay": {"autoHideSec": h}}))); }
                        });
                        ui.horizontal(|ui| {
                            ui.label(l!("Порт:", "Port:"));
                            let r = ui.add(egui::TextEdit::singleline(port).desired_width(70.0));
                            if r.lost_focus() { if let Ok(p) = port.parse::<u16>() { acts.push(Act::Settings(json!({"overlay": {"port": p}}))); } }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if pill(ui, if self.overlay_open { l!("Скрыть оверлей поверх игр", "Hide the overlay over games") } else { l!("Показать оверлей поверх игр", "Show the overlay over games") }, self.overlay_open).clicked() { acts.push(Act::ToggleOverlay); }
                            if pill(ui, if s.overlay_window.locked { l!("\u{E306} Двигать", "\u{E306} Move") } else { l!("\u{E2FA} Закрепить", "\u{E2FA} Lock") }, false).clicked() { acts.push(Act::ToggleLock); }
                        });
                        ui.label(RichText::new(l!("Ctrl+Shift+O — показать/скрыть, Ctrl+Shift+L — закрепить/двигать. Поверх игр в режиме «оконный без рамки».", "Ctrl+Shift+O — show/hide, Ctrl+Shift+L — lock/move. Works over games in “borderless window” mode.")).color(th().muted).size(12.0));

                        section(ui, "\u{E4FC}  YouTube (yt-dlp)");
                        match &self.yt_version {
                            None => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Проверяю… (при первом запуске скачивается yt-dlp)", "Checking… (yt-dlp is downloaded on first launch)")).color(th().muted)); }); }
                            Some(Ok(v)) => { ui.label(RichText::new(format!("yt-dlp {v}")).color(OK)); }
                            Some(Err(e)) => { ui.label(RichText::new(e.as_str()).color(BAD)); }
                        }
                        match self.yt.deno_status() {
                            wavy_core::youtube::Deno::Ready(_) => { ui.label(RichText::new(l!("JS-движок deno: готов", "JS engine deno: ready")).color(OK)); }
                            wavy_core::youtube::Deno::Installing => {
                                ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Скачиваю JS-движок deno (~42 МБ, один раз)…", "Downloading the JS engine deno (~42 MB, once)…")).color(th().muted)); });
                                ui.ctx().request_repaint_after(Duration::from_millis(500));
                            }
                            wavy_core::youtube::Deno::Failed(e) => { ui.label(RichText::new(format!("deno: {e}")).color(BAD)); }
                            wavy_core::youtube::Deno::Unknown => {}
                        }
                        ui.horizontal(|ui| {
                            if pill(ui, l!("Обновить yt-dlp", "Update yt-dlp"), false).clicked() { acts.push(Act::YtUpdate); }
                            if pill(ui, l!("Открыть папку с логами", "Open the logs folder"), false).on_hover_text(l!("wavy.log — пришли его, если что-то не работает", "wavy.log — send it if something doesn't work")).clicked() {
                                acts.push(Act::Reveal(wavy_core::data_dir().join("wavy.log").to_string_lossy().into_owned()));
                            }
                        });
                        ui.label(RichText::new(l!("Если YouTube перестал играть — почти всегда помогает обновление yt-dlp.", "If YouTube stopped playing — updating yt-dlp almost always helps.")).color(th().muted).size(12.0));

                        section(ui, l!("\u{E4DA}  Стрим", "\u{E4DA}  Stream"));
                        ui.horizontal(|ui| {
                            let mut sr = s.twitch_sr;
                            if ui.checkbox(&mut sr, l!("Заказы из чата Twitch: !sr ссылка или название", "Twitch chat requests: !sr link or title")).changed() { acts.push(Act::Settings(json!({"twitchSr": sr}))); }
                        });
                        ui.horizontal(|ui| {
                            ui.label(l!("Канал:", "Channel:"));
                            let mut ch = s.twitch_channel.clone();
                            if ui.add(egui::TextEdit::singleline(&mut ch).hint_text("avice").desired_width(160.0)).changed() { acts.push(Act::Settings(json!({"twitchChannel": ch.trim()}))); }
                            ui.label(l!("не длиннее, мин:", "max length, min:"));
                            let mut m = s.twitch_max_min;
                            if ui.add(egui::DragValue::new(&mut m).range(0..=60)).changed() { acts.push(Act::Settings(json!({"twitchMaxMin": m}))); }
                        });
                        if let Some((key, _, chat)) = &self.twitch {
                            let ch = key.split('|').next().unwrap_or_default();
                            ui.label(RichText::new(lf!("Слушаю чат #{ch}{}. Заказ встаёт в очередь после текущего трека, от одного зрителя — не чаще раза в 30 с.", "Listening to chat #{ch}{}. A request goes into the queue after the current track, one per viewer every 30 s at most.", if chat.can_say() { l!(" и отвечаю в него", " and replying in it") } else { "" })).color(OK).size(12.0));
                        }
                        ui.label(RichText::new(l!("Команды: !sr — заказ · !skip — голосование за пропуск (модер и стример пропускают сразу) · !song — что играет · !queue — что дальше", "Commands: !sr — request · !skip — vote to skip (mods and the streamer skip right away) · !song — what's playing · !queue — what's next")).color(th().muted).size(12.0));
                        ui.horizontal(|ui| {
                            ui.label(l!("!skip пропускает после голосов:", "!skip skips after votes:"));
                            let mut v = s.twitch_skip_votes;
                            if ui.add(egui::DragValue::new(&mut v).range(0..=50)).on_hover_text(l!("0 — голосование выключено", "0 — voting is off")).changed() { acts.push(Act::Settings(json!({"twitchSkipVotes": v}))); }
                        });
                        ui.horizontal(|ui| {
                            ui.label(l!("Заказ за баллы канала:", "Request for channel points:"));
                            if s.twitch_reward.is_empty() {
                                if self.reward_bind { ui.label(RichText::new(l!("жду: выкупи награду на своём канале…", "waiting: redeem the reward on your channel…")).color(OK)); if pill(ui, l!("Отмена", "Cancel"), false).clicked() { self.reward_bind = false; } }
                                else if pill(ui, l!("Привязать награду", "Link a reward"), false).on_hover_text(l!("Сделай на Twitch награду с галочкой «Зритель должен ввести текст», нажми здесь и выкупи её сам — Wavy запомнит", "Create a reward on Twitch with “Require viewer to enter text”, click here and redeem it yourself — Wavy will remember it")).clicked() { self.reward_bind = true; }
                            } else {
                                ui.label(RichText::new(l!("привязана", "linked")).color(OK));
                                if pill(ui, l!("Отвязать", "Unlink"), false).clicked() { acts.push(Act::Settings(json!({"twitchReward": ""}))); }
                            }
                        });
                        // sign-in: so Wavy can reply in chat
                        let acc = s.twitch_account.clone();
                        ui.horizontal(|ui| {
                            if acc.logged_in() {
                                ui.label(RichText::new(lf!("Отвечаю в чат как {}", "Replying in chat as {}", acc.login)).color(OK));
                                if pill(ui, l!("Выйти", "Sign out"), false).clicked() { acts.push(Act::Settings(json!({"twitchAccount": {"access": "", "refresh": "", "login": ""}}))); }
                            } else if let Some(d) = &self.twitch_device {
                                ui.label(RichText::new(lf!("Введи код {} на twitch.tv/activate", "Enter the code {} at twitch.tv/activate", d.user_code)).family(semi()));
                                if pill(ui, l!("Копировать", "Copy"), false).clicked() { ui.ctx().copy_text(d.user_code.clone()); }
                                if pill(ui, l!("Открыть", "Open"), false).clicked() { open_url(&d.uri); }
                            } else {
                                ui.label("Client ID:");
                                let mut id = acc.client_id.clone();
                                if ui.add(egui::TextEdit::singleline(&mut id).desired_width(200.0).hint_text(l!("с dev.twitch.tv", "from dev.twitch.tv"))).changed() { acts.push(Act::Settings(json!({"twitchAccount": {"clientId": id.trim()}}))); }
                                if ui.add_enabled(!acc.client_id.is_empty(), pill_btn(l!("Войти", "Sign in"), false)).clicked() {
                                    let (tx, c, id) = (self.tx.clone(), ui.ctx().clone(), acc.client_id.clone());
                                    std::thread::spawn(move || { let _ = tx.send(Bg::TwitchDevice(wavy_core::twitch::device_start(&id))); c.request_repaint(); });
                                }
                            }
                        });
                        if !acc.logged_in() && self.twitch_device.is_none() {
                            ui.label(RichText::new(l!("Чтобы Wavy отвечал в чат (!song, !queue, «добавил»): dev.twitch.tv → Applications → Register: название Wavy, OAuth Redirect URL http://localhost, категория Application Integration, тип клиента Public. Скопируй Client ID сюда и нажми «Войти». Без входа заказы и !skip всё равно работают.", "For Wavy to reply in chat (!song, !queue, “added”): dev.twitch.tv → Applications → Register: name Wavy, OAuth Redirect URL http://localhost, category Application Integration, client type Public. Copy the Client ID here and click “Sign in”. Requests and !skip work without signing in anyway.")).color(th().muted).size(12.0));
                        }
                        let mut np = s.now_playing_file;
                        if ui.checkbox(&mut np, l!("Писать текущий трек в файл (для OBS → Текст → «Читать из файла»)", "Write the current track to a file (for OBS → Text → “Read from file”)")).changed() { acts.push(Act::Settings(json!({"nowPlayingFile": np}))); }
                        if s.now_playing_file {
                            let f = wavy_core::data_dir().join("now_playing.txt").to_string_lossy().into_owned();
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&f).color(th().muted).size(12.0));
                                if pill(ui, l!("Копировать путь", "Copy path"), false).clicked() { ui.ctx().copy_text(f.clone()); }
                            });
                        }
                        section(ui, l!("\u{E248}  Кэш", "\u{E248}  Cache"));
                        match self.cache_size {
                            Some((b, n)) => { ui.label(RichText::new(lf!("Скачано с YouTube: {} ({n} файлов)", "Downloaded from YouTube: {} ({n} files)", fmt_bytes(b))).color(th().text)); }
                            None => {
                                ui.label(RichText::new(l!("Считаю…", "Counting…")).color(th().muted));
                                if !self.cache_counting { self.cache_counting = true; let (dir, tx, c) = (self.cache.clone(), self.tx.clone(), ui.ctx().clone()); std::thread::spawn(move || { let (b, n) = wavy_core::stream::dir_size(&dir); let _ = tx.send(Bg::CacheSize(b, n)); c.request_repaint(); }); }
                            }
                        }
                        ui.horizontal(|ui| {
                            ui.label(l!("Лимит:", "Limit:"));
                            let mut l = s.cache_limit_gb;
                            egui::ComboBox::from_id_salt("cache-limit").selected_text(if l == 0 { l!("без лимита", "no limit").to_string() } else { lf!("{l} ГБ", "{l} GB") }).show_ui(ui, |ui| {
                                for v in [0u32, 1, 2, 5, 10, 20] { ui.selectable_value(&mut l, v, if v == 0 { l!("без лимита", "no limit").to_string() } else { lf!("{v} ГБ", "{v} GB") }); }
                            });
                            if l != s.cache_limit_gb { acts.push(Act::Settings(json!({"cacheLimitGb": l}))); self.cache_trimmed = Instant::now() - Duration::from_secs(3600); }
                            if pill(ui, l!("Очистить", "Clear"), false).on_hover_text(l!("Удалить скачанное (кроме того, что играет сейчас) — при прослушивании скачается снова", "Delete downloaded files (except what's playing now) — they'll download again when played")).clicked() {
                                let keep: Vec<PathBuf> = self.current.as_ref().and_then(|t| t.url.clone()).map(|u| self.cache.join(wavy_core::sha16(u.as_bytes()))).into_iter().collect();
                                let freed = wavy_core::stream::trim_dir(&self.cache, 0, &keep);
                                self.toast(lf!("Освобождено {}", "Freed {}", fmt_bytes(freed)), false);
                                self.cache_size = None;
                                self.cache_counting = false;
                            }
                        });
                        section(ui, "\u{E4FC}  YouTube 18+");
                        ui.label(RichText::new(l!("Видео с возрастным ограничением YouTube без входа не отдаёт. Wavy сначала ищет такой же трек в другом видео, а если задать куки — сыграет и само видео, как будто ты вошёл. Куки берутся только для таких видео.", "YouTube doesn't serve age-restricted videos without signing in. Wavy first looks for the same track in another video, and with cookies set it will play the video itself, as if you were signed in. Cookies are only used for such videos.")).color(th().muted).size(12.0));
                        ui.horizontal(|ui| {
                            ui.label(l!("Куки из:", "Cookies from:"));
                            let cur = s.yt_cookies.clone();
                            let is_file = !cur.is_empty() && !wavy_core::youtube::COOKIE_BROWSERS.iter().any(|b| b.0 == cur);
                            let shown = if cur.is_empty() { l!("не брать", "don't use").to_string() } else if is_file { l!("файла cookies.txt", "cookies.txt file").to_string() }
                                else { wavy_core::youtube::COOKIE_BROWSERS.iter().find(|b| b.0 == cur).map(|b| b.1.to_string()).unwrap_or_default() };
                            let mut pick: Option<String> = None;
                            egui::ComboBox::from_id_salt("yt-cookies").selected_text(shown).show_ui(ui, |ui| {
                                if ui.selectable_label(cur.is_empty(), l!("не брать", "don't use")).clicked() { pick = Some(String::new()); }
                                for (id, name) in wavy_core::youtube::COOKIE_BROWSERS {
                                    if ui.selectable_label(cur == *id, *name).clicked() { pick = Some(id.to_string()); }
                                }
                                if ui.selectable_label(is_file, l!("файла cookies.txt…", "cookies.txt file…")).clicked() {
                                    if let Some(f) = rfd::FileDialog::new().set_title(l!("Файл cookies.txt", "cookies.txt file")).add_filter("cookies.txt", &["txt"]).pick_file() { pick = Some(f.to_string_lossy().into_owned()); }
                                }
                            });
                            if let Some(v) = pick { if v != cur { acts.push(Act::Settings(json!({"ytCookies": v}))); } }
                            if is_file { ui.label(RichText::new(&cur).color(th().muted).size(12.0)); }
                        });
                        if matches!(s.yt_cookies.as_str(), "chrome" | "edge" | "brave" | "opera" | "vivaldi" | "chromium") {
                            ui.label(RichText::new(l!("Chrome и похожие браузеры на Windows прячут куки, и часто достать их не выходит. Надёжнее Firefox (с входом в YouTube) или файл cookies.txt из расширения «Get cookies.txt LOCALLY».", "Chrome and similar browsers on Windows hide cookies, and often they can't be read. Firefox (signed in to YouTube) or a cookies.txt file from the “Get cookies.txt LOCALLY” extension is more reliable.")).color(th().muted).size(12.0));
                        }
                        section(ui, l!("\u{E4FC}  Аккаунт YouTube Music", "\u{E4FC}  YouTube Music account"));
                        ui.label(RichText::new(l!("Вход открывает «Мою волну» (радио YouTube Music по твоим лайкам и истории) и импорт лайков и плейлистов. Wavy откроет отдельное окно Chrome, Edge или Brave: войди в Google как обычно, Wavy сам заберёт вход и закроет окно. Пароль Wavy не видит.", "Signing in unlocks “My wave” (YouTube Music radio from your likes and history) and importing likes and playlists. Wavy opens a separate Chrome, Edge or Brave window: sign in to Google as usual, Wavy takes the sign-in and closes the window. Wavy never sees your password.")).color(th().muted).size(12.0));
                        ui.add_space(4.0);
                        if self.google_login.is_some() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(l!("Войди в Google в открывшемся окне…", "Sign in to Google in the window that opened…"));
                                if pill(ui, l!("Отмена", "Cancel"), false).clicked() { acts.push(Act::GoogleLoginCancel); }
                            });
                        } else {
                            ui.horizontal(|ui| {
                                match &self.acc {
                                    AccState::On => {
                                        ui.label(RichText::new(l!("\u{E182} Аккаунт подключён", "\u{E182} Account connected")).color(th().text));
                                        if pill(ui, l!("Импорт лайков", "Import likes"), false).clicked() { acts.push(Act::YtImportLikes); }
                                        if pill(ui, l!("Импорт плейлистов", "Import playlists"), false).clicked() { acts.push(Act::YtImportPlaylists); }
                                        if pill(ui, l!("Выйти", "Sign out"), false).clicked() { acts.push(Act::AccountLogout); }
                                    }
                                    AccState::Connecting => { ui.spinner(); ui.label(l!("Подключаю аккаунт…", "Connecting the account…")); }
                                    st => {
                                        if pill(ui, l!("Войти через Google", "Sign in with Google"), true).clicked() { acts.push(Act::GoogleLogin); }
                                        if !s.yt_cookies.is_empty() && pill(ui, l!("По кукам выше", "With the cookies above"), false).on_hover_text(l!("Взять вход из браузера или файла, выбранного в «Куки из»", "Use the sign-in from the browser or file chosen in “Cookies from”")).clicked() {
                                            acts.push(Act::Settings(json!({"ytAccount": true})));
                                            acts.push(Act::AccountConnect);
                                        }
                                        if let AccState::Err(e) = st { ui.label(RichText::new(e.as_str()).color(BAD).size(12.0)); }
                                    }
                                }
                            });
                        }
                        ui.label(RichText::new(l!("Wavy — неофициальный клиент: риск блокировки аккаунта маленький, но не нулевой. Надёжнее войти во второй аккаунт.", "Wavy is an unofficial client: the risk of an account ban is small but not zero. A secondary account is safer.")).color(th().muted).size(12.0));
                        section(ui, "\u{E340}  Last.fm");
                        let lf = s.lastfm.clone();
                        if !lf.session.is_empty() {
                            ui.horizontal(|ui| {
                                let mut on = lf.enabled;
                                if ui.checkbox(&mut on, lf!("Скробблить в Last.fm ({})", "Scrobble to Last.fm ({})", lf.user)).changed() { acts.push(Act::Settings(json!({"lastfm": {"enabled": on}}))); }
                                if pill(ui, l!("Выйти", "Sign out"), false).clicked() { acts.push(Act::Settings(json!({"lastfm": {"session": "", "user": "", "enabled": false}}))); }
                            });
                            ui.label(RichText::new(l!("Трек засчитывается, когда проиграна половина (или 4 минуты). Серии аниме не скробблятся.", "A track counts once half of it has played (or 4 minutes). Anime episodes aren't scrobbled.")).color(th().muted).size(12.0));
                        } else {
                            ui.label(RichText::new(l!("Нужен свой ключ: last.fm/api/account/create (название — Wavy, остальное можно не заполнять). Скопируй сюда API key и Shared secret.", "You need your own key: last.fm/api/account/create (name — Wavy, the rest can be left empty). Copy the API key and Shared secret here.")).color(th().muted).size(12.0));
                            ui.horizontal(|ui| {
                                ui.label("API key:");
                                let mut k = lf.api_key.clone();
                                if ui.add(egui::TextEdit::singleline(&mut k).desired_width(240.0)).changed() { acts.push(Act::Settings(json!({"lastfm": {"apiKey": k.trim()}}))); }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Shared secret:");
                                let mut k = lf.secret.clone();
                                if ui.add(egui::TextEdit::singleline(&mut k).desired_width(240.0).password(true)).changed() { acts.push(Act::Settings(json!({"lastfm": {"secret": k.trim()}}))); }
                            });
                            ui.horizontal(|ui| {
                                let ok = !lf.api_key.is_empty() && !lf.secret.is_empty();
                                match self.lfm_token.clone() {
                                    None => if ui.add_enabled(ok, pill_btn(l!("Войти через Last.fm", "Sign in with Last.fm"), true)).clicked() {
                                        let (tx, c) = (self.tx.clone(), ui.ctx().clone());
                                        std::thread::spawn(move || { let _ = tx.send(Bg::LastfmBegin(wavy_core::lastfm::begin(&lf))); c.request_repaint(); });
                                    },
                                    Some(token) => {
                                        ui.label(l!("На странице Last.fm нажми «Allow access», потом:", "On the Last.fm page click “Allow access”, then:"));
                                        if pill(ui, l!("Готово", "Done"), true).clicked() {
                                            let (tx, c, mut a) = (self.tx.clone(), ui.ctx().clone(), lf.clone());
                                            std::thread::spawn(move || { let r = wavy_core::lastfm::finish(&mut a, &token).map(|_| a); let _ = tx.send(Bg::LastfmDone(r)); c.request_repaint(); });
                                        }
                                        if pill(ui, l!("Отмена", "Cancel"), false).clicked() { self.lfm_token = None; }
                                    }
                                }
                            });
                        }
                        section(ui, l!("\u{E754}  Аниме", "\u{E754}  Anime"));
                        let mut a = s.anime_autoskip;
                        if ui.checkbox(&mut a, l!("Сам пропускать опенинги (время берётся из AniSkip)", "Skip openings automatically (timing from AniSkip)")).changed() { acts.push(Act::Settings(json!({"animeAutoskip": a}))); }
                        let mut n = s.anime_next;
                        if ui.checkbox(&mut n, l!("На эндинге предлагать следующую серию", "Offer the next episode at the ending")).changed() { acts.push(Act::Settings(json!({"animeNext": n}))); }
                        ui.add_space(4.0);
                        ui.label(RichText::new(l!("Шикимори: отмечать просмотренные серии в твоём списке", "Shikimori: mark watched episodes in your list")).family(semi()));
                        let acc = s.shiki.clone();
                        if acc.logged_in() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(lf!("Вошёл как {}", "Signed in as {}", acc.nickname)).color(OK));
                                if pill(ui, l!("Выйти", "Sign out"), false).clicked() { acts.push(Act::Settings(json!({"shiki": wavy_core::shikimori::Account { client_id: acc.client_id.clone(), client_secret: acc.client_secret.clone(), ..Default::default() }}))); }
                            });
                        } else {
                            ui.label(RichText::new(l!("1) На shikimori.io/oauth/applications создай приложение: название Wavy, Redirect URI — urn:ietf:wg:oauth:2.0:oob, доступ user_rates. 2) Вставь сюда его Client ID и Client Secret. 3) Нажми «Войти», разреши доступ и вставь код со страницы.", "1) At shikimori.io/oauth/applications create an application: name Wavy, Redirect URI — urn:ietf:wg:oauth:2.0:oob, access user_rates. 2) Paste its Client ID and Client Secret here. 3) Click “Sign in”, allow access and paste the code from the page.")).color(th().muted).size(12.0));
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
                                if ui.add_enabled(!acc.client_id.is_empty(), pill_btn(l!("Войти", "Sign in"), false)).clicked() { open_url(&wavy_core::shikimori::authorize_url(&acc.client_id)); }
                                ui.add(egui::TextEdit::singleline(&mut self.shiki_code).hint_text(l!("код со страницы Шикимори", "code from the Shikimori page")).desired_width(200.0));
                                if ui.add_enabled(!self.shiki_code.trim().is_empty() && !acc.client_secret.is_empty(), pill_btn(l!("Подтвердить", "Confirm"), true)).clicked() {
                                    let (mut acc, code, tx, c) = (acc.clone(), self.shiki_code.clone(), self.tx.clone(), ui.ctx().clone());
                                    std::thread::spawn(move || { let r = wavy_core::shikimori::login(&mut acc, &code).map(|_| acc); let _ = tx.send(Bg::Shiki(r, true)); c.request_repaint(); });
                                }
                            });
                        }
                        section(ui, l!("\u{E58C}  Видео", "\u{E58C}  Video"));
                        ui.label(RichText::new(l!("H.264 показывается встроенным декодером. Для HEVC (H.265), 10-битного видео, VP9, AV1 и звука Opus/AC3/DTS Wavy один раз скачивает свой маленький ffmpeg (≈6 МБ) — сам, когда такое видео встретится. Если видеокарта умеет этот формат, декодирует она.", "H.264 is shown by the built-in decoder. For HEVC (H.265), 10-bit video, VP9, AV1 and Opus/AC3/DTS audio Wavy downloads its own small ffmpeg once (≈6 MB) — by itself, when such a video comes up. If your graphics card supports the format, it does the decoding.")).color(th().muted).size(12.5));
                        match self.ff.state() {
                            wavy_core::ffmpeg::State::Ready(_) => { ui.label(RichText::new(l!("Декодер ffmpeg: готов", "ffmpeg decoder: ready")).color(OK)); }
                            wavy_core::ffmpeg::State::Downloading(p) => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(lf!("Скачиваю декодер… {p}%", "Downloading the decoder… {p}%")).color(th().muted)); }); }
                            wavy_core::ffmpeg::State::Failed(e) => { ui.label(RichText::new(lf!("Декодер: {e}", "Decoder: {e}")).color(BAD)); }
                            wavy_core::ffmpeg::State::Missing => {
                                if pill(ui, l!("Скачать декодер сейчас", "Download the decoder now"), false).clicked() { let c = ui.ctx().clone(); self.ff.install_background(move || c.request_repaint()); }
                            }
                        }

                        section(ui, l!("\u{E58C}  Синхронизация с телефоном", "\u{E58C}  Sync with phone"));
                        ui.label(RichText::new(l!("Плейлисты и треки YouTube / Spotify переносятся файлом wavy-export.json в обе стороны. Файл с телефона можно просто перетащить в окно. Локальные файлы не переносятся.", "Playlists and YouTube / Spotify tracks are transferred in both directions with the wavy-export.json file. You can just drag the file from the phone into the window. Local files aren't transferred.")).color(th().muted).size(12.5));
                        ui.horizontal(|ui| {
                            if pill(ui, l!("Экспорт для телефона", "Export for phone"), true).clicked() { acts.push(Act::SyncExport); }
                            if pill(ui, l!("Импорт с телефона", "Import from phone"), false).clicked() { acts.push(Act::SyncImport(None)); }
                        });
                        let mut w = s.wifi_sync;
                        if ui.checkbox(&mut w, l!("По Wi-Fi без файлов (телефон и ПК в одной сети)", "Over Wi-Fi without files (phone and PC on the same network)")).changed() {
                            let pin = if s.wifi_pin.is_empty() { wavy_core::wifisync::new_pin() } else { s.wifi_pin.clone() };
                            acts.push(Act::Settings(json!({"wifiSync": w, "wifiPin": pin})));
                        }
                        if self.wifi.is_some() {
                            let addr = format!("{}:{}", wavy_core::wifisync::local_ip().unwrap_or_else(|| l!("IP-адрес ПК", "PC IP address").into()), wavy_core::wifisync::PORT);
                            ui.label(RichText::new(lf!("В приложении на телефоне: Настройки → Синхронизация с ПК → адрес {addr}, PIN {}", "In the phone app: Settings → Sync with PC → address {addr}, PIN {}", s.wifi_pin)).color(OK));
                            ui.label(RichText::new(l!("Windows может спросить разрешение для сети — разреши для частных сетей.", "Windows may ask for network permission — allow it for private networks.")).color(th().muted).size(12.0));
                        }

                        section(ui, "\u{E340}  Spotify");
                        ui.label(RichText::new(l!("Ссылки на плейлисты, альбомы и треки Spotify импортируются без настроек: каждый трек находится на YouTube. Без ключа — до 100 треков из плейлиста.", "Spotify playlist, album and track links import without setup: each track is found on YouTube. Without a key — up to 100 tracks from a playlist.")).color(th().muted).size(12.5));
                        ui.label(RichText::new(l!("Для больших плейлистов: developer.spotify.com → Create app → скопируй Client ID и Client Secret.", "For big playlists: developer.spotify.com → Create app → copy the Client ID and Client Secret.")).color(th().muted).size(12.5));
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

                        section(ui, l!("\u{E272}  Удобство", "\u{E272}  Convenience"));
                        if cfg!(windows) {
                            let mut ct = s.close_to_tray;
                            if ui.checkbox(&mut ct, l!("Крестик сворачивает в трей (музыка продолжает играть)", "The close button minimizes to tray (music keeps playing)")).changed() { acts.push(Act::Settings(json!({"closeToTray": ct}))); }
                        }
                        let mut mini = s.mini;
                        if ui.checkbox(&mut mini, l!("Мини-плеер поверх всех окон", "Mini player on top of all windows")).changed() { acts.push(Act::ToggleMini); }
                        ui.add_space(6.0);
                        ui.label(RichText::new(l!("Горячие клавиши везде (и в игре)", "Global hotkeys (work in games too)")).family(semi()));
                        // recording a combination: the next key press with Ctrl/Alt/Shift
                        if let Some(action) = self.hk_capture.clone() {
                            let ev = ui.ctx().input(|i| i.events.iter().find_map(|e| match e { egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)), _ => None }));
                            if let Some((key, m)) = ev {
                                let mut map = s.hotkeys.clone();
                                match key {
                                    egui::Key::Escape => { self.hk_capture = None; }
                                    egui::Key::Backspace | egui::Key::Delete if !m.any() => { map.insert(action, String::new()); acts.push(Act::Settings(json!({"hotkeys": map}))); self.hk_capture = None; }
                                    _ => if let Some(c) = hotkeys::from_egui(key, m) { map.insert(action, c); acts.push(Act::Settings(json!({"hotkeys": map}))); self.hk_capture = None; }
                                }
                            }
                        }
                        egui::Grid::new("hotkeys").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
                            for (action, label, _) in hotkeys::ACTIONS {
                                ui.label(wavy_core::i18n::tr(label));
                                let c = hotkeys::combo(&s.hotkeys, action);
                                let busy = self.hk_keys.failed.contains(&c);
                                let shown = if self.hk_capture.as_deref() == Some(*action) { l!("нажми сочетание… (Esc — отмена, Backspace — выкл)", "press a combination… (Esc — cancel, Backspace — off)").to_string() } else if c.is_empty() { l!("выкл", "off").into() } else { c.clone() };
                                let r = ui.add(egui::Button::new(RichText::new(shown).color(if busy { BAD } else { th().text })).min_size(Vec2::new(150.0, 24.0)));
                                if r.clicked() { self.hk_capture = Some(action.to_string()); }
                                if busy { r.on_hover_text(l!("Это сочетание занято другой программой", "This combination is taken by another program")); }
                                // settings are merged, not replaced — so "default" is written explicitly
                                let def = hotkeys::ACTIONS.iter().find(|a| a.0 == *action).map(|a| a.2).unwrap_or("");
                                if c != def && pill(ui, l!("по умолчанию", "default"), false).clicked() {
                                    acts.push(Act::Settings(json!({"hotkeys": {*action: def}})));
                                } else if c == def { ui.label(""); }
                                ui.end_row();
                            }
                        });
                        ui.label(RichText::new(l!("Клавиши в окне: Пробел — пауза · ←/→ — ±5 с · Ctrl+←/→ — трек · ↑/↓ — громкость · M — без звука · L — лайк · Q — очередь · S — перемешать · R — повтор · Ctrl+F — поиск · Ctrl+K — команды", "Keys in the window: Space — pause · ←/→ — ±5 s · Ctrl+←/→ — track · ↑/↓ — volume · M — mute · L — like · Q — queue · S — shuffle · R — repeat · Ctrl+F — search · Ctrl+K — commands")).color(th().muted).size(12.0));
                        section(ui, l!("Аддоны", "Addons"));
                        ui.label(RichText::new(l!("Аддоны — отдельные программы, которые управляют Wavy через локальный API: кладёшь папку аддона в папку аддонов и включаешь его здесь. Описание API — docs/addons.md в репозитории.", "Addons are separate programs that control Wavy through a local API: put an addon's folder into the addons folder and enable it here. The API is described in docs/addons.md in the repository.")).color(th().muted).size(12.5));
                        ui.horizontal(|ui| {
                            if pill(ui, l!("Открыть папку аддонов", "Open addons folder"), false).clicked() {
                                let d = wavy_core::addons::dir();
                                let _ = std::fs::create_dir_all(&d);
                                acts.push(Act::Reveal(d.join("api-token").to_string_lossy().into_owned()));
                            }
                            if pill(ui, l!("Обновить список", "Refresh list"), false).clicked() { acts.push(Act::AddonsRescan); }
                            if pill(ui, l!("Копировать токен API", "Copy API token"), false).clicked() {
                                ui.ctx().copy_text(self.api_token.clone());
                                self.toast = Some((l!("Токен скопирован. Он даёт управление плеером — не публикуй его", "Token copied. It gives control of the player — don't share it").into(), false, Instant::now()));
                            }
                        });
                        if let Some(api) = self.overlay.api_url() { ui.label(RichText::new(format!("API: {api}")).color(th().muted).size(12.0)); }
                        if self.addon_list.is_empty() { ui.label(RichText::new(l!("Аддонов пока нет", "No addons yet")).color(th().muted)); }
                        for a in self.addon_list.clone() {
                            let on = s.addons.contains(&a.id);
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                let mut en = on;
                                let title = match &a.manifest { Ok(m) => format!("{}  {}", m.name, m.version), Err(_) => a.id.clone() };
                                if ui.add_enabled(a.manifest.is_ok(), egui::Checkbox::new(&mut en, RichText::new(title).family(semi()))).changed() { acts.push(Act::AddonToggle(a.id.clone(), en)); }
                                let (status, col) = if self.addon_run.is_running(&a.id) { (l!("работает", "running").to_string(), OK) }
                                    else if let Some(msg) = self.addon_run.exited.get(&a.id) { (msg.clone(), BAD) }
                                    else { (if on { l!("не запущен", "not running") } else { l!("выключен", "off") }.to_string(), th().muted) };
                                ui.label(RichText::new(status).color(col).size(12.0));
                                if on && ui.link(l!("перезапустить", "restart")).clicked() { acts.push(Act::AddonRestart(a.id.clone())); }
                                if ui.link(l!("лог", "log")).clicked() { acts.push(Act::Reveal(a.dir.join("addon.log").to_string_lossy().into_owned())); }
                            });
                            let desc = match &a.manifest { Ok(m) if !m.author.is_empty() => format!("{} · {}", m.description, m.author), Ok(m) => m.description.clone(), Err(e) => e.clone() };
                            if !desc.is_empty() { ui.label(RichText::new(desc).color(th().muted).size(12.0)); }
                        }
                        section(ui, l!("\u{E248}  Резервные копии", "\u{E248}  Backups"));
                        let bdir = if s.backup_dir.is_empty() { wavy_core::data_dir().join("backups") } else { PathBuf::from(&s.backup_dir) };
                        ui.label(RichText::new(l!("Раз в день Wavy сохраняет плейлисты, лайки, историю и настройки (14 последних копий). Чтобы копии пережили переустановку Windows, выбери папку на другом диске или в облаке (OneDrive, Google Диск).", "Once a day Wavy saves playlists, likes, history and settings (the last 14 copies). For backups to survive a Windows reinstall, choose a folder on another drive or in the cloud (OneDrive, Google Drive).")).color(th().muted).size(12.0));
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(bdir.to_string_lossy()).color(th().text).size(12.0));
                            if pill(ui, l!("Папка…", "Folder…"), false).clicked() {
                                if let Some(d) = rfd::FileDialog::new().set_title(l!("Куда сохранять копии", "Where to save backups")).pick_folder() { acts.push(Act::Settings(json!({"backupDir": d.to_string_lossy()}))); }
                            }
                            if pill(ui, l!("Открыть", "Open"), false).clicked() { let _ = std::fs::create_dir_all(&bdir); open_folder(&bdir); }
                            if pill(ui, l!("Сделать сейчас", "Back up now"), false).clicked() { self.backup_tick(true); }
                            if pill(ui, l!("Восстановить…", "Restore…"), false).on_hover_text(l!("Заменить библиотеку копией (текущая сохранится рядом)", "Replace the library with a backup (the current one is saved next to it)")).clicked() {
                                if let Some(f) = rfd::FileDialog::new().set_title(l!("Резервная копия Wavy", "Wavy backup")).set_directory(&bdir).add_filter(l!("Копия", "Backup"), &["json"]).pick_file() {
                                    let r = self.store.lock().unwrap().restore(&f);
                                    match r { Ok(n) => self.toast(lf!("Библиотека восстановлена: {n} треков", "Library restored: {n} tracks"), false), Err(e) => self.toast(lf!("Не восстановилось: {e}", "Restore failed: {e}"), true) }
                                }
                            }
                        });
                        if s.last_backup > 0 { ui.label(RichText::new(lf!("Последняя копия: {}", "Last backup: {}", fmt_date(s.last_backup))).color(th().muted).size(12.0)); }
                        section(ui, l!("\u{E036}  Обновления", "\u{E036}  Updates"));
                        match update::current_build() {
                            Some(b) => { ui.label(RichText::new(lf!("Версия: сборка {b}", "Version: build {b}")).color(th().text)); }
                            None => { ui.label(RichText::new(l!("Локальная сборка — обновляется только exe из GitHub Releases", "Local build — only the exe from GitHub Releases updates")).color(th().muted)); }
                        }
                        let mut au = s.auto_update;
                        if ui.checkbox(&mut au, l!("Обновляться автоматически", "Update automatically")).changed() { acts.push(Act::Settings(json!({"autoUpdate": au}))); }
                        match &self.upd {
                            Some(UpdState::Checking) => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(l!("Проверяю…", "Checking…")).color(th().muted)); }); }
                            Some(UpdState::Downloading(n)) => { ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(lf!("Скачиваю сборку {n}…", "Downloading build {n}…")).color(th().muted)); }); }
                            Some(UpdState::Ready(n)) => { ui.label(RichText::new(lf!("Сборка {n} установлена, осталось перезапустить", "Build {n} installed, just restart")).color(OK)); }
                            Some(UpdState::UpToDate) => { ui.label(RichText::new(l!("У тебя последняя версия", "You have the latest version")).color(OK)); }
                            Some(UpdState::Error(e)) => { ui.label(RichText::new(e.as_str()).color(BAD)); }
                            None => {}
                        }
                        ui.horizontal(|ui| {
                            let busy = matches!(self.upd, Some(UpdState::Checking | UpdState::Downloading(_)));
                            if ui.add_enabled(update::current_build().is_some() && !busy, pill_btn(l!("Проверить обновления", "Check for updates"), false)).clicked() { acts.push(Act::CheckUpdate); }
                            if matches!(self.upd, Some(UpdState::Ready(_))) && pill(ui, l!("Перезапустить", "Restart"), true).clicked() { acts.push(Act::Restart); }
                            if let Some(b) = update::current_build() { if pill(ui, l!("Что нового", "What's new"), false).clicked() { acts.push(Act::WhatsNew(b.saturating_sub(5).max(1), b)); } }
                        });
                    });
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| { if pill(ui, l!("Готово", "Done"), true).clicked() { close = true; } });
                }
            }
        });
        if resp.should_close() && !matches!(m, Modal::ImportUrl { busy: true, .. } | Modal::Shazam { state: ShazamState::Adding, .. }) { close = true; }
        if !close { self.modal = Some(m); }
    }

    fn toast_ui(&self, ctx: &egui::Context) {
        let Some((msg, err, _)) = &self.toast else { return };
        egui::Area::new(Id::unique("toast")).anchor(Align2::CENTER_BOTTOM, [0.0, -100.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::new().fill(th().bg3).stroke(Stroke::new(1.0, if *err { BAD } else { th().line })).corner_radius(99).inner_margin(egui::Margin::symmetric(18, 10)).show(ui, |ui| {
                ui.set_max_width(640.0);
                ui.label(RichText::new(msg.as_str()).color(th().text));
            });
        });
    }

    /// In-game overlay: a separate transparent window on top of everything.
    fn sound_ui(&mut self, ui: &mut egui::Ui, s: &wavy_core::store::Settings, acts: &mut Vec<Act>) {
        ui.set_width(330.0);
        ui.label(RichText::new(l!("Вывод", "Output")).family(semi()));
        // an inline list, not a ComboBox: egui keeps one popup open at a time, and a dropdown would close this window.
        // Listing devices is slow on some systems, so it's refreshed every few seconds (headphones plugged in show up)
        if self.devices.as_ref().is_none_or(|d| d.0.elapsed() > Duration::from_secs(3)) { self.devices = Some((Instant::now(), player::output_devices())); }
        let devices = self.devices.as_ref().map(|d| d.1.clone()).unwrap_or_default();
        let missing = !s.audio_device.is_empty() && !devices.contains(&s.audio_device);
        // scrollable: Linux (ALSA) lists every plugin as a device, Windows only the real outputs
        egui::ScrollArea::vertical().id_salt("devices").max_height(130.0).show(ui, |ui| {
            if ui.selectable_label(s.audio_device.is_empty(), l!("Как в системе", "System default")).clicked() && !s.audio_device.is_empty() { acts.push(Act::Settings(json!({"audioDevice": ""}))); }
            for d in devices.iter().chain(missing.then_some(&s.audio_device)) {
                let label = if missing && d == &s.audio_device { lf!("{d} (не подключено — играет по умолчанию)", "{d} (not connected — using the default)") } else { d.clone() };
                if ui.selectable_label(&s.audio_device == d, label).clicked() && &s.audio_device != d { acts.push(Act::Settings(json!({"audioDevice": d}))); }
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new(l!("Скорость", "Speed")).family(semi()));
        ui.horizontal(|ui| {
            for (name, v) in [("Slowed", 0.85f32), (l!("Обычно", "Normal"), 1.0), ("Nightcore", 1.25)] {
                if ui.selectable_label((s.speed - v).abs() < 0.01, name).clicked() { acts.push(Act::Settings(json!({"speed": v}))); }
            }
        });
        let mut sp = s.speed;
        if ui.add(egui::Slider::new(&mut sp, 0.5..=2.0).step_by(0.05).custom_formatter(|v, _| format!("×{v:.2}"))).changed() { acts.push(Act::Settings(json!({"speed": sp}))); }
        ui.label(RichText::new(l!("Тон меняется вместе со скоростью — как в nightcore/slowed", "The pitch changes with the speed — like nightcore/slowed")).color(th().muted).size(11.5));
        ui.separator();
        let mut cf = s.crossfade;
        ui.horizontal(|ui| {
            ui.label(RichText::new(l!("Кроссфейд", "Crossfade")).family(semi()));
            if ui.add(egui::Slider::new(&mut cf, 0.0..=12.0).step_by(1.0).custom_formatter(|v, _| if v == 0.0 { l!("выкл", "off").into() } else { lf!("{v:.0} с", "{v:.0} s") })).changed() { acts.push(Act::Settings(json!({"crossfade": cf}))); }
        });
        let mut nz = s.normalize;
        if ui.checkbox(&mut nz, l!("Выравнивать громкость (тихие треки громче, громкие тише)", "Normalize volume (quiet tracks louder, loud ones quieter)")).changed() { acts.push(Act::Settings(json!({"normalize": nz}))); }
        ui.separator();
        let mut on = s.eq_on;
        ui.horizontal(|ui| {
            if ui.checkbox(&mut on, RichText::new(l!("Эквалайзер", "Equalizer")).family(semi())).changed() { acts.push(Act::Settings(json!({"eqOn": on}))); }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button(l!("Пресеты", "Presets"), |ui| {
                    for (name, eq) in fx::PRESETS {
                        if ui.button(wavy_core::i18n::tr(name)).clicked() { acts.push(Act::Settings(json!({"eq": eq, "eqOn": true}))); ui.close(); }
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
                        changed |= ui.add(egui::Slider::new(&mut eq[i], -12.0..=12.0).vertical().step_by(1.0).show_value(false)).on_hover_text(lf!("{:+.0} дБ", "{:+.0} dB", eq[i])).changed();
                        ui.label(RichText::new(if *f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") }).size(10.0).color(th().muted));
                    });
                }
                if changed { acts.push(Act::Settings(json!({"eq": eq}))); }
            });
        });
    }

    /// Mini player: a small always-on-top window, draggable with the mouse.
    fn mini_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let s = self.settings();
        let pos = s.mini_pos.unwrap_or([60.0, 60.0]);
        let vb = egui::ViewportBuilder::default().with_title("Wavy mini").with_decorations(false).with_always_on_top()
            .with_taskbar(false).with_resizable(false).with_position(pos).with_inner_size([360.0, 84.0]);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("wavy-mini"), vb, |ui, _| {
            let ctx = ui.ctx().clone();
            let rect = ui.max_rect();
            let p = ui.painter().clone();
            p.rect_filled(rect, 0, th().bg2);
            p.rect_stroke(rect, 0, Stroke::new(1.0, th().line), egui::StrokeKind::Inside);
            if ctx.input(|i| i.viewport().close_requested()) { acts.push(Act::ToggleMini); }
            // drag by any empty spot
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
                        None => { p.rect_filled(cr, 8, th().bg3); glyph(&p, cr.center(), "\u{E33C}", FontId::proportional(22.0), th().muted); }
                    }
                    let tx = cr.right() + 12.0;
                    let w = rect.right() - 128.0 - tx;
                    let clip = Rect::from_min_max(Pos2::new(tx, rect.top()), Pos2::new(tx + w, rect.bottom()));
                    let pc = p.with_clip_rect(clip);
                    pc.text(Pos2::new(tx, cr.top() + 12.0), Align2::LEFT_CENTER, &t.title, FontId::new(14.0, semi()), th().text);
                    pc.text(Pos2::new(tx, cr.top() + 32.0), Align2::LEFT_CENTER, wavy_core::i18n::tr(&t.artist), FontId::proportional(12.0), th().muted);
                    // progress
                    let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
                    let f = if dur > 0.0 { (self.audio.position() / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
                    let bar = Rect::from_min_size(Pos2::new(tx, cr.bottom() - 6.0), Vec2::new(w, 3.0));
                    p.rect_filled(bar, 2, th().bg3);
                    p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * f, 3.0)), 2, th().text);
                }
                None => { p.text(Pos2::new(cr.right() + 12.0, rect.center().y), Align2::LEFT_CENTER, l!("Ничего не играет", "Nothing is playing"), FontId::proportional(13.0), th().muted); }
            }
            // buttons
            let mut x = rect.right() - 120.0;
            let cy = rect.center().y;
            for (icon, act, big) in [("\u{E5A4}", 0, false), (if self.audio.is_playing() { "\u{E39E}" } else { "\u{E3D0}" }, 1, true), ("\u{E5A6}", 2, false)] {
                let sz = if big { 40.0 } else { 32.0 };
                let r = Rect::from_center_size(Pos2::new(x + sz / 2.0, cy), Vec2::splat(sz));
                let resp = ui.interact(r, Id::unique(("mini-btn", act)), Sense::click());
                if big { p.circle_filled(r.center(), sz / 2.0, if resp.hovered() { th().accent } else { th().text }); }
                else if resp.hovered() { p.circle_filled(r.center(), sz / 2.0, th().bg3); }
                let font = if big { FontId::new(18.0, egui::FontFamily::Name("fill".into())) } else { FontId::proportional(16.0) };
                glyph(&p, r.center(), icon, font, if big { th().bg } else { th().text });
                if resp.clicked() { acts.push([Act::Prev, Act::TogglePlay, Act::Next].into_iter().nth(act).unwrap()); }
                x += sz + 2.0;
            }
            // close
            let xr = Rect::from_min_size(Pos2::new(rect.right() - 18.0, rect.top() + 2.0), Vec2::splat(16.0));
            let xresp = ui.interact(xr, Id::unique("mini-x"), Sense::click());
            p.text(xr.center(), Align2::CENTER_CENTER, "×", FontId::proportional(14.0), if xresp.hovered() { th().text } else { th().muted });
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
                p.rect_filled(rect, 8, if th().dark { Color32::from_white_alpha(12) } else { Color32::from_black_alpha(10) });
                p.rect_stroke(rect.shrink(1.0), 8, Stroke::new(2.0, th().accent), egui::StrokeKind::Inside);
                p.text(rect.center_top() + Vec2::new(0.0, 10.0), Align2::CENTER_CENTER, l!("тяни — двигать · угол — ресайз · Ctrl+Shift+L — закрепить", "drag — move · corner — resize · Ctrl+Shift+L — lock"), FontId::proportional(11.0), th().accent_soft);
                let resp = ui.interact(rect, Id::unique("ov-drag"), Sense::click_and_drag());
                let grip = Rect::from_min_size(rect.right_bottom() - Vec2::splat(16.0), Vec2::splat(16.0));
                p.line_segment([grip.right_top(), grip.left_bottom()], Stroke::new(2.0, th().accent));
                if resp.drag_started() {
                    let at = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
                    ctx.send_viewport_cmd(if grip.contains(at) { egui::ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast) } else { egui::ViewportCommand::StartDrag });
                }
                // remember the position
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
            // the overlay window has its own three styles: the newer OBS styles show as the closest one
            let theme = match s.overlay.theme.as_str() { "text" => "minimal", "compact" | "vertical" => "card", t => t };
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
                    None => { p.rect_filled(cr, 10, a(theme::overlay().bg3)); p.text(cr.center(), Align2::CENTER_CENTER, "\u{E340}", FontId::proportional(cs * 0.4), a(theme::overlay().accent)); }
                },
            }
            let tw = card.right() - tx - 14.0;
            let mut yy = if minimal { card.center().y - 38.0 } else { cr.top() + 2.0 };
            // in minimal the text sits right over the game — give it a shadow
            let text = |pos: Pos2, align: Align2, txt: String, font: FontId, c: Color32| {
                if minimal { p.text(pos + Vec2::new(1.0, 1.5), align, &txt, font.clone(), Color32::from_black_alpha((200.0 * fade) as u8)); }
                p.text(pos, align, txt, font, c);
            };
            text(Pos2::new(tx, yy), Align2::LEFT_TOP, if playing { l!("\u{E340}  СЕЙЧАС ИГРАЕТ", "\u{E340}  NOW PLAYING").into() } else { l!("\u{E39E}  ПАУЗА", "\u{E39E}  PAUSED").into() }, FontId::proportional(10.5), a(theme::overlay().accent_soft));
            yy += 16.0;
            text(Pos2::new(tx, yy), Align2::LEFT_TOP, ellipsize(&t.title, tw, 18.0), FontId::new(18.0, bold()), a(Color32::WHITE));
            yy += 24.0;
            text(Pos2::new(tx, yy), Align2::LEFT_TOP, ellipsize(wavy_core::i18n::tr(&t.artist), tw, 13.5), FontId::proportional(13.5), a(Color32::from_white_alpha(190)));
            yy += 24.0;
            let dur = if self.audio.duration > 0.0 { self.audio.duration } else { t.duration };
            let pos = self.resume.as_ref().map(|r| r.1).unwrap_or_else(|| self.audio.position());
            let bw = if minimal { tw.min(320.0) } else { tw };
            let bar = Rect::from_min_size(Pos2::new(tx, yy), Vec2::new(bw, 4.0));
            p.rect_filled(bar, 2, a(Color32::from_white_alpha(40)));
            if dur > 0.0 { p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bw * (pos / dur).clamp(0.0, 1.0) as f32, 4.0)), 2, a(theme::overlay().accent)); }
            if !minimal {
                p.text(Pos2::new(tx, yy + 8.0), Align2::LEFT_TOP, fmt_time(pos), FontId::proportional(10.5), a(Color32::from_white_alpha(140)));
                p.text(Pos2::new(tx + tw, yy + 8.0), Align2::RIGHT_TOP, fmt_time(dur), FontId::proportional(10.5), a(Color32::from_white_alpha(140)));
            }
            if hide_after > 0 && fade > 0.0 { ctx.request_repaint_after(Duration::from_millis(100)); }
        });
    }
}

enum AudioSrc { File(PathBuf), Stream(wavy_core::stream::Download) }

/// "Video is gone" error: deleted, private, blocked in the country or after a complaint.
/// YouTube asks to sign in to confirm age (18+ video). The "not a bot" check doesn't count here:
/// it affects all videos, and a replacement won't help.
fn age_restricted(err: &str) -> bool {
    let e = err.to_lowercase();
    ["confirm your age", "age-restricted", "age restricted", "inappropriate for some users"].iter().any(|m| e.contains(m))
}

/// 18+ video with cookies from settings: only streaming through yt-dlp (rustypipe can't do cookies).
fn fetch_with_cookies(yt: &YouTube, t: &Track, cache: &Path) -> Result<AudioSrc, String> {
    let key = t.url.clone().unwrap_or_default();
    let path = cache.join(format!("{}.s.m4a", wavy_core::sha16(key.as_bytes())));
    let dl = yt.stream_audio(t, &path, true)?;
    if dl.wait_for(200 * 1024, Duration::from_secs(60)) { return Ok(AudioSrc::Stream(dl)); }
    let e = dl.error().unwrap_or_else(|| l!("долго нет данных", "no data for a long time").into());
    dl.cancel();
    Err(e)
}

/// A readable reason why the cookies didn't work.
fn cookie_hint(err: &str) -> String {
    let e = err.to_lowercase();
    if e.contains("could not copy") || e.contains("database is locked") { l!("браузер держит файл кук — закрой его полностью и попробуй ещё раз", "the browser holds the cookie file — close it completely and try again").into() }
    else if e.contains("decrypt") || e.contains("dpapi") { l!("Chrome/Edge шифруют куки так, что их не достать — возьми Firefox или файл cookies.txt", "Chrome/Edge encrypt cookies so they can't be read — use Firefox or a cookies.txt file").into() }
    else if e.contains("netscape") { l!("это не файл cookies.txt — выгрузи его расширением «Get cookies.txt LOCALLY» на youtube.com", "this isn't a cookies.txt file — export it with the “Get cookies.txt LOCALLY” extension on youtube.com").into() }
    else if e.contains("could not find") { l!("не нашёл профиль браузера — он установлен?", "couldn't find the browser profile — is it installed?").into() }
    else if age_restricted(&e) { l!("в браузере нет входа в YouTube (или аккаунту меньше 18)", "the browser isn't signed in to YouTube (or the account is under 18)").into() }
    else { err.lines().last().unwrap_or(err).chars().take(160).collect() }
}

fn gone_from_youtube(err: &str) -> bool {
    let e = err.to_lowercase();
    ["video unavailable", "not available", "private video", "has been removed", "been terminated", "copyright", "unavailable"].iter().any(|m| e.contains(m))
}

/// Audio for a YouTube track: finished file from the cache → stream through rustypipe (plays from the first KB) → yt-dlp.
fn fetch_audio(yt: &YouTube, t: &Track, cache: &Path) -> Result<AudioSrc, String> {
    let _ = std::fs::create_dir_all(cache);
    let key = t.url.clone().unwrap_or_default();
    let path = cache.join(format!("{}.s.m4a", wavy_core::sha16(key.as_bytes())));
    if wavy_core::stream::is_complete(&path) { return Ok(AudioSrc::File(path)); }
    if let Some(vid) = &t.video_id {
        match yt.direct(vid, false) {
            Ok(d) => {
                let dl = wavy_core::stream::Download::start(d.url, path.clone(), d.size);
                // ~200 KB is enough for the decoder to start
                if dl.wait_for(200 * 1024, Duration::from_secs(20)) { return Ok(AudioSrc::Stream(dl)); }
                wavy_core::log(&format!("поток «{}» не пошёл: {:?} — пробую yt-dlp", t.title, dl.error()));
                dl.cancel();
                yt.forget(vid);
            }
            Err(e) => wavy_core::log(&format!("rustypipe «{}»: {e} — пробую yt-dlp", t.title)),
        }
        // streaming through yt-dlp: play while it downloads (since 2025 YouTube often rejects rustypipe URLs — 403)
        let t0 = std::time::Instant::now();
        match yt.stream_audio(t, &path, false) {
            Ok(dl) => {
                if dl.wait_for(200 * 1024, Duration::from_secs(40)) {
                    wavy_core::log(&format!("«{}»: поток yt-dlp пошёл через {:.1} с", t.title, t0.elapsed().as_secs_f32()));
                    return Ok(AudioSrc::Stream(dl));
                }
                let e = dl.error().unwrap_or_else(|| l!("долго нет данных", "no data for a long time").into());
                dl.cancel();
                // the video is gone — no point downloading further; let a replacement be found
                if gone_from_youtube(&e) || age_restricted(&e) { return Err(e); }
                wavy_core::log(&format!("поток yt-dlp «{}» не пошёл: {e} — качаю целиком", t.title));
            }
            Err(e) => wavy_core::log(&format!("yt-dlp потоком «{}»: {e}", t.title)),
        }
    }
    yt.download_audio(t, cache).map(AudioSrc::File)
}

/// Playlist from YouTube Music → library: the same playlist (by `source`) is updated, otherwise created.
fn save_ytm_playlist(store: &Arc<Mutex<Store>>, source: &str, name: &str, tracks: Vec<Track>) -> String {
    let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
    let mut s = store.lock().unwrap();
    s.add_tracks(tracks);
    if let Some(id) = s.lib.playlists.iter().find(|p| p.source.as_deref() == Some(source)).map(|p| p.id.clone()) {
        s.set_playlist_tracks(&id, ids);
        id
    } else { s.create_playlist(name, ids, Some(source.to_string())).id }
}

/// Link → tracks: Spotify (through YouTube search) or anything yt-dlp supports.
/// Returns (playlist name, source, tracks, note for the user).
fn fetch_url(url: &str, yt: &YouTube, store: &Arc<Mutex<Store>>, tx: &Sender<Bg>) -> Result<(Option<String>, String, Vec<Track>, String, Vec<wavy_core::spotify::SpTrack>), String> {
    let t2 = tx.clone();
    let status = move |m: &str| { let _ = t2.send(Bg::Status(m.into())); };
    if let Some((kind, _)) = wavy_core::spotify::parse_url(url) {
        let sp = store.lock().unwrap().lib.settings.spotify.clone();
        let r = wavy_core::spotify::import(url, yt, Some((&sp.client_id, &sp.client_secret)), &status)?;
        let mut note = String::new();
        if !r.not_found.is_empty() { note += &lf!(", не нашлось на YouTube: {} — можно выбрать вручную", ", not found on YouTube: {} — you can pick them manually", r.not_found.len()); }
        if r.truncated { note += l!(". Spotify без ключа отдаёт только первые 100 треков — ключ добавляется в Настройках", ". Spotify without a key only gives the first 100 tracks — the key is added in Settings"); }
        Ok(((kind != "track").then_some(r.name), r.source, r.tracks, note, r.not_found))
    } else {
        let r = yt.import_url(url.trim(), &status)?;
        Ok((r.name, r.source, r.tracks, String::new(), Vec::new()))
    }
}

// ---------- small widgets ----------
fn track_menu(ui: &mut egui::Ui, ids: &[String], t: &Track, pid: Option<&str>, pls: &[(String, String)], acts: &mut Vec<Act>) {
    let ids = ids.to_vec();
    if ui.button(l!("Играть далее", "Play next")).clicked() { acts.push(Act::PlayNext(ids.clone())); ui.close(); }
    if ui.button(l!("Добавить в очередь", "Add to queue")).clicked() { acts.push(Act::Enqueue(ids.clone())); ui.close(); }
    ui.separator();
    ui.menu_button(l!("Добавить в плейлист", "Add to playlist"), |ui| {
        if ui.button(l!("+ Новый плейлист…", "+ New playlist…")).clicked() { acts.push(Act::Modal(Modal::NewPlaylist { name: String::new(), ids: ids.clone() })); ui.close(); }
        if !pls.is_empty() { ui.separator(); }
        for (id, name) in pls { if ui.button(name).clicked() { acts.push(Act::AddTo(id.clone(), ids.clone())); ui.close(); } }
    });
    let lbl = if ids.len() > 1 { lf!("Сохранить как MP3 ({})…", "Save as MP3 ({})…", ids.len()) } else { l!("Сохранить как MP3…", "Save as MP3…").into() };
    if ui.button(lbl).clicked() { acts.push(Act::Export(ids.clone(), None)); ui.close(); }
    if let Some(p) = pid { if ui.button(l!("Убрать из плейлиста", "Remove from playlist")).clicked() { acts.push(Act::RemoveFrom(p.into(), ids.clone())); ui.close(); } }
    if ids.len() == 1 {
        if t.video_id.is_some() && ui.button(l!("Радио по треку", "Track radio")).on_hover_text(l!("Похожие песни с YouTube — в очередь", "Similar songs from YouTube — into the queue")).clicked() { acts.push(Act::Radio(t.id.clone())); ui.close(); }
        if t.video_id.is_some() && t.anime.is_none() && ui.button(l!("Выбрать другое видео…", "Pick another video…")).on_hover_text(l!("Если нашлось не то: другая версия, кавер, клип вместо песни", "If the wrong one was found: another version, a cover, a music video instead of the song")).clicked() {
            let q = if t.artist.is_empty() || t.artist == "YouTube" { t.title.clone() } else { format!("{} {}", t.artist.split(',').next().unwrap_or("").trim(), t.title) };
            acts.push(Act::Modal(Modal::Pick { query: q.clone(), results: Vec::new(), busy: true, err: None, target: PickTarget::Replace(t.id.clone()), back: Vec::new() }));
            acts.push(Act::PickSearch(q));
            ui.close();
        }
        if ui.button(l!("Изменить…", "Edit…")).clicked() { acts.push(Act::Modal(Modal::EditTrack { id: t.id.clone(), title: t.title.clone(), artist: t.artist.clone(), album: t.album.clone() })); ui.close(); }
        if let Some(p) = &t.path { if ui.button(l!("Показать в папке", "Show in folder")).clicked() { acts.push(Act::Reveal(p.clone())); ui.close(); } }
        if let Some(u) = &t.url { if ui.button(l!("Открыть в браузере", "Open in browser")).clicked() { acts.push(Act::OpenUrl(u.clone())); ui.close(); } }
    }
    ui.separator();
    let label = if ids.len() > 1 { lf!("Удалить из библиотеки ({})", "Delete from library ({})", ids.len()) } else { l!("Удалить из библиотеки", "Delete from library").into() };
    if ui.button(RichText::new(label).color(BAD)).clicked() { acts.push(Act::RemoveTracks(ids)); ui.close(); }
}

/// Thin slider bar (seek, volume): white fill, handle on hover.
fn bar(ui: &mut egui::Ui, frac: f32, width: f32) -> (egui::Response, Option<f32>) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::click_and_drag());
    let new = if resp.dragged() || resp.clicked() {
        resp.interact_pointer_pos().map(|p| ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0))
    } else { None };
    let f = new.unwrap_or(frac).clamp(0.0, 1.0);
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 4.0));
    let active = resp.hovered() || resp.dragged();
    ui.painter().rect_filled(track, 2, th().track);
    ui.painter().rect_filled(Rect::from_min_size(track.min, Vec2::new(track.width() * f, 4.0)), 2, if active { th().accent } else { th().text });
    if active { ui.painter().circle_filled(Pos2::new(track.left() + track.width() * f, track.center().y), 6.0, th().accent); }
    (resp, new)
}

fn nav(ui: &mut egui::Ui, text: &str, active: bool, count: Option<usize>) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    if active || resp.hovered() { ui.painter().rect_filled(r, 8, th().bg3); }
    let c = if active || resp.hovered() { th().text } else { th().muted };
    let w = r.width() - if count.is_some() { 50.0 } else { 20.0 };
    ui.painter().text(r.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, ellipsize(text, w, 14.0), FontId::proportional(14.0), c);
    if let Some(n) = count { ui.painter().text(r.right_center() - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER, n.to_string(), FontId::proportional(12.0), th().muted); }
    resp
}

fn icon_btn(ui: &mut egui::Ui, icon: &str, size: f32, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if resp.hovered() { ui.painter().circle_filled(r.center(), size / 2.0, th().bg3); }
    let c = if on { th().accent } else if resp.hovered() { th().text } else { th().muted };
    // with an indicator dot the icon is raised a bit so the "icon + dot" pair is centered
    glyph(ui.painter(), r.center() - Vec2::new(0.0, if on { 2.0 } else { 0.0 }), icon, FontId::proportional(size * 0.5), c);
    if on { ui.painter().circle_filled(r.center_bottom() - Vec2::new(0.0, 2.0), 2.0, th().accent); }
    resp
}

/// Icon exactly centered: by the glyph's own outline, not the font's line height
/// (in the icon font glyphs sit differently in the line — hence the "slightly off-center" buttons).
fn glyph(p: &egui::Painter, center: Pos2, icon: &str, font: FontId, color: Color32) {
    let g = p.layout_no_wrap(icon.to_string(), font, color);
    let b = g.mesh_bounds;
    let at = if b.is_positive() { center - b.center().to_vec2() } else { center - g.rect.center().to_vec2() };
    p.galley(at, g, color);
}

fn pill_btn(text: &str, primary: bool) -> egui::Button<'_> {
    let b = egui::Button::new(RichText::new(text).color(if primary { th().on_accent } else { th().text }).family(semi())).corner_radius(99).min_size(Vec2::new(0.0, 32.0));
    if primary { b.fill(th().accent).stroke(Stroke::NONE) } else { b.fill(th().bg3).stroke(Stroke::new(1.0, th().line)) }
}
fn pill(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response { ui.add(pill_btn(text, primary)) }

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(15.0).family(semi()).color(th().text));
    ui.add_space(4.0);
}

/// Vinyl: the cover is stretched over a circle (fan mesh) and spins.
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
        None => { p.circle_filled(c, inner, a(theme::overlay().accent)); }
    }
    for k in [0.55f32, 0.75] { p.circle_stroke(c, r * k, Stroke::new(1.0, a(Color32::from_black_alpha(60)))); }
    p.circle_filled(c, r * 0.16, a(Color32::from_rgb(12, 12, 14)));
    p.circle_stroke(c, r * 0.16, Stroke::new(1.5, a(Color32::from_white_alpha(60))));
    // highlight
    let (sn, cs) = rot.sin_cos();
    p.circle_filled(c + Vec2::new(cs, sn) * r * 0.62, r * 0.05, a(Color32::from_white_alpha(70)));
}

/// Subtitles: white text with a black outline at the bottom of the frame (\an8 lines at the top).
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
    // each line separately — that way all lines are centered
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
        ui.painter().rect_stroke(r.shrink(40.0), 24, Stroke::new(2.0, th().accent), egui::StrokeKind::Inside);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, l!("Отпусти, чтобы добавить музыку \u{E340}", "Drop to add music \u{E340}"), FontId::proportional(22.0), th().text);
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

/// Whether the text has Japanese/Chinese/Korean characters (Inter doesn't draw them — they'd be boxes).
fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F))
}

/// System CJK font: not embedded in the exe (+10 MB) but taken from Windows,
/// and only once such characters actually appear. Korean uses a separate font, if available.
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
    // Inter (OFL) is the main font; Phosphor icons (MIT) are the fallback, plus the "fill" variant for the play button
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("inter".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/Inter-Regular.otf"))));
    fonts.font_data.insert("inter-semi".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/Inter-SemiBold.otf"))));
    fonts.font_data.insert("inter-bold".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/Inter-Bold.otf"))));
    // Inter's Private Use Area glyphs are cut out (assets/README) so they don't shadow the icons
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
    // CJK last: used only for characters Inter doesn't have
    for (name, bytes) in extra {
        fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes.clone())));
        for fam in fonts.families.values_mut() { fam.push(name.clone()); }
    }
    fonts
}

fn style(ctx: &egui::Context) {
    ctx.set_fonts(font_defs(&[]));
    apply_theme(ctx);
}

/// Colors and spacing from the current theme (called at startup and when the theme changes).
fn apply_theme(ctx: &egui::Context) {
    let t = th();
    ctx.set_visuals(if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() });
    ctx.all_styles_mut(|s| {
        let v = &mut s.visuals;
        v.panel_fill = th().bg;
        v.window_fill = th().bg2;
        v.extreme_bg_color = th().bg3;
        v.faint_bg_color = th().bg2;
        v.selection.bg_fill = Color32::from_rgba_unmultiplied(t.accent.r(), t.accent.g(), t.accent.b(), if t.dark { 30 } else { 40 });
        v.selection.stroke = Stroke::new(1.0, th().text);
        v.hyperlink_color = th().accent;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, th().line);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, th().text);
        v.widgets.inactive.bg_fill = th().bg3;
        v.widgets.inactive.weak_bg_fill = th().bg3;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, th().text);
        v.widgets.hovered.weak_bg_fill = t.hover;
        v.widgets.hovered.bg_fill = t.hover2;
        v.widgets.active.bg_fill = t.active;
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

/// Custom playlist cover: center square, up to 600 px, JPEG in the covers folder
/// (a new name every time — so the old image from the thumbnail cache doesn't show).
fn playlist_cover(src: &Path, covers: &Path, id: &str) -> Result<PathBuf, String> {
    let img = image::open(src).map_err(|e| lf!("не открылась картинка: {e}", "couldn't open the image: {e}"))?;
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
    // one Wavy at a time: after an update the old process quits; a repeated launch shows the window
    let restarted = std::env::args().any(|a| a == "--restarted");
    let listener = match instance::start(restarted) {
        instance::Start::First(l) => l,
        instance::Start::AlreadyRunning => return Ok(()),
    };
    // give the old process time to release the overlay port and files
    if restarted { std::thread::sleep(Duration::from_millis(500)); }
    // panic → log (the Windows build has no console)
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
    eframe::run_native("Wavy", opts, Box::new(move |cc| {
        let mut app = App::new(cc);
        let c = cc.egui_ctx.clone();
        app.instance = Some(instance::listen(listener, move || c.request_repaint()));
        Ok(Box::new(app))
    }))
}

/// Unix milliseconds → "21.11.2026".
fn fmt_date(ms: u64) -> String {
    // days since 1970-01-01 → date (Howard Hinnant's algorithm)
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

fn fmt_bytes(b: u64) -> String {
    if b >= 1 << 30 { lf!("{:.1} ГБ", "{:.1} GB", b as f64 / (1u64 << 30) as f64) } else { lf!("{} МБ", "{} MB", b / (1 << 20)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_errors() {
        let age = "ERROR: [youtube] pLT99PuEOmY: Sign in to confirm your age. This video may be inappropriate for some users.";
        assert!(age_restricted(age));
        assert!(age_restricted("ERROR: [youtube] x: Sign in to confirm your age. This video is age-restricted and only available on YouTube."));
        // the "not a bot" check isn't about age: a replacement won't help here
        assert!(!age_restricted("ERROR: [youtube] x: Sign in to confirm you’re not a bot."));
        assert!(!gone_from_youtube(age));
        assert!(cookie_hint("ERROR: could not find firefox cookies database in '/root/.mozilla/firefox'").contains("профиль"));
        assert!(cookie_hint("ERROR: 'c.txt' does not look like a Netscape format cookies file").contains("cookies.txt"));
        assert!(cookie_hint("ERROR: Failed to decrypt with DPAPI").contains("Firefox"));
        assert!(cookie_hint(age).contains("нет входа"));
    }
}
