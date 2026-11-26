//! AniList: mark watched episodes in your AniList list (like the Shikimori sync).
//! AniList only lets registered API clients in, so the user creates one once at
//! anilist.co/settings/developer with the redirect URL https://anilist.co/api/v2/oauth/pin and pastes
//! its Client ID into Wavy. Signing in opens AniList, which then shows a token to paste back (valid a year).
//! Titles are found by their MyAnimeList id, which is the same as the Shikimori id Wavy already has.
use serde_json::{json, Value};

const API: &str = "https://graphql.anilist.co";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub client_id: String,
    pub token: String,
    pub user: String,
    pub enabled: bool,
}

impl Account {
    pub fn ready(&self) -> bool { self.enabled && !self.token.is_empty() }
}

/// Where the user signs in; AniList then shows the token to copy ("pin" redirect).
pub fn auth_url(client_id: &str) -> String {
    format!("https://anilist.co/api/v2/oauth/authorize?client_id={}&response_type=token", client_id.trim())
}

fn gql(token: Option<&str>, query: &str, vars: Value) -> Result<Value, String> {
    let mut req = crate::http().post(API).header("Content-Type", "application/json").header("Accept", "application/json");
    if let Some(t) = token { req = req.header("Authorization", &format!("Bearer {t}")); }
    let mut r = req.send(json!({ "query": query, "variables": vars }).to_string()).map_err(|e| lf!("AniList не ответил: {e}", "AniList didn't respond: {e}"))?;
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) { return Err(format!("AniList: {}", e["message"].as_str().unwrap_or("error"))); }
    Ok(v["data"].clone())
}

/// Check a pasted token; Ok(user name).
pub fn viewer(token: &str) -> Result<String, String> {
    let d = gql(Some(token.trim()), "query { Viewer { name } }", json!({}))?;
    d["Viewer"]["name"].as_str().map(str::to_string).ok_or_else(|| l!("AniList не принял токен — войди заново", "AniList didn't accept the token — sign in again").into())
}

/// List status for `episodes` watched out of `total` (0 — unknown).
pub fn status(episodes: u32, total: u32) -> &'static str { if total > 0 && episodes >= total { "COMPLETED" } else { "CURRENT" } }

/// Record `episodes` watched for the title with MyAnimeList id `mal_id`. Never moves progress back.
/// Ok(false) — AniList doesn't have the title, or it already shows as much.
pub fn set_progress(acc: &Account, mal_id: u64, episodes: u32, total: u32) -> Result<bool, String> {
    let d = gql(Some(&acc.token), "query($mal: Int) { Media(idMal: $mal, type: ANIME) { id mediaListEntry { progress } } }", json!({ "mal": mal_id }))?;
    let Some(id) = d["Media"]["id"].as_u64() else { return Ok(false) };
    let have = d["Media"]["mediaListEntry"]["progress"].as_u64().unwrap_or(0) as u32;
    if have >= episodes { return Ok(false); }
    gql(Some(&acc.token), "mutation($id: Int, $p: Int, $s: MediaListStatus) { SaveMediaListEntry(mediaId: $id, progress: $p, status: $s) { id } }",
        json!({ "id": id, "p": episodes, "s": status(episodes, total) }))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_status() {
        assert_eq!(auth_url(" 12345 "), "https://anilist.co/api/v2/oauth/authorize?client_id=12345&response_type=token");
        assert_eq!(status(12, 12), "COMPLETED");
        assert_eq!(status(5, 12), "CURRENT");
        assert_eq!(status(40, 0), "CURRENT"); // ongoing, total unknown
        assert!(!Account { token: "t".into(), ..Default::default() }.ready());
    }
}
