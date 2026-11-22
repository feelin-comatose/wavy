//! Чат Twitch: заказы «!sr», голосование «!skip», «!song» и «!queue», награды за баллы канала.
//! Без входа Wavy читает чат как гость (ничего настраивать не надо). Чтобы отвечать в чат
//! («Сейчас играет …»), нужен вход: Twitch-приложение стримера (dev.twitch.tv, тип «Public»)
//! и код на twitch.tv/activate. Дальше токен обновляется сам.
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Сообщение из чата.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Msg {
    pub user: String,
    pub text: String,
    /// модератор или сам стример
    pub is_mod: bool,
    /// сообщение пришло вместе с наградой за баллы канала (награда «с текстом»)
    pub reward: Option<String>,
}

impl Msg {
    /// «!sr запрос» → Some("запрос"); `cmd` — с восклицательным знаком.
    pub fn command(&self, cmd: &str) -> Option<String> {
        let rest = self.text.trim().strip_prefix(cmd)?;
        if !rest.is_empty() && !rest.starts_with(' ') { return None; }
        Some(rest.trim().chars().take(200).collect())
    }
}

/// Разобрать строку IRC (с тегами или без): сообщение в чат канала.
pub fn parse(line: &str) -> Option<Msg> {
    let (tags, rest) = match line.strip_prefix('@') {
        Some(t) => { let (t, r) = t.split_once(' ')?; (t, r) }
        None => ("", line),
    };
    // :ник!ник@ник.tmi.twitch.tv PRIVMSG #канал :текст
    let rest = rest.strip_prefix(':')?;
    let (prefix, rest) = rest.split_once(' ')?;
    let user = prefix.split('!').next()?.to_string();
    let rest = rest.strip_prefix("PRIVMSG ")?;
    let (_, text) = rest.split_once(" :")?;
    let mut m = Msg { user, text: text.trim().to_string(), ..Default::default() };
    for kv in tags.split(';') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "mod" if v == "1" => m.is_mod = true,
            "badges" if v.split(',').any(|b| b.starts_with("broadcaster/") || b.starts_with("moderator/")) => m.is_mod = true,
            "custom-reward-id" if !v.is_empty() => m.reward = Some(v.to_string()),
            _ => {}
        }
    }
    Some(m)
}

/// Аккаунт Twitch, от имени которого Wavy пишет в чат.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub client_id: String,
    pub access: String,
    pub refresh: String,
    pub login: String,
}

impl Account {
    pub fn logged_in(&self) -> bool { !self.access.is_empty() && !self.login.is_empty() }
}

/// Отправка сообщений в чат (если вошли).
#[derive(Clone)]
pub struct Chat { out: Option<Sender<String>> }

impl Chat {
    pub fn can_say(&self) -> bool { self.out.is_some() }
    pub fn say(&self, text: &str) {
        if let Some(o) = &self.out { let _ = o.send(text.replace(['\r', '\n'], " ").chars().take(450).collect()); }
    }
}

pub fn clean_channel(ch: &str) -> String {
    ch.trim().trim_start_matches("https://").trim_start_matches("www.").trim_start_matches("twitch.tv/").trim_start_matches('#').trim_end_matches('/').to_lowercase()
}

/// Слушать чат канала, пока `stop` не поднят. Переподключается сам.
/// `account` — Some, если вошли: тогда можно писать в чат. Новый токен (после обновления) приходит в `on_token`.
pub fn listen(channel: &str, account: Option<Account>, stop: Arc<AtomicBool>, on: impl Fn(Msg) + Send + 'static, on_token: impl Fn(Account) + Send + 'static) -> Chat {
    let channel = clean_channel(channel);
    let (out, rx) = channel_pair(account.is_some());
    let rx = Arc::new(Mutex::new(rx));
    std::thread::Builder::new().name("twitch".into()).spawn(move || {
        let mut acc = account;
        let mut wait = 2;
        while !stop.load(Ordering::Relaxed) {
            match session(&channel, acc.as_ref(), &stop, &on, &rx) {
                Ok(()) => wait = 2,
                Err(e) => {
                    crate::log(&format!("twitch: {e} — переподключаюсь через {wait} с"));
                    // токен протух — обновляем и пробуем снова
                    if e.contains("authentication failed") || e.contains("Login unsuccessful") {
                        if let Some(a) = acc.as_mut() {
                            match refresh(a) { Ok(()) => { on_token(a.clone()); wait = 1; } Err(e) => { crate::log(&format!("twitch: не обновил токен: {e}")); acc = None; } }
                        }
                    }
                }
            }
            for _ in 0..wait * 10 { if stop.load(Ordering::Relaxed) { return; } std::thread::sleep(Duration::from_millis(100)); }
            wait = (wait * 2).min(60);
        }
    }).ok();
    Chat { out }
}

fn channel_pair(can_say: bool) -> (Option<Sender<String>>, Receiver<String>) {
    let (tx, rx) = channel();
    (can_say.then_some(tx), rx)
}

fn session(channel: &str, acc: Option<&Account>, stop: &AtomicBool, on: &impl Fn(Msg), out: &Mutex<Receiver<String>>) -> Result<(), String> {
    // WAVY_TWITCH_IRC — подменить сервер (для проверки без Twitch)
    let host = std::env::var("WAVY_TWITCH_IRC").unwrap_or_else(|_| "irc.chat.twitch.tv:6667".into());
    let mut s = TcpStream::connect(&host).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_millis(300))).ok();
    match acc {
        Some(a) => write!(s, "PASS oauth:{}\r\nNICK {}\r\n", a.access, a.login),
        None => write!(s, "PASS SCHMOOPIIE\r\nNICK justinfan{}\r\n", 10000 + crate::now_ms() % 80000),
    }.map_err(|e| e.to_string())?;
    write!(s, "CAP REQ :twitch.tv/tags twitch.tv/commands\r\nJOIN #{channel}\r\n").map_err(|e| e.to_string())?;
    crate::log(&format!("twitch: слушаю чат #{channel}{}", acc.map(|a| lf!(" (пишу как {})", " (writing as {})", a.login)).unwrap_or_default()));
    let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    loop {
        if stop.load(Ordering::Relaxed) { return Ok(()); }
        while let Ok(text) = out.lock().unwrap().try_recv() {
            write!(s, "PRIVMSG #{channel} :{text}\r\n").map_err(|e| e.to_string())?;
        }
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => return Err(l!("сервер закрыл соединение", "the server closed the connection").into()),
            Ok(_) => {
                let l = line.trim_end();
                if l.starts_with("PING") { let _ = write!(s, "PONG :tmi.twitch.tv\r\n"); continue; }
                if l.contains(" NOTICE ") && (l.contains("authentication failed") || l.contains("Login unsuccessful")) { return Err("authentication failed".into()); }
                if let Some(m) = parse(l) { on(m); }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
}

// ---------- вход (Device Code Flow: код вводится на twitch.tv/activate) ----------

const SCOPES: &str = "chat:read chat:edit";

#[derive(Clone, Debug)]
pub struct Device { pub device_code: String, pub user_code: String, pub uri: String, pub interval: u64 }

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<(u16, Value), String> {
    let body: String = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
    let mut r = crate::http().post(url).config().http_status_as_error(false).build()
        .header("Content-Type", "application/x-www-form-urlencoded").send(body).map_err(|e| lf!("Twitch недоступен: {e}", "Twitch is unavailable: {e}"))?;
    let code = r.status().as_u16();
    let v = r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    Ok((code, v))
}

/// Начать вход: код для twitch.tv/activate.
pub fn device_start(client_id: &str) -> Result<Device, String> {
    let (code, v) = post_form("https://id.twitch.tv/oauth2/device", &[("client_id", client_id.trim()), ("scopes", SCOPES)])?;
    if code != 200 { return Err(lf!("Twitch не начал вход: {}", "Twitch didn't start the sign-in: {}", v["message"].as_str().unwrap_or(l!("проверь Client ID (тип приложения — Public)", "check the Client ID (the application type must be Public)")))); }
    Ok(Device {
        device_code: v["device_code"].as_str().unwrap_or_default().to_string(),
        user_code: v["user_code"].as_str().unwrap_or_default().to_string(),
        uri: v["verification_uri"].as_str().unwrap_or("https://www.twitch.tv/activate").to_string(),
        interval: v["interval"].as_u64().unwrap_or(5),
    })
}

/// Проверить, ввёл ли человек код. Ok(None) — ещё ждём.
pub fn device_poll(client_id: &str, d: &Device) -> Result<Option<Account>, String> {
    let (code, v) = post_form("https://id.twitch.tv/oauth2/token", &[
        ("client_id", client_id.trim()), ("scopes", SCOPES), ("device_code", &d.device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ])?;
    if code != 200 {
        let msg = v["message"].as_str().unwrap_or("");
        return if msg == "authorization_pending" || msg == "slow_down" { Ok(None) } else { Err(lf!("Twitch: вход не удался ({msg})", "Twitch: sign-in failed ({msg})")) };
    }
    let mut acc = Account { client_id: client_id.trim().to_string(), access: v["access_token"].as_str().unwrap_or_default().to_string(), refresh: v["refresh_token"].as_str().unwrap_or_default().to_string(), login: String::new() };
    acc.login = whoami(&acc.access)?;
    Ok(Some(acc))
}

fn whoami(token: &str) -> Result<String, String> {
    let mut r = crate::http().get("https://id.twitch.tv/oauth2/validate").header("Authorization", &format!("OAuth {token}")).call().map_err(|e| format!("Twitch: {e}"))?;
    let v: Value = r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v["login"].as_str().map(str::to_string).ok_or_else(|| l!("Twitch не сказал, кто вошёл", "Twitch didn't say who signed in").into())
}

/// Новый токен по refresh (у публичных приложений — без секрета).
pub fn refresh(acc: &mut Account) -> Result<(), String> {
    let (code, v) = post_form("https://id.twitch.tv/oauth2/token", &[("client_id", acc.client_id.as_str()), ("grant_type", "refresh_token"), ("refresh_token", acc.refresh.as_str())])?;
    if code != 200 { return Err(v["message"].as_str().unwrap_or(l!("войди заново", "sign in again")).to_string()); }
    acc.access = v["access_token"].as_str().unwrap_or_default().to_string();
    if let Some(r) = v["refresh_token"].as_str() { acc.refresh = r.to_string(); }
    Ok(())
}

/// Голосование за пропуск: сколько разных людей написали !skip за этот трек.
#[derive(Default, Debug)]
pub struct SkipVote { track: String, voters: Vec<String> }

impl SkipVote {
    /// Голос от `user` за трек `track`. true — набралось `need` голосов (и счёт сброшен).
    pub fn vote(&mut self, track: &str, user: &str, need: u32) -> bool {
        if self.track != track { self.track = track.to_string(); self.voters.clear(); }
        if !self.voters.iter().any(|v| v == user) { self.voters.push(user.to_string()); }
        if need > 0 && self.voters.len() as u32 >= need { self.voters.clear(); return true; }
        false
    }
    pub fn count(&self, track: &str) -> u32 { if self.track == track { self.voters.len() as u32 } else { 0 } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_messages() {
        let m = parse(":vasya!vasya@vasya.tmi.twitch.tv PRIVMSG #avice :!sr LiSA Gurenge").unwrap();
        assert_eq!(m.user, "vasya");
        assert_eq!(m.command("!sr").as_deref(), Some("LiSA Gurenge"));
        assert_eq!(m.command("!skip"), None);
        assert!(!m.is_mod && m.reward.is_none());
        assert_eq!(parse(":a!a@a PRIVMSG #c :!srx q").unwrap().command("!sr"), None);
        assert_eq!(parse(":a!a@a PRIVMSG #c :!skip").unwrap().command("!skip").as_deref(), Some(""));
        assert_eq!(parse("PING :tmi.twitch.tv"), None);
        let t = parse("@badge-info=;badges=moderator/1,premium/1;custom-reward-id=abc-123;display-name=Petya;mod=1 :petya!petya@petya.tmi.twitch.tv PRIVMSG #avice :Unravel TK").unwrap();
        assert!(t.is_mod);
        assert_eq!(t.reward.as_deref(), Some("abc-123"));
        assert_eq!(t.text, "Unravel TK");
        let b = parse("@badges=broadcaster/1;mod=0 :avice!avice@avice.tmi.twitch.tv PRIVMSG #avice :!skip").unwrap();
        assert!(b.is_mod);
    }

    #[test]
    fn skip_votes() {
        let mut v = SkipVote::default();
        assert!(!v.vote("t1", "a", 3));
        assert!(!v.vote("t1", "a", 3));
        assert!(!v.vote("t1", "b", 3));
        assert_eq!(v.count("t1"), 2);
        assert!(v.vote("t1", "c", 3));
        assert_eq!(v.count("t1"), 0);
        // новый трек — счёт заново
        v.vote("t1", "a", 3);
        assert!(!v.vote("t2", "b", 3));
        assert_eq!(v.count("t2"), 1);
    }

    #[test]
    fn channel_names() {
        assert_eq!(clean_channel("https://www.twitch.tv/Avice/"), "avice");
        assert_eq!(clean_channel("#avice"), "avice");
    }
}
