//! A title's openings and endings: AnimeThemes (by MyAnimeList id, which is also the Shikimori id),
//! falling back to MyAnimeList through Jikan. The tracks themselves are then looked up on YouTube.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// "OP" or "ED"
    pub kind: String,
    pub n: u32,
    pub title: String,
    pub artist: String,
}

impl Theme {
    /// Label in the playlist: "OP1 · Kaikai Kitan".
    pub fn label(&self) -> String { format!("{}{}", self.kind, if self.n > 0 { self.n.to_string() } else { String::new() }) }
    /// Query for searching YouTube.
    pub fn query(&self) -> String { if self.artist.is_empty() { self.title.clone() } else { format!("{} - {}", self.artist, self.title) } }
}

fn get(url: &str) -> Result<Value, String> {
    let mut r = crate::http().get(url).header("User-Agent", "Wavy (music & anime player)").header("Accept", "application/json")
        .call().map_err(|e| e.to_string())?;
    let t = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&t).map_err(|_| l!("непонятный ответ", "unexpected response").to_string())
}

pub fn fetch(mal_id: u64) -> Result<Vec<Theme>, String> {
    let at = get(&format!("https://api.animethemes.moe/anime?filter[has]=resources&filter[site]=MyAnimeList&filter[external_id]={mal_id}&include=animethemes.song.artists"))
        .map(|v| parse_animethemes(&v));
    match at {
        Ok(list) if !list.is_empty() => Ok(list),
        r => {
            if let Err(e) = &r { crate::log(&format!("animethemes: {e} — пробую MyAnimeList")); }
            let j = get(&format!("https://api.jikan.moe/v4/anime/{mal_id}/themes")).map_err(|e| lf!("не получил список опенингов: {e}", "didn't get the openings list: {e}"))?;
            Ok(parse_jikan(&j))
        }
    }
}

pub fn parse_animethemes(v: &Value) -> Vec<Theme> {
    let mut out = Vec::new();
    for a in v["anime"].as_array().into_iter().flatten().take(1) {
        for t in a["animethemes"].as_array().into_iter().flatten() {
            let title = t["song"]["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() { continue; }
            let artist = t["song"]["artists"].as_array().into_iter().flatten().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ");
            out.push(Theme { kind: t["type"].as_str().unwrap_or("OP").to_string(), n: t["sequence"].as_u64().unwrap_or(0) as u32, title, artist });
        }
    }
    sort(&mut out);
    out
}

/// '1: "Kaikai Kitan (廻廻奇譚)" by Eve (eps 1-13)' → title and artist.
fn parse_jikan_line(s: &str, kind: &str, idx: usize) -> Option<Theme> {
    let s = s.trim();
    let (n, rest) = match s.split_once(':') {
        Some((n, r)) if n.trim().chars().all(|c| c.is_ascii_digit()) && !n.trim().is_empty() => (n.trim().parse().unwrap_or(idx as u32 + 1), r.trim()),
        _ => (idx as u32 + 1, s),
    };
    let rest = rest.trim_start_matches('#');
    let (title, after) = match rest.strip_prefix('"').and_then(|r| r.split_once('"')) {
        Some((t, a)) => (t.to_string(), a.trim()),
        None => (rest.split(" by ").next().unwrap_or(rest).to_string(), rest.split_once(" by ").map(|x| x.1).unwrap_or("")),
    };
    // the Japanese title in brackets is noise for search
    let title = match title.rfind(" (") { Some(i) if title.ends_with(')') && title[i..].chars().any(|c| !c.is_ascii()) => title[..i].to_string(), _ => title };
    let artist = after.trim_start_matches("by ").trim();
    let artist = artist.split(" (ep").next().unwrap_or(artist).trim();
    let artist = match artist.rfind(" (") { Some(i) if artist.ends_with(')') && artist[i..].chars().any(|c| !c.is_ascii()) => &artist[..i], _ => artist };
    if title.trim().is_empty() { return None; }
    Some(Theme { kind: kind.into(), n, title: title.trim().to_string(), artist: artist.to_string() })
}

pub fn parse_jikan(v: &Value) -> Vec<Theme> {
    let mut out = Vec::new();
    for (key, kind) in [("openings", "OP"), ("endings", "ED")] {
        for (i, s) in v["data"][key].as_array().into_iter().flatten().filter_map(Value::as_str).enumerate() {
            if let Some(t) = parse_jikan_line(s, kind, i) { out.push(t); }
        }
    }
    sort(&mut out);
    out
}

fn sort(v: &mut [Theme]) { v.sort_by(|a, b| (a.kind != "OP", a.n).cmp(&(b.kind != "OP", b.n))); }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn animethemes() {
        let v = json!({"anime": [{"name": "Jujutsu Kaisen", "animethemes": [
            {"type": "ED", "sequence": 1, "song": {"title": "LOST IN PARADISE feat. AKLO", "artists": [{"name": "ALI"}, {"name": "AKLO"}]}},
            {"type": "OP", "sequence": 2, "song": {"title": "VIVID VICE", "artists": [{"name": "Who-ya Extended"}]}},
            {"type": "OP", "sequence": 1, "song": {"title": "Kaikai Kitan", "artists": [{"name": "Eve"}]}},
            {"type": "OP", "sequence": 3, "song": null}
        ]}]});
        let t = parse_animethemes(&v);
        assert_eq!(t.len(), 3);
        assert_eq!((t[0].label(), t[0].query()), ("OP1".into(), "Eve - Kaikai Kitan".into()));
        assert_eq!(t[1].label(), "OP2");
        assert_eq!((t[2].label(), t[2].artist.as_str()), ("ED1".into(), "ALI, AKLO"));
    }

    #[test]
    fn jikan() {
        let v = json!({"data": {
            "openings": ["1: \"Kaikai Kitan (廻廻奇譚)\" by Eve (eps 1-13)", "2: \"VIVID VICE\" by Who-ya Extended (eps 14-24)"],
            "endings": ["\"LOST IN PARADISE feat. AKLO\" by ALI (eps 1-13)", "2: \"give it back\" by Cö shu Nie (コシュニエ) (eps 14-24)"]
        }});
        let t = parse_jikan(&v);
        assert_eq!(t.len(), 4);
        assert_eq!((t[0].title.as_str(), t[0].artist.as_str(), t[0].n), ("Kaikai Kitan", "Eve", 1));
        assert_eq!((t[1].title.as_str(), t[1].artist.as_str()), ("VIVID VICE", "Who-ya Extended"));
        assert_eq!((t[2].label(), t[2].title.as_str(), t[2].artist.as_str()), ("ED1".into(), "LOST IN PARADISE feat. AKLO", "ALI"));
        assert_eq!(t[3].artist, "Cö shu Nie");
    }
}
