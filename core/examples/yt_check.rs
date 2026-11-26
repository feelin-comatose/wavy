//! Check the YouTube chain: cargo run -p wavy-core --example yt_check -- <data_dir> [url]
use wavy_core::{store::Store, youtube::YouTube};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("data dir"));
    let url = std::env::args().nth(2);
    std::fs::create_dir_all(&dir).unwrap();
    let yt = YouTube::new(dir.join("bin"));
    let t = std::time::Instant::now();
    match yt.ensure_binary(&|m| println!("  {m}")) {
        Ok(b) => println!("yt-dlp: {b} ({:.1}s), версия {}", t.elapsed().as_secs_f32(), yt.version().unwrap_or_default()),
        Err(e) => { println!("yt-dlp НЕ получен: {e}"); return; }
    }
    let t = std::time::Instant::now();
    match yt.search("lofi hip hop", 5) {
        Ok(r) => {
            println!("поиск: {} результатов за {:.1}s", r.len(), t.elapsed().as_secs_f32());
            for x in r.iter().take(3) { println!("  - {} — {} [{}]", x.title, x.artist, wavy_core::fmt_time(x.duration)); }
        }
        Err(e) => println!("поиск: ОШИБКА {e}"),
    }
    // wait for the deno JS engine (downloads in the background)
    let t = std::time::Instant::now();
    while matches!(yt.deno_status(), wavy_core::youtube::Deno::Installing) { std::thread::sleep(std::time::Duration::from_millis(300)); }
    println!("deno: {:?} (ждали {:.1}s)", yt.deno_status(), t.elapsed().as_secs_f32());
    match yt.search("synthwave", 3) {
        Ok(r) => println!("поиск уже с deno: {} результатов", r.len()),
        Err(e) => println!("поиск с deno: ОШИБКА {e}"),
    }
    let Some(url) = url else { return };
    let t = std::time::Instant::now();
    let res = match yt.import_url(&url, &|m| println!("  {m}")) {
        Ok(r) => r,
        Err(e) => { println!("импорт: ОШИБКА {e}"); return; }
    };
    println!("импорт: «{}», {} треков за {:.1}s", res.name.clone().unwrap_or("-".into()), res.tracks.len(), t.elapsed().as_secs_f32());
    for x in res.tracks.iter().take(3) {
        println!("  - {} — {} [{}] {}", x.title, x.artist, wavy_core::fmt_time(x.duration), x.url.clone().unwrap_or_default());
    }
    let Some(first) = res.tracks.first().cloned() else { return };
    let t = std::time::Instant::now();
    match yt.download_audio(&first, &dir.join("cache")) {
        Ok(p) => println!("скачан: {} ({} КБ) за {:.1}s", p.display(), std::fs::metadata(&p).map(|m| m.len() / 1024).unwrap_or(0), t.elapsed().as_secs_f32()),
        Err(e) => println!("скачивание: ОШИБКА {e}"),
    }
    let mut s = Store::open(&dir);
    let ids: Vec<String> = res.tracks.iter().map(|t| t.id.clone()).collect();
    s.add_tracks(res.tracks);
    s.create_playlist(&res.name.unwrap_or("YT".into()), ids, Some(res.source));
    println!("записано в {}", dir.join("library.json").display());
}
