//! Один Wavy за раз. Первый запуск слушает 127.0.0.1:PORT. Второй запуск
//! (например, ярлык, пока старый сидит в трее) просит его показать окно и выходит сам.
//! Перезапуск после обновления (--restarted) просит старый процесс закрыться и ждёт, пока порт освободится.
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

const ADDR: &str = "127.0.0.1:37391";

pub enum Cmd { Show, Quit }

/// Что делать с этим запуском.
pub enum Start {
    /// мы — единственный Wavy: команды от следующих запусков придут сюда
    First(TcpListener),
    /// Wavy уже открыт и показал своё окно — этот процесс должен выйти
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
        // порт занят: если там Wavy — он покажет окно; если что-то чужое, запускаемся как обычно
        return if send("show") { Start::AlreadyRunning } else { Start::First(TcpListener::bind("127.0.0.1:0").expect("порт")) };
    }
    // после обновления: старый процесс должен уйти (а с ним и его музыка)
    send("quit");
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(l) = TcpListener::bind(ADDR) { return Start::First(l); }
    }
    wavy_core::log("старый Wavy не закрылся за 10 с после обновления");
    Start::First(TcpListener::bind("127.0.0.1:0").expect("порт"))
}

/// Слушать команды от следующих запусков; `wake` будит интерфейс.
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
        // перезапуск: старый получает quit; порт он не отпускает — новый через 10 с запустится на запасном
        let h = std::thread::spawn(|| start(true));
        assert!(matches!(rx.recv_timeout(Duration::from_secs(2)), Ok(Cmd::Quit)));
        drop(rx);
        let _ = h;
    }
}
