//! ListenBrainz scrobbling (an open alternative to Last.fm): "playing now" and listens.
//! Sign-in is just the user token from listenbrainz.org/settings — no app registration.
//! The same rules as Last.fm decide what counts as a listen (`lastfm::due`) and clean YouTube titles.
use crate::lastfm::{clean, Play};
use serde_json::{json, Value};

const API: &str = "https://api.listenbrainz.org/1";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub token: String,
    pub user: String,
    pub enabled: bool,
}

impl Account {
    pub fn ready(&self) -> bool { self.enabled && !self.token.is_empty() }
}

/// Check a token; Ok(user name).
pub fn validate(token: &str) -> Result<String, String> {
    let mut r = crate::http().get(&format!("{API}/validate-token")).header("Authorization", &format!("Token {}", token.trim()))
        .call().map_err(|e| lf!("ListenBrainz не ответил: {e}", "ListenBrainz didn't respond: {e}"))?;
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if v["valid"].as_bool() == Some(true) { Ok(v["user_name"].as_str().unwrap_or("").to_string()) }
    else { Err(l!("неверный токен — скопируй его со страницы listenbrainz.org/settings", "invalid token — copy it from listenbrainz.org/settings").into()) }
}

/// Request body for a play: `listened_at` (Unix seconds) makes it a listen, None — "playing now".
pub fn body(p: &Play, listened_at: Option<u64>) -> Option<Value> {
    let (artist, title) = clean(p.artist, p.title);
    if artist.is_empty() || title.is_empty() { return None; }
    let mut meta = json!({ "artist_name": artist, "track_name": title,
        "additional_info": { "media_player": "Wavy", "submission_client": "Wavy" } });
    if !p.album.is_empty() && p.album != "YouTube" { meta["release_name"] = json!(p.album); }
    if p.duration > 0.0 { meta["additional_info"]["duration_ms"] = json!((p.duration * 1000.0) as u64); }
    let mut item = json!({ "track_metadata": meta });
    if let Some(ts) = listened_at { item["listened_at"] = json!(ts); }
    Some(json!({ "listen_type": if listened_at.is_some() { "single" } else { "playing_now" }, "payload": [item] }))
}

fn submit(acc: &Account, body: &Value) -> Result<(), String> {
    crate::http().post(&format!("{API}/submit-listens")).header("Authorization", &format!("Token {}", acc.token))
        .header("Content-Type", "application/json").send(body.to_string()).map(|_| ()).map_err(|e| e.to_string())
}

pub fn now_playing(acc: &Account, p: &Play) -> Result<(), String> {
    match body(p, None) { Some(b) => submit(acc, &b), None => Ok(()) }
}

/// A listen; `started` is when the track began playing (Unix, seconds).
pub fn listen(acc: &Account, p: &Play, started: u64) -> Result<(), String> {
    match body(p, Some(started)) { Some(b) => submit(acc, &b), None => Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies() {
        let p = Play { artist: "LiSA Official", title: "LiSA - Gurenge (Official Video)", album: "YouTube", duration: 238.4 };
        let b = body(&p, Some(1_700_000_000)).unwrap();
        assert_eq!(b["listen_type"], "single");
        let item = &b["payload"][0];
        assert_eq!(item["listened_at"], 1_700_000_000);
        assert_eq!(item["track_metadata"]["artist_name"], "LiSA");
        assert_eq!(item["track_metadata"]["track_name"], "Gurenge");
        assert!(item["track_metadata"].get("release_name").is_none()); // "YouTube" isn't an album
        assert_eq!(item["track_metadata"]["additional_info"]["duration_ms"], 238_400);
        let np = body(&Play { artist: "Mira Okada", title: "Kyoto at 3 AM", album: "Paper Moons", duration: 0.0 }, None).unwrap();
        assert_eq!(np["listen_type"], "playing_now");
        assert!(np["payload"][0].get("listened_at").is_none());
        assert_eq!(np["payload"][0]["track_metadata"]["release_name"], "Paper Moons");
        assert!(body(&Play { artist: "", title: "Untitled", album: "", duration: 0.0 }, None).is_none());
    }
}
