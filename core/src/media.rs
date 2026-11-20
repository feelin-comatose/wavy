//! Что внутри видеофайла: длительность, звуковые дорожки, субтитры (встроенные в MKV и внешние .ass/.srt).
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct AudioTrack {
    pub label: String,
    pub lang: String,
    /// Умеем ли декодировать без ffmpeg (AAC, FLAC, MP3, Vorbis, PCM; Opus/AC3/DTS — через ffmpeg).
    pub supported: bool,
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubSrc {
    /// Номер дорожки в MKV.
    Embedded(u64),
    External(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubTrack {
    pub src: SubSrc,
    pub label: String,
    pub lang: String,
    pub default: bool,
    pub forced: bool,
}

impl SubTrack {
    /// Ключ для запоминания выбора между сериями: язык+название встроенной дорожки или хвост имени файла.
    pub fn key(&self) -> String {
        match &self.src {
            SubSrc::Embedded(_) => format!("e:{}|{}", self.lang, self.label),
            SubSrc::External(p) => format!("x:{}", p.parent().and_then(|d| d.file_name()).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub audio: Vec<AudioTrack>,
    pub subs: Vec<SubTrack>,
    /// Кодек видео, если его не показать (HEVC, 10-бит H.264 и т.п.).
    pub video_codec: String,
}

pub fn is_mkv(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("mkv") || e.eq_ignore_ascii_case("webm"))
}

fn lang_name(l: &str) -> &str {
    match l {
        "rus" | "ru" => "Русский",
        "jpn" | "ja" => "Японский",
        "eng" | "en" => "Английский",
        "ukr" | "uk" => "Украинский",
        "chi" | "zho" | "zh" => "Китайский",
        "kor" | "ko" => "Корейский",
        "ger" | "deu" | "de" => "Немецкий",
        "fre" | "fra" | "fr" => "Французский",
        "spa" | "es" => "Испанский",
        "und" | "" => "",
        other => other,
    }
}

fn label(name: Option<&str>, lang: &str, extra: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let ln = lang_name(lang);
    if !ln.is_empty() { parts.push(ln.to_string()); }
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
        if !parts.iter().any(|p| p.eq_ignore_ascii_case(n)) { parts.push(n.to_string()); }
    }
    if !extra.is_empty() { parts.push(extra.to_string()); }
    if parts.is_empty() { "Без названия".into() } else { parts.join(" · ") }
}

fn audio_supported(codec: &str) -> bool {
    matches!(codec, "A_AAC" | "A_FLAC" | "A_MPEG/L3" | "A_MPEG/L2" | "A_VORBIS" | "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE") || codec.starts_with("A_AAC")
}

fn audio_codec_name(codec: &str) -> &str {
    match codec {
        c if c.starts_with("A_AAC") => "AAC",
        "A_FLAC" => "FLAC",
        "A_OPUS" => "Opus",
        "A_AC3" | "A_EAC3" => "AC3",
        c if c.starts_with("A_DTS") => "DTS",
        "A_VORBIS" => "Vorbis",
        "A_MPEG/L3" => "MP3",
        c => c.trim_start_matches("A_"),
    }
}

/// Разобрать файл (только заголовок — быстро даже для гигабайтного MKV).
pub fn probe(path: &Path) -> MediaInfo {
    let mut info = MediaInfo::default();
    if is_mkv(path) {
        if let Ok(f) = std::fs::File::open(path) {
            if let Ok(mkv) = matroska_demuxer::MatroskaFile::open(std::io::BufReader::new(f)) {
                let scale = mkv.info().timestamp_scale().get() as f64 / 1e9;
                info.duration = mkv.info().duration().map(|d| d * scale);
                for t in mkv.tracks() {
                    let lang = t.language().unwrap_or("").to_string();
                    match t.track_type() {
                        matroska_demuxer::TrackType::Audio => {
                            let supported = audio_supported(t.codec_id());
                            let extra = if supported { String::new() } else { audio_codec_name(t.codec_id()).to_string() };
                            info.audio.push(AudioTrack { label: label(t.name(), &lang, &extra), lang, supported, default: t.flag_default() });
                        }
                        matroska_demuxer::TrackType::Subtitle => {
                            let codec = t.codec_id();
                            if !matches!(codec, "S_TEXT/ASS" | "S_TEXT/SSA" | "S_TEXT/UTF8" | "S_ASS" | "S_SSA") { continue; } // картинки (PGS/VobSub) не умеем
                            info.subs.push(SubTrack { src: SubSrc::Embedded(t.track_number().get()), label: label(t.name(), &lang, ""), lang, default: t.flag_default(), forced: t.flag_forced() });
                        }
                        matroska_demuxer::TrackType::Video => {
                            info.video_codec = t.codec_id().to_string();
                        }
                        _ => {}
                    }
                }
            }
        }
    } else {
        info.duration = lofty::read_from_path(path).ok().map(|t| lofty::prelude::AudioFile::properties(&t).duration().as_secs_f64()).filter(|d| *d > 0.0);
        info.audio.push(AudioTrack { label: "Основная".into(), lang: String::new(), supported: true, default: true });
    }
    for p in crate::anime::external_subs(path) {
        let folder = p.parent().filter(|d| Some(*d) != path.parent()).and_then(|d| d.file_name()).map(|d| d.to_string_lossy().into_owned());
        let stem = path.file_stem().map(|s| s.to_string_lossy().len()).unwrap_or(0);
        let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let tail = name.get(stem..).unwrap_or("").trim_matches(|c: char| c == '.' || c == ' ' || c == '_' || c == '-').to_string();
        let lang = tail.split('.').last().unwrap_or("").to_lowercase();
        let lang = if lang_name(&lang) != lang { lang } else { String::new() };
        let title = [folder.unwrap_or_default(), tail].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        let title = if title.is_empty() { "Внешние".to_string() } else { format!("Внешние · {title}") };
        info.subs.push(SubTrack { src: SubSrc::External(p), label: title, lang, default: false, forced: false });
    }
    info
}

pub fn duration_of(path: &Path) -> Option<f64> {
    if is_mkv(path) { probe(path).duration } else {
        lofty::read_from_path(path).ok().map(|t| lofty::prelude::AudioFile::properties(&t).duration().as_secs_f64()).filter(|d| *d > 0.0)
    }
}

/// Какие субтитры включить по умолчанию: запомненные для тайтла → помеченные «по умолчанию» → русские → первые.
pub fn pick_subs(info: &MediaInfo, pref: Option<&str>) -> Option<usize> {
    match pref {
        Some("off") => return None,
        Some(k) => if let Some(i) = info.subs.iter().position(|s| s.key() == k) { return Some(i) },
        None => {}
    }
    info.subs.iter().position(|s| s.default && !s.forced)
        .or_else(|| info.subs.iter().position(|s| (s.lang == "rus" || s.lang == "ru") && !s.forced))
        .or(if info.subs.is_empty() { None } else { Some(0) })
}

/// Какую звуковую дорожку играть: запомненную → помеченную «по умолчанию» → первую.
/// Формат не важен: то, чего нет в symphonia (Opus, AC3…), сыграет ffmpeg.
pub fn pick_audio(info: &MediaInfo, pref: Option<usize>) -> usize {
    if let Some(p) = pref.filter(|p| *p < info.audio.len()) { return p; }
    info.audio.iter().position(|a| a.default).unwrap_or(0)
}

// ---------- субтитры ----------

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// Надпись сверху (\an8 и т.п.) — надписи на экране, переводы вывесок.
    pub top: bool,
}

/// Текст ASS без тегов оформления: {\b1}, {\pos(..)} → ничего; \N → перенос. Векторные рисунки ({\p1}) — пропускаем.
pub fn clean_ass(text: &str) -> Option<(String, bool)> {
    let mut out = String::with_capacity(text.len());
    let mut top = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut tag = String::new();
                for c in chars.by_ref() { if c == '}' { break; } tag.push(c); }
                if tag.contains("\\p1") || tag.contains("\\p2") || tag.contains("\\p3") || tag.contains("\\p4") { return None; }
                if ["\\an7", "\\an8", "\\an9"].iter().any(|t| tag.contains(t)) { top = true; }
            }
            '\\' => match chars.peek() {
                Some('N') | Some('n') => { chars.next(); out.push('\n'); }
                Some('h') => { chars.next(); out.push(' '); }
                _ => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    let t = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    if t.is_empty() { None } else { Some((t, top)) }
}

/// Строка события ASS из MKV-блока: «ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text».
pub fn mkv_ass_block(data: &[u8], start: f64, dur: f64) -> Option<Cue> {
    let s = String::from_utf8_lossy(data);
    let text = s.splitn(9, ',').nth(8)?;
    let (text, top) = clean_ass(text)?;
    Some(Cue { start, end: start + dur.max(0.5), text, top })
}

/// Блок SRT из MKV (S_TEXT/UTF8): просто текст, иногда с <i>.
pub fn mkv_srt_block(data: &[u8], start: f64, dur: f64) -> Option<Cue> {
    let text = strip_html(&String::from_utf8_lossy(data));
    if text.trim().is_empty() { return None; }
    Some(Cue { start, end: start + dur.max(0.5), text: text.trim().to_string(), top: false })
}

fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c { '<' => in_tag = true, '>' if in_tag => in_tag = false, c if !in_tag => out.push(c), _ => {} }
    }
    out.replace("{\\an8}", "")
}

fn ass_time(s: &str) -> Option<f64> {
    // H:MM:SS.cc
    let mut it = s.trim().split(':');
    let h: f64 = it.next()?.parse().ok()?;
    let m: f64 = it.next()?.parse().ok()?;
    let sec: f64 = it.next()?.replace(',', ".").parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Внешний .ass/.ssa/.srt целиком.
pub fn load_sub_file(path: &Path) -> Vec<Cue> {
    let bytes = std::fs::read(path).unwrap_or_default();
    let text = decode_text(&bytes);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let mut cues = if ext == "srt" { parse_srt(&text) } else { parse_ass(&text) };
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

/// UTF-8 (с BOM или без), UTF-16 или, если не вышло, Windows-1251 — старые русские сабы часто в ней.
fn decode_text(b: &[u8]) -> String {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) { return String::from_utf8_lossy(&b[3..]).into_owned(); }
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let u: Vec<u16> = b[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return String::from_utf16_lossy(&u);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| cp1251(c)).collect(),
    }
}

fn cp1251(c: u8) -> char {
    match c {
        0..=0x7F => c as char,
        0xC0..=0xFF => char::from_u32(0x410 + (c - 0xC0) as u32).unwrap_or('?'),
        0xA8 => 'Ё',
        0xB8 => 'ё',
        0x96 => '–',
        0x97 => '—',
        0x85 => '…',
        0xAB => '«',
        0xBB => '»',
        0x93 | 0x94 => '"',
        0x91 | 0x92 => '\'',
        0xA0 => ' ',
        0xB9 => '№',
        _ => '?',
    }
}

fn parse_ass(text: &str) -> Vec<Cue> {
    let mut fields: Vec<String> = Vec::new();
    let mut in_events = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim_start();
        if l.starts_with('[') { in_events = l.eq_ignore_ascii_case("[events]"); continue; }
        if !in_events { continue; }
        if let Some(f) = l.strip_prefix("Format:") {
            fields = f.split(',').map(|x| x.trim().to_lowercase()).collect();
            continue;
        }
        let Some(rest) = l.strip_prefix("Dialogue:") else { continue };
        if fields.is_empty() { fields = "layer,start,end,style,name,marginl,marginr,marginv,effect,text".split(',').map(String::from).collect(); }
        let parts: Vec<&str> = rest.splitn(fields.len(), ',').collect();
        let get = |n: &str| fields.iter().position(|f| f == n).and_then(|i| parts.get(i)).copied();
        let (Some(s), Some(e), Some(t)) = (get("start").and_then(ass_time), get("end").and_then(ass_time), get("text")) else { continue };
        if let Some((text, top)) = clean_ass(t) { out.push(Cue { start: s, end: e, text, top }); }
    }
    out
}

fn parse_srt(text: &str) -> Vec<Cue> {
    let mut out = Vec::new();
    let norm = text.replace("\r\n", "\n");
    for block in norm.split("\n\n") {
        let mut lines = block.lines().map(str::trim).filter(|l| !l.is_empty());
        let mut first = lines.next();
        if first.is_some_and(|l| l.chars().all(|c| c.is_ascii_digit())) { first = lines.next(); }
        let Some(times) = first else { continue };
        let Some((a, b)) = times.split_once("-->") else { continue };
        let (Some(s), Some(e)) = (ass_time(a), ass_time(b.split_whitespace().next().unwrap_or(""))) else { continue };
        let body = strip_html(&lines.collect::<Vec<_>>().join("\n"));
        if !body.trim().is_empty() { out.push(Cue { start: s, end: e, text: body.trim().to_string(), top: false }); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_cleanup() {
        assert_eq!(clean_ass(r"{\i1}Привет,\Nмир{\i0}"), Some(("Привет,\nмир".into(), false)));
        assert_eq!(clean_ass(r"{\an8\pos(10,10)}Вывеска"), Some(("Вывеска".into(), true)));
        assert_eq!(clean_ass(r"{\p1}m 0 0 l 100 0 100 100{\p0}"), None);
        let cue = mkv_ass_block(b"12,0,Default,,0,0,0,,Hello, world", 1.0, 2.0).unwrap();
        assert_eq!((cue.text.as_str(), cue.end), ("Hello, world", 3.0));
    }

    #[test]
    fn ass_and_srt_files() {
        let ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,Первая, строка\n";
        let c = parse_ass(ass);
        assert_eq!(c, vec![Cue { start: 1.5, end: 3.0, text: "Первая, строка".into(), top: false }]);
        let srt = "1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Hi</i>\r\nthere\r\n\r\n2\r\n00:01:00,000 --> 00:01:01,000\r\nBye\r\n";
        let c = parse_srt(srt);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "Hi\nthere");
        assert_eq!(c[1].start, 60.0);
        assert_eq!(decode_text(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]), "Привет");
    }
}
