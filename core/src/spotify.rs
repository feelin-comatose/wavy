//! Импорт из Spotify: плейлист / альбом / трек → поиск каждого трека на YouTube.
//! Без настроек читаем публичную embed-страницу (до 100 треков). С ключом Spotify API
//! (Client ID + Secret в настройках) — весь плейлист любого размера.
use crate::{now_ms, youtube::YouTube, Track};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct SpTrack {
    pub title: String,
    pub artists: String,
    pub album: String,
    pub duration: f64,
}

pub struct SpList {
    pub name: String,
    pub tracks: Vec<SpTrack>,
    /// embed-страница отдаёт не больше 100 треков — плейлист мог обрезаться
    pub maybe_truncated: bool,
}

/// open.spotify.com/(intl-xx/)playlist/ID, album/ID, track/ID или spotify:playlist:ID
pub fn parse_url(url: &str) -> Option<(String, String)> {
    let u = url.trim();
    if let Some(rest) = u.strip_prefix("spotify:") {
        let mut p = rest.split(':');
        return Some((p.next()?.to_string(), p.next()?.to_string()));
    }
    let parsed = url::Url::parse(u).ok()?;
    if !parsed.host_str()?.ends_with("spotify.com") { return None; }
    let segs: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty() && !s.starts_with("intl-") && *s != "embed").collect();
    match segs.as_slice() {
        [kind, id, ..] if matches!(*kind, "playlist" | "album" | "track") => Some((kind.to_string(), id.to_string())),
        _ => None,
    }
}

pub fn is_spotify(url: &str) -> bool { parse_url(url).is_some() }

fn get(url: &str) -> Result<String, String> {
    let mut r = crate::http().get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
        .header("Accept-Language", "en")
        .call().map_err(|e| lf!("Spotify не ответил: {e}", "Spotify didn't respond: {e}"))?;
    r.body_mut().with_config().limit(20 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())
}

/// Публичная embed-страница: без ключей, до 100 треков.
pub fn fetch_embed(kind: &str, id: &str) -> Result<SpList, String> {
    let html = get(&format!("https://open.spotify.com/embed/{kind}/{id}"))?;
    let start = html.find(r#"<script id="__NEXT_DATA__" type="application/json">"#).ok_or(l!("Spotify изменил страницу — не нашёл данные плейлиста", "Spotify changed the page — couldn't find the playlist data"))?;
    let json = &html[start..];
    let json = &json[json.find('>').unwrap() + 1..];
    let json = &json[..json.find("</script>").ok_or(l!("битая страница Spotify", "broken Spotify page"))?];
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let e = &v["props"]["pageProps"]["state"]["data"]["entity"];
    if e.is_null() { return Err(l!("плейлист не найден или приватный", "the playlist wasn't found or is private").into()); }
    let name = e["name"].as_str().unwrap_or("Spotify").to_string();
    let album = if kind == "album" { name.clone() } else { String::new() };
    let list: Vec<SpTrack> = if kind == "track" {
        vec![SpTrack {
            title: name.clone(),
            artists: e["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            album: String::new(),
            duration: e["duration"].as_f64().unwrap_or(0.0) / 1000.0,
        }]
    } else {
        e["trackList"].as_array().map(|a| a.iter().map(|t| SpTrack {
            title: t["title"].as_str().unwrap_or("").to_string(),
            artists: t["subtitle"].as_str().unwrap_or("").replace('\u{a0}', " "),
            album: album.clone(),
            duration: t["duration"].as_f64().unwrap_or(0.0) / 1000.0,
        }).filter(|t| !t.title.is_empty()).collect()).unwrap_or_default()
    };
    let maybe_truncated = kind == "playlist" && list.len() >= 100;
    Ok(SpList { name, tracks: list, maybe_truncated })
}

/// Официальный Web API (Client Credentials): весь плейлист любого размера.
pub fn fetch_api(kind: &str, id: &str, client_id: &str, secret: &str) -> Result<SpList, String> {
    use std::io::Write;
    let mut basic = Vec::new();
    write!(basic, "{client_id}:{secret}").ok();
    let auth = base64(&basic);
    let mut r = crate::http().post("https://accounts.spotify.com/api/token")
        .header("Authorization", &format!("Basic {auth}"))
        .content_type("application/x-www-form-urlencoded")
        .send("grant_type=client_credentials")
        .map_err(|e| lf!("Spotify API: не удалось войти (проверь Client ID / Secret): {e}", "Spotify API: couldn't sign in (check the Client ID / Secret): {e}"))?;
    let tok: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let token = tok["access_token"].as_str().ok_or(l!("Spotify API не выдал токен", "Spotify API didn't give a token"))?.to_string();
    let api = |url: &str| -> Result<Value, String> {
        let mut r = crate::http().get(url).header("Authorization", &format!("Bearer {token}")).call().map_err(|e| format!("Spotify API: {e}"))?;
        serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    };
    let artists = |t: &Value| t["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    let mk = |t: &Value, album: Option<&str>| SpTrack {
        title: t["name"].as_str().unwrap_or("").to_string(),
        artists: artists(t),
        album: album.map(str::to_string).or_else(|| t["album"]["name"].as_str().map(str::to_string)).unwrap_or_default(),
        duration: t["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0,
    };
    match kind {
        "track" => {
            let t = api(&format!("https://api.spotify.com/v1/tracks/{id}"))?;
            Ok(SpList { name: t["name"].as_str().unwrap_or("Spotify").into(), tracks: vec![mk(&t, None)], maybe_truncated: false })
        }
        "album" | "playlist" => {
            let meta = api(&format!("https://api.spotify.com/v1/{kind}s/{id}?fields=name"))?;
            let name = meta["name"].as_str().unwrap_or("Spotify").to_string();
            let mut tracks = Vec::new();
            let mut next = Some(format!("https://api.spotify.com/v1/{kind}s/{id}/tracks?limit={}", if kind == "album" { 50 } else { 100 }));
            while let Some(url) = next {
                let page = api(&url)?;
                for it in page["items"].as_array().into_iter().flatten() {
                    let t = if kind == "playlist" { &it["track"] } else { it };
                    if t.is_null() || t["type"].as_str() == Some("episode") { continue; }
                    tracks.push(mk(t, (kind == "album").then_some(name.as_str())));
                }
                next = page["next"].as_str().map(str::to_string);
            }
            Ok(SpList { name, tracks, maybe_truncated: false })
        }
        _ => Err(l!("неизвестный тип ссылки Spotify", "unknown Spotify link type").into()),
    }
}

/// Найти трек на YouTube: из 5 результатов берём лучший по длительности и «официальности».
pub fn match_on_youtube(yt: &YouTube, t: &SpTrack) -> Result<Option<Track>, String> { Ok(ranked(yt, t, None)?.into_iter().next()) }

/// Видео трека пропало с YouTube (удалено, закрыто, недоступно в стране) — найти такой же трек в другом видео.
/// Возвращает трек с тем же id (плейлисты не ломаются), но с новым видео.
/// Лучшая замена трека, чьё видео пропало.
pub fn find_replacement(yt: &YouTube, old: &Track) -> Result<Option<Track>, String> { Ok(find_replacements(yt, old)?.into_iter().next()) }

/// Кандидаты на замену (лучшие первыми): если первый тоже не играет (например, тоже 18+), пробуем следующий.
pub fn find_replacements(yt: &YouTube, old: &Track) -> Result<Vec<Track>, String> {
    // «Artist - Title» в названии видео без исполнителя (часто у загрузок с обычных каналов)
    let (artist, title) = match old.title.split_once(" - ") {
        Some((a, t)) if old.artist.is_empty() || old.artist == "YouTube" => (a.trim().to_string(), t.trim().to_string()),
        _ => (old.artist.trim_end_matches(" - Topic").to_string(), old.title.clone()),
    };
    let sp = SpTrack { title, artists: artist, album: old.album.clone(), duration: old.duration };
    let mut list = ranked(yt, &sp, old.video_id.as_deref())?;
    // исполнитель часто — просто канал загрузившего («ThePajiloyStalker»): с ним поиск находит мало, ищем ещё по названию
    if list.len() < 3 && !sp.artists.is_empty() {
        let only_title = SpTrack { artists: String::new(), ..sp };
        for c in ranked(yt, &only_title, old.video_id.as_deref()).unwrap_or_default() {
            if !list.iter().any(|x| x.video_id == c.video_id) { list.push(c); }
        }
    }
    Ok(list.into_iter().map(|mut t| {
        t.id = old.id.clone();
        t.title = old.title.clone();
        t.artist = old.artist.clone();
        t.added_at = old.added_at;
        t
    }).collect())
}

fn ranked(yt: &YouTube, t: &SpTrack, exclude: Option<&str>) -> Result<Vec<Track>, String> {
    let main_artist = t.artists.split(',').next().unwrap_or("").trim().to_string();
    let q = if main_artist.is_empty() { t.title.clone() } else { format!("{main_artist} - {}", t.title) };
    let mut cands = yt.search(&q, 6)?;
    cands.retain(|c| c.video_id.is_some() && c.video_id.as_deref() != exclude);
    let norm = |s: &str| s.to_lowercase();
    let title_l = norm(&t.title);
    let best = cands.into_iter().map(|c| {
        let mut score = 0.0f64;
        if t.duration > 0.0 && c.duration > 0.0 {
            let d = (c.duration - t.duration).abs();
            score -= d.min(120.0) / 4.0; // каждые 4 с расхождения — минус балл
            if d > 30.0 { score -= 20.0; }
        }
        let ct = norm(&c.title);
        if ct.contains(&title_l) { score += 6.0; }
        if !main_artist.is_empty() && (norm(&c.artist).contains(&norm(&main_artist)) || ct.contains(&norm(&main_artist))) { score += 4.0; }
        for bad in ["live", "cover", "remix", "karaoke", "8d", "slowed", "sped up", "nightcore", "reaction"] {
            if ct.contains(bad) && !title_l.contains(bad) { score -= 8.0; }
        }
        if ct.contains("official audio") || ct.contains("audio") { score += 2.0; }
        (score, c)
    }).filter(|(s, _)| *s > -25.0).collect::<Vec<_>>();
    let mut best = best;
    best.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(best.into_iter().map(|(_, c)| {
        let vid = c.video_id.clone().unwrap_or_default();
        Track {
            id: c.id.clone(),
            kind: "youtube".into(),
            path: None,
            url: Some(format!("https://music.youtube.com/watch?v={vid}")),
            cover: c.cover.clone(),
            video_id: c.video_id.clone(),
            title: t.title.clone(),
            artist: if t.artists.is_empty() { c.artist.clone() } else { t.artists.clone() },
            album: t.album.clone(),
            duration: if t.duration > 0.0 { t.duration } else { c.duration },
            added_at: now_ms(),
            video: false, anime: None, episode: None,
        }
    }).collect())
}

pub struct SpImport {
    pub name: String,
    pub source: String,
    pub tracks: Vec<Track>,
    pub not_found: Vec<String>,
    pub truncated: bool,
}

/// Полный импорт: список из Spotify → поиск на YouTube в несколько потоков.
pub fn import(url: &str, yt: &YouTube, creds: Option<(&str, &str)>, status: &(dyn Fn(&str) + Sync)) -> Result<SpImport, String> {
    let (kind, id) = parse_url(url).ok_or(l!("это не ссылка на Spotify", "this isn't a Spotify link"))?;
    status(l!("Spotify: читаю список…", "Spotify: reading the list…"));
    let list = match creds {
        Some((cid, sec)) if !cid.is_empty() && !sec.is_empty() => fetch_api(&kind, &id, cid, sec)?,
        _ => fetch_embed(&kind, &id)?,
    };
    if list.tracks.is_empty() { return Err(l!("в плейлисте Spotify нет треков (или он приватный)", "the Spotify playlist has no tracks (or it's private)").into()); }
    yt.ensure_binary(&|m| status(m))?;
    let n = list.tracks.len();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Arc<Mutex<Vec<Option<Track>>>> = Arc::new(Mutex::new(vec![None; n]));
    std::thread::scope(|sc| {
        for _ in 0..6 {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= n { break; }
                let found = match_on_youtube(yt, &list.tracks[i]).unwrap_or(None);
                results.lock().unwrap()[i] = found;
                let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                if d % 5 == 0 || d == n { status(&lf!("Spotify: ищу на YouTube {d}/{n}…", "Spotify: searching YouTube {d}/{n}…")); }
            });
        }
    });
    let results = Arc::try_unwrap(results).map(|m| m.into_inner().unwrap()).unwrap_or_default();
    let base = now_ms();
    let mut tracks = Vec::new();
    let mut not_found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, r) in results.into_iter().enumerate() {
        match r {
            Some(mut t) if seen.insert(t.id.clone()) => { t.added_at = base + (n - i) as u64; tracks.push(t); }
            Some(_) => {}
            None => not_found.push(format!("{} — {}", list.tracks[i].artists, list.tracks[i].title)),
        }
    }
    if !not_found.is_empty() { crate::log(&format!("Spotify: не найдено на YouTube: {}", not_found.join("; "))); }
    Ok(SpImport { name: list.name, source: url.trim().to_string(), tracks, not_found, truncated: list.maybe_truncated })
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for k in 0..4 {
            if k <= c.len() { out.push(T[(n >> (18 - 6 * k) & 63) as usize] as char); } else { out.push('='); }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls() {
        assert_eq!(parse_url("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc"), Some(("playlist".into(), "37i9dQZF1DXcBWIGoYBM5M".into())));
        assert_eq!(parse_url("https://open.spotify.com/intl-ru/album/4aawyAB9vmqN3uQ7FjRGTy"), Some(("album".into(), "4aawyAB9vmqN3uQ7FjRGTy".into())));
        assert_eq!(parse_url("spotify:track:3h5T5JypYU7huFiVYhv1dr"), Some(("track".into(), "3h5T5JypYU7huFiVYhv1dr".into())));
        assert_eq!(parse_url("https://music.youtube.com/playlist?list=x"), None);
        assert_eq!(base64(b"ab:cd"), "YWI6Y2Q=");
    }
}
