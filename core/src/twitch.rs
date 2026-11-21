//! Заказ треков из чата Twitch: «!sr ссылка или название». Читаем чат анонимно (как гость,
//! без входа и токенов) — писать в чат так нельзя, зато ничего не надо настраивать.
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub user: String,
    pub query: String,
}

/// Разобрать строку IRC: заказ, если это сообщение «!sr …».
pub fn parse(line: &str, cmd: &str) -> Option<Request> {
    // :ник!ник@ник.tmi.twitch.tv PRIVMSG #канал :текст
    let rest = line.strip_prefix(':')?;
    let (prefix, rest) = rest.split_once(' ')?;
    let user = prefix.split('!').next()?.to_string();
    let rest = rest.strip_prefix("PRIVMSG ")?;
    let (_, text) = rest.split_once(" :")?;
    let text = text.trim();
    let q = text.strip_prefix(cmd)?;
    if !q.starts_with(' ') { return None; }
    let q = q.trim();
    (!q.is_empty()).then(|| Request { user, query: q.chars().take(200).collect() })
}

/// Слушать чат канала, пока `stop` не поднят. Переподключается сам.
pub fn listen(channel: &str, cmd: &str, stop: Arc<AtomicBool>, on: impl Fn(Request) + Send + 'static) {
    let channel = channel.trim().trim_start_matches('#').trim_start_matches("https://www.twitch.tv/").trim_start_matches("twitch.tv/").to_lowercase();
    let cmd = cmd.to_string();
    std::thread::Builder::new().name("twitch".into()).spawn(move || {
        let mut wait = 2;
        while !stop.load(Ordering::Relaxed) {
            match session(&channel, &cmd, &stop, &on) {
                Ok(()) => wait = 2,
                Err(e) => { crate::log(&format!("twitch: {e} — переподключаюсь через {wait} с")); }
            }
            for _ in 0..wait * 10 { if stop.load(Ordering::Relaxed) { return; } std::thread::sleep(Duration::from_millis(100)); }
            wait = (wait * 2).min(60);
        }
    }).ok();
}

fn session(channel: &str, cmd: &str, stop: &AtomicBool, on: &impl Fn(Request)) -> Result<(), String> {
    // WAVY_TWITCH_IRC — подменить сервер (для проверки без Twitch)
    let host = std::env::var("WAVY_TWITCH_IRC").unwrap_or_else(|_| "irc.chat.twitch.tv:6667".into());
    let mut s = TcpStream::connect(&host).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(1))).ok();
    let nick = format!("justinfan{}", 10000 + crate::now_ms() % 80000);
    write!(s, "PASS SCHMOOPIIE\r\nNICK {nick}\r\nJOIN #{channel}\r\n").map_err(|e| e.to_string())?;
    crate::log(&format!("twitch: слушаю чат #{channel}"));
    let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    loop {
        if stop.load(Ordering::Relaxed) { return Ok(()); }
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => return Err("сервер закрыл соединение".into()),
            Ok(_) => {
                let l = line.trim_end();
                if l.starts_with("PING") { let _ = write!(s, "PONG :tmi.twitch.tv\r\n"); continue; }
                if let Some(req) = parse(l, cmd) { on(req); }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sr() {
        let l = ":vasya!vasya@vasya.tmi.twitch.tv PRIVMSG #avice :!sr LiSA Gurenge";
        assert_eq!(parse(l, "!sr"), Some(Request { user: "vasya".into(), query: "LiSA Gurenge".into() }));
        assert_eq!(parse(":a!a@a PRIVMSG #c :!srx q", "!sr"), None);
        assert_eq!(parse(":a!a@a PRIVMSG #c :привет", "!sr"), None);
        assert_eq!(parse(":a!a@a PRIVMSG #c :!sr   ", "!sr"), None);
        assert_eq!(parse("PING :tmi.twitch.tv", "!sr"), None);
    }
}
