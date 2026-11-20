//! Проверка подбора замены для пропавшего с YouTube видео:
//! cargo run -p wavy-core --example replace_check -- <videoId> <название> [исполнитель] [длительность]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let t = wavy_core::Track {
        id: format!("yt_{}", a[1]), kind: "youtube".into(), path: None, video_id: Some(a[1].clone()),
        url: Some(format!("https://music.youtube.com/watch?v={}", a[1])), title: a[2].clone(),
        artist: a.get(3).cloned().unwrap_or_default(), album: String::new(), duration: a.get(4).and_then(|d| d.parse().ok()).unwrap_or(0.0),
        cover: None, added_at: 1, video: false, anime: None, episode: None,
    };
    let yt = wavy_core::youtube::YouTube::new(wavy_core::data_dir().join("bin"));
    match wavy_core::spotify::find_replacement(&yt, &t) {
        Ok(Some(n)) => println!("замена: {} — {} [{:?}] {:.0} с, id {}", n.artist, n.title, n.video_id, n.duration, n.id),
        Ok(None) => println!("замены не нашлось"),
        Err(e) => println!("ошибка: {e}"),
    }
}
