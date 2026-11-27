//! Check the "For you" rows against YouTube Music: cargo run --release -p wavy-core --example recs_check -- [ru]
use std::collections::HashSet;
fn main() {
    let ru = std::env::args().nth(1).as_deref() == Some("ru");
    let yt = wavy_core::youtube::YouTube::new(wavy_core::data_dir().join("bin"));
    let show = |name: &str, r: Result<Vec<(String, String)>, String>| match r {
        Ok(v) => { println!("{name}: {}", v.len()); for (a, b) in v.iter().take(3) { println!("  {a} — {b}"); } }
        Err(e) => println!("{name}: ошибка {e}"),
    };
    let seed = wavy_core::Track { id: "s".into(), kind: "local".into(), path: None, video_id: None, url: None, title: "Get Lucky".into(), artist: "Daft Punk".into(),
        album: String::new(), duration: 0.0, cover: None, added_at: 0, video: false, anime: None, episode: None };
    let vid = yt.seed_video(&seed);
    println!("seed video: {vid:?}");
    if let Some(v) = vid { show("similar", yt.similar(&seed, &v, &HashSet::new(), 15).map(|v| v.into_iter().map(|t| (t.artist, t.title)).collect())); }
    let cards = |name: &str, r: Result<Vec<wavy_core::recs::Card>, String>| {
        show(name, r.clone().map(|v| v.into_iter().map(|c| (format!("{:?}", std::mem::discriminant(&c.kind)), format!("{} | {} | {}", c.title, c.subtitle, c.id))).collect()));
        if let Some(c) = r.ok().and_then(|v| v.into_iter().find(|c| !matches!(c.kind, wavy_core::recs::CardKind::Track(_)))) {
            show(&format!("  tracks of {}", c.title), yt.card_tracks(&c).map(|(_, _, v)| v.into_iter().map(|t| (t.artist, t.title)).collect()));
        }
    };
    cards("new releases", yt.new_releases(ru));
    cards("charts", yt.charts(ru));
    cards("mood: relax", yt.mood_playlists("relax", ru));
    cards("mood: anime songs", yt.mood_playlists("anime songs", ru));
}
