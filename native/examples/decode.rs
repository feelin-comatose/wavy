// Check decoding a file: cargo run -p wavy-native --example decode -- <file>
fn main() {
    let path = std::env::args().nth(1).expect("file");
    let f = std::fs::File::open(&path).unwrap();
    let len = f.metadata().unwrap().len();
    let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
    match rodio::Decoder::builder().with_data(std::io::BufReader::new(f)).with_byte_len(len).with_seekable(true).with_hint(&ext).build() {
        Ok(d) => {
            let dur = rodio::Source::total_duration(&d);
            let n = d.take(44100 * 2 * 3).count();
            println!("OK: длительность {dur:?}, прочитано сэмплов {n}");
        }
        Err(e) => println!("ОШИБКА: {e}"),
    }
}
