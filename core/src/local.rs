//! Импорт локальных файлов: папки, m3u, теги и обложки.
use crate::{now_ms, sha16, Track};
use lofty::prelude::*;
use std::path::{Path, PathBuf};

pub const AUDIO_EXT: &[&str] = &["mp3", "flac", "wav", "ogg", "oga", "opus", "m4a", "aac", "webm", "weba", "wma", "aiff", "alac"];
pub const PLAYLIST_EXT: &[&str] = &["m3u", "m3u8"];

fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}
pub fn is_audio(p: &Path) -> bool { AUDIO_EXT.contains(&ext(p).as_str()) }
fn is_playlist(p: &Path) -> bool { PLAYLIST_EXT.contains(&ext(p).as_str()) }

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() { walk(&p, out) } else if is_audio(&p) { out.push(p) }
    }
}

fn parse_m3u(file: &Path) -> Vec<PathBuf> {
    let base = file.parent().unwrap_or(Path::new("."));
    std::fs::read_to_string(file).unwrap_or_default().lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            if let Some(rest) = l.strip_prefix("file://") {
                PathBuf::from(url::Url::parse(&format!("file://{rest}")).ok().and_then(|u| u.to_file_path().ok()).unwrap_or_else(|| PathBuf::from(rest)))
            } else {
                base.join(l)
            }
        })
        .filter(|p| is_audio(p) && p.exists())
        .collect()
}

pub fn local_id(p: &Path) -> String {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    format!("loc_{}", sha16(abs.to_string_lossy().as_bytes()))
}

pub fn read_track(file: &Path, covers: &Path) -> Track {
    let abs = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let mut t = Track {
        id: local_id(file),
        kind: "local".into(),
        path: Some(abs.to_string_lossy().into_owned()),
        video_id: None,
        url: None,
        title: file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        artist: "Неизвестный исполнитель".into(),
        album: String::new(),
        duration: 0.0,
        cover: None,
        added_at: now_ms(),
    };
    if let Ok(tagged) = lofty::read_from_path(file) {
        t.duration = tagged.properties().duration().as_secs_f64();
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(v) = tag.title() { if !v.trim().is_empty() { t.title = v.into_owned(); } }
            if let Some(v) = tag.artist() { if !v.trim().is_empty() { t.artist = v.into_owned(); } }
            if let Some(v) = tag.album() { t.album = v.into_owned(); }
            if let Some(pic) = tag.pictures().first() {
                let data = pic.data();
                let ext = if pic.mime_type().is_some_and(|m| m.as_str().contains("png")) { "png" } else { "jpg" };
                let path = covers.join(format!("{}.{ext}", sha16(data)));
                if !path.exists() { let _ = std::fs::write(&path, data); }
                t.cover = Some(path.to_string_lossy().into_owned());
            }
        }
    }
    t
}

pub struct ImportResult {
    pub tracks: Vec<Track>,
    pub playlists: Vec<(String, Vec<String>)>,
}

/// Файлы, папки и m3u → треки. `progress(i, n)` вызывается по ходу.
pub fn import_paths(paths: &[PathBuf], covers: &Path, mut progress: impl FnMut(usize, usize)) -> ImportResult {
    let _ = std::fs::create_dir_all(covers);
    let mut files = Vec::new();
    let mut playlists = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, &mut files);
        } else if is_audio(p) {
            files.push(p.clone());
        } else if is_playlist(p) {
            let list = parse_m3u(p);
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            playlists.push((name, list.iter().map(|f| local_id(f)).collect()));
            files.extend(list);
        }
    }
    let mut seen = std::collections::HashSet::new();
    files.retain(|f| seen.insert(f.clone()));
    let n = files.len();
    let base = now_ms();
    let tracks = files.iter().enumerate().map(|(i, f)| {
        let mut t = read_track(f, covers);
        t.added_at = base + (n - i) as u64; // сохраняем порядок файлов в «новые сверху»
        progress(i + 1, n);
        t
    }).collect();
    ImportResult { tracks, playlists }
}
