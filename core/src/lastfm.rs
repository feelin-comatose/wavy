//! Скробблинг в Last.fm: «сейчас слушает» и прослушанные треки (половина трека или 4 минуты).
//! Нужен ключ API Last.fm (last.fm/api/account/create — бесплатно): API key и Shared secret.
//! Вход: Wavy открывает страницу Last.fm, там жмёшь «Разрешить», потом «Готово» в Wavy.
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

/// Подпись запроса: md5 от «имязначение» по алфавиту + секрет.
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
    let mut r = r.map_err(|e| format!("Last.fm недоступен: {e}"))?;
    let v: Value = r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if let Some(msg) = v["message"].as_str().filter(|_| v.get("error").is_some()) { return Err(format!("Last.fm: {msg}")); }
    Ok(v)
}

/// Шаг 1: токен и страница, где человек разрешает доступ.
pub fn begin(acc: &Account) -> Result<(String, String), String> {
    let v = call(&[("method", "auth.getToken"), ("api_key", acc.api_key.trim())], acc.secret.trim(), false)?;
    let token = v["token"].as_str().ok_or("Last.fm не выдал токен — проверь API key")?.to_string();
    let url = format!("https://www.last.fm/api/auth/?api_key={}&token={token}", acc.api_key.trim());
    Ok((token, url))
}

/// Шаг 2: после «Разрешить» на сайте — постоянная сессия.
pub fn finish(acc: &mut Account, token: &str) -> Result<(), String> {
    let v = call(&[("method", "auth.getSession"), ("api_key", acc.api_key.trim()), ("token", token)], acc.secret.trim(), false)
        .map_err(|e| if e.contains("not authorized") || e.contains("Unauthorized") { "сначала нажми «Разрешить» на странице Last.fm".to_string() } else { e })?;
    acc.session = v["session"]["key"].as_str().ok_or("Last.fm не выдал сессию")?.to_string();
    acc.user = v["session"]["name"].as_str().unwrap_or("").to_string();
    acc.enabled = true;
    Ok(())
}

pub struct Play<'a> { pub artist: &'a str, pub title: &'a str, pub album: &'a str, pub duration: f64 }

/// Исполнитель и название для Last.fm: у видео с YouTube «Исполнитель - Название» часто в заголовке,
/// а «исполнитель» — это канал («LiSA Official», «… - Topic»).
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

/// Засчитать прослушивание; `started` — когда трек начал играть (Unix, секунды).
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

/// Пора ли скробблить: трек длиннее 30 с и проиграно больше половины или 4 минуты.
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
