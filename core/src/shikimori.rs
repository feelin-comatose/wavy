//! Шикимори: поиск тайтла (GraphQL API, без ключа) и постер.
use crate::anime::Anime;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Основной домен и запасные (Шикимори периодически переезжает).
const HOSTS: &[&str] = &["https://shikimori.io", "https://shikimori.one"];
/// Шикимори просит представляться в User-Agent.
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

/// Поиск тайтлов по названию (любой язык).
pub fn search(q: &str, limit: u32) -> Result<Vec<ShikiAnime>, String> {
    let mut last = String::new();
    for host in HOSTS {
        let r = crate::http().post(&format!("{host}/api/graphql")).header("User-Agent", UA).header("Content-Type", "application/json").send(query(q, limit).to_string());
        match r {
            Ok(mut resp) => {
                let v: Value = resp.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).ok_or("Шикимори: непонятный ответ")?;
                if let Some(e) = v.get("errors") { return Err(format!("Шикимори: {e}")); }
                let list = v["data"]["animes"].as_array().cloned().unwrap_or_default();
                return Ok(list.iter().map(|a| parse(a, host)).collect());
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("Шикимори недоступен: {last}"))
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

/// Похожесть строк: доля общих биграмм (Дайс). 1.0 — совпадение.
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

/// Выбрать тайтл для серий: по похожести названия, с учётом сезона и числа серий.
pub fn best_match<'a>(title: &str, season: Option<u32>, episodes: usize, list: &'a [ShikiAnime]) -> Option<&'a ShikiAnime> {
    let want_season = season.unwrap_or(1);
    list.iter().map(|a| {
        let names = [&a.name, &a.russian, &a.english, &a.japanese].into_iter().chain(a.synonyms.iter());
        let mut score = names.map(|n| similarity(title, n)).fold(0.0f32, f32::max);
        // сезон: «2nd Season», «Season 2», « 2» в названии
        let has_season = |n: u32| {
            let l = a.name.to_lowercase();
            l.contains(&format!("season {n}")) || l.contains(&format!("{n}{} season", match n { 2 => "nd", 3 => "rd", _ => "th" })) || l.ends_with(&format!(" {n}"))
                || a.russian.ends_with(&format!(" {n}")) || a.russian.contains(&format!("{n} сезон"))
        };
        if want_season > 1 { if has_season(want_season) { score += 0.3; } else { score -= 0.15; } }
        else if (2..=6).any(has_season) { score -= 0.2; }
        // одна серия — скорее фильм; много — сериал
        match (episodes, a.kind.as_str()) {
            (1, "movie") => score += 0.1,
            (n, "tv") if n > 1 => score += 0.1,
            (n, "movie") if n > 2 => score -= 0.3,
            _ => {}
        }
        // реклама, клипы и промо почти никогда не то, что смотрят
        if matches!(a.kind.as_str(), "cm" | "pv" | "music") { score -= 0.35; }
        if a.episodes > 0 && episodes as u32 > a.episodes + 2 { score -= 0.1; }
        (score, a)
    }).filter(|(s, _)| *s > 0.35).max_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, a)| a)
}

/// Скачать постер в папку обложек (чтобы работало без интернета).
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

/// Перенести данные Шикимори в тайтл (с постером).
pub fn apply(a: &mut Anime, s: &ShikiAnime, covers: &Path) {
    a.shiki_id = Some(s.id);
    a.name = s.name.clone();
    a.russian = s.russian.clone();
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
