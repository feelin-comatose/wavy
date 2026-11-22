//! Проверка Shazam: cargo run -p wavy-native --example shazam_file -- <файл> [секунда] [дорожка]
#[allow(dead_code)]
#[macro_use]
extern crate wavy_core;
#[path = "../src/fx.rs"]
#[allow(dead_code)]
mod fx;
#[path = "../src/player.rs"]
mod player;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = std::path::PathBuf::from(a.get(1).expect("файл"));
    let start: f64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let track: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let t0 = std::time::Instant::now();
    let s = player::sample_for_shazam(&path, start, wavy_core::shazam::SECONDS as f64, track, std::env::var_os("WAVY_FFMPEG").map(std::path::PathBuf::from).as_deref()).unwrap();
    println!("звук: {:.1} с за {:?}", s.len() as f32 / 16000.0, t0.elapsed());
    println!("отпечаток: {} байт", wavy_core::shazam::signature(&s).len());
    match wavy_core::shazam::recognize(&s) {
        Ok(Some(f)) => println!("НАШЁЛ: {} — {} ({:?})", f.artist, f.title, f.url),
        Ok(None) => println!("не узнал"),
        Err(e) => println!("ошибка: {e}"),
    }
}
