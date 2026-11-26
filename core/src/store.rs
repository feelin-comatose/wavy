use crate::{now_ms, Track};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub track_ids: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    /// custom cover image (file in data_dir/covers)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// folder in the sidebar
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

impl Playlist {
    pub fn is_remote(&self) -> bool {
        self.source.as_deref().is_some_and(|s| s.starts_with("http"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscordSettings { pub enabled: bool, pub client_id: String, pub show_buttons: bool }
impl Default for DiscordSettings { fn default() -> Self { Self { enabled: false, client_id: String::new(), show_buttons: true } } }

/// Optional Spotify Web API key — only needed for playlists over 100 tracks.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpotifySettings { pub client_id: String, pub client_secret: String }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlaySettings { pub port: u16, pub theme: String, pub position: String, pub auto_hide_sec: u32, pub show_queue: bool, pub show_lyrics: bool }
impl Default for OverlaySettings { fn default() -> Self { Self { port: 3737, theme: "card".into(), position: "bottom-left".into(), auto_hide_sec: 0, show_queue: false, show_lyrics: false } } }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlayWindowSettings { pub visible: bool, pub locked: bool, pub bounds: Option<[f32; 4]> }
impl Default for OverlayWindowSettings { fn default() -> Self { Self { visible: false, locked: true, bounds: None } } }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: String, // off | all | one
    pub discord: DiscordSettings,
    pub overlay: OverlaySettings,
    pub overlay_window: OverlayWindowSettings,
    /// Download and install new versions from GitHub Releases automatically.
    pub auto_update: bool,
    /// Interface language: "ru" or "en".
    pub lang: String,
    /// Interface theme id (see native/src/theme.rs; "dark" is the original black-and-white look).
    pub theme: String,
    /// Custom accent color "#rrggbb" on top of the theme; empty — the theme's own accent.
    pub accent: String,
    /// Enabled addons (folder names in data_dir/addons), started when Wavy launches.
    pub addons: Vec<String>,
    /// Audio output device name; empty — the system default.
    pub audio_device: String,
    pub spotify: SpotifySettings,
    /// Sorting: "library" or playlist id → method (see Sort).
    pub sort: HashMap<String, String>,
    /// Build the user has already seen (for the "What's new" dialog after an update).
    pub last_build: Option<u32>,
    /// Mini player: whether it's open and where.
    pub mini: bool,
    pub mini_pos: Option<[f32; 2]>,
    /// The close button minimizes to the tray instead of quitting (Windows).
    pub close_to_tray: bool,
    /// Sound: speed (1.0 — as is), crossfade in seconds (0 — off), volume normalization, equalizer.
    pub speed: f32,
    pub crossfade: f32,
    pub normalize: bool,
    pub eq_on: bool,
    pub eq: [f32; 10],
    /// Anime: skip openings automatically (via AniSkip), offer the next episode during the ending, Shikimori account.
    pub anime_autoskip: bool,
    pub anime_next: bool,
    pub shiki: crate::shikimori::Account,
    /// Twitch chat song requests: channel, enabled or not, max track length (min).
    pub twitch_channel: String,
    pub twitch_sr: bool,
    pub twitch_max_min: u32,
    /// Twitch sign-in (to reply in chat), how many "!skip" votes skip a track (0 — off),
    /// the channel points reward that requests a track (id; empty — none).
    pub twitch_account: crate::twitch::Account,
    pub twitch_skip_votes: u32,
    pub twitch_reward: String,
    /// Write "Artist — Title" to now_playing.txt (for OBS "Read from file").
    pub now_playing_file: bool,
    /// Cache limit (GB, 0 — no limit): old files are removed automatically.
    pub cache_limit_gb: u32,
    /// Wi-Fi sync with the phone: enabled or not, and the PIN.
    pub wifi_sync: bool,
    pub wifi_pin: String,
    /// YouTube cookies for 18+ videos: browser name (firefox, chrome, edge…) or path to cookies.txt. Empty — off.
    pub yt_cookies: String,
    /// YouTube Music account through these cookies: likes, playlists and "My wave".
    pub yt_account: bool,
    /// Global hotkeys: action → "Ctrl+Alt+P" (empty — disabled; no key — default).
    pub hotkeys: HashMap<String, String>,
    /// Last.fm scrobbling.
    pub lastfm: crate::lastfm::Account,
    /// Daily library auto-backup: folder (empty — data_dir/backups) and when the last one ran (ms).
    pub backup_dir: String,
    pub last_backup: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self { volume: 0.8, shuffle: false, repeat: "off".into(), discord: Default::default(), overlay: Default::default(), overlay_window: Default::default(), auto_update: true, lang: "ru".into(), theme: "dark".into(), accent: String::new(), addons: Vec::new(), audio_device: String::new(), spotify: Default::default(), sort: HashMap::new(), last_build: None, mini: false, mini_pos: None, close_to_tray: false, speed: 1.0, crossfade: 0.0, normalize: false, eq_on: false, eq: [0.0; 10], anime_autoskip: false, anime_next: true, shiki: Default::default(), twitch_channel: String::new(), twitch_sr: false, twitch_max_min: 10, twitch_account: Default::default(), twitch_skip_votes: 3, twitch_reward: String::new(), now_playing_file: false, cache_limit_gb: 0, wifi_sync: false, wifi_pin: String::new(), yt_cookies: String::new(), yt_account: false, hotkeys: HashMap::new(), lastfm: Default::default(), backup_dir: String::new(), last_backup: 0 }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub tracks: HashMap<String, Track>,
    pub playlists: Vec<Playlist>,
    pub settings: Settings,
    /// "Anime" section: titles; their episodes live in tracks (with the anime field).
    pub anime: Vec<crate::anime::Anime>,
    /// How many times a track was played (over 30 seconds) — for the "Most played" sort.
    pub plays: HashMap<String, u32>,
    /// Liked tracks, newest first (the "Liked" section).
    pub liked: Vec<String>,
    /// Listening history: (track, when, ms) — for "Recap". The last 50,000 are kept.
    pub history: Vec<(String, u64)>,
}

pub struct Store {
    file: PathBuf,
    pub lib: Library,
}

impl Store {
    pub fn open(dir: &std::path::Path) -> Self {
        let file = dir.join("library.json");
        let lib = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Self { file, lib }
    }

    pub fn save(&self) {
        let tmp = self.file.with_extension("json.tmp");
        if let Ok(s) = serde_json::to_string(&self.lib) {
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &self.file);
            }
        }
    }

    /// Back up the library (playlists, tracks, likes, history, settings) to `dir`.
    /// Keeps the last `keep` copies and deletes older ones.
    pub fn backup(&self, dir: &std::path::Path, name: &str, keep: usize) -> Result<std::path::PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| lf!("папка недоступна: {e}", "the folder is unavailable: {e}"))?;
        let path = dir.join(name);
        let s = serde_json::to_string(&self.lib).map_err(|e| e.to_string())?;
        std::fs::write(&path, s).map_err(|e| lf!("не записалось: {e}", "couldn't write: {e}"))?;
        let mut old: Vec<std::path::PathBuf> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| { let n = n.to_string_lossy(); n.starts_with("wavy-backup-") && n.ends_with(".json") })).collect();
        old.sort();
        while old.len() > keep { let _ = std::fs::remove_file(old.remove(0)); }
        Ok(path)
    }

    /// Restore the library from a backup. The current library is saved alongside first (library.before-restore.json).
    pub fn restore(&mut self, path: &std::path::Path) -> Result<usize, String> {
        let text = std::fs::read_to_string(path).map_err(|e| lf!("не читается: {e}", "can't be read: {e}"))?;
        let lib: Library = serde_json::from_str(&text).map_err(|_| l!("это не резервная копия Wavy", "this isn't a Wavy backup").to_string())?;
        if lib.tracks.is_empty() && lib.playlists.is_empty() { return Err(l!("копия пустая", "the backup is empty").into()); }
        let _ = std::fs::copy(&self.file, self.file.with_file_name("library.before-restore.json"));
        let n = lib.tracks.len();
        self.lib = lib;
        self.save();
        Ok(n)
    }

    /// Apply a JSON patch to the settings (deep merge).
    pub fn patch_settings(&mut self, patch: &Value) {
        let mut cur = serde_json::to_value(&self.lib.settings).unwrap_or(Value::Null);
        merge(&mut cur, patch);
        if let Ok(s) = serde_json::from_value(cur) {
            self.lib.settings = s;
        }
        self.save();
    }

    pub fn add_tracks(&mut self, tracks: Vec<Track>) {
        for mut t in tracks {
            if let Some(old) = self.lib.tracks.get(&t.id) {
                t.added_at = old.added_at; // keep the order they were added in
            }
            self.lib.tracks.insert(t.id.clone(), t);
        }
        self.save();
    }

    pub fn remove_tracks(&mut self, ids: &[String]) {
        for id in ids {
            self.lib.tracks.remove(id);
        }
        for p in &mut self.lib.playlists {
            p.track_ids.retain(|t| !ids.contains(t));
        }
        self.lib.liked.retain(|t| !ids.contains(t));
        self.save();
    }

    /// Reorder tracks within a playlist: `moving` go before position `to` (index in the original list).
    pub fn move_in_playlist(&mut self, pid: &str, moving: &[String], to: usize) {
        let Some(p) = self.lib.playlists.iter_mut().find(|p| p.id == pid) else { return };
        p.track_ids = move_items(&p.track_ids, moving, to);
        self.save();
    }

    pub fn move_liked(&mut self, moving: &[String], to: usize) {
        self.lib.liked = move_items(&self.lib.liked, moving, to);
        self.save();
    }

    pub fn create_playlist(&mut self, name: &str, track_ids: Vec<String>, source: Option<String>) -> Playlist {
        let p = Playlist { id: format!("pl_{:x}{:04x}", now_ms(), rand16()), name: name.into(), track_ids, source, created_at: now_ms(), cover: None, folder: None };
        self.lib.playlists.push(p.clone());
        self.save();
        p
    }

    pub fn playlist_mut(&mut self, id: &str) -> Option<&mut Playlist> {
        self.lib.playlists.iter_mut().find(|p| p.id == id)
    }

    pub fn rename_playlist(&mut self, id: &str, name: &str) {
        if let Some(p) = self.playlist_mut(id) { p.name = name.into(); }
        self.save();
    }

    pub fn set_playlist_tracks(&mut self, id: &str, ids: Vec<String>) {
        if let Some(p) = self.playlist_mut(id) { p.track_ids = ids; }
        self.save();
    }

    pub fn delete_playlist(&mut self, id: &str) {
        if let Some(c) = self.lib.playlists.iter().find(|p| p.id == id).and_then(|p| p.cover.clone()) { let _ = std::fs::remove_file(c); }
        self.lib.playlists.retain(|p| p.id != id);
        self.save();
    }

    pub fn add_to_playlist(&mut self, id: &str, ids: &[String]) -> Option<String> {
        let p = self.playlist_mut(id)?;
        for t in ids {
            if !p.track_ids.contains(t) { p.track_ids.push(t.clone()); }
        }
        let name = p.name.clone();
        self.save();
        Some(name)
    }

    pub fn remove_from_playlist(&mut self, id: &str, ids: &[String]) {
        if let Some(p) = self.playlist_mut(id) { p.track_ids.retain(|t| !ids.contains(t)); }
        self.save();
    }

    /// Library tracks (without anime episodes), newest first.
    pub fn is_liked(&self, id: &str) -> bool { self.lib.liked.iter().any(|x| x == id) }

    /// Like/unlike. New likes go to the top of "Liked".
    pub fn set_liked(&mut self, ids: &[String], on: bool) {
        self.lib.liked.retain(|x| !ids.contains(x));
        if on { for id in ids.iter().rev() { self.lib.liked.insert(0, id.clone()); } }
        self.save();
    }

    /// Count a play.
    /// Smart playlist: builds itself by a rule (see SMART). Without anime episodes.
    pub fn smart(&self, rule: &str) -> Vec<String> {
        const DAY: u64 = 86_400_000;
        let now = now_ms();
        let music = |id: &String| self.lib.tracks.get(id).is_some_and(|t| t.anime.is_none());
        let mut last: HashMap<&str, u64> = HashMap::new();
        for (id, at) in &self.lib.history { let e = last.entry(id.as_str()).or_default(); *e = (*e).max(*at); }
        let top = |since: u64| -> Vec<String> {
            let mut n: HashMap<&str, u32> = HashMap::new();
            for (id, at) in &self.lib.history { if *at >= since { *n.entry(id.as_str()).or_default() += 1; } }
            let mut v: Vec<(&str, u32)> = n.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            v.into_iter().map(|(id, _)| id.to_string()).filter(music).take(50).collect()
        };
        let unplayed = |since: u64| -> Vec<String> {
            let mut v: Vec<&Track> = self.lib.tracks.values().filter(|t| t.anime.is_none() && t.added_at >= since && self.lib.plays.get(&t.id).copied().unwrap_or(0) == 0).collect();
            v.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(a.id.cmp(&b.id)));
            v.into_iter().map(|t| t.id.clone()).collect()
        };
        match rule {
            // liked but not played for a month
            "forgotten" => self.lib.liked.iter().filter(|id| music(id) && last.get(id.as_str()).is_none_or(|at| *at < now.saturating_sub(30 * DAY))).cloned().collect(),
            "week" => top(now.saturating_sub(7 * DAY)),
            "month" => top(now.saturating_sub(30 * DAY)),
            "alltime" => {
                let mut v: Vec<(&String, &u32)> = self.lib.plays.iter().filter(|(id, n)| **n > 0 && music(id)).collect();
                v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                v.into_iter().take(50).map(|(id, _)| id.clone()).collect()
            }
            // added within two weeks and never played yet
            "fresh" => unplayed(now.saturating_sub(14 * DAY)),
            "never" => unplayed(0),
            _ => vec![],
        }
    }

    pub fn count_play(&mut self, id: &str) {
        *self.lib.plays.entry(id.to_string()).or_default() += 1;
        self.lib.history.push((id.to_string(), now_ms()));
        if self.lib.history.len() > 50_000 { self.lib.history.drain(..10_000); }
        self.save();
    }

    /// Recap for a period (since `since` ms): total plays, minutes, top tracks and artists.
    pub fn stats(&self, since: u64) -> Stats {
        let mut st = Stats::default();
        let mut tracks: HashMap<&str, u32> = HashMap::new();
        let mut artists: HashMap<String, u32> = HashMap::new();
        let music = |id: &String| self.lib.tracks.get(id).is_none_or(|t| t.anime.is_none());
        for (id, at) in self.lib.history.iter().filter(|(id, at)| *at >= since && music(id)) {
            st.plays += 1;
            *tracks.entry(id).or_default() += 1;
            if let Some(t) = self.lib.tracks.get(id) {
                st.minutes += t.duration / 60.0;
                let a = t.artist.split([',', '&']).next().unwrap_or("").trim().trim_end_matches(" - Topic").to_string();
                if !a.is_empty() { *artists.entry(a).or_default() += 1; }
            }
            let _ = at;
        }
        let mut tv: Vec<(String, u32)> = tracks.into_iter().filter(|(id, _)| self.lib.tracks.contains_key(*id)).map(|(k, v)| (k.to_string(), v)).collect();
        tv.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut av: Vec<(String, u32)> = artists.into_iter().collect();
        av.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        st.top_tracks = tv.into_iter().take(10).collect();
        st.top_artists = av.into_iter().take(10).collect();
        st
    }

    /// Edit a track's title / artist / album / cover.
    pub fn edit_track(&mut self, id: &str, title: &str, artist: &str, album: &str, cover: Option<Option<String>>) {
        if let Some(t) = self.lib.tracks.get_mut(id) {
            t.title = title.trim().to_string();
            t.artist = artist.trim().to_string();
            t.album = album.trim().to_string();
            if let Some(c) = cover { t.cover = c; }
            self.save();
        }
    }

    /// Duplicate groups: the same YouTube video or file, or matching artist+title
    /// and duration (±3 s). Within a group the oldest track comes first (that's the one kept).
    pub fn duplicates(&self) -> Vec<Vec<String>> {
        let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
        let mut groups: Vec<Vec<&Track>> = Vec::new();
        let mut all: Vec<&Track> = self.lib.tracks.values().filter(|t| t.anime.is_none()).collect();
        all.sort_by(|a, b| a.added_at.cmp(&b.added_at).then(a.id.cmp(&b.id)));
        'outer: for t in all {
            for g in groups.iter_mut() {
                let f = g[0];
                let same_src = (t.video_id.is_some() && t.video_id == f.video_id) || (t.path.is_some() && t.path == f.path);
                let same_song = !t.title.is_empty() && norm(&t.title) == norm(&f.title) && norm(&t.artist) == norm(&f.artist)
                    && (t.duration - f.duration).abs() <= 3.0;
                if same_src || same_song { g.push(t); continue 'outer; }
            }
            groups.push(vec![t]);
        }
        groups.into_iter().filter(|g| g.len() > 1).map(|g| g.into_iter().map(|t| t.id.clone()).collect()).collect()
    }

    /// Keep one from each group: in playlists and "Liked" the extras are replaced by the kept one.
    pub fn merge_duplicates(&mut self, groups: &[Vec<String>]) -> usize {
        let mut removed = 0;
        for g in groups {
            let Some(keep) = g.first() else { continue };
            for d in &g[1..] {
                for p in &mut self.lib.playlists {
                    for x in p.track_ids.iter_mut() { if x == d { *x = keep.clone(); } }
                    let mut seen = std::collections::HashSet::new();
                    p.track_ids.retain(|x| seen.insert(x.clone()));
                }
                if self.lib.liked.contains(d) && !self.lib.liked.contains(keep) { self.lib.liked.push(keep.clone()); }
                self.lib.liked.retain(|x| x != d);
                let n = self.lib.plays.remove(d).unwrap_or(0);
                *self.lib.plays.entry(keep.clone()).or_default() += n;
                for h in &mut self.lib.history { if &h.0 == d { h.0 = keep.clone(); } }
                if self.lib.tracks.remove(d).is_some() { removed += 1; }
            }
        }
        self.save();
        removed
    }

    pub fn set_folder(&mut self, pid: &str, folder: Option<String>) {
        if let Some(p) = self.lib.playlists.iter_mut().find(|p| p.id == pid) { p.folder = folder.map(|f| f.trim().to_string()).filter(|f| !f.is_empty()); }
        self.save();
    }

    /// Track order for a sort method. `ids` come in their "native" order (library — newest first,
    /// playlist — as arranged). An unknown method keeps the native order.
    pub fn sorted(&self, mut ids: Vec<String>, how: &str, playlist: bool) -> Vec<String> {
        let t = |id: &String| self.lib.tracks.get(id);
        let key = |s: &str| s.trim().to_lowercase();
        let by = |f: &dyn Fn(&Track) -> String, ids: &mut Vec<String>| {
            ids.sort_by_cached_key(|id| t(id).map(|t| (f(t).is_empty(), f(t), key(&t.title))).unwrap_or((true, String::new(), String::new())));
        };
        match how {
            "plays" => { ids.sort_by_key(|id| std::cmp::Reverse(self.lib.plays.get(id).copied().unwrap_or(0))); } // stable: on a tie — native order
            "newest" if playlist => ids.reverse(),
            "oldest" if !playlist => ids.reverse(),
            "title" => by(&|t| key(&t.title), &mut ids),
            "artist" => by(&|t| key(&t.artist), &mut ids),
            "album" => by(&|t| key(&t.album), &mut ids),
            _ => {}
        }
        ids
    }

    pub fn library_ids(&self) -> Vec<String> {
        let mut v: Vec<&Track> = self.lib.tracks.values().filter(|t| t.anime.is_none()).collect();
        v.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(a.title.cmp(&b.title)));
        v.into_iter().map(|t| t.id.clone()).collect()
    }
}

impl Store {
    pub fn anime(&self, id: &str) -> Option<&crate::anime::Anime> { self.lib.anime.iter().find(|a| a.id == id) }
    pub fn anime_mut(&mut self, id: &str) -> Option<&mut crate::anime::Anime> { self.lib.anime.iter_mut().find(|a| a.id == id) }

    /// Add episodes to a title (found by key or created). Returns the title id and whether it was new.
    pub fn add_episodes(&mut self, key: &str, parsed: &crate::anime::Parsed, tracks: Vec<Track>) -> (String, bool) {
        let idx = match self.lib.anime.iter().position(|a| a.key == key) {
            Some(i) => i,
            None => { self.lib.anime.push(crate::anime::new_anime(key, parsed)); self.lib.anime.len() - 1 }
        };
        let is_new = self.lib.anime[idx].episodes.is_empty();
        let id = self.lib.anime[idx].id.clone();
        let display = self.lib.anime[idx].display().to_string();
        for mut t in tracks {
            t.anime = Some(id.clone());
            t.artist = display.clone();
            if let Some(old) = self.lib.tracks.get(&t.id) { t.added_at = old.added_at; }
            if !self.lib.anime[idx].episodes.contains(&t.id) { self.lib.anime[idx].episodes.push(t.id.clone()); }
            self.lib.tracks.insert(t.id.clone(), t);
        }
        self.sort_anime(&id);
        self.save();
        (id, is_new)
    }

    /// Episodes by number.
    pub fn sort_anime(&mut self, id: &str) {
        let Some(a) = self.lib.anime.iter().find(|a| a.id == id) else { return };
        let mut eps: Vec<Track> = a.episodes.iter().filter_map(|e| self.lib.tracks.get(e).cloned()).collect();
        crate::anime::sort_episodes(&mut eps);
        let ids = eps.into_iter().map(|t| t.id).collect();
        if let Some(a) = self.anime_mut(id) { a.episodes = ids; }
    }

    /// Update the artist of episodes (after the Russian title arrives from Shikimori).
    pub fn retitle_episodes(&mut self, id: &str) {
        let Some(a) = self.anime(id).cloned() else { return };
        for e in &a.episodes {
            if let Some(t) = self.lib.tracks.get_mut(e) { t.artist = a.display().to_string(); t.cover = a.poster.clone(); }
        }
    }

    pub fn delete_anime(&mut self, id: &str) {
        if let Some(a) = self.anime(id).cloned() {
            for e in &a.episodes { self.lib.tracks.remove(e); }
        }
        self.lib.anime.retain(|a| a.id != id);
        self.save();
    }

    /// Episode watched / not.
    pub fn mark_episode(&mut self, track_id: &str, watched: bool) {
        let Some(aid) = self.lib.tracks.get(track_id).and_then(|t| t.anime.clone()) else { return };
        if let Some(a) = self.anime_mut(&aid) {
            if watched && !a.watched.iter().any(|w| w == track_id) { a.watched.push(track_id.to_string()); }
            if !watched { a.watched.retain(|w| w != track_id); }
        }
        self.save();
    }

    /// Remember where we stopped (episode and position).
    pub fn set_progress(&mut self, track_id: &str, pos: f64) {
        let Some(aid) = self.lib.tracks.get(track_id).and_then(|t| t.anime.clone()) else { return };
        if let Some(a) = self.anime_mut(&aid) { a.last = Some(track_id.to_string()); a.last_pos = pos; }
        self.save();
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub plays: u32,
    pub minutes: f64,
    pub top_tracks: Vec<(String, u32)>,
    pub top_artists: Vec<(String, u32)>,
}

/// List with elements moved: `moving` (in their original order) go before position `to`.
pub fn move_items(list: &[String], moving: &[String], to: usize) -> Vec<String> {
    let before = list.iter().take(to).filter(|x| moving.contains(x)).count();
    let mut rest: Vec<String> = list.iter().filter(|x| !moving.contains(x)).cloned().collect();
    let at = (to - before).min(rest.len());
    let moved: Vec<String> = list.iter().filter(|x| moving.contains(x)).cloned().collect();
    rest.splice(at..at, moved);
    rest
}

fn rand16() -> u16 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(now_ms());
    h.finish() as u16
}

fn merge(a: &mut Value, b: &Value) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (a, b) => *a = b.clone(),
    }
}

/// Smart playlists: (rule, name, description).
pub const SMART: &[(&str, &str, &str)] = &[
    ("week", "Топ недели", "Чаще всего за 7 дней"),
    ("month", "Топ месяца", "Чаще всего за 30 дней"),
    ("alltime", "На повторе", "Самое слушаемое за всё время"),
    ("forgotten", "Забытое любимое", "Лайкнутое, что не играло больше месяца"),
    ("fresh", "Свежее", "Добавлено за две недели, ещё не слушал"),
    ("never", "Ни разу не слушал", "Всё, что ещё ни разу не играло"),
];

#[cfg(test)]
mod sort_tests {
    use super::*;

    fn tr(id: &str, title: &str, artist: &str, album: &str) -> Track {
        serde_json::from_value(serde_json::json!({ "id": id, "type": "local", "title": title, "artist": artist, "album": album })).unwrap()
    }

    #[test]
    fn sorts() {
        let mut s = Store { file: std::env::temp_dir().join(format!("wavy-sort-{}.json", std::process::id())), lib: Library::default() };
        for t in [tr("a", "Бета", "Zed", ""), tr("b", "альфа", "abc", "X"), tr("c", "Gamma", "", "A")] { s.lib.tracks.insert(t.id.clone(), t); }
        let ids: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let j = |v: Vec<String>| v.join("");
        assert_eq!(j(s.sorted(ids.clone(), "manual", true)), "abc");
        assert_eq!(j(s.sorted(ids.clone(), "newest", true)), "cba");
        assert_eq!(j(s.sorted(ids.clone(), "title", true)), "cba"); // Latin before Cyrillic, case-insensitive
        assert_eq!(j(s.sorted(ids.clone(), "artist", true)), "bac"); // no artist — at the end
        assert_eq!(j(s.sorted(ids.clone(), "album", true)), "cba");
        s.lib.plays.insert("c".into(), 5);
        s.lib.plays.insert("a".into(), 2);
        assert_eq!(j(s.sorted(ids, "plays", true)), "cab");
    }

    #[test]
    fn backup_restore() {
        let dir = std::env::temp_dir().join(format!("wavy-bk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store { file: dir.join("library.json"), lib: Library::default() };
        s.lib.tracks.insert("a".into(), tr("a", "Gurenge", "LiSA", ""));
        s.save();
        for d in 1..=5 { s.backup(&dir.join("bk"), &format!("wavy-backup-2026-11-0{d}.json"), 3).unwrap(); }
        let mut left: Vec<String> = std::fs::read_dir(dir.join("bk")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["wavy-backup-2026-11-03.json", "wavy-backup-2026-11-04.json", "wavy-backup-2026-11-05.json"]);
        s.lib.tracks.clear();
        assert_eq!(s.restore(&dir.join("bk/wavy-backup-2026-11-05.json")), Ok(1));
        assert!(s.lib.tracks.contains_key("a"));
        assert!(dir.join("library.before-restore.json").exists());
        assert!(s.restore(&dir.join("library.json.nope")).is_err());
    }

    #[test]
    fn smart_lists() {
        let mut s = Store { file: std::env::temp_dir().join(format!("wavy-smart-{}.json", std::process::id())), lib: Library::default() };
        let now = now_ms();
        let day = 86_400_000;
        for (id, added) in [("a", now - 40 * day), ("b", now - 3 * day), ("c", now - day), ("d", now - 50 * day)] {
            let mut t = tr(id, id, "", ""); t.added_at = added; s.lib.tracks.insert(id.into(), t);
        }
        let mut ep = tr("e", "Серия 1", "", ""); ep.anime = Some("an".into()); s.lib.tracks.insert("e".into(), ep);
        s.lib.liked = vec!["a".into(), "d".into()];
        // a played long ago, d yesterday, c twice this week, the anime episode doesn't count
        s.lib.history = vec![("a".into(), now - 45 * day), ("d".into(), now - day), ("c".into(), now - 2 * day), ("c".into(), now - day), ("e".into(), now)];
        s.lib.plays = [("a", 1), ("d", 1), ("c", 2), ("e", 9)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        assert_eq!(s.smart("forgotten"), vec!["a"]);
        assert_eq!(s.smart("week"), vec!["c", "d"]);
        assert_eq!(s.smart("month"), vec!["c", "d"]);
        assert_eq!(s.smart("alltime"), vec!["c", "a", "d"]);
        assert_eq!(s.smart("fresh"), vec!["b"]);
        assert_eq!(s.smart("never"), vec!["b"]);
    }

    #[test]
    fn dupes_and_stats() {
        let mut s = Store { file: std::env::temp_dir().join(format!("wavy-dup-{}.json", std::process::id())), lib: Library::default() };
        let mut a = tr("a", "Gurenge", "LiSA", ""); a.duration = 237.0; a.added_at = 1;
        let mut b = tr("b", "gurenge", "lisa", "X"); b.duration = 236.0; b.added_at = 2;
        let mut c = tr("c", "Other", "LiSA", ""); c.duration = 200.0; c.added_at = 3;
        for t in [a, b, c] { s.lib.tracks.insert(t.id.clone(), t); }
        s.lib.playlists.push(Playlist { id: "p".into(), name: "P".into(), track_ids: vec!["b".into(), "c".into(), "a".into()], source: None, created_at: 0, cover: None, folder: None });
        s.lib.liked = vec!["b".into()];
        let g = s.duplicates();
        assert_eq!(g, vec![vec!["a".to_string(), "b".to_string()]]);
        s.count_play("b"); s.count_play("b"); s.count_play("c");
        assert_eq!(s.merge_duplicates(&g), 1);
        assert_eq!(s.lib.playlists[0].track_ids, vec!["a".to_string(), "c".to_string()]);
        assert_eq!(s.lib.liked, vec!["a".to_string()]);
        let st = s.stats(0);
        assert_eq!(st.plays, 3);
        assert_eq!(st.top_tracks[0], ("a".to_string(), 2));
        assert_eq!(st.top_artists[0], ("LiSA".to_string(), 3));
        let _ = std::fs::remove_file(&s.file);
    }

    #[test]
    fn moves() {
        let l: Vec<String> = "abcde".chars().map(String::from).collect();
        let j = |v: Vec<String>| v.join("");
        let m = |x: &str| x.chars().map(String::from).collect::<Vec<_>>();
        assert_eq!(j(move_items(&l, &m("a"), 3)), "bcade");
        assert_eq!(j(move_items(&l, &m("e"), 0)), "eabcd");
        assert_eq!(j(move_items(&l, &m("bd"), 5)), "acebd");
        assert_eq!(j(move_items(&l, &m("bd"), 0)), "bdace");
        assert_eq!(j(move_items(&l, &m("c"), 2)), "abcde");
        assert_eq!(j(move_items(&l, &m("c"), 3)), "abcde");
    }
}
