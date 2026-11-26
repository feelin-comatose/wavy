//! Playlist exchange PC ↔ phone: the wavy-export.json file (same format as the Android version).
//! YouTube/Spotify/stream tracks and playlists are transferred; local files are not.
use crate::{now_ms, store::Store, Track};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlaylist {
    pub name: String,
    #[serde(default)]
    pub source: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Export {
    pub wavy_export: u32,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub exported_at: u64,
    #[serde(default)]
    pub playlists: Vec<ExportPlaylist>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// likes (ids of transferable tracks), newest first; merged on import
    #[serde(default)]
    pub liked: Vec<String>,
}

pub struct Summary {
    pub tracks: usize,
    pub new_playlists: usize,
    pub updated_playlists: usize,
    pub skipped_local: usize,
}

pub fn export(store: &Store) -> String {
    let lib = &store.lib;
    let portable: Vec<Track> = lib.tracks.values().filter(|t| !t.is_local()).cloned().collect();
    let playlists = lib.playlists.iter().map(|p| ExportPlaylist {
        name: p.name.clone(),
        source: p.source.clone().filter(|s| s.starts_with("http")),
        tracks: p.track_ids.iter().filter_map(|id| lib.tracks.get(id)).filter(|t| !t.is_local()).cloned().collect(),
    }).filter(|p| !p.tracks.is_empty()).collect();
    let liked = lib.liked.iter().filter(|id| lib.tracks.get(*id).is_some_and(|t| !t.is_local())).cloned().collect();
    serde_json::to_string_pretty(&Export { wavy_export: 1, app: "desktop".into(), exported_at: now_ms(), playlists, tracks: portable, liked }).unwrap_or_default()
}

pub fn is_export(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains("\"wavyExport\"")
}

pub fn import(store: &mut Store, text: &str) -> Result<Summary, String> {
    let e: Export = serde_json::from_str(text).map_err(|err| lf!("это не файл экспорта Wavy: {err}", "this isn't a Wavy export file: {err}"))?;
    let all: Vec<Track> = e.tracks.iter().chain(e.playlists.iter().flat_map(|p| p.tracks.iter())).cloned().collect();
    let skipped_local = all.iter().filter(|t| t.is_local()).count();
    let mut seen = std::collections::HashSet::new();
    let portable: Vec<Track> = all.into_iter().filter(|t| !t.is_local() && seen.insert(t.id.clone())).collect();
    let n = portable.len();
    store.add_tracks(portable);
    let (mut created, mut updated) = (0, 0);
    for p in &e.playlists {
        let ids: Vec<String> = p.tracks.iter().filter(|t| !t.is_local()).map(|t| t.id.clone()).collect();
        let existing = store.lib.playlists.iter()
            .find(|x| (p.source.is_some() && x.source == p.source) || x.name == p.name)
            .map(|x| x.id.clone());
        match existing {
            Some(id) => { store.add_to_playlist(&id, &ids); updated += 1; }
            None => { store.create_playlist(&p.name, ids, p.source.clone()); created += 1; }
        }
    }
    let new_likes: Vec<String> = e.liked.iter().filter(|id| store.lib.tracks.contains_key(*id) && !store.is_liked(id)).cloned().collect();
    if !new_likes.is_empty() { store.set_liked(&new_likes, true); }
    Ok(Summary { tracks: n, new_playlists: created, updated_playlists: updated, skipped_local })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_android_format() {
        let dir = std::env::temp_dir().join(format!("wavy-sync-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(&dir);
        let yt = |id: &str| Track { id: format!("yt_{id}"), kind: "youtube".into(), path: None, video_id: Some(id.into()), url: None, title: id.into(), artist: "A".into(), album: String::new(), duration: 1.0, cover: None, added_at: 1, video: false, anime: None, episode: None };
        let mut local = yt("x"); local.id = "loc_1".into(); local.kind = "local".into(); local.path = Some("C:/m.mp3".into());
        s.add_tracks(vec![yt("aaaaaaaaaaa"), local]);
        s.create_playlist("Mix", vec!["yt_aaaaaaaaaaa".into(), "loc_1".into()], None);
        let text = export(&s);
        assert!(is_export(&text) && !text.contains("C:/m.mp3"));
        // file from the phone (Android version fields, extras are ignored)
        let android = r#"{"wavyExport":1,"app":"android","exportedAt":5,"liked":["yt_bbbbbbbbbbb","yt_gone"],"playlists":[{"name":"Phone","source":"https://open.spotify.com/album/x","tracks":[{"id":"yt_bbbbbbbbbbb","type":"youtube","videoId":"bbbbbbbbbbb","title":"B","artist":"Y","album":"","duration":5.0,"addedAt":1,"video":false}]}],"tracks":[{"id":"and_a_7","type":"local","path":"content://media/7","title":"L"}]}"#;
        let r = import(&mut s, android).unwrap();
        assert_eq!((r.tracks, r.new_playlists, r.skipped_local), (1, 1, 1));
        let r2 = import(&mut s, android).unwrap();
        assert_eq!(r2.new_playlists, 0);
        assert!(s.lib.tracks.contains_key("yt_bbbbbbbbbbb") && !s.lib.tracks.contains_key("and_a_7"));
        // likes carry over, unknown tracks are skipped
        assert!(s.is_liked("yt_bbbbbbbbbbb") && !s.is_liked("yt_gone"));
        s.set_liked(&["yt_aaaaaaaaaaa".into()], true);
        assert!(export(&s).contains("\"liked\""));
    }
}
