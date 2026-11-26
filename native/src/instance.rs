//! One Wavy at a time. The first launch listens on 127.0.0.1:PORT. A second launch
//! (e.g. a shortcut while the old one sits in the tray) asks it to show its window and exits.
//! A restart after an update (--restarted) asks the old process to quit and waits for the port to free up.
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

const ADDR: &str = "127.0.0.1:37391";

pub enum Cmd { Show, Quit }

/// What to do with this launch.
pub enum Start {
    /// we're the only Wavy: commands from later launches will come here
    First(TcpListener),
    /// Wavy is already open and has shown its window — this process should exit
    AlreadyRunning,
}

fn send(cmd: &str) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&ADDR.parse().unwrap(), Duration::from_millis(500)) else { return false };
    let _ = s.set_write_timeout(Some(Duration::from_millis(500)));
    s.write_all(format!("wavy {cmd}\n").as_bytes()).is_ok()
}

pub fn start(restarted: bool) -> Start {
    if let Ok(l) = TcpListener::bind(ADDR) { return Start::First(l); }
    if !restarted {
        // port is taken: if it's Wavy it will show its window; if it's something else, start as usual
        return if send("show") { Start::AlreadyRunning } else { Start::First(TcpListener::bind("127.0.0.1:0").expect("порт")) };
    }
    // after an update: the old process must go (and its music with it)
    send("quit");
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(l) = TcpListener::bind(ADDR) { return Start::First(l); }
    }
    wavy_core::log("старый Wavy не закрылся за 10 с после обновления");
    Start::First(TcpListener::bind("127.0.0.1:0").expect("порт"))
}

/// Listen for commands from later launches; `wake` wakes the UI.
pub fn listen(l: TcpListener, wake: impl Fn() + Send + 'static) -> Receiver<Cmd> {
    let (tx, rx) = channel();
    std::thread::Builder::new().name("instance".into()).spawn(move || {
        for s in l.incoming().flatten() {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(s).read_line(&mut line).is_err() { continue; }
            let cmd = match line.trim() { "wavy show" => Cmd::Show, "wavy quit" => Cmd::Quit, _ => continue };
            if tx.send(cmd).is_err() { break; }
            wake();
        }
    }).ok();
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_start_asks_first() {
        let Start::First(l) = start(false) else { panic!("порт уже занят") };
        let rx = listen(l, || {});
        assert!(matches!(start(false), Start::AlreadyRunning));
        assert!(matches!(rx.recv_timeout(Duration::from_secs(2)), Ok(Cmd::Show)));
        // restart: the old one gets quit; it doesn't release the port — the new one starts on the fallback after 10 s
        let h = std::thread::spawn(|| start(true));
        assert!(matches!(rx.recv_timeout(Duration::from_secs(2)), Ok(Cmd::Quit)));
        drop(rx);
        let _ = h;
    }
}
