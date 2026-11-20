//! Аниме: тайтлы из видеофайлов серий. Название и номер серии берутся из имени файла
//! (или папки), обложка и русское название — с Шикимори.
use crate::{local, now_ms, sha16, Track};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Видео, которое можно добавить как серию.
pub const EPISODE_EXT: &[&str] = &["mkv", "mp4", "m4v", "mov"];
/// Внешние субтитры рядом с серией.
pub const SUB_EXT: &[&str] = &["ass", "ssa", "srt"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Anime {
    pub id: String,
    /// Нормализованное название из имён файлов — по нему серии собираются в тайтл.
    pub key: String,
    /// Название, как его удалось вытащить из файлов (пока нет данных Шикимори).
    pub title: String,
    pub shiki_id: Option<u64>,
    pub name: String,
    pub russian: String,
    pub kind: String,
    pub episodes_total: u32,
    pub score: f32,
    pub year: u32,
    /// Постер, сохранённый локально (covers/poster_<id>.jpg).
    pub poster: Option<String>,
    pub url: Option<String>,
    /// Серии (id треков) по порядку.
    pub episodes: Vec<String>,
    /// Просмотренные серии.
    pub watched: Vec<String>,
    /// На какой серии остановились и где.
    pub last: Option<String>,
    pub last_pos: f64,
    /// Выбранная звуковая дорожка (номер среди звуковых) — переносится на следующие серии.
    pub audio: Option<usize>,
    /// Выбранные субтитры: "off", "e:<язык или номер>" или "x:<суффикс файла>".
    pub subs: Option<String>,
    pub added_at: u64,
    /// Шикимори уже спрашивали (даже если не нашли).
    pub matched: bool,
}

impl Anime {
    /// Название для показа: русское с Шикимори → оригинальное → из файлов.
    pub fn display(&self) -> &str {
        if !self.russian.is_empty() { &self.russian } else if !self.name.is_empty() { &self.name } else { &self.title }
    }
    /// Строка «ТВ · 2023 · 28 эп. · ★ 9.25».
    pub fn meta(&self) -> String {
        let mut v = Vec::new();
        if !self.kind.is_empty() { v.push(kind_ru(&self.kind).to_string()); }
        if self.year > 0 { v.push(self.year.to_string()); }
        if self.episodes_total > 0 { v.push(format!("{} эп.", self.episodes_total)); }
        if self.score > 0.0 { v.push(format!("★ {:.2}", self.score)); }
        v.join(" · ")
    }
}

pub fn kind_ru(kind: &str) -> &str {
    match kind {
        "tv" => "ТВ",
        "movie" => "Фильм",
        "ova" => "OVA",
        "ona" => "ONA",
        "special" | "tv_special" => "Спешл",
        "music" => "Клип",
        "pv" | "cm" => "Промо",
        _ => kind,
    }
}

/// Что удалось понять из имени файла серии.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub title: String,
    pub episode: Option<f32>,
    pub season: Option<u32>,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

/// Мусор из имён релизов: качество, кодеки, источники.
const JUNK: &[&str] = &[
    "1080p", "720p", "480p", "2160p", "4k", "1080", "720", "x264", "x265", "h264", "h265", "h.264", "h.265", "hevc", "avc", "aac", "flac", "ac3", "opus",
    "10bit", "10-bit", "8bit", "hi10p", "web", "web-dl", "webdl", "webrip", "bd", "bdrip", "bdremux", "bluray", "blu-ray", "dvd", "dvdrip", "tvrip", "hdtv",
    "rus", "eng", "jap", "jpn", "sub", "subs", "dub", "multi", "multisub", "raw", "uncensored", "remux", "dual", "audio", "hdr", "sdr",
];

fn is_junk(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    !w.is_empty() && (JUNK.contains(&w.as_str()) || w.ends_with("bit"))
}

/// Убрать скобки с группами, качеством и CRC; разделители → пробелы.
fn clean(s: &str) -> String {
    static BR: OnceLock<Regex> = OnceLock::new();
    static PAR: OnceLock<Regex> = OnceLock::new();
    let mut s = re(&BR, r"\[[^\]]*\]|【[^】]*】|\{[^}]*\}").replace_all(s, " ").into_owned();
    // (1080p), (BD 1080p HEVC), (2023), (ABCD1234) — убираем; (TV) и прочее осмысленное оставляем
    s = re(&PAR, r"\(([^)]*)\)").replace_all(&s, |c: &regex::Captures| {
        let inner = &c[1];
        let crc = inner.len() == 8 && inner.chars().all(|ch| ch.is_ascii_hexdigit());
        let year = inner.len() == 4 && inner.chars().all(|ch| ch.is_ascii_digit());
        if crc || year || inner.split_whitespace().any(is_junk) { " ".to_string() } else { c[0].to_string() }
    }).into_owned();
    // точки и подчёркивания как разделители (Title.S01E02.1080p / Title_-_02)
    if s.contains('_') { s = s.replace('_', " "); }
    if !s.contains(' ') || s.matches('.').count() >= 2 {
        static DOT: OnceLock<Regex> = OnceLock::new();
        s = re(&DOT, r"\.(\D)").replace_all(&s, " $1").into_owned();
        s = s.replace('.', " ").replace("  ", " ");
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Отрезать хвост из мусора («Title 1080p x264» → «Title»).
fn strip_tail(s: &str) -> String {
    let mut words: Vec<&str> = s.split_whitespace().collect();
    while words.last().is_some_and(|w| is_junk(w) || *w == "-" || *w == "–" || *w == "|") { words.pop(); }
    while words.first().is_some_and(|w| *w == "-" || *w == "–") { words.remove(0); }
    words.join(" ")
}

/// Сезон из названия: «Season 2», «2nd Season», «S2», «Сезон 2».
fn season_of(title: &str) -> Option<u32> {
    static S: OnceLock<Regex> = OnceLock::new();
    let r = re(&S, r"(?i)\b(?:season|сезон)\s*(\d{1,2})\b|\b(\d{1,2})(?:st|nd|rd|th)\s+season\b|\bs(\d{1,2})\b");
    let c = r.captures(title)?;
    c.get(1).or(c.get(2)).or(c.get(3)).and_then(|m| m.as_str().parse().ok())
}

/// Разобрать имя файла серии. Если в имени нет названия («01.mkv») — берём папку.
pub fn parse_path(path: &Path) -> Parsed {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut p = parse_name(&stem);
    // «Серия 01», «01», «Episode 5» — название в папке (а «Season 2» / «Сезон 2» — ещё уровнем выше)
    if p.title.chars().filter(|c| c.is_alphabetic()).count() < 2 || is_generic(&p.title) {
        let mut dir = path.parent();
        while let Some(d) = dir {
            let name = d.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = parse_name(&name);
            let generic = is_generic(&folder.title) || season_of(&name).is_some() && strip_season(&folder.title).trim().is_empty();
            if !generic && folder.title.chars().filter(|c| c.is_alphabetic()).count() >= 2 {
                p.title = folder.title;
                p.season = p.season.or(folder.season);
                break;
            }
            if p.season.is_none() { p.season = season_of(&name); }
            dir = d.parent();
        }
    }
    if p.season.is_none() { p.season = season_of(&p.title); }
    p
}

fn is_generic(t: &str) -> bool {
    let l = t.to_lowercase();
    let l = l.trim();
    l.is_empty() || ["episode", "серия", "ep", "e", "эпизод", "video", "видео", "anime", "аниме", "downloads", "загрузки", "сериалы"].contains(&l)
        || l.starts_with("season") || l.starts_with("сезон")
}

fn strip_season(t: &str) -> String {
    static S: OnceLock<Regex> = OnceLock::new();
    re(&S, r"(?i)\b(?:season|сезон)\s*\d{1,2}\b|\b\d{1,2}(?:st|nd|rd|th)\s+season\b|\bs\d{1,2}\b").replace_all(t, "").into_owned()
}

/// Разобрать имя без расширения.
pub fn parse_name(stem: &str) -> Parsed {
    let s = clean(stem);
    static SE: OnceLock<Regex> = OnceLock::new();
    static DASH: OnceLock<Regex> = OnceLock::new();
    static WORD: OnceLock<Regex> = OnceLock::new();
    static LEAD: OnceLock<Regex> = OnceLock::new();
    static TAIL: OnceLock<Regex> = OnceLock::new();
    let num = |m: &str| m.parse::<f32>().ok();

    // Title S01E05 / Title - S2E05
    if let Some(c) = re(&SE, r"(?i)^(.*?)[\s\-]*\bS(\d{1,2})\s?E(\d{1,4}(?:\.\d)?)(?:v\d)?\b").captures(&s) {
        let title = strip_tail(&c[1]);
        return Parsed { season: c[2].parse().ok(), episode: num(&c[3]), title };
    }
    // [Group] Title - 05v2 [1080p]  — самый частый формат фансаба
    if let Some(c) = re(&DASH, r"^(.*?)\s+[-–]\s+(?:(?i:ep|episode|e|серия)\.?\s*)?(\d{1,4}(?:\.\d)?)(?:v\d)?(?:\s|$)").captures(&s) {
        let title = strip_tail(&c[1]);
        if !title.is_empty() { return Parsed { episode: num(&c[2]), season: season_of(&title), title }; }
    }
    // Title Episode 5 / Title Серия 5 / Title EP05
    if let Some(c) = re(&WORD, r"(?i)^(.*?)\s*\b(?:episode|ep|e|серия|эпизод)\.?\s*(\d{1,4}(?:\.\d)?)\b").captures(&s) {
        return Parsed { title: strip_tail(&c[1]), episode: num(&c[2]), season: None };
    }
    // 05. Title / 05 - Title / 05
    if let Some(c) = re(&LEAD, r"^(\d{1,4})(?:\s*[.\-–)]\s*|\s+|$)(.*)$").captures(&s) {
        let rest = strip_tail(&c[2]);
        if c[1].len() <= 3 || rest.is_empty() { return Parsed { title: rest, episode: num(&c[1]), season: None }; }
    }
    // Title 05 (число в конце после чистки мусора). «Mob Psycho 100» без серии тут тоже станет серией 100 —
    // но такие имена файлов у серий встречаются редко, а «Title 05» — часто
    let st = strip_tail(&s);
    if let Some(c) = re(&TAIL, r"^(.*\D)\s(\d{1,4})(?:v\d)?$").captures(&st) {
        let title = strip_tail(&c[1]);
        if title.chars().any(|ch| ch.is_alphabetic()) && season_of(&format!("{} {}", &c[1], &c[2])).is_none() {
            return Parsed { season: season_of(&title), episode: num(&c[2]), title };
        }
    }
    Parsed { season: season_of(&st), title: st, episode: None }
}

/// Ключ для группировки серий: только буквы и цифры в нижнем регистре + сезон.
pub fn key_of(p: &Parsed) -> String {
    let base: String = strip_season(&p.title).to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
    match p.season { Some(s) if s > 1 => format!("{base}#s{s}"), _ => base }
}

/// Строка поиска на Шикимори: название + сезон («Title 2nd Season» Шикимори находит и по «Title 2»).
pub fn search_query(p: &Parsed) -> String {
    let base = strip_season(&p.title).split_whitespace().collect::<Vec<_>>().join(" ");
    match p.season { Some(s) if s > 1 => format!("{base} {s}"), _ => base }
}

fn ext(p: &Path) -> String { p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase() }
pub fn is_episode(p: &Path) -> bool { EPISODE_EXT.contains(&ext(p).as_str()) }

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() { if depth < 6 { walk(&p, out, depth + 1) } } else if is_episode(&p) { out.push(p) }
    }
}

/// Серия как трек: название — «Серия N», исполнитель — тайтл (видно в плеере, Discord и оверлее).
pub fn episode_track(path: &Path, p: &Parsed, anime_title: &str) -> Track {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let title = match p.episode {
        Some(e) if e.fract() == 0.0 => format!("Серия {}", e as u32),
        Some(e) => format!("Серия {e}"),
        None => path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    Track {
        id: local::local_id(path),
        kind: "local".into(),
        path: Some(abs.to_string_lossy().into_owned()),
        video_id: None,
        url: None,
        title,
        artist: anime_title.to_string(),
        album: String::new(),
        duration: crate::media::duration_of(path).unwrap_or(0.0),
        cover: None,
        added_at: now_ms(),
        video: true,
        anime: None,
        episode: p.episode,
    }
}

pub struct ImportGroup {
    pub key: String,
    pub parsed: Parsed,
    pub tracks: Vec<Track>,
}

/// Файлы и папки → серии, сгруппированные по тайтлам.
pub fn scan(paths: &[PathBuf], mut progress: impl FnMut(usize, usize)) -> Vec<ImportGroup> {
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() { walk(p, &mut files, 0) } else if is_episode(p) { files.push(p.clone()) }
    }
    let mut seen = std::collections::HashSet::new();
    files.retain(|f| seen.insert(f.clone()));
    let mut groups: Vec<ImportGroup> = Vec::new();
    let n = files.len();
    for (i, f) in files.iter().enumerate() {
        progress(i + 1, n);
        let p = parse_path(f);
        let key = key_of(&p);
        let t = episode_track(f, &p, &p.title);
        match groups.iter_mut().find(|g| g.key == key) {
            Some(g) => g.tracks.push(t),
            None => groups.push(ImportGroup { key, parsed: p, tracks: vec![t] }),
        }
    }
    for g in &mut groups { sort_episodes(&mut g.tracks); }
    groups
}

pub fn sort_episodes(v: &mut [Track]) {
    v.sort_by(|a, b| {
        let (ea, eb) = (a.episode.unwrap_or(f32::MAX), b.episode.unwrap_or(f32::MAX));
        ea.partial_cmp(&eb).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.path.cmp(&b.path))
    });
}

pub fn new_anime(key: &str, p: &Parsed) -> Anime {
    Anime { id: format!("an_{}", sha16(key.as_bytes())), key: key.into(), title: p.title.clone(), added_at: now_ms(), ..Default::default() }
}

/// Внешние субтитры к серии: файлы .ass/.srt с тем же началом имени рядом или в подпапках (Subs/, RUS Sub/ …).
pub fn external_subs(video: &Path) -> Vec<PathBuf> {
    let Some(dir) = video.parent() else { return vec![] };
    let stem = video.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    fn go(d: &Path, depth: u32, stem: &str, out: &mut Vec<PathBuf>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            v.sort();
            for p in v {
                if p.is_dir() { if depth > 0 { go(&p, depth - 1, stem, out) } continue; }
                let name = p.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
                if SUB_EXT.contains(&ext(&p).as_str()) && name.starts_with(stem) { out.push(p); }
            }
    }
    let mut out = Vec::new();
    go(dir, 2, &stem, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> (String, Option<f32>, Option<u32>) {
        let r = parse_name(s);
        (r.title, r.episode, r.season)
    }

    #[test]
    fn fansub_names() {
        assert_eq!(p("[SubsPlease] Sousou no Frieren - 01 (1080p) [ABCD1234]"), ("Sousou no Frieren".into(), Some(1.0), None));
        assert_eq!(p("[Erai-raws] Kimetsu no Yaiba - Hashira Geiko-hen - 08 [1080p][Multiple Subtitle][F2A7C0D1]"), ("Kimetsu no Yaiba - Hashira Geiko-hen".into(), Some(8.0), None));
        assert_eq!(p("[AniLibria.TV] Jujutsu Kaisen - 12 [WEBRip 1080p]"), ("Jujutsu Kaisen".into(), Some(12.0), None));
        assert_eq!(p("Jujutsu_Kaisen_-_05_[AniLibria.TV]_[WEBRip_1080p]"), ("Jujutsu Kaisen".into(), Some(5.0), None));
        assert_eq!(p("[Judas] Shingeki no Kyojin - S04E16v2 [1080p][HEVC x265 10bit]"), ("Shingeki no Kyojin".into(), Some(16.0), Some(4)));
        assert_eq!(p("Spy.x.Family.S02E03.1080p.WEB-DL.AAC.x264"), ("Spy x Family".into(), Some(3.0), Some(2)));
        assert_eq!(p("[SubsPlease] Mushoku Tensei S2 - 13 (1080p) [0B3A1B7C]"), ("Mushoku Tensei S2".into(), Some(13.0), Some(2)));
        assert_eq!(p("[SubsPlease] One Piece - 1100 (1080p) [ABCDEF12]"), ("One Piece".into(), Some(1100.0), None));
        assert_eq!(p("[Group] Oshi no Ko - 11.5 (1080p)"), ("Oshi no Ko".into(), Some(11.5), None));
        assert_eq!(p("Chainsaw Man Episode 3"), ("Chainsaw Man".into(), Some(3.0), None));
        assert_eq!(p("Chainsaw Man 03"), ("Chainsaw Man".into(), Some(3.0), None));
        assert_eq!(p("Атака титанов - Серия 7"), ("Атака титанов".into(), Some(7.0), None));
        assert_eq!(p("07. Атака титанов"), ("Атака титанов".into(), Some(7.0), None));
        assert_eq!(p("Kimi no Na wa (2016) [BDRip 1080p]"), ("Kimi no Na wa".into(), None, None));
        assert_eq!(p("Dr. Stone - 05 [1080p]").0, "Dr. Stone");
    }

    #[test]
    fn name_from_folder() {
        let r = parse_path(Path::new("/anime/Sousou no Frieren [BD 1080p]/Season 1/01.mkv"));
        assert_eq!((r.title.as_str(), r.episode, r.season), ("Sousou no Frieren", Some(1.0), Some(1)));
        let r = parse_path(Path::new("/anime/Тетрадь смерти/Серия 05.mkv"));
        assert_eq!((r.title.as_str(), r.episode), ("Тетрадь смерти", Some(5.0)));
    }

    #[test]
    fn keys_group_seasons_apart() {
        let a = key_of(&parse_name("[SubsPlease] Mushoku Tensei S2 - 13 (1080p)"));
        let b = key_of(&parse_name("[SubsPlease] Mushoku Tensei S2 - 14 (1080p)"));
        let c = key_of(&parse_name("[SubsPlease] Mushoku Tensei - 03 (1080p)"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(search_query(&parse_name("[SubsPlease] Mushoku Tensei S2 - 13 (1080p)")), "Mushoku Tensei 2");
    }
}
