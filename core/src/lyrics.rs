//! Тексты песен с LRCLIB (открытая база, без ключа): синхронные (построчно со временем) или простые.
//! Ответы кэшируются на диске — второй раз без интернета.
use crate::Track;
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    /// строки со временем (секунды); пусто — синхронного текста нет
    pub synced: Vec<(f64, String)>,
    pub plain: String,
    pub instrumental: bool,
}

impl Lyrics {
    pub fn is_empty(&self) -> bool { self.synced.is_empty() && self.plain.trim().is_empty() && !self.instrumental }
    /// Номер текущей строки.
    pub fn line_at(&self, t: f64) -> Option<usize> {
        let i = self.synced.partition_point(|(s, _)| *s <= t + 0.15);
        i.checked_sub(1)
    }
}

/// «[01:23.45] текст» → (83.45, «текст»). Несколько меток на строке — несколько строк.
pub fn parse_lrc(s: &str) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    for line in s.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { break };
            let tag = &r[..end];
            let parsed = tag.split_once(':').and_then(|(m, s)| Some(m.trim().parse::<f64>().ok()? * 60.0 + s.trim().parse::<f64>().ok()?));
            match parsed { Some(t) => times.push(t), None => break }
            rest = r[end + 1..].trim_start();
        }
        for t in times { out.push((t, rest.trim().to_string())); }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// Название без мусора YouTube: «(Official Video)», «[MV]», «feat.» и т.п.; «Артист - Песня» → разделить.
pub fn clean(track: &Track) -> (String, String) {
    let mut artist = track.artist.trim_end_matches(" - Topic").trim().to_string();
    let re = regex::Regex::new(r"(?i)\s*[\(\[【][^\)\]】]*(official|video|audio|lyric|mv|m/v|hd|4k|remaster|visuali[sz]er|music|clip|full|ver\.?|version|opening|ending)[^\)\]】]*[\)\]】]").unwrap();
    let mut title = re.replace_all(&track.title, "").to_string();
    title = regex::Regex::new(r"(?i)\s*(official\s+)?(music\s+)?(video|audio|mv|lyric video)\s*$").unwrap().replace(&title, "").to_string();
    // японский стиль: «YOASOBI「アイドル」»
    if let Some(c) = regex::Regex::new(r"^(.*?)\s*「(.+?)」").unwrap().captures(&title) {
        if !c[1].trim().is_empty() { artist = c[1].trim().to_string(); }
        title = c[2].to_string();
    } else if let Some((a, t)) = title.split_once(" - ").or_else(|| title.split_once(" – ")).or_else(|| title.split_once(" — ")) {
        // «Артист - Песня» — так называют видео на YouTube
        if track.kind != "local" || artist.is_empty() {
            artist = a.trim().to_string();
            title = t.trim().to_string();
        }
    }
    let title = regex::Regex::new(r"(?i)\s+(feat\.?|ft\.?)\s.*$").unwrap().replace(&title, "").trim().trim_matches('"').trim().to_string();
    (artist, title)
}

fn from_json(v: &Value) -> Lyrics {
    Lyrics {
        synced: v["syncedLyrics"].as_str().map(parse_lrc).unwrap_or_default(),
        plain: v["plainLyrics"].as_str().unwrap_or("").to_string(),
        instrumental: v["instrumental"].as_bool().unwrap_or(false),
    }
}

fn get_json(url: &str) -> Result<Option<Value>, String> {
    match crate::http().get(url).header("User-Agent", "Wavy (https://github.com/feelin-comatose/wavy)").call() {
        Ok(mut r) => Ok(r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok())),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(e) => Err(format!("LRCLIB недоступен: {e}")),
    }
}

/// Найти текст трека (сначала в кэше).
pub fn fetch(track: &Track, cache: &Path) -> Result<Lyrics, String> {
    let file = cache.join(format!("{}.json", crate::sha16(track.id.as_bytes())));
    if let Some(v) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) { return Ok(from_json(&v)); }
    let (artist, title) = clean(track);
    if title.is_empty() { return Ok(Lyrics::default()); }
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let mut found = None;
    if !artist.is_empty() {
        let mut u = format!("https://lrclib.net/api/get?artist_name={}&track_name={}", enc(&artist), enc(&title));
        if track.duration > 0.0 { u += &format!("&duration={}", track.duration.round()); }
        found = get_json(&u)?;
    }
    if found.is_none() {
        // поиск: берём вариант с близкой длительностью, лучше — с синхронным текстом
        let q = format!("{artist} {title}");
        if let Some(Value::Array(list)) = get_json(&format!("https://lrclib.net/api/search?q={}", enc(q.trim())))? {
            let close = |v: &&Value| track.duration <= 0.0 || (v["duration"].as_f64().unwrap_or(0.0) - track.duration).abs() < 8.0;
            found = list.iter().filter(close).find(|v| v["syncedLyrics"].is_string()).or_else(|| list.iter().find(close)).cloned();
        }
    }
    let v = found.unwrap_or(Value::Null);
    let _ = std::fs::create_dir_all(cache);
    let _ = std::fs::write(&file, v.to_string());
    Ok(from_json(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc() {
        let v = parse_lrc("[00:12.50] Привет\n[01:02.00][01:30.10]Припев\n[ar: x]\nбез времени");
        assert_eq!(v, vec![(12.5, "Привет".into()), (62.0, "Припев".into()), (90.1, "Припев".into())]);
        let l = Lyrics { synced: v, ..Default::default() };
        assert_eq!(l.line_at(5.0), None);
        assert_eq!(l.line_at(13.0), Some(0));
        assert_eq!(l.line_at(100.0), Some(2));
    }

    #[test]
    fn cleans_youtube_titles() {
        let t: Track = serde_json::from_value(serde_json::json!({"id": "x", "type": "youtube", "title": "LiSA - Gurenge (Official Music Video)", "artist": "LiSA Official YouTube"})).unwrap();
        assert_eq!(clean(&t), ("LiSA".into(), "Gurenge".into()));
        let t: Track = serde_json::from_value(serde_json::json!({"id": "x", "type": "youtube", "title": "Героиня", "artist": "9mice - Topic"})).unwrap();
        assert_eq!(clean(&t), ("9mice".into(), "Героиня".into()));
        let t: Track = serde_json::from_value(serde_json::json!({"id": "x", "type": "youtube", "title": "YOASOBI「アイドル」 Official Music Video", "artist": "Ayase / YOASOBI"})).unwrap();
        assert_eq!(clean(&t), ("YOASOBI".into(), "アイドル".into()));
    }
}
