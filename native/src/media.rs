//! System media controls: keyboard media keys, the Windows media flyout
//! (next to the volume, with the cover), MPRIS on Linux, Now Playing on macOS.
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};
use std::sync::mpsc::Sender;
use std::time::Duration;
use wavy_core::Track;

pub enum MediaCmd { Toggle, Play, Pause, Next, Prev, SeekTo(f64), SeekBy(f64) }

pub struct Media {
    controls: Option<MediaControls>,
    last_track: Option<String>,
    last_playing: Option<bool>,
}

impl Media {
    pub fn new(cc: &eframe::CreationContext, tx: Sender<MediaCmd>) -> Self {
        #[cfg(windows)]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match cc.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get() as *mut std::ffi::c_void),
                _ => None,
            }
        };
        #[cfg(not(windows))]
        let hwnd = None;
        let ctx = cc.egui_ctx.clone();
        // on Linux without a D-Bus session (e.g. in a container) media controls are simply off
        #[cfg(all(unix, not(target_os = "macos")))]
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            return Self { controls: None, last_track: None, last_playing: None };
        }
        let controls = MediaControls::new(PlatformConfig { display_name: "Wavy", dbus_name: "wavy_player", hwnd }).ok().and_then(|mut c| {
            c.attach(move |e| {
                let cmd = match e {
                    MediaControlEvent::Toggle => MediaCmd::Toggle,
                    MediaControlEvent::Play => MediaCmd::Play,
                    MediaControlEvent::Pause | MediaControlEvent::Stop => MediaCmd::Pause,
                    MediaControlEvent::Next => MediaCmd::Next,
                    MediaControlEvent::Previous => MediaCmd::Prev,
                    MediaControlEvent::SetPosition(MediaPosition(d)) => MediaCmd::SeekTo(d.as_secs_f64()),
                    MediaControlEvent::SeekBy(dir, d) => MediaCmd::SeekBy(if dir == SeekDirection::Forward { d.as_secs_f64() } else { -d.as_secs_f64() }),
                    MediaControlEvent::Seek(dir) => MediaCmd::SeekBy(if dir == SeekDirection::Forward { 10.0 } else { -10.0 }),
                    _ => return,
                };
                let _ = tx.send(cmd);
                ctx.request_repaint();
            }).ok().map(|_| c)
        });
        Self { controls, last_track: None, last_playing: None }
    }

    pub fn update(&mut self, track: Option<&Track>, playing: bool, position: f64, duration: f64) {
        let Some(c) = self.controls.as_mut() else { return };
        let id = track.map(|t| t.id.clone());
        if id != self.last_track {
            self.last_track = id;
            if let Some(t) = track {
                let cover = t.cover.as_ref().map(|c| {
                    if c.starts_with("http") { c.clone() } else { url::Url::from_file_path(c).map(|u| u.to_string()).unwrap_or_default() }
                });
                let _ = c.set_metadata(MediaMetadata {
                    title: Some(&t.title), artist: Some(&t.artist), album: Some(&t.album),
                    cover_url: cover.as_deref(), duration: (duration > 0.0).then(|| Duration::from_secs_f64(duration)),
                });
            }
        }
        self.last_playing = Some(playing);
        let progress = Some(MediaPosition(Duration::from_secs_f64(position.max(0.0))));
        let _ = c.set_playback(match (track, playing) {
            (None, _) => MediaPlayback::Stopped,
            (_, true) => MediaPlayback::Playing { progress },
            (_, false) => MediaPlayback::Paused { progress },
        });
    }
}
