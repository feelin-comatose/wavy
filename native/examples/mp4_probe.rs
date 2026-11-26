// Debug MP4 timestamps: cargo run -p wavy-native --example mp4_probe -- <file>
fn main() {
    let path = std::env::args().nth(1).expect("file");
    let f = std::fs::File::open(&path).unwrap();
    let size = f.metadata().unwrap().len();
    let mut mp4 = mp4::Mp4Reader::read_header(std::io::BufReader::new(f), size).unwrap();
    let (id, ts) = mp4.tracks().values().find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H264))).map(|t| (t.track_id(), t.timescale())).unwrap();
    let n = mp4.sample_count(id).unwrap();
    println!("fragmented={} samples={n} timescale={ts}", mp4.is_fragmented());
    for i in [1, 2, 3, 4, 30, 31, 60, 61, 62, 200, n] {
        if let Ok(Some(s)) = mp4.read_sample(id, i) { println!("#{i}: start={} off={} sync={} len={} head={:02x?}", s.start_time, s.rendering_offset, s.is_sync, s.bytes.len(), &s.bytes[..8.min(s.bytes.len())]); }
    }
}
