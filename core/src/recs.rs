//! "For you" page: rows of recommendations from YouTube Music, like the YouTube home page.
//! Everything here works without an account: "Because you listened to …" rows are radios of
//! tracks from the local listening history, plus new releases, charts and moods/genres.
//! ("Mixed for you" is the account's "My wave", see ytaccount.rs.)
use crate::store::Library;
use crate::youtube::{rt, ytm_track, YouTube};
use crate::Track;
use std::collections::{HashMap, HashSet};

/// What a card opens: a single track (played in place) or a list of tracks to fetch first.
#[derive(Clone, Debug, PartialEq)]
pub enum CardKind { Track(Box<Track>), Album, Playlist }

/// One cover in a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub kind: CardKind,
    /// album or playlist id on YouTube Music (for tracks — the track id)
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub cover: Option<String>,
}

impl Card {
    /// Title without the artist at the start ("Radiohead - Creep" by Radiohead → "Creep"): video titles repeat it.
    pub fn short_title(&self) -> &str {
        let (t, a) = (self.title.as_str(), self.subtitle.split(',').next().unwrap_or("").trim());
        if a.is_empty() { return t; }
        for sep in [" - ", " – ", " — "] {
            let head = format!("{a}{sep}");
            if t.len() > head.len() && t.get(..head.len()).is_some_and(|h| h.eq_ignore_ascii_case(&head)) { return t[head.len()..].trim(); }
        }
        t
    }

    pub fn track(t: Track) -> Self {
        Card { id: t.id.clone(), title: t.title.clone(), subtitle: t.artist.clone(), cover: t.cover.as_deref().map(small_cover), kind: CardKind::Track(Box::new(t)) }
    }
}

/// Moods and genres for the chips: (label — Russian, shown through i18n::tr; search query on YouTube Music).
pub const MOODS: &[(&str, &str)] = &[
    ("Расслабиться", "relax"), ("Энергия", "energize"), ("Тренировка", "workout"), ("Хорошее настроение", "feel good"),
    ("Фокус", "focus"), ("Вечеринка", "party"), ("Грусть", "sad songs"), ("Романтика", "romance"), ("Сон", "sleep"),
    ("Дорога", "commute"), ("Поп", "pop hits"), ("Рок", "rock classics"), ("Хип-хоп", "hip hop"), ("Электроника", "electronic dance"),
    ("Джаз", "jazz"), ("Классика", "classical essentials"), ("Метал", "metal"), ("Лоу-фай", "lofi"), ("K-pop", "k-pop hits"), ("Аниме", "anime songs"),
];

fn best(th: &[rustypipe::model::Thumbnail]) -> Option<String> { th.iter().max_by_key(|c| c.width).map(|c| small_cover(&c.url)) }

/// A cover just big enough for a card (~150 px): less to download and decode than the full-size one.
/// Video frames: the 16:9 one (crops to a square without the black bars of the 4:3 one);
/// YouTube Music covers: asked for at 226 px.
pub fn small_cover(url: &str) -> String {
    if url.contains("i.ytimg.com/vi/") {
        for big in ["/hqdefault.jpg", "/sddefault.jpg", "/maxresdefault.jpg", "/hq720.jpg"] {
            if let Some(i) = url.find(big) { return format!("{}/mqdefault.jpg", &url[..i]); }
        }
    }
    if url.contains("googleusercontent.com/") {
        if let Some(i) = url.rfind("=w") {
            let tail = &url[i + 2..];
            let n = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
            if n > 0 && tail[n..].starts_with("-h") { return format!("{}=w226-h226{}", &url[..i], tail[n + 2..].trim_start_matches(|c: char| c.is_ascii_digit())); }
        }
    }
    url.to_string()
}

fn playlist_card(p: rustypipe::model::MusicPlaylistItem) -> Card {
    let sub = p.channel.map(|c| c.name).unwrap_or_else(|| "YouTube Music".into());
    Card { kind: CardKind::Playlist, cover: best(&p.thumbnail), id: p.id, title: p.name, subtitle: sub }
}

/// Main artist of a track (the first one in "A, B feat. C"), lowercased — so two rows don't come from the same artist.
fn main_artist(a: &str) -> String {
    let a = a.to_lowercase();
    let cut = [",", "&", " feat", " ft.", " x ", " и "].iter().filter_map(|s| a.find(s)).min().unwrap_or(a.len());
    a[..cut].trim().to_string()
}

/// Seed tracks for the "Because you listened to …" rows: what's played most over the last month
/// (then all time, then likes, then recently added), one per artist. `salt` rotates through the
/// top candidates so "Refresh" gives other rows.
pub fn seeds(lib: &Library, now: u64, n: usize, salt: u64) -> Vec<Track> {
    const MONTH: u64 = 30 * 86_400_000;
    let ok = |id: &str| lib.tracks.get(id).filter(|t| t.anime.is_none() && t.kind != "radio" && !t.title.trim().is_empty());
    let mut score: HashMap<&str, (u32, u64)> = HashMap::new();
    for (id, at) in &lib.history {
        if *at + MONTH < now || ok(id).is_none() { continue; }
        let e = score.entry(id.as_str()).or_default();
        e.0 += 1;
        e.1 = e.1.max(*at);
    }
    let mut ranked: Vec<(&str, (u32, u64))> = score.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)).then(a.0.cmp(b.0)));
    let mut ids: Vec<String> = ranked.into_iter().map(|(id, _)| id.to_string()).collect();
    let mut all: Vec<(&String, &u32)> = lib.plays.iter().filter(|(id, n)| **n > 0 && ok(id).is_some()).collect();
    all.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    ids.extend(all.into_iter().map(|(id, _)| id.clone()));
    ids.extend(lib.liked.iter().filter(|id| ok(id).is_some()).cloned());
    let mut recent: Vec<&Track> = lib.tracks.values().filter(|t| ok(&t.id).is_some()).collect();
    recent.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(a.id.cmp(&b.id)));
    ids.extend(recent.into_iter().take(30).map(|t| t.id.clone()));
    // candidates: the first few distinct tracks, rotated by salt
    let mut seen = HashSet::new();
    let mut cand: Vec<&Track> = ids.iter().filter(|id| seen.insert(id.as_str())).filter_map(|id| ok(id)).take(n * 4).collect();
    if !cand.is_empty() { let k = (salt as usize) % cand.len(); cand.rotate_left(k); }
    let mut artists = HashSet::new();
    let mut out = Vec::new();
    for t in cand {
        let a = main_artist(&t.artist);
        if !a.is_empty() && !artists.insert(a) { continue; }
        out.push(t.clone());
        if out.len() >= n { break; }
    }
    out
}

/// Text of `{"runs": [{"text": …}, …]}`.
fn runs(v: &serde_json::Value) -> String {
    v["runs"].as_array().map(|r| r.iter().filter_map(|x| x["text"].as_str()).collect()).unwrap_or_default()
}

/// Every `musicTwoRowItemRenderer` (the square cards of YouTube Music pages) on a page, in order:
/// playlists, albums and videos. Unknown kinds (artists, podcasts) are skipped.
pub fn two_row_cards(page: &serde_json::Value) -> Vec<Card> {
    fn walk(v: &serde_json::Value, out: &mut Vec<Card>, seen: &mut HashSet<String>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(it) = m.get("musicTwoRowItemRenderer") {
                    if let Some(c) = two_row(it) { if seen.insert(c.id.clone()) { out.push(c); } }
                    return;
                }
                for x in m.values() { walk(x, out, seen); }
            }
            serde_json::Value::Array(a) => for x in a { walk(x, out, seen); },
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(page, &mut out, &mut HashSet::new());
    out
}

fn two_row(it: &serde_json::Value) -> Option<Card> {
    let title = runs(&it["title"]);
    if title.is_empty() { return None; }
    let cover = it["thumbnailRenderer"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"].as_array()
        .and_then(|a| a.iter().max_by_key(|t| t["width"].as_u64().unwrap_or(0))).and_then(|t| t["url"].as_str()).map(small_cover);
    let nav = &it["navigationEndpoint"];
    // the subtitle is "Playlist • YouTube Music" / "Album • Artist • 2025" — without the kind word
    let sub_runs: Vec<String> = it["subtitle"]["runs"].as_array().map(|r| r.iter().filter_map(|x| x["text"].as_str()).map(String::from).collect()).unwrap_or_default();
    if let Some(vid) = nav["watchEndpoint"]["videoId"].as_str() {
        let artist = sub_runs.iter().find(|r| r.trim() != "•" && !r.trim().is_empty()).cloned().unwrap_or_default();
        let t = Track {
            id: format!("yt_{vid}"), kind: "youtube".into(), path: None, video_id: Some(vid.into()),
            url: Some(format!("https://music.youtube.com/watch?v={vid}")), title, artist, album: String::new(), duration: 0.0,
            cover: Some(format!("https://i.ytimg.com/vi/{vid}/hqdefault.jpg")), added_at: crate::now_ms(), video: false, anime: None, episode: None,
        };
        return Some(Card::track(t));
    }
    let b = &nav["browseEndpoint"];
    let id = b["browseId"].as_str()?;
    let kind = match b["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]["pageType"].as_str()? {
        "MUSIC_PAGE_TYPE_PLAYLIST" => CardKind::Playlist,
        "MUSIC_PAGE_TYPE_ALBUM" => CardKind::Album,
        _ => return None,
    };
    let parts: Vec<&str> = sub_runs.iter().map(|r| r.trim()).filter(|r| !r.is_empty() && *r != "•").collect();
    let subtitle = if parts.len() > 1 { parts[1..].join(" · ") } else { parts.join("") };
    // playlist pages are "VL<playlist id>"
    let id = if kind == CardKind::Playlist { id.strip_prefix("VL").unwrap_or(id) } else { id };
    Some(Card { kind, id: id.to_string(), title, subtitle, cover })
}

/// Uploads that aren't the song itself: live versions, lyric videos, covers, edits.
fn noise(title: &str) -> bool {
    let t = format!(" {} ", title.to_lowercase().replace(['(', ')', '[', ']', '|', '-', ',', '"'], " "));
    [" live ", " lyrics ", " lyric ", " cover ", " karaoke ", " concert ", " slowed ", " sped up ", " nightcore ", " 8d ", " bass boost", " bassboost ", " reverb ", " 1 hour ", " instrumental "].iter().any(|w| t.contains(w))
}

/// Similar songs → a row: without "noise" uploads (unless the seed itself is one, or too little would be left)
/// and no more than two songs per artist.
pub fn clean_similar(list: Vec<Track>, seed_title: &str, n: usize) -> Vec<Track> {
    let clean: Vec<Track> = if noise(seed_title) { list.clone() } else { list.iter().filter(|t| !noise(&t.title)).cloned().collect() };
    let list = if clean.len() >= n.min(6) { clean } else { list };
    let mut per: HashMap<String, usize> = HashMap::new();
    list.into_iter().filter(|t| { let c = per.entry(main_artist(&t.artist)).or_default(); *c += 1; *c <= 2 }).take(n).collect()
}

impl YouTube {
    /// YouTube video id for a seed: its own, or (a local file, own server) the first YouTube Music match.
    pub fn seed_video(&self, t: &Track) -> Option<String> {
        if let Some(v) = &t.video_id { return Some(v.clone()); }
        let q = if t.artist.is_empty() { t.title.clone() } else { format!("{} {}", t.artist, t.title) };
        self.search_music(&q, 1).ok()?.into_iter().next()?.video_id
    }

    /// Songs similar to a track, without the track itself and `exclude` (video ids): its YouTube Music
    /// radio, or (if that fails) the YouTube mix through yt-dlp. Live versions, lyric videos and covers are
    /// dropped unless the seed is one too.
    pub fn similar(&self, seed: &Track, video_id: &str, exclude: &HashSet<String>, n: usize) -> Result<Vec<Track>, String> {
        let ytm = self.rustypipe().and_then(|rp| rt().block_on(rp.query().music_radio_track(video_id)).map_err(|e| format!("YouTube Music: {e}")))
            .map(|r| r.items.into_iter().map(ytm_track).collect::<Vec<_>>());
        let list = match ytm {
            Ok(v) if v.len() > 3 => v,
            other => match self.radio(video_id, 30) { Ok(v) if !v.is_empty() => v, Ok(_) => other?, Err(e) => other.map_err(|_| e)? },
        };
        let list: Vec<Track> = list.into_iter().filter(|t| t.video_id.as_deref().is_some_and(|v| v != video_id && !exclude.contains(v))).collect();
        Ok(clean_similar(list, &seed.title, n))
    }

    /// A rustypipe query in the interface language (titles of charts and playlists come localized),
    /// region: Russia for the Russian interface, otherwise the default one.
    fn ytm_query(&self, ru: bool) -> Result<rustypipe::client::RustyPipeQuery, String> {
        let q = self.rustypipe()?.query();
        Ok(if ru { q.lang(rustypipe::param::Language::Ru).country(rustypipe::param::Country::Ru) } else { q })
    }

    fn browse(&self, ru: bool, body: serde_json::Value) -> Result<serde_json::Value, String> {
        let q = self.ytm_query(ru)?;
        let txt = rt().block_on(q.raw(rustypipe::client::ClientType::DesktopMusic, "browse", &body)).map_err(|e| format!("YouTube Music: {e}"))?;
        serde_json::from_str(&txt).map_err(|e| format!("YouTube Music: {e}"))
    }

    /// New releases: the songs of YouTube Music's "Released" playlist (new songs of the week),
    /// otherwise the page's cards as they are (new music videos).
    /// (YouTube Music dropped its "new albums" list, so this page is parsed as it is now.)
    pub fn new_releases(&self, ru: bool) -> Result<Vec<Card>, String> {
        let v = self.browse(ru, serde_json::json!({"browseId": "FEmusic_new_releases"}))?;
        let cards = two_row_cards(&v);
        if let Some(pl) = cards.iter().find(|c| c.kind == CardKind::Playlist) {
            if let Ok((_, _, tracks)) = self.card_tracks(pl) { if !tracks.is_empty() { return Ok(tracks.into_iter().take(40).map(Card::track).collect()); } }
        }
        if cards.is_empty() { return Err(l!("YouTube Music не отдал новинки", "YouTube Music returned no new releases").into()); }
        Ok(cards)
    }

    /// Charts: YouTube Music chart playlists (top music videos, trending, by genre) for the region.
    pub fn charts(&self, ru: bool) -> Result<Vec<Card>, String> {
        let body = serde_json::json!({"browseId": "FEmusic_charts", "params": "sgYPRkVtdXNpY19leHBsb3Jl"});
        let mut cards: Vec<Card> = two_row_cards(&self.browse(ru, body.clone())?).into_iter().filter(|c| c.kind == CardKind::Playlist).collect();
        // no charts for the region — the global ones
        if cards.is_empty() && ru { cards = two_row_cards(&self.browse(false, body)?).into_iter().filter(|c| c.kind == CardKind::Playlist).collect(); }
        if cards.is_empty() { return Err(l!("YouTube Music не отдал чарты", "YouTube Music returned no charts").into()); }
        Ok(cards)
    }

    /// Playlists for a mood or genre, through playlist search.
    /// (YouTube Music's "Moods & genres" page no longer opens without an account.)
    pub fn mood_playlists(&self, query: &str, ru: bool) -> Result<Vec<Card>, String> {
        let q = self.ytm_query(ru)?;
        let mut v = Vec::new();
        // YouTube Music's own playlists first (that search is often empty now), then community ones
        for community in [false, true] {
            if let Ok(r) = rt().block_on(q.music_search_playlists(query, community)) { v.extend(r.items.items.into_iter().filter(|p| !p.is_podcast)); }
            if v.len() >= 10 { break; }
        }
        if v.is_empty() { return Err(l!("ничего не нашлось", "nothing found").into()); }
        let mut seen = HashSet::new();
        Ok(v.into_iter().filter(|p| seen.insert(p.id.clone())).take(20).map(playlist_card).collect())
    }

    /// Tracks of a card's album or playlist: (name, link for a synced playlist, tracks).
    pub fn card_tracks(&self, card: &Card) -> Result<(String, String, Vec<Track>), String> {
        let rp = self.rustypipe()?;
        let link = |id: &str| format!("https://music.youtube.com/playlist?list={id}");
        match &card.kind {
            CardKind::Track(t) => Ok((t.title.clone(), t.url.clone().unwrap_or_default(), vec![(**t).clone()])),
            CardKind::Album => {
                let a = rt().block_on(rp.query().music_album(&card.id)).map_err(|e| format!("YouTube Music: {e}"))?;
                let cover = best(&a.cover);
                let tracks = a.tracks.into_iter().map(|t| {
                    let mut t = ytm_track(t);
                    // album pages have no per-track covers — the album's one instead of a video frame
                    if cover.is_some() { t.cover = cover.clone(); }
                    if t.album.is_empty() { t.album = a.name.clone(); }
                    t
                }).collect();
                Ok((a.name, a.playlist_id.as_deref().map(link).unwrap_or_else(|| format!("https://music.youtube.com/browse/{}", card.id)), tracks))
            }
            CardKind::Playlist => {
                let q = rp.query();
                let (name, items) = rt().block_on(async {
                    let mut p = q.music_playlist(&card.id).await?;
                    p.tracks.extend_limit(&q, 200).await?;
                    Ok::<_, rustypipe::error::Error>((p.name, p.tracks.items))
                }).map_err(|e| format!("YouTube Music: {e}"))?;
                Ok((name, link(&card.id), items.into_iter().take(200).map(ytm_track).collect()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(id: &str, artist: &str) -> Track {
        Track { id: id.into(), kind: "youtube".into(), path: None, video_id: Some(id.into()), url: None, title: id.into(), artist: artist.into(),
            album: String::new(), duration: 0.0, cover: None, added_at: 0, video: false, anime: None, episode: None }
    }

    #[test]
    fn main_artist_cuts_features() {
        assert_eq!(main_artist("Daft Punk, Pharrell Williams"), "daft punk");
        assert_eq!(main_artist("Eminem feat. Rihanna"), "eminem");
        assert_eq!(main_artist("Simon & Garfunkel"), "simon");
        assert_eq!(main_artist(""), "");
    }

    #[test]
    fn seeds_most_played_one_per_artist() {
        let now = 100 * 86_400_000;
        let mut lib = Library::default();
        for x in [t("a1", "A"), t("a2", "A, B"), t("b1", "B"), t("c1", "C"), t("old", "D")] { lib.tracks.insert(x.id.clone(), x); }
        let mut radio = t("r", "R");
        radio.kind = "radio".into();
        lib.tracks.insert("r".into(), radio);
        // a1 ×3, a2 ×2, b1 ×1 this month; r — a radio station, not a seed; old — played long ago, only in plays
        for (id, n) in [("a1", 3), ("a2", 2), ("b1", 1), ("r", 9)] { for i in 0..n { lib.history.push((id.into(), now - 1000 - i)); } }
        lib.history.push(("old".into(), 1));
        lib.plays.insert("old".into(), 1);
        let ids = |v: Vec<Track>| v.into_iter().map(|t| t.id).collect::<Vec<_>>();
        // a2 is the same main artist as a1 — skipped; then b1, then the all-time "old"
        assert_eq!(ids(seeds(&lib, now, 3, 0)), ["a1", "b1", "old"]);
        // salt rotates the candidates
        assert_eq!(ids(seeds(&lib, now, 2, 1)), ["a2", "b1"]);
        assert!(seeds(&Library::default(), now, 3, 0).is_empty());
    }

    #[test]
    fn parses_two_row_cards() {
        let page = serde_json::json!({"contents": [{"musicCarouselShelfRenderer": {"contents": [
            {"musicTwoRowItemRenderer": {"title": {"runs": [{"text": "Top 100"}]}, "subtitle": {"runs": [{"text": "Playlist"}, {"text": " • "}, {"text": "YouTube Music"}]},
                "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [{"url": "s", "width": 60}, {"url": "big", "width": 544}]}}},
                "navigationEndpoint": {"browseEndpoint": {"browseId": "VLPL123", "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_PLAYLIST"}}}}}},
            {"musicTwoRowItemRenderer": {"title": {"runs": [{"text": "Song"}]}, "subtitle": {"runs": [{"text": "Artist"}, {"text": " • "}, {"text": "1M views"}]},
                "navigationEndpoint": {"watchEndpoint": {"videoId": "abc"}}}},
            {"musicTwoRowItemRenderer": {"title": {"runs": [{"text": "Someone"}]},
                "navigationEndpoint": {"browseEndpoint": {"browseId": "UC1", "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}}},
            {"musicTwoRowItemRenderer": {"title": {"runs": [{"text": "Top 100 again"}]},
                "navigationEndpoint": {"browseEndpoint": {"browseId": "VLPL123", "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_PLAYLIST"}}}}}}
        ]}}]});
        let c = two_row_cards(&page);
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].kind.clone(), c[0].id.as_str(), c[0].subtitle.as_str(), c[0].cover.as_deref()), (CardKind::Playlist, "PL123", "YouTube Music", Some("big")));
        match &c[1].kind { CardKind::Track(t) => assert_eq!((t.video_id.as_deref(), t.artist.as_str(), t.title.as_str()), (Some("abc"), "Artist", "Song")), k => panic!("{k:?}") }
    }

    #[test]
    fn similar_drops_noise() {
        let mut v: Vec<Track> = ["Song A (Live at Wembley)", "Song B (Official Video)", "Song C [Lyrics]", "Song D", "Song E - Cover", "Song F", "Song G", "Song H", "Song I"]
            .iter().enumerate().map(|(i, x)| { let mut t = t(&format!("v{i}"), &format!("Artist {i}")); t.title = x.to_string(); t }).collect();
        let titles = |v: Vec<Track>| v.into_iter().map(|t| t.title).collect::<Vec<_>>();
        assert_eq!(titles(clean_similar(v.clone(), "Get Lucky", 10)), ["Song B (Official Video)", "Song D", "Song F", "Song G", "Song H", "Song I"]);
        // the seed is a live version — live ones are fine
        assert_eq!(clean_similar(v.clone(), "Get Lucky (Live)", 10).len(), 9);
        // at most two per artist
        for t in v.iter_mut() { t.artist = "Same".into(); }
        assert_eq!(clean_similar(v, "x", 10).len(), 2);
    }

    #[test]
    fn short_title_drops_the_artist() {
        let c = |title: &str, artist: &str| { let mut x = t("v", artist); x.title = title.into(); Card::track(x) };
        assert_eq!(c("Radiohead - No Surprises", "Radiohead").short_title(), "No Surprises");
        assert_eq!(c("RADIOHEAD – Creep (Live)", "Radiohead, Someone").short_title(), "Creep (Live)");
        assert_eq!(c("Creep", "Radiohead").short_title(), "Creep");
        assert_eq!(c("Radiohead - ", "Radiohead").short_title(), "Radiohead - ");
        assert_eq!(c("Кино - Группа крови", "Кино").short_title(), "Группа крови");
    }

    #[test]
    fn small_covers() {
        assert_eq!(small_cover("https://i.ytimg.com/vi/abc/hqdefault.jpg"), "https://i.ytimg.com/vi/abc/mqdefault.jpg");
        assert_eq!(small_cover("https://lh3.googleusercontent.com/x=w544-h544-l90-rj"), "https://lh3.googleusercontent.com/x=w226-h226-l90-rj");
        assert_eq!(small_cover("https://yt3.googleusercontent.com/x=w544-h544"), "https://yt3.googleusercontent.com/x=w226-h226");
        assert_eq!(small_cover("https://lh3.googleusercontent.com/x=s120"), "https://lh3.googleusercontent.com/x=s120");
        assert_eq!(small_cover("/home/a.jpg"), "/home/a.jpg");
    }
}
