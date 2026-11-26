//! Shikimori: title search (GraphQL API, no key) and poster.
use crate::anime::Anime;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Main domain and fallbacks (Shikimori moves from time to time).
const HOSTS: &[&str] = &["https://shikimori.io", "https://shikimori.one"];
/// Shikimori asks clients to identify themselves in the User-Agent.
const UA: &str = "Wavy (desktop music & anime player)";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShikiAnime {
    pub id: u64,
    pub name: String,
    pub russian: String,
    pub english: String,
    pub japanese: String,
    pub synonyms: Vec<String>,
    pub kind: String,
    pub episodes: u32,
    pub score: f32,
    pub year: u32,
    pub poster: Option<String>,
    pub url: String,
}

impl ShikiAnime {
    pub fn display(&self) -> &str { if self.russian.is_empty() { &self.name } else { &self.russian } }
}

fn query(q: &str, limit: u32) -> Value {
    json!({
        "query": "query($search: String, $limit: PositiveInt) { animes(search: $search, limit: $limit) { id name russian english japanese synonyms kind episodes score airedOn { year } poster { mainUrl originalUrl } url } }",
        "variables": { "search": q, "limit": limit }
    })
}

/// Search titles by name (any language).
pub fn search(q: &str, limit: u32) -> Result<Vec<ShikiAnime>, String> { run(query(q, limit)) }

/// Titles by Shikimori id (to fill in data added later, like English titles).
pub fn by_ids(ids: &[u64]) -> Result<Vec<ShikiAnime>, String> {
    let ids = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    run(json!({
        "query": "query($ids: String, $limit: PositiveInt) { animes(ids: $ids, limit: $limit) { id name russian english japanese synonyms kind episodes score airedOn { year } poster { mainUrl originalUrl } url } }",
        "variables": { "ids": ids, "limit": 50 }
    }))
}

fn run(body: Value) -> Result<Vec<ShikiAnime>, String> {
    let mut last = String::new();
    for host in HOSTS {
        let r = crate::http().post(&format!("{host}/api/graphql")).header("User-Agent", UA).header("Content-Type", "application/json").send(body.to_string());
        match r {
            Ok(mut resp) => {
                let v: Value = resp.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).ok_or(l!("Шикимори: непонятный ответ", "Shikimori: unexpected response"))?;
                if let Some(e) = v.get("errors") { return Err(format!("Шикимори: {e}")); }
                let list = v["data"]["animes"].as_array().cloned().unwrap_or_default();
                return Ok(list.iter().map(|a| parse(a, host)).collect());
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(lf!("Шикимори недоступен: {last}", "Shikimori is unavailable: {last}"))
}

fn parse(a: &Value, host: &str) -> ShikiAnime {
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let url = s("url");
    ShikiAnime {
        id: s("id").parse().ok().or_else(|| a["id"].as_u64()).unwrap_or(0),
        name: s("name"),
        russian: s("russian"),
        english: s("english"),
        japanese: s("japanese"),
        synonyms: a["synonyms"].as_array().map(|v| v.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default(),
        kind: s("kind"),
        episodes: a["episodes"].as_u64().unwrap_or(0) as u32,
        score: a["score"].as_f64().unwrap_or(0.0) as f32,
        year: a["airedOn"]["year"].as_u64().unwrap_or(0) as u32,
        poster: a["poster"]["originalUrl"].as_str().or(a["poster"]["mainUrl"].as_str()).filter(|u| !u.contains("missing")).map(String::from),
        url: if url.starts_with("http") { url } else if url.is_empty() { String::new() } else { format!("{host}{url}") },
    }
}

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// String similarity: share of common bigrams (Dice). 1.0 is a match.
fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() { return 0.0; }
    if a == b { return 1.0; }
    let bi = |s: &str| { let c: Vec<char> = s.chars().collect(); c.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>() };
    let (x, mut y) = (bi(&a), bi(&b));
    if x.is_empty() || y.is_empty() { return 0.0; }
    let total = (x.len() + y.len()) as f32;
    let mut common = 0;
    for p in &x { if let Some(i) = y.iter().position(|q| q == p) { y.swap_remove(i); common += 1; } }
    2.0 * common as f32 / total
}

/// Pick a title for episodes: by name similarity, taking season and episode count into account.
pub fn best_match<'a>(title: &str, season: Option<u32>, episodes: usize, list: &'a [ShikiAnime]) -> Option<&'a ShikiAnime> {
    let want_season = season.unwrap_or(1);
    list.iter().map(|a| {
        let names = [&a.name, &a.russian, &a.english, &a.japanese].into_iter().chain(a.synonyms.iter());
        let mut score = names.map(|n| similarity(title, n)).fold(0.0f32, f32::max);
        // season: "2nd Season", "Season 2", " 2" in the title
        let has_season = |n: u32| {
            let l = a.name.to_lowercase();
            l.contains(&format!("season {n}")) || l.contains(&format!("{n}{} season", match n { 2 => "nd", 3 => "rd", _ => "th" })) || l.ends_with(&format!(" {n}"))
                || a.russian.ends_with(&format!(" {n}")) || a.russian.contains(&format!("{n} сезон"))
        };
        if want_season > 1 { if has_season(want_season) { score += 0.3; } else { score -= 0.15; } }
        else if (2..=6).any(has_season) { score -= 0.2; }
        // one episode — probably a movie; many — a series
        match (episodes, a.kind.as_str()) {
            (1, "movie") => score += 0.1,
            (n, "tv") if n > 1 => score += 0.1,
            (n, "movie") if n > 2 => score -= 0.3,
            _ => {}
        }
        // ads, music videos and promos are almost never what people watch
        if matches!(a.kind.as_str(), "cm" | "pv" | "music") { score -= 0.35; }
        if a.episodes > 0 && episodes as u32 > a.episodes + 2 { score -= 0.1; }
        (score, a)
    }).filter(|(s, _)| *s > 0.35).max_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, a)| a)
}

/// Download the poster into the covers folder (so it works offline).
pub fn download_poster(url: &str, covers: &Path, id: u64) -> Result<PathBuf, String> {
    let _ = std::fs::create_dir_all(covers);
    let ext = if url.ends_with(".webp") { "webp" } else if url.ends_with(".png") { "png" } else { "jpg" };
    let path = covers.join(format!("poster_{id}.{ext}"));
    if path.exists() { return Ok(path); }
    let mut r = crate::http().get(url).header("User-Agent", UA).call().map_err(|e| e.to_string())?;
    let bytes = r.body_mut().with_config().limit(15 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Copy Shikimori data into the title (with the poster).
pub fn apply(a: &mut Anime, s: &ShikiAnime, covers: &Path) {
    a.shiki_id = Some(s.id);
    a.name = s.name.clone();
    a.russian = s.russian.clone();
    a.english = s.english.clone();
    a.en_checked = true;
    a.kind = s.kind.clone();
    a.episodes_total = s.episodes;
    a.score = s.score;
    a.year = s.year;
    a.url = Some(s.url.clone()).filter(|u| !u.is_empty());
    a.matched = true;
    a.poster = s.poster.as_deref().and_then(|u| match download_poster(u, covers, s.id) {
        Ok(p) => Some(p.to_string_lossy().into_owned()),
        Err(e) => { crate::log(&format!("постер {u}: {e}")); Some(u.to_string()) }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(id: u64, name: &str, russian: &str, kind: &str, episodes: u32) -> ShikiAnime {
        ShikiAnime { id, name: name.into(), russian: russian.into(), kind: kind.into(), episodes, ..Default::default() }
    }

    #[test]
    fn picks_right_season() {
        let list = vec![
            a(1, "Sousou no Frieren", "Провожающая в последний путь Фрирен", "tv", 28),
            a(2, "Sousou no Frieren: ●● no Mahou", "", "ona", 24),
            a(3, "Sousou no Frieren 2nd Season", "Провожающая в последний путь Фрирен 2", "tv", 10),
        ];
        assert_eq!(best_match("Sousou no Frieren", None, 12, &list).map(|a| a.id), Some(1));
        assert_eq!(best_match("Sousou no Frieren", Some(2), 10, &list).map(|a| a.id), Some(3));
        assert_eq!(best_match("Провожающая в последний путь Фрирен", None, 3, &list).map(|a| a.id), Some(1));
        assert_eq!(best_match("Совсем другое", None, 3, &list), None);
    }
}

// ---------- account: mark watched episodes in the Shikimori list ----------
//
// Shikimori only lets registered apps in: the user creates their own at
// shikimori.io/oauth/applications (Redirect URI: urn:ietf:wg:oauth:2.0:oob) and pastes its
// Client ID and Client Secret into Wavy. Sign-in uses a code from the Shikimori page; tokens refresh on their own after that.

pub const OOB: &str = "urn:ietf:wg:oauth:2.0:oob";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub client_id: String,
    pub client_secret: String,
    pub access: String,
    pub refresh: String,
    /// when the access token expires, ms
    pub expires: u64,
    pub user_id: u64,
    pub nickname: String,
}

impl Account {
    pub fn logged_in(&self) -> bool { !self.refresh.is_empty() && self.user_id > 0 }
}

/// Shikimori sign-in page (the user presses "Allow" there and gets a code).
pub fn authorize_url(client_id: &str) -> String {
    format!("{}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope=user_rates", HOSTS[0],
        url::form_urlencoded::byte_serialize(client_id.as_bytes()).collect::<String>(), url::form_urlencoded::byte_serialize(OOB.as_bytes()).collect::<String>())
}

fn token(acc: &mut Account, form: &[(&str, &str)]) -> Result<(), String> {
    let body: String = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
    let mut r = crate::http().post(&format!("{}/oauth/token", HOSTS[0])).header("User-Agent", UA_APP).header("Content-Type", "application/x-www-form-urlencoded")
        .send(body).map_err(|e| format!("Шикимори не принял вход: {e}"))?;
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    acc.access = v["access_token"].as_str().ok_or("Шикимори не выдал токен")?.to_string();
    if let Some(rt) = v["refresh_token"].as_str() { acc.refresh = rt.to_string(); }
    acc.expires = crate::now_ms() + v["expires_in"].as_u64().unwrap_or(86_400) * 1000 - 60_000;
    Ok(())
}

/// Shikimori requires the User-Agent to match the app name — so we ask users to name it "Wavy".
const UA_APP: &str = "Wavy";

/// Exchange the code from the sign-in page for tokens and find out who signed in.
pub fn login(acc: &mut Account, code: &str) -> Result<(), String> {
    let (id, secret) = (acc.client_id.clone(), acc.client_secret.clone());
    token(acc, &[("grant_type", "authorization_code"), ("client_id", &id), ("client_secret", &secret), ("code", code.trim()), ("redirect_uri", OOB)])?;
    let me = api(acc, "GET", "/api/users/whoami", None)?;
    acc.user_id = me["id"].as_u64().ok_or("Шикимори не сказал, кто вошёл")?;
    acc.nickname = me["nickname"].as_str().unwrap_or("").to_string();
    Ok(())
}

fn api(acc: &mut Account, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    if acc.expires <= crate::now_ms() && !acc.refresh.is_empty() {
        let (id, secret, rt) = (acc.client_id.clone(), acc.client_secret.clone(), acc.refresh.clone());
        token(acc, &[("grant_type", "refresh_token"), ("client_id", &id), ("client_secret", &secret), ("refresh_token", &rt)])?;
    }
    let url = format!("{}{path}", HOSTS[0]);
    let auth = format!("Bearer {}", acc.access);
    let a = crate::http();
    let r = match (method, body) {
        ("GET", _) => a.get(&url).header("User-Agent", UA_APP).header("Authorization", &auth).call(),
        ("POST", Some(b)) => a.post(&url).header("User-Agent", UA_APP).header("Authorization", &auth).header("Content-Type", "application/json").send(b.to_string()),
        ("PATCH", Some(b)) => a.patch(&url).header("User-Agent", UA_APP).header("Authorization", &auth).header("Content-Type", "application/json").send(b.to_string()),
        _ => return Err("неизвестный запрос".into()),
    };
    let mut r = r.map_err(|e| format!("Шикимори: {e}"))?;
    let t = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(serde_json::from_str(&t).unwrap_or(Value::Null))
}

/// Write to the Shikimori list: `episodes` episodes watched (all of them — "completed", otherwise "watching").
pub fn set_progress(acc: &mut Account, anime_id: u64, episodes: u32, total: u32) -> Result<(), String> {
    let status = if total > 0 && episodes >= total { "completed" } else { "watching" };
    let existing = api(acc, "GET", &format!("/api/v2/user_rates?user_id={}&target_id={anime_id}&target_type=Anime", acc.user_id), None)?;
    let rate = existing.as_array().and_then(|a| a.first()).cloned();
    match rate {
        Some(r) => {
            // don't roll back if the site already has more
            if r["episodes"].as_u64().unwrap_or(0) as u32 >= episodes && r["status"] != "planned" { return Ok(()); }
            let id = r["id"].as_u64().ok_or("непонятный ответ Шикимори")?;
            api(acc, "PATCH", &format!("/api/v2/user_rates/{id}"), Some(json!({ "user_rate": { "episodes": episodes, "status": status } })))?;
        }
        None => {
            api(acc, "POST", "/api/v2/user_rates", Some(json!({ "user_rate": { "user_id": acc.user_id, "target_id": anime_id, "target_type": "Anime", "episodes": episodes, "status": status } })))?;
        }
    }
    Ok(())
}
