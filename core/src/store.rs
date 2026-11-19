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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlaySettings { pub port: u16, pub theme: String, pub position: String, pub auto_hide_sec: u32 }
impl Default for OverlaySettings { fn default() -> Self { Self { port: 3737, theme: "card".into(), position: "bottom-left".into(), auto_hide_sec: 0 } } }

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
}
impl Default for Settings {
    fn default() -> Self {
        Self { volume: 0.8, shuffle: false, repeat: "off".into(), discord: Default::default(), overlay: Default::default(), overlay_window: Default::default() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub tracks: HashMap<String, Track>,
    pub playlists: Vec<Playlist>,
    pub settings: Settings,
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
        self.save();
    }

    pub fn create_playlist(&mut self, name: &str, track_ids: Vec<String>, source: Option<String>) -> Playlist {
        let p = Playlist { id: format!("pl_{:x}{:04x}", now_ms(), rand16()), name: name.into(), track_ids, source, created_at: now_ms() };
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

    /// Треки библиотеки, новые сверху.
    pub fn library_ids(&self) -> Vec<String> {
        let mut v: Vec<&Track> = self.lib.tracks.values().collect();
        v.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(a.title.cmp(&b.title)));
        v.into_iter().map(|t| t.id.clone()).collect()
    }
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
