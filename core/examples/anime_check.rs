//! Проверка распознавания тайтла: cargo run -p wavy-core --example anime_check -- "<имя файла серии>"...
fn main() {
    for name in std::env::args().skip(1) {
        let p = wavy_core::anime::parse_path(std::path::Path::new(&name));
        let q = wavy_core::anime::search_query(&p);
        print!("{name}\n  → «{}» серия {:?} сезон {:?}; поиск «{q}»: ", p.title, p.episode, p.season);
        match wavy_core::shikimori::search(&q, 8) {
            Ok(list) => match wavy_core::shikimori::best_match(&p.title, p.season, 12, &list) {
                Some(a) => println!("{} / {} ({} {}, {} эп.) постер: {}", a.russian, a.name, a.kind, a.year, a.episodes, a.poster.is_some()),
                None => println!("не нашлось ({} вариантов)", list.len()),
            },
            Err(e) => println!("ошибка {e}"),
        }
    }
}
