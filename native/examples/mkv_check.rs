//! Check an MKV without a window: tracks, H.264 frames, subtitles, seeking.
//! cargo run -p wavy-native --example mkv_check -- <file.mkv> [shazam]
#[allow(dead_code)]
#[macro_use]
extern crate wavy_core;
#[path = "../src/video.rs"]
mod video;
#[allow(dead_code)]
#[path = "../src/fx.rs"]
#[allow(dead_code)]
mod fx;
#[path = "../src/player.rs"]
mod player;

use std::time::{Duration, Instant};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = std::path::PathBuf::from(&a[1]);
    // WAVY_FFMPEG=<path to ffmpeg> — also check the formats that go through it
    let ff = std::env::var_os("WAVY_FFMPEG").map(std::path::PathBuf::from);
    let info = wavy_core::media::probe(&path);
    println!("длительность {:?}, видео «{}»", info.duration, info.video_codec);
    for (i, t) in info.audio.iter().enumerate() { println!("  звук {i}: {} (поддержка {})", t.label, t.supported); }
    for (i, s) in info.subs.iter().enumerate() { println!("  сабы {i}: {} [{}] {:?}", s.label, s.key(), s.src); }
    let pick = wavy_core::media::pick_subs(&info, None);
    println!("по умолчанию: звук {}, сабы {:?}", wavy_core::media::pick_audio(&info, None), pick);

    let ctx = eframe::egui::Context::default();
    let sub = pick.map(|i| info.subs[i].src.clone());
    let mut v = video::Video::open(path.clone(), 0.0, &ctx, sub, ff.clone());
    let t0 = Instant::now();
    let (mut shown, mut last) = (0, None::<eframe::egui::TextureId>);
    // "play" 12 seconds at x4 speed
    let mut clock = 0.0;
    while clock < 12.0 {
        v.update(clock, &ctx);
        if let Some(e) = &v.error { println!("ОШИБКА видео: {e}"); break; }
        if let Some(t) = &v.texture { if Some(t.id()) != last || shown == 0 { shown += 1; last = Some(t.id()); } }
        std::thread::sleep(Duration::from_millis(10));
        clock += 0.04;
    }
    if v.error.is_none() {
        println!("кадров показано (обновлений текстуры): {shown}, размер {:?}, за {:?}", v.texture.as_ref().map(|t| t.size()), t0.elapsed());
        for t in [0.5, 2.0, 4.0, 11.0] { println!("  сабы @{t}: {:?}", v.subs_at(t).iter().map(|c| (c.text.clone(), c.top)).collect::<Vec<_>>()); }
        // seek forward and back
        for target in [20.0, 3.0] {
            v.seek(target);
            let t1 = Instant::now();
            loop {
                v.update(target, &ctx);
                if v.shown().is_some_and(|p| (p - target).abs() < 0.2) { break; }
                if t1.elapsed() > Duration::from_secs(5) { println!("перемотка {target}: кадра нет!"); break; }
                std::thread::sleep(Duration::from_millis(10));
            }
            println!("перемотка на {target}: кадр {:?}, за {:?}; сабы {:?}", v.shown(), t1.elapsed(), v.subs_at(target + 0.5).iter().map(|c| c.text.clone()).collect::<Vec<_>>());
        }
    }
    // as when moving to the next episode: the video opened at the previous episode's position (27.8) while audio starts from zero
    let mut v2 = video::Video::open(path.clone(), 27.8, &ctx, None, ff.clone());
    let mut clock = 0.0;
    let t2 = Instant::now();
    while t2.elapsed() < Duration::from_secs(4) {
        v2.update(clock, &ctx);
        std::thread::sleep(Duration::from_millis(20));
        clock += 0.02;
    }
    println!("после открытия с 27.8 и часов {clock:.2}: показан кадр {:?}", v2.shown());
    let mut au = player::AudioOfVideo::open(&path, 1.min(info.audio.len().saturating_sub(1)));
    match &mut au { Ok(s) => { let n: usize = s.by_ref().take(48000).count(); println!("звук дорожка 1 (FLAC): {n} сэмплов"); } Err(e) => println!("звук 1: {e}") }
    if a.get(2).map(|s| s == "shazam").unwrap_or(false) {
        let s = player::sample_for_shazam(&path, 8.0, 12.0, 0, ff.as_deref()).unwrap();
        println!("Shazam: {:?}", wavy_core::shazam::recognize(&s));
    }
}
