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
    /// своя картинка обложки (файл в data_dir/covers)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// папка в боковой панели
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

/// Необязательный ключ Spotify Web API — нужен только для плейлистов больше 100 треков.
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
    /// Сам скачивать и ставить новые версии с GitHub Releases.
    pub auto_update: bool,
    pub spotify: SpotifySettings,
    /// Сортировка: "library" или id плейлиста → способ (см. Sort).
    pub sort: HashMap<String, String>,
    /// Сборка, которую человек уже видел (для окна «Что нового» после обновления).
    pub last_build: Option<u32>,
    /// Мини-плеер: открыт ли и где.
    pub mini: bool,
    pub mini_pos: Option<[f32; 2]>,
    /// Крестик сворачивает в трей, а не закрывает (Windows).
    pub close_to_tray: bool,
    /// Звук: скорость (1.0 — как есть), кроссфейд в секундах (0 — выкл), выравнивание громкости, эквалайзер.
    pub speed: f32,
    pub crossfade: f32,
    pub normalize: bool,
    pub eq_on: bool,
    pub eq: [f32; 10],
    /// Аниме: сам пропускать опенинги (по AniSkip), предлагать следующую серию на эндинге, аккаунт Шикимори.
    pub anime_autoskip: bool,
    pub anime_next: bool,
    pub shiki: crate::shikimori::Account,
    /// Заказы из чата Twitch: канал, включено ли, макс. длина трека (мин).
    pub twitch_channel: String,
    pub twitch_sr: bool,
    pub twitch_max_min: u32,
    /// Вход в Twitch (чтобы отвечать в чат), сколько голосов «!skip» пропускает трек (0 — выкл),
    /// награда за баллы канала, которая заказывает трек (id; пусто — нет).
    pub twitch_account: crate::twitch::Account,
    pub twitch_skip_votes: u32,
    pub twitch_reward: String,
    /// Писать «Исполнитель — Название» в файл now_playing.txt (для OBS «Текст из файла»).
    pub now_playing_file: bool,
    /// Лимит кэша (ГБ, 0 — без лимита): старое удаляется само.
    pub cache_limit_gb: u32,
    /// Синхронизация с телефоном по Wi-Fi: включена ли и PIN.
    pub wifi_sync: bool,
    pub wifi_pin: String,
    /// Куки YouTube для видео 18+: имя браузера (firefox, chrome, edge…) или путь к cookies.txt. Пусто — выкл.
    pub yt_cookies: String,
    /// Глобальные горячие клавиши: действие → «Ctrl+Alt+P» (пусто — выключено; нет ключа — по умолчанию).
    pub hotkeys: HashMap<String, String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self { volume: 0.8, shuffle: false, repeat: "off".into(), discord: Default::default(), overlay: Default::default(), overlay_window: Default::default(), auto_update: true, spotify: Default::default(), sort: HashMap::new(), last_build: None, mini: false, mini_pos: None, close_to_tray: false, speed: 1.0, crossfade: 0.0, normalize: false, eq_on: false, eq: [0.0; 10], anime_autoskip: false, anime_next: true, shiki: Default::default(), twitch_channel: String::new(), twitch_sr: false, twitch_max_min: 10, twitch_account: Default::default(), twitch_skip_votes: 3, twitch_reward: String::new(), now_playing_file: false, cache_limit_gb: 0, wifi_sync: false, wifi_pin: String::new(), yt_cookies: String::new(), hotkeys: HashMap::new() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub tracks: HashMap<String, Track>,
    pub playlists: Vec<Playlist>,
    pub settings: Settings,
    /// Раздел «Аниме»: тайтлы, их серии лежат в tracks (с полем anime).
    pub anime: Vec<crate::anime::Anime>,
    /// Сколько раз трек слушали (больше 30 секунд) — для сортировки «Часто слушаю».
    pub plays: HashMap<String, u32>,
    /// Лайкнутые треки, свежие первыми (раздел «Любимое»).
    pub liked: Vec<String>,
    /// История прослушиваний: (трек, когда, мс) — для «Итогов». Хранится последние 50 000.
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

    /// Применить JSON-патч к настройкам (глубокое слияние).
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
                t.added_at = old.added_at; // сохраняем порядок добавления
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

    /// Переставить треки внутри плейлиста: `moving` встают перед позицией `to` (индекс в исходном списке).
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

    /// Треки библиотеки (без серий аниме), новые сверху.
    pub fn is_liked(&self, id: &str) -> bool { self.lib.liked.iter().any(|x| x == id) }

    /// Лайкнуть/снять лайк. Новые лайки — наверх «Любимого».
    pub fn set_liked(&mut self, ids: &[String], on: bool) {
        self.lib.liked.retain(|x| !ids.contains(x));
        if on { for id in ids.iter().rev() { self.lib.liked.insert(0, id.clone()); } }
        self.save();
    }

    /// Засчитать прослушивание.
    pub fn count_play(&mut self, id: &str) {
        *self.lib.plays.entry(id.to_string()).or_default() += 1;
        self.lib.history.push((id.to_string(), now_ms()));
        if self.lib.history.len() > 50_000 { self.lib.history.drain(..10_000); }
        self.save();
    }

    /// Итоги за период (с `since` мс): всего прослушиваний, минут, топ треков и артистов.
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

    /// Поправить название / исполнителя / альбом / обложку трека.
    pub fn edit_track(&mut self, id: &str, title: &str, artist: &str, album: &str, cover: Option<Option<String>>) {
        if let Some(t) = self.lib.tracks.get_mut(id) {
            t.title = title.trim().to_string();
            t.artist = artist.trim().to_string();
            t.album = album.trim().to_string();
            if let Some(c) = cover { t.cover = c; }
            self.save();
        }
    }

    /// Группы дублей: одно и то же видео YouTube или тот же файл, либо совпадают исполнитель+название
    /// и длительность (±3 с). Внутри группы первым — самый старый трек (его и оставляем).
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

    /// Оставить по одному из каждой группы: в плейлистах и «Любимом» лишние заменяются оставленным.
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

    /// Порядок треков по способу сортировки. `ids` приходят в «родном» порядке (библиотека — новые сверху,
    /// плейлист — как составлен). Неизвестный способ — родной порядок.
    pub fn sorted(&self, mut ids: Vec<String>, how: &str, playlist: bool) -> Vec<String> {
        let t = |id: &String| self.lib.tracks.get(id);
        let key = |s: &str| s.trim().to_lowercase();
        let by = |f: &dyn Fn(&Track) -> String, ids: &mut Vec<String>| {
            ids.sort_by_cached_key(|id| t(id).map(|t| (f(t).is_empty(), f(t), key(&t.title))).unwrap_or((true, String::new(), String::new())));
        };
        match how {
            "plays" => { ids.sort_by_key(|id| std::cmp::Reverse(self.lib.plays.get(id).copied().unwrap_or(0))); } // стабильно: при равенстве — родной порядок
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

    /// Добавить серии в тайтл (найти по ключу или создать). Возвращает id тайтла и был ли он новым.
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

    /// Серии по номерам.
    pub fn sort_anime(&mut self, id: &str) {
        let Some(a) = self.lib.anime.iter().find(|a| a.id == id) else { return };
        let mut eps: Vec<Track> = a.episodes.iter().filter_map(|e| self.lib.tracks.get(e).cloned()).collect();
        crate::anime::sort_episodes(&mut eps);
        let ids = eps.into_iter().map(|t| t.id).collect();
        if let Some(a) = self.anime_mut(id) { a.episodes = ids; }
    }

    /// Обновить исполнителя у серий (после того как с Шикимори пришло русское название).
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

    /// Серия просмотрена / нет.
    pub fn mark_episode(&mut self, track_id: &str, watched: bool) {
        let Some(aid) = self.lib.tracks.get(track_id).and_then(|t| t.anime.clone()) else { return };
        if let Some(a) = self.anime_mut(&aid) {
            if watched && !a.watched.iter().any(|w| w == track_id) { a.watched.push(track_id.to_string()); }
            if !watched { a.watched.retain(|w| w != track_id); }
        }
        self.save();
    }

    /// Запомнить, где остановились (серия и позиция).
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

/// Список с перенесёнными элементами: `moving` (в их исходном порядке) встают перед позицией `to`.
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
        assert_eq!(j(s.sorted(ids.clone(), "title", true)), "cba"); // латиница раньше кириллицы, без учёта регистра
        assert_eq!(j(s.sorted(ids.clone(), "artist", true)), "bac"); // без исполнителя — в конце
        assert_eq!(j(s.sorted(ids.clone(), "album", true)), "cba");
        s.lib.plays.insert("c".into(), 5);
        s.lib.plays.insert("a".into(), 2);
        assert_eq!(j(s.sorted(ids, "plays", true)), "cab");
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
