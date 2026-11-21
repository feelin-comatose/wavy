//! Проверка потока через yt-dlp: cargo run -p wavy-core --example stream_check -- <ссылка>
fn main() {
    let url = std::env::args().nth(1).unwrap();
    let t = wavy_core::Track { id: "x".into(), kind: "stream".into(), path: None, video_id: None, url: Some(url), title: "t".into(), artist: String::new(),
        album: String::new(), duration: 0.0, cover: None, added_at: 0, video: false, anime: None, episode: None };
    let yt = wavy_core::youtube::YouTube::new(wavy_core::data_dir().join("bin"));
    let path = std::env::temp_dir().join("wavy-stream-check.m4a");
    let t0 = std::time::Instant::now();
    let dl = yt.stream_audio(&t, &path).unwrap();
    let first = dl.wait_for(200 * 1024, std::time::Duration::from_secs(40));
    println!("первые 200 КБ: {first} через {:.1} с", t0.elapsed().as_secs_f32());
    dl.wait_for(u64::MAX, std::time::Duration::from_secs(60));
    println!("готово: {} ошибка {:?}, размер {:?}, файл {} байт", dl.done(), dl.error(), dl.total(), std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0));
}
