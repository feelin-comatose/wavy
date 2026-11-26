//! Last.fm scrobbling: "now playing" and listened tracks (half the track or 4 minutes).
//! Needs a Last.fm API key (last.fm/api/account/create — free): API key and Shared secret.
//! Sign-in: Wavy opens the Last.fm page, you press "Allow" there, then "Done" in Wavy.
use md5::{Digest, Md5};
use serde_json::Value;

const API: &str = "https://ws.audioscrobbler.com/2.0/";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub api_key: String,
    pub secret: String,
    pub session: String,
    pub user: String,
    pub enabled: bool,
}

impl Account {
    pub fn ready(&self) -> bool { self.enabled && !self.session.is_empty() && !self.api_key.is_empty() && !self.secret.is_empty() }
}

/// Request signature: md5 of "namevalue" pairs sorted alphabetically + the secret.
pub fn sign(params: &[(&str, &str)], secret: &str) -> String {
    let mut p: Vec<&(&str, &str)> = params.iter().filter(|(k, _)| *k != "format" && *k != "callback").collect();
    p.sort_by(|a, b| a.0.cmp(b.0));
    let mut s = String::new();
    for (k, v) in p { s.push_str(k); s.push_str(v); }
    s.push_str(secret);
    format!("{:x}", Md5::digest(s.as_bytes()))
}

fn call(params: &[(&str, &str)], secret: &str, post: bool) -> Result<Value, String> {
    let sig = sign(params, secret);
    let mut all: Vec<(&str, &str)> = params.to_vec();
    all.push(("api_sig", &sig));
    all.push(("format", "json"));
    let body: String = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(&all).finish();
    let a = crate::http();
    let r = if post {
        a.post(API).config().http_status_as_error(false).build().header("Content-Type", "application/x-www-form-urlencoded").header("User-Agent", "Wavy").send(body)
    } else {
        a.get(&format!("{API}?{body}")).config().http_status_as_error(false).build().header("User-Agent", "Wavy").call()
    };
    let mut r = r.map_err(|e| lf!("Last.fm недоступен: {e}", "Last.fm is unavailable: {e}"))?;
    let v: Value = r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if let Some(msg) = v["message"].as_str().filter(|_| v.get("error").is_some()) { return Err(format!("Last.fm: {msg}")); }
    Ok(v)
}

/// Step 1: a token and the page where the user grants access.
pub fn begin(acc: &Account) -> Result<(String, String), String> {
    let v = call(&[("method", "auth.getToken"), ("api_key", acc.api_key.trim())], acc.secret.trim(), false)?;
    let token = v["token"].as_str().ok_or(l!("Last.fm не выдал токен — проверь API key", "Last.fm didn't give a token — check the API key"))?.to_string();
    let url = format!("https://www.last.fm/api/auth/?api_key={}&token={token}", acc.api_key.trim());
    Ok((token, url))
}

/// Step 2: after "Allow" on the site — a permanent session.
pub fn finish(acc: &mut Account, token: &str) -> Result<(), String> {
    let v = call(&[("method", "auth.getSession"), ("api_key", acc.api_key.trim()), ("token", token)], acc.secret.trim(), false)
        .map_err(|e| if e.contains("not authorized") || e.contains("Unauthorized") { l!("сначала нажми «Разрешить» на странице Last.fm", "first click “Allow access” on the Last.fm page").to_string() } else { e })?;
    acc.session = v["session"]["key"].as_str().ok_or(l!("Last.fm не выдал сессию", "Last.fm didn't give a session"))?.to_string();
    acc.user = v["session"]["name"].as_str().unwrap_or("").to_string();
    acc.enabled = true;
    Ok(())
}

pub struct Play<'a> { pub artist: &'a str, pub title: &'a str, pub album: &'a str, pub duration: f64 }

/// Artist and title for Last.fm: YouTube videos often have "Artist - Title" in the title,
/// while the "artist" is the channel ("LiSA Official", "… - Topic").
pub fn clean<'a>(artist: &'a str, title: &'a str) -> (String, String) {
    let artist = artist.trim().trim_end_matches(" - Topic").trim_end_matches("VEVO").trim();
    if let Some((a, t)) = title.split_once(" - ").or_else(|| title.split_once(" – ")) {
        if artist.is_empty() || artist.contains("Official") || artist.contains("Channel") || a.trim().eq_ignore_ascii_case(artist) {
            return (a.trim().to_string(), strip_junk(t));
        }
    }
    (artist.to_string(), strip_junk(title))
}

fn strip_junk(t: &str) -> String {
    let mut t = t.trim().to_string();
    for j in ["(Official Video)", "(Official Music Video)", "(Official Audio)", "[Official Video]", "(Lyrics)", "(MV)", "[MV]", "(Official MV)", "【MV】"] {
        if let Some(i) = t.to_lowercase().find(&j.to_lowercase()) { t.replace_range(i..i + j.len(), ""); }
    }
    t.trim().to_string()
}

pub fn now_playing(acc: &Account, p: &Play) -> Result<(), String> {
    let (artist, title) = clean(p.artist, p.title);
    if artist.is_empty() { return Ok(()); }
    let dur = (p.duration as u64).to_string();
    let mut params = vec![("method", "track.updateNowPlaying"), ("api_key", acc.api_key.as_str()), ("sk", acc.session.as_str()), ("artist", artist.as_str()), ("track", title.as_str())];
    if !p.album.is_empty() && p.album != "YouTube" { params.push(("album", p.album)); }
    if p.duration > 0.0 { params.push(("duration", &dur)); }
    call(&params, &acc.secret, true).map(|_| ())
}

/// Scrobble a play; `started` is when the track began playing (Unix, seconds).
pub fn scrobble(acc: &Account, p: &Play, started: u64) -> Result<(), String> {
    let (artist, title) = clean(p.artist, p.title);
    if artist.is_empty() { return Ok(()); }
    let ts = started.to_string();
    let dur = (p.duration as u64).to_string();
    let mut params = vec![("method", "track.scrobble"), ("api_key", acc.api_key.as_str()), ("sk", acc.session.as_str()), ("artist", artist.as_str()), ("track", title.as_str()), ("timestamp", ts.as_str())];
    if !p.album.is_empty() && p.album != "YouTube" { params.push(("album", p.album)); }
    if p.duration > 0.0 { params.push(("duration", &dur)); }
    call(&params, &acc.secret, true).map(|_| ())
}

/// Whether it's time to scrobble: track longer than 30 s and more than half or 4 minutes played.
pub fn due(position: f64, duration: f64) -> bool {
    duration > 30.0 && position >= (duration / 2.0).min(240.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signs() {
        // md5("api_keyxxmethodauth.getTokensecret")
        assert_eq!(sign(&[("method", "auth.getToken"), ("api_key", "xx"), ("format", "json")], "secret"), format!("{:x}", Md5::digest(b"api_keyxxmethodauth.getTokensecret")));
    }
    #[test]
    fn cleans() {
        assert_eq!(clean("LiSA Official YouTube Channel", "LiSA - Gurenge (Official Video)"), ("LiSA".into(), "Gurenge".into()));
        assert_eq!(clean("LiSA - Topic", "紅蓮華"), ("LiSA".into(), "紅蓮華".into()));
        assert_eq!(clean("Eve", "Eve - Kaikai Kitan"), ("Eve".into(), "Kaikai Kitan".into()));
        assert_eq!(clean("YOASOBI", "アイドル"), ("YOASOBI".into(), "アイドル".into()));
    }
    #[test]
    fn due_rule() {
        assert!(!due(20.0, 25.0));
        assert!(!due(100.0, 238.0));
        assert!(due(120.0, 238.0));
        assert!(due(240.0, 900.0));
    }
}
