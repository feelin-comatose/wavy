//! Wavy's shared core: library, import, YouTube (yt-dlp), Discord RPC, overlay server.
//! Used by the native (egui) app.
#[macro_use]
pub mod i18n;
pub mod addons;
pub mod radio;
pub mod anilist;
pub mod anime;
pub mod animethemes;
pub mod aniskip;
pub mod discord;
pub mod ffmpeg;
pub mod local;
pub mod lastfm;
pub mod listenbrainz;
pub mod lyrics;
pub mod media;
pub mod overlay;
pub mod shazam;
pub mod shikimori;
pub mod spotify;
pub mod stream;
pub mod sync;
pub mod twitch;
pub mod wifisync;
pub mod store;
pub mod subsonic;
pub mod youtube;
pub mod ytaccount;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // local | youtube | stream
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub added_at: u64,
    /// Local video file (mp4/m4v/mov). YouTube tracks always have video (by video_id).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub video: bool,
    /// Anime episode: title id (such tracks live in the "Anime" section, not in music).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<f32>,
}

impl Track {
    pub fn is_local(&self) -> bool {
        self.kind == "local"
    }
    /// Whether video can be shown for this track.
    pub fn has_video(&self) -> bool {
        self.video || self.video_id.is_some()
    }
    /// Small cover (for lists) — saves memory.
    pub fn thumb(&self) -> Option<String> {
        match (&self.video_id, &self.cover) {
            (Some(v), _) => Some(format!("https://i.ytimg.com/vi/{v}/default.jpg")),
            (_, c) => c.clone(),
        }
    }
}

/// Player state sent to Discord and the overlay.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerState {
    pub track: Option<Track>,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
}

/// Shared HTTP client. Certificates come from the system (not a built-in list),
/// so everything works behind antivirus software/proxies that inspect HTTPS.
pub fn http() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        use ureq::tls::{RootCerts, TlsConfig};
        ureq::Agent::config_builder()
            .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build()
            .into()
    })
}

/// Diagnostic log: data_dir/wavy.log (over 1 MB the old one moves to wavy.old.log).
pub fn log(msg: &str) {
    use std::io::Write;
    let path = data_dir().join("wavy.log");
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, data_dir().join("wavy.old.log"));
    }
    let secs = now_ms() / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{} {h:02}:{m:02}:{s:02} UTC] {msg}", secs / 86400);
    }
}

/// Where to save frame screenshots: "Pictures/Wavy".
pub fn pictures_dir() -> PathBuf {
    let d = dirs::picture_dir().unwrap_or_else(data_dir).join("Wavy");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn data_dir() -> PathBuf {
    let d = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("wavy");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn sha16(s: &[u8]) -> String {
    sha1_smol::Sha1::from(s).digest().to_string()[..16].to_string()
}

pub fn fmt_time(sec: f64) -> String {
    if !sec.is_finite() || sec <= 0.0 {
        return "0:00".into();
    }
    let s = sec as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}
