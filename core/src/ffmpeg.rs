//! ffmpeg для видео и звука, которые не умеет встроенный декодер (HEVC/H.265, 10-битное H.264, VP9, AV1,
//! звук Opus/AC3/DTS/TrueHD). Это минимальная сборка (scripts/build-ffmpeg.sh, несколько МБ) из релизов Wavy:
//! скачивается один раз и только когда впервые понадобилась. H.264 по-прежнему играет без неё.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Начало текста ошибки «нужен ffmpeg» (видео/звук в формате, который встроенный декодер не умеет).
pub const NEED: &str = "NEED_FFMPEG:";

/// Ошибка «нужен ffmpeg» → текст для человека.
pub fn human(err: &str) -> &str { err.strip_prefix(NEED).unwrap_or(err) }

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Missing,
    Downloading(u8),
    Ready(PathBuf),
    Failed(String),
}

#[derive(Clone)]
pub struct Ffmpeg {
    bin_dir: PathBuf,
    state: Arc<Mutex<State>>,
}

fn exe() -> &'static str { if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" } }

impl Ffmpeg {
    /// Ищет готовый ffmpeg: data_dir/bin/codecs/, затем PATH. Ничего не качает.
    ///
    /// Отдельная папка — не прихоть: yt-dlp ищет ffmpeg рядом с собой (в bin/) и, найдя нашу урезанную
    /// сборку, пытается ей перепаковывать скачанное — и падает («Error splitting the argument list»).
    pub fn new(bin_dir: PathBuf) -> Self {
        let bin_dir = bin_dir.join("codecs");
        let local = bin_dir.join(exe());
        // сборки 16–17 клали ffmpeg прямо в bin/ — переносим
        if let Some(old) = bin_dir.parent().map(|d| d.join(exe())).filter(|p| p.exists()) {
            let _ = std::fs::create_dir_all(&bin_dir);
            if std::fs::rename(&old, &local).is_err() { let _ = std::fs::remove_file(&old); }
        }
        let state = if local.exists() { State::Ready(local) }
            else if !cfg!(windows) && cmd(Path::new("ffmpeg")).arg("-version").output().is_ok_and(|o| o.status.success()) { State::Ready("ffmpeg".into()) }
            else { State::Missing };
        Self { bin_dir, state: Arc::new(Mutex::new(state)) }
    }

    pub fn state(&self) -> State { self.state.lock().unwrap().clone() }
    pub fn path(&self) -> Option<PathBuf> { match self.state() { State::Ready(p) => Some(p), _ => None } }

    /// Скачать в фоне, если ещё нет (повторный вызов во время загрузки ничего не делает).
    pub fn install_background(&self, on_change: impl Fn() + Send + 'static) {
        {
            let mut st = self.state.lock().unwrap();
            if matches!(*st, State::Ready(_) | State::Downloading(_)) { return; }
            if !cfg!(windows) { *st = State::Failed("поставь ffmpeg из пакетов системы (apt install ffmpeg)".into()); return; }
            *st = State::Downloading(0);
        }
        let (state, dir) = (self.state.clone(), self.bin_dir.clone());
        std::thread::Builder::new().name("ffmpeg-install".into()).spawn(move || {
            let r = download(&dir, |p| { *state.lock().unwrap() = State::Downloading(p); on_change(); });
            crate::log(&format!("ffmpeg: {}", match &r { Ok(p) => format!("скачан в {}", p.display()), Err(e) => format!("ошибка: {e}") }));
            *state.lock().unwrap() = match r { Ok(p) => State::Ready(p), Err(e) => State::Failed(e) };
            on_change();
        }).ok();
    }
}

fn download(dir: &Path, progress: impl Fn(u8)) -> Result<PathBuf, String> {
    use std::io::{Read, Write};
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let url = "https://github.com/feelin-comatose/wavy/releases/latest/download/ffmpeg.exe";
    let resp = crate::http().get(url).call().map_err(|e| format!("не удалось скачать декодер: {e}"))?;
    let total: u64 = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let dest = dir.join(exe());
    let part = dest.with_extension("part");
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut r = resp.into_body().into_reader();
    let (mut buf, mut got, mut last) = (vec![0u8; 256 * 1024], 0u64, 255u8);
    loop {
        let n = r.read(&mut buf).map_err(|e| format!("загрузка прервалась: {e}"))?;
        if n == 0 { break; }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        let p = if total > 0 { (got * 100 / total).min(99) as u8 } else { 0 };
        if p != last { last = p; progress(p); }
    }
    drop(f);
    if got < 1_000_000 { let _ = std::fs::remove_file(&part); return Err("скачался не тот файл (слишком маленький)".into()); }
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
    Ok(dest)
}

/// Запуск без мигающей консоли в Windows.
pub fn cmd(bin: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Строка showinfo → (время кадра, ширина, высота).
pub fn parse_showinfo(line: &str) -> Option<(f64, usize, usize)> {
    let t = line.split("pts_time:").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let s = line.split(" s:").nth(1)?.split_whitespace().next()?;
    let (w, h) = s.split_once('x')?;
    Some((t, w.parse().ok()?, h.parse().ok()?))
}

#[cfg(test)]
mod tests {
    #[test]
    fn showinfo() {
        let l = "[Parsed_showinfo_2 @ 0x7f] n:   1 pts:   2042 pts_time:2.042   duration:     41 duration_time:0.041   fmt:rgba cl:left sar:1/1 s:640x360 i:P iskey:0 type:P";
        assert_eq!(super::parse_showinfo(l), Some((2.042, 640, 360)));
        assert_eq!(super::parse_showinfo("Input #0, matroska"), None);
    }

    /// ffmpeg из bin/ переезжает в bin/codecs/ — там его не найдёт yt-dlp.
    #[test]
    fn moves_out_of_ytdlp_dir() {
        let bin = std::env::temp_dir().join(format!("wavy-ff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bin);
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(super::exe()), b"x").unwrap();
        let ff = super::Ffmpeg::new(bin.clone());
        assert!(!bin.join(super::exe()).exists());
        assert_eq!(ff.path(), Some(bin.join("codecs").join(super::exe())));
        let _ = std::fs::remove_dir_all(&bin);
    }
}
