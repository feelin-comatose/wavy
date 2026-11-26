//! Discord Rich Presence on a separate thread: "Listening to Wavy" + track, cover, progress.
use crate::{PlayerState, now_ms};
use discord_rich_presence::{activity::*, DiscordIpc, DiscordIpcClient};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DiscordStatus {
    pub connected: bool,
    pub error: Option<String>,
}

enum Msg {
    Configure { enabled: bool, client_id: String, show_buttons: bool },
    Update(PlayerState),
}

pub struct Discord {
    tx: Sender<Msg>,
    pub status: Arc<Mutex<DiscordStatus>>,
}

impl Discord {
    /// `on_status` is called from the background thread when the status changes.
    pub fn start(on_status: impl Fn(&DiscordStatus) + Send + 'static) -> Self {
        let (tx, rx) = channel::<Msg>();
        let status = Arc::new(Mutex::new(DiscordStatus::default()));
        let st = status.clone();
        std::thread::Builder::new().name("discord".into()).spawn(move || {
            let mut client: Option<DiscordIpcClient> = None;
            let (mut enabled, mut client_id, mut buttons) = (false, String::new(), true);
            let mut last: Option<PlayerState> = None;
            let mut next_retry = 0u64;
            let set = |s: DiscordStatus| {
                let prev = st.lock().unwrap().clone();
                if prev.connected != s.connected || prev.error != s.error { crate::log(&format!("discord: подключён={}, ошибка={:?}", s.connected, s.error)); }
                *st.lock().unwrap() = s.clone();
                on_status(&s);
            };
            loop {
                let msg = rx.recv_timeout(Duration::from_secs(5));
                let mut dirty = false;
                match msg {
                    Ok(Msg::Configure { enabled: e, client_id: id, show_buttons }) => {
                        buttons = show_buttons;
                        if id != client_id || e != enabled {
                            if let Some(mut c) = client.take() { let _ = c.close(); }
                            next_retry = 0;
                            if !e || id.is_empty() { set(DiscordStatus::default()); }
                        }
                        enabled = e; client_id = id; dirty = true;
                    }
                    Ok(Msg::Update(s)) => { last = Some(s); dirty = true; }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                if !enabled || client_id.is_empty() { continue; }
                if client.is_none() && now_ms() >= next_retry {
                    let mut c = DiscordIpcClient::new(&client_id);
                    match c.connect() {
                        Ok(()) => { client = Some(c); dirty = true; set(DiscordStatus { connected: true, error: None }); }
                        Err(e) => {
                            next_retry = now_ms() + 15_000;
                            let m = e.to_string();
                            if st.lock().unwrap().error.is_none() { crate::log(&format!("discord: connect() -> {m}")); }
                            let human = if m.to_lowercase().contains("client") || m.contains("4000") { l!("Неверный Application ID", "Invalid Application ID").into() } else { l!("Discord не запущен (нужно десктоп-приложение)", "Discord isn't running (the desktop app is needed)").to_string() };
                            set(DiscordStatus { connected: false, error: Some(human) });
                        }
                    }
                }
                if dirty {
                    if let Some(c) = client.as_mut() {
                        let res = match last.as_ref().filter(|s| s.track.is_some()) {
                            Some(s) => c.set_activity(build(s, buttons)),
                            None => c.clear_activity(),
                        };
                        if res.is_err() {
                            client = None;
                            next_retry = now_ms() + 5_000;
                            set(DiscordStatus { connected: false, error: Some(l!("Соединение с Discord потеряно", "Lost connection to Discord").into()) });
                        }
                    }
                }
            }
        }).expect("discord thread");
        Self { tx, status }
    }

    pub fn configure(&self, enabled: bool, client_id: &str, show_buttons: bool) {
        let _ = self.tx.send(Msg::Configure { enabled, client_id: client_id.trim().into(), show_buttons });
    }

    pub fn update(&self, s: &PlayerState) {
        let _ = self.tx.send(Msg::Update(s.clone()));
    }
}

fn clip(s: &str, n: usize) -> String {
    let mut s: String = s.chars().take(n).collect();
    while s.chars().count() < 2 { s.push(' '); } // Discord requires ≥ 2 characters
    s
}

fn build(s: &PlayerState, buttons: bool) -> Activity<'static> {
    let t = s.track.as_ref().unwrap();
    let mut a = Activity::new()
        .activity_type(ActivityType::Listening)
        .status_display_type(StatusDisplayType::Details)
        .details(clip(&t.title, 128))
        .state(clip(&if s.playing { t.artist.clone() } else { format!("⏸ {}", t.artist) }, 128));
    let img = t.cover.clone().filter(|c| c.starts_with("http")).unwrap_or_else(|| "logo".into());
    a = a.assets(Assets::new().large_image(img).large_text(clip(if t.album.is_empty() { &t.title } else { &t.album }, 128)));
    if s.playing && s.duration > 0.0 {
        let now = now_ms() as i64;
        a = a.timestamps(Timestamps::new().start(now - (s.position * 1000.0) as i64).end(now + ((s.duration - s.position) * 1000.0) as i64));
    }
    if buttons {
        if let Some(u) = t.url.clone().filter(|u| u.starts_with("http")) {
            let label = if u.contains("music.youtube") { l!("Открыть в YouTube Music", "Open in YouTube Music") } else { l!("Открыть трек", "Open track") };
            a = a.buttons(vec![Button::new(label, u)]);
        }
    }
    a
}
