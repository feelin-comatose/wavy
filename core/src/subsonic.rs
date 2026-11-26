//! Your own music server through the Subsonic API (Navidrome, Jellyfin with its Subsonic plugin, Airsonic,
//! Gonic, …): import its playlists and albums, stream the songs (cached like YouTube tracks).
//! The password is never stored: only a salted token (md5(password + salt)), which is how the API signs requests.
//! Tracks: kind "subsonic", url "subsonic:<song id>.<file suffix>"; the server address comes from settings,
//! so the library doesn't break when the server moves.
use crate::{now_ms, sha16, Track};
use md5::{Digest, Md5};
use serde_json::Value;
use std::sync::RwLock;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Server {
    /// base address, e.g. https://music.example.com
    pub url: String,
    pub user: String,
    pub salt: String,
    pub token: String,
}

impl Server {
    pub fn connected(&self) -> bool { !self.url.is_empty() && !self.token.is_empty() }
}

/// The connected server, for playback threads (set by the app from settings).
static CURRENT: RwLock<Option<Server>> = RwLock::new(None);
pub fn set_current(s: Option<Server>) { *CURRENT.write().unwrap() = s.filter(Server::connected); }
pub fn current() -> Option<Server> { CURRENT.read().unwrap().clone() }

fn md5hex(s: &str) -> String { format!("{:x}", Md5::digest(s.as_bytes())) }

/// Query string with the auth parameters (the token, never the password).
fn auth_query(s: &Server) -> String {
    let enc = |v: &str| url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>();
    format!("u={}&t={}&s={}&v=1.16.1&c=Wavy&f=json", enc(&s.user), s.token, s.salt)
}

pub fn method_url(s: &Server, method: &str, params: &str) -> String {
    let base = s.url.trim_end_matches('/');
    let sep = if params.is_empty() { "" } else { "&" };
    format!("{base}/rest/{method}.view?{}{sep}{params}", auth_query(s))
}

fn api(s: &Server, method: &str, params: &str) -> Result<Value, String> {
    let mut r = crate::http().get(&method_url(s, method, params)).call().map_err(|e| lf!("сервер не ответил: {e}", "the server didn't respond: {e}"))?;
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?)
        .map_err(|_| l!("это не Subsonic-сервер (проверь адрес)", "this isn't a Subsonic server (check the address)").to_string())?;
    let resp = &v["subsonic-response"];
    if resp["status"] == "ok" { return Ok(resp.clone()); }
    Err(resp["error"]["message"].as_str().map(str::to_string).unwrap_or_else(|| l!("сервер вернул ошибку", "the server returned an error").into()))
}

/// Sign in: check the address and password; returns the server with a fresh salt + token.
pub fn connect(url: &str, user: &str, password: &str) -> Result<Server, String> {
    let mut url = url.trim().trim_end_matches('/').to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") { url = format!("https://{url}"); }
    let salt = sha16(format!("{}{}{url}", now_ms(), std::process::id()).as_bytes());
    let s = Server { token: md5hex(&format!("{password}{salt}")), url, user: user.trim().to_string(), salt };
    api(&s, "ping", "")?;
    Ok(s)
}

/// A playlist or album on the server: (id, name, subtitle, number of songs).
#[derive(Clone, Debug, PartialEq)]
pub struct Item { pub id: String, pub name: String, pub sub: String, pub count: u32 }

pub fn playlists(s: &Server) -> Result<Vec<Item>, String> {
    let r = api(s, "getPlaylists", "")?;
    Ok(r["playlists"]["playlist"].as_array().cloned().unwrap_or_default().iter().map(|p| Item {
        id: p["id"].as_str().unwrap_or("").into(), name: p["name"].as_str().unwrap_or("").into(),
        sub: p["owner"].as_str().unwrap_or("").into(), count: p["songCount"].as_u64().unwrap_or(0) as u32,
    }).collect())
}

/// Newest albums (up to 50).
pub fn albums(s: &Server) -> Result<Vec<Item>, String> {
    let r = api(s, "getAlbumList2", "type=newest&size=50")?;
    Ok(r["albumList2"]["album"].as_array().cloned().unwrap_or_default().iter().map(|a| Item {
        id: a["id"].as_str().unwrap_or("").into(), name: a["name"].as_str().unwrap_or("").into(),
        sub: a["artist"].as_str().unwrap_or("").into(), count: a["songCount"].as_u64().unwrap_or(0) as u32,
    }).collect())
}

/// Library track for a server song.
pub fn song_track(s: &Server, v: &Value) -> Track {
    let id = v["id"].as_str().unwrap_or("");
    let suffix = v["suffix"].as_str().unwrap_or("mp3");
    let cover = v["coverArt"].as_str().map(|c| method_url(s, "getCoverArt", &format!("id={c}&size=300")));
    Track {
        id: format!("sub_{}", sha16(format!("{}|{id}", s.url).as_bytes())), kind: "subsonic".into(), path: None, video_id: None,
        url: Some(format!("subsonic:{id}.{suffix}")),
        title: v["title"].as_str().unwrap_or("").into(), artist: v["artist"].as_str().unwrap_or("").into(),
        album: v["album"].as_str().unwrap_or("").into(), duration: v["duration"].as_f64().unwrap_or(0.0),
        cover, added_at: now_ms(), video: false, anime: None, episode: None,
    }
}

/// Songs of a playlist (`album` false) or an album, with its name.
pub fn songs(s: &Server, id: &str, album: bool) -> Result<(String, Vec<Track>), String> {
    let enc: String = url::form_urlencoded::byte_serialize(id.as_bytes()).collect();
    let (method, key) = if album { ("getAlbum", "album") } else { ("getPlaylist", "playlist") };
    let r = api(s, method, &format!("id={enc}"))?;
    let o = &r[key];
    let list = o[if album { "song" } else { "entry" }].as_array().cloned().unwrap_or_default();
    let base = now_ms();
    let n = list.len() as u64;
    // keep the server's order under "newest first"
    let tracks = list.iter().enumerate().map(|(i, v)| { let mut t = song_track(s, v); t.added_at = base + n - i as u64; t }).collect();
    Ok((o["name"].as_str().unwrap_or("").to_string(), tracks))
}

/// "subsonic:<id>.<suffix>" → (song id, file suffix).
pub fn parse_ref(url: &str) -> Option<(&str, &str)> {
    let r = url.strip_prefix("subsonic:")?;
    Some(r.rsplit_once('.').unwrap_or((r, "mp3")))
}

/// Direct stream address of a song (the original file, no transcoding).
pub fn stream_url(s: &Server, song_id: &str) -> String {
    let enc: String = url::form_urlencoded::byte_serialize(song_id.as_bytes()).collect();
    method_url(s, "stream", &format!("id={enc}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_tracks_and_refs() {
        let s = Server { url: "https://music.example.com".into(), user: "me & you".into(), salt: "abc".into(), token: md5hex("secretabc") };
        assert_eq!(s.token, "c42b93a1ac3b382e88071510296a96ce"); // md5("secret" + salt), as the Subsonic API defines it
        let u = method_url(&s, "getAlbum", "id=7");
        assert!(u.starts_with("https://music.example.com/rest/getAlbum.view?u=me+%26+you&t="));
        assert!(u.ends_with("&s=abc&v=1.16.1&c=Wavy&f=json&id=7"));
        assert!(!u.contains("secret"));
        let song = serde_json::json!({"id": "tr-1", "title": "Undertow", "artist": "Halcyon Drift", "album": "Low Tide", "duration": 199, "suffix": "flac", "coverArt": "al-3"});
        let t = song_track(&s, &song);
        assert_eq!((t.kind.as_str(), t.url.as_deref(), t.duration), ("subsonic", Some("subsonic:tr-1.flac"), 199.0));
        assert!(t.cover.unwrap().contains("getCoverArt.view") );
        assert_eq!(parse_ref("subsonic:tr-1.flac"), Some(("tr-1", "flac")));
        assert_eq!(parse_ref("subsonic:noext"), Some(("noext", "mp3")));
        assert_eq!(parse_ref("https://x"), None);
        // the same song on another server is another track
        let other = Server { url: "https://other".into(), ..s.clone() };
        assert_ne!(song_track(&other, &song).id, t.id);
    }
}
