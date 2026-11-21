//! Экспорт песен в MP3: в папку, с тегами (название, исполнитель, альбом) и обложкой.
//! Звук с YouTube берётся из кэша (или качается), перекодирует ffmpeg с LAME. Песни по одной — памяти мало.
use std::path::{Path, PathBuf};
use wavy_core::{ffmpeg::Ffmpeg, youtube::YouTube, Track};

pub struct Report {
    pub dir: PathBuf,
    pub done: usize,
    pub failed: Vec<(String, String)>,
}

/// Имя файла без символов, запрещённых в Windows.
pub fn safe_name(s: &str) -> String {
    let s: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches(['.', ' ']).to_string();
    let s: String = s.chars().take(150).collect();
    if s.is_empty() { "track".into() } else { s }
}

fn file_name(t: &Track, n: usize, total: usize) -> String {
    let base = if t.artist.trim().is_empty() || t.title.to_lowercase().contains(&t.artist.to_lowercase()) { t.title.clone() } else { format!("{} - {}", t.artist, t.title) };
    let base = safe_name(&base);
    if total > 1 { format!("{:0w$}. {base}.mp3", n, w = total.to_string().len().max(2)) } else { format!("{base}.mp3") }
}

/// Исходный звук трека: локальный файл, готовый кэш или скачать.
fn source(t: &Track, yt: &YouTube, cache: &Path) -> Result<PathBuf, String> {
    if let Some(p) = t.path.as_ref().map(PathBuf::from) {
        return if p.exists() { Ok(p) } else { Err("файла больше нет на диске".into()) };
    }
    let key = t.url.clone().ok_or("у трека нет ни файла, ни ссылки")?;
    let streamed = cache.join(format!("{}.s.m4a", wavy_core::sha16(key.as_bytes())));
    if wavy_core::stream::is_complete(&streamed) { return Ok(streamed); }
    yt.download_audio(t, cache)
}

/// Обложка в JPEG (квадрат до 600 px). У YouTube берём кадр 16:9 без чёрных полос и режем по центру.
fn cover(t: &Track) -> Option<Vec<u8>> {
    let get = |u: &str| -> Option<Vec<u8>> {
        let mut r = wavy_core::http().get(u).call().ok()?;
        r.body_mut().with_config().limit(10 * 1024 * 1024).read_to_vec().ok()
    };
    let raw = match (&t.video_id, &t.cover) {
        (Some(v), _) => get(&format!("https://i.ytimg.com/vi/{v}/hq720.jpg")).or_else(|| get(&format!("https://i.ytimg.com/vi/{v}/mqdefault.jpg")))
            .or_else(|| t.cover.as_deref().and_then(get)),
        (None, Some(c)) if c.starts_with("http") => get(c),
        (None, Some(c)) => std::fs::read(c).ok(),
        _ => None,
    }?;
    let img = image::load_from_memory(&raw).ok()?;
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let img = if w != h { img.crop_imm((w - side) / 2, (h - side) / 2, side, side) } else { img };
    let img = if side > 600 { img.resize_exact(600, 600, image::imageops::FilterType::Triangle) } else { img };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88).encode_image(&img.to_rgb8()).ok()?;
    Some(out)
}

fn tag(path: &Path, t: &Track, pic: Option<Vec<u8>>) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::id3::v2::Id3v2Tag;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::*;
    let mut tag = Id3v2Tag::new();
    tag.set_title(t.title.clone());
    if !t.artist.trim().is_empty() { tag.set_artist(t.artist.clone()); }
    if !t.album.trim().is_empty() { tag.set_album(t.album.clone()); }
    if let Some(data) = pic {
        tag.insert_picture(Picture::unchecked(data).pic_type(PictureType::CoverFront).mime_type(MimeType::Jpeg).build());
    }
    // ID3v2.3 — его понимают и Проводник, и старые плееры, и магнитолы
    let mut opts = WriteOptions::default();
    tag.save_to_path(path, opts.use_id3v23(true)).map_err(|e| format!("теги: {e}"))
}

fn convert(ff: &Path, src: &Path, dest: &Path) -> Result<(), String> {
    let out = wavy_core::ffmpeg::cmd(ff)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-threads", "1", "-i"]).arg(src)
        .args(["-map", "0:a:0", "-vn", "-sn", "-map_metadata", "-1", "-c:a", "libmp3lame", "-q:a", "2", "-f", "mp3"]).arg(dest)
        .output().map_err(|e| format!("ffmpeg не запустился: {e}"))?;
    if out.status.success() { return Ok(()); }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg: ошибка").trim().to_string())
}

/// Одна песня → dest. Уже готовый MP3 копируется как есть (без потерь), остальное перекодируется.
fn one(t: &Track, dest: &Path, ff: &Path, yt: &YouTube, cache: &Path) -> Result<(), String> {
    let src = source(t, yt, cache)?;
    let part = dest.with_extension("mp3.part");
    let is_mp3 = src.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    let r = if is_mp3 {
        std::fs::copy(&src, &part).map(|_| ()).map_err(|e| e.to_string())
    } else {
        convert(ff, &src, &part).and_then(|_| tag(&part, t, cover(t)))
    };
    if let Err(e) = r { let _ = std::fs::remove_file(&part); return Err(e); }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

/// Экспорт списка треков в папку `dir`. `status` получает строку прогресса.
pub fn run(tracks: &[Track], dir: &Path, ff: &Ffmpeg, yt: &YouTube, cache: &Path, status: impl Fn(String)) -> Result<Report, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("не удалось создать папку: {e}"))?;
    // MP3-исходники перекодировать не нужно — ffmpeg нужен, только если есть что-то кроме них
    let need_ff = tracks.iter().any(|t| t.path.as_deref().is_none_or(|p| !p.to_lowercase().ends_with(".mp3")));
    let ffp = if need_ff {
        status("Экспорт: готовлю кодировщик MP3…".into());
        ff.ready_with_mp3(|p| status(format!("Экспорт: качаю кодировщик MP3… {p}%")))?
    } else { PathBuf::new() };
    let total = tracks.len();
    let mut rep = Report { dir: dir.to_path_buf(), done: 0, failed: Vec::new() };
    for (i, t) in tracks.iter().enumerate() {
        let dest = dir.join(file_name(t, i + 1, total));
        if dest.exists() { rep.done += 1; continue; }
        status(format!("Экспорт MP3: {} / {total} — {}", i + 1, t.title));
        match one(t, &dest, &ffp, yt, cache) {
            Ok(()) => rep.done += 1,
            Err(e) => { wavy_core::log(&format!("экспорт «{}»: {e}", t.title)); rep.failed.push((t.title.clone(), e)); }
        }
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(title: &str, artist: &str) -> Track {
        serde_json::from_value(serde_json::json!({ "id": "x", "type": "local", "title": title, "artist": artist })).unwrap()
    }

    #[test]
    fn names() {
        assert_eq!(safe_name("AC/DC: Back in Black?"), "AC_DC_ Back in Black_");
        assert_eq!(safe_name("  ...  "), "track");
        let t = tr("Героиня", "9mice");
        assert_eq!(file_name(&t, 3, 25), "03. 9mice - Героиня.mp3");
        assert_eq!(file_name(&t, 1, 1), "9mice - Героиня.mp3");
        let t = tr("LiSA - Gurenge", "LiSA");
        assert_eq!(file_name(&t, 7, 120), "007. LiSA - Gurenge.mp3");
    }
}

#[cfg(test)]
mod e2e {
    /// Настоящий экспорт: WAVY_TEST_SRC=файл.m4a WAVY_TEST_BIN=папка-с-codecs/ffmpeg cargo test -- --ignored export_real
    #[test]
    #[ignore]
    fn export_real() {
        let src = std::env::var("WAVY_TEST_SRC").unwrap();
        let bin = std::path::PathBuf::from(std::env::var("WAVY_TEST_BIN").unwrap());
        let out = std::env::temp_dir().join(format!("wavy-export-{}", std::process::id()));
        let t: wavy_core::Track = serde_json::from_value(serde_json::json!({ "id": "x", "type": "local", "title": "Gurenge", "artist": "LiSA", "album": "LEO-NiNE", "path": src, "cover": std::env::var("WAVY_TEST_COVER").ok() })).unwrap();
        let ff = wavy_core::ffmpeg::Ffmpeg::new(bin);
        let yt = wavy_core::youtube::YouTube::new(std::env::temp_dir());
        let rep = super::run(&[t.clone(), t], &out, &ff, &yt, &std::env::temp_dir(), |s| eprintln!("{s}")).unwrap();
        assert_eq!(rep.done, 2, "{:?}", rep.failed);
        for e in std::fs::read_dir(&out).unwrap() { eprintln!("{}", e.unwrap().path().display()); }
    }
}
