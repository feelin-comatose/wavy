//! Проверка импорта из Spotify: cargo run -p wavy-core --example sp_check -- <data_dir> <spotify_url>
fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("data dir"));
    let url = std::env::args().nth(2).expect("url");
    let yt = wavy_core::youtube::YouTube::new(dir.join("bin"));
    let t = std::time::Instant::now();
    match wavy_core::spotify::import(&url, &yt, None, &|m: &str| eprintln!("  {m}")) {
        Ok(r) => {
            println!("«{}»: найдено {} из {}, за {:.0} с{}", r.name, r.tracks.len(), r.tracks.len() + r.not_found.len(), t.elapsed().as_secs_f32(), if r.truncated { " (возможно обрезано до 100)" } else { "" });
            for x in &r.tracks { println!("  ✓ {} — {} [{}] {}", x.artist, x.title, wavy_core::fmt_time(x.duration), x.url.clone().unwrap_or_default()); }
            for x in &r.not_found { println!("  ✗ {} — {}", x.artists, x.title); }
        }
        Err(e) => println!("ОШИБКА: {e}"),
    }
}
