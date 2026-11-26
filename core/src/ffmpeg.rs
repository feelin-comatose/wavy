//! ffmpeg for video and audio the built-in decoder can't handle (HEVC/H.265, 10-bit H.264, VP9, AV1,
//! Opus/AC3/DTS/TrueHD audio). It's a minimal build (scripts/build-ffmpeg.sh, a few MB) from Wavy's releases:
//! downloaded once, and only when first needed. H.264 still plays without it.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Prefix of the "ffmpeg needed" error text (video/audio in a format the built-in decoder can't handle).
pub const NEED: &str = "NEED_FFMPEG:";

/// "ffmpeg needed" error → human-readable text.
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
    /// Looks for an existing ffmpeg: data_dir/bin/codecs/, then PATH. Downloads nothing.
    ///
    /// A separate folder isn't a whim: yt-dlp looks for ffmpeg next to itself (in bin/) and, on finding our stripped-down
    /// build, tries to remux downloads with it — and fails ("Error splitting the argument list").
    pub fn new(bin_dir: PathBuf) -> Self {
        let bin_dir = bin_dir.join("codecs");
        let local = bin_dir.join(exe());
        // builds 16–17 put ffmpeg straight into bin/ — move it
        if let Some(old) = bin_dir.parent().map(|d| d.join(exe())).filter(|p| p.exists()) {
            let _ = std::fs::create_dir_all(&bin_dir);
            if std::fs::rename(&old, &local).is_err() { let _ = std::fs::remove_file(&old); }
        }
        // old copies that couldn't be deleted during an update (Windows won't delete a running exe)
        if let Ok(rd) = std::fs::read_dir(&bin_dir) {
            for e in rd.flatten() { if e.file_name().to_string_lossy().starts_with("ffmpeg.old") { let _ = std::fs::remove_file(e.path()); } }
        }
        let state = if local.exists() { State::Ready(local) }
            else if !cfg!(windows) && cmd(Path::new("ffmpeg")).arg("-version").output().is_ok_and(|o| o.status.success()) { State::Ready("ffmpeg".into()) }
            else { State::Missing };
        Self { bin_dir, state: Arc::new(Mutex::new(state)) }
    }

    pub fn state(&self) -> State { self.state.lock().unwrap().clone() }
    pub fn path(&self) -> Option<PathBuf> { match self.state() { State::Ready(p) => Some(p), _ => None } }

    /// Download in the background if missing (calling again during the download does nothing).
    pub fn install_background(&self, on_change: impl Fn() + Send + 'static) {
        {
            let mut st = self.state.lock().unwrap();
            if matches!(*st, State::Ready(_) | State::Downloading(_)) { return; }
            if !cfg!(windows) { *st = State::Failed(l!("поставь ffmpeg из пакетов системы (apt install ffmpeg)", "install ffmpeg from your system packages (apt install ffmpeg)").into()); return; }
            *st = State::Downloading(0);
        }
        let (state, dir) = (self.state.clone(), self.bin_dir.clone());
        std::thread::Builder::new().name("ffmpeg-install".into()).spawn(move || {
            let r = download(&dir, |p| { *state.lock().unwrap() = State::Downloading(p); on_change(); });
            crate::log(&format!("ffmpeg: {}", match &r { Ok(p) => format!("скачан в {}", p.display()), Err(e) => lf!("ошибка: {e}", "error: {e}") }));
            *state.lock().unwrap() = match r { Ok(p) => State::Ready(p), Err(e) => State::Failed(e) };
            on_change();
        }).ok();
    }

    /// ffmpeg with an MP3 encoder (export). Waits for an ongoing download; if the existing ffmpeg is old and lacks MP3 (builds
    /// before 23), downloads a new one. Blocking: call from a background thread.
    pub fn ready_with_mp3(&self, progress: impl Fn(u8)) -> Result<PathBuf, String> {
        loop {
            {
                let mut st = self.state.lock().unwrap();
                match &*st {
                    State::Downloading(_) => {}
                    State::Ready(p) if has_encoder(p, "libmp3lame") => return Ok(p.clone()),
                    _ if !cfg!(windows) => return Err(l!("нужен ffmpeg с кодировщиком MP3 — поставь его из пакетов системы (apt install ffmpeg)", "ffmpeg with an MP3 encoder is needed — install it from your system packages (apt install ffmpeg)").into()),
                    _ => { *st = State::Downloading(0); break; }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        let r = download(&self.bin_dir, |p| { *self.state.lock().unwrap() = State::Downloading(p); progress(p); });
        crate::log(&format!("ffmpeg (MP3): {}", match &r { Ok(p) => lf!("скачан в {}", "downloaded to {}", p.display()), Err(e) => lf!("ошибка: {e}", "error: {e}") }));
        let had_old = self.bin_dir.join(exe());
        *self.state.lock().unwrap() = match &r { Ok(p) => State::Ready(p.clone()), Err(_) if had_old.exists() => State::Ready(had_old), Err(e) => State::Failed(e.clone()) };
        let p = r?;
        if has_encoder(&p, "libmp3lame") { Ok(p) } else { Err(l!("в декодере с сайта ещё нет MP3 — новая сборка Wavy выйдет через несколько минут, попробуй позже", "the decoder from the site doesn't have MP3 yet — a new Wavy build will be out in a few minutes, try later").into()) }
    }
}

/// Whether this ffmpeg has encoder `name`.
pub fn has_encoder(bin: &Path, name: &str) -> bool {
    cmd(bin).args(["-hide_banner", "-h", &format!("encoder={name}")]).output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("Encoder {name}")))
}

fn download(dir: &Path, progress: impl Fn(u8)) -> Result<PathBuf, String> {
    use std::io::{Read, Write};
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let url = "https://github.com/feelin-comatose/wavy/releases/latest/download/ffmpeg.exe";
    let resp = crate::http().get(url).call().map_err(|e| lf!("не удалось скачать декодер: {e}", "couldn't download the decoder: {e}"))?;
    let total: u64 = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let dest = dir.join(exe());
    let part = dest.with_extension("part");
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut r = resp.into_body().into_reader();
    let (mut buf, mut got, mut last) = (vec![0u8; 256 * 1024], 0u64, 255u8);
    loop {
        let n = r.read(&mut buf).map_err(|e| lf!("загрузка прервалась: {e}", "the download was interrupted: {e}"))?;
        if n == 0 { break; }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        let p = if total > 0 { (got * 100 / total).min(99) as u8 } else { 0 };
        if p != last { last = p; progress(p); }
    }
    drop(f);
    if got < 1_000_000 { let _ = std::fs::remove_file(&part); return Err(l!("скачался не тот файл (слишком маленький)", "the wrong file was downloaded (too small)").into()); }
    if dest.exists() && std::fs::remove_file(&dest).is_err() {
        // the old ffmpeg is running right now (video is playing) — it can't be deleted, but Windows allows renaming it
        let _ = std::fs::rename(&dest, dir.join(format!("ffmpeg.old{}.exe", crate::now_ms())));
    }
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
    Ok(dest)
}

/// Launch without a flashing console window on Windows.
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

/// showinfo line → (frame time, width, height).
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

    /// ffmpeg moves from bin/ to bin/codecs/ — yt-dlp won't find it there.
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
