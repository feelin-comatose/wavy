//! Cases shared with the Android version (testdata/parity/*.json). The same set is read by
//! android/app/src/test/.../ParityLogicTest.kt — if one side changed an algorithm and the other didn't,
//! the test fails on the side that drifted from the cases.
use serde_json::Value;
use wavy_core::{anime, lyrics, spotify, Track};

fn cases(name: &str) -> Vec<Value> {
    let path = format!("{}/../testdata/parity/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let v: Value = serde_json::from_str(&text).unwrap();
    v["cases"].as_array().unwrap().clone()
}

fn s(v: &Value) -> &str { v.as_str().unwrap() }

#[test]
fn anime_names() {
    let mut bad = Vec::new();
    for c in cases("anime") {
        let p = anime::parse_name(s(&c["name"]));
        let got = (p.title.clone(), p.episode.map(|e| e as f64), p.season.map(|x| x as u64), anime::key_of(&p), anime::search_query(&p));
        let want = (s(&c["title"]).to_string(), c["episode"].as_f64(), c["season"].as_u64(), s(&c["key"]).to_string(), s(&c["query"]).to_string());
        if got != want { bad.push(format!("{}\n  ждали {want:?}\n  вышло {got:?}", s(&c["name"]))); }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn spotify_clean_title() {
    for c in cases("spotify_clean") {
        assert_eq!(spotify::clean_title(s(&c["in"])), s(&c["out"]), "{}", s(&c["in"]));
    }
}

#[test]
fn spotify_match() {
    for c in cases("spotify_match") {
        let sp = &c["spotify"];
        let t = spotify::SpTrack {
            title: s(&sp["title"]).into(),
            artists: s(&sp["artists"]).into(),
            album: s(&sp["album"]).into(),
            duration: sp["duration"].as_f64().unwrap(),
        };
        let cands: Vec<Track> = serde_json::from_value(c["candidates"].clone()).unwrap();
        let best = spotify::rank(&t, cands).into_iter().next().and_then(|t| t.video_id);
        assert_eq!(best.as_deref(), c["expect"].as_str(), "{}", s(&c["what"]));
    }
}

#[test]
fn lyrics_clean() {
    for c in cases("lyrics_clean") {
        let t: Track = serde_json::from_value(serde_json::json!({"id": "x", "type": c["type"], "title": c["title"], "artist": c["artist"]})).unwrap();
        let want = (s(&c["clean"][0]).to_string(), s(&c["clean"][1]).to_string());
        assert_eq!(lyrics::clean(&t), want, "{}", s(&c["title"]));
    }
}

#[test]
fn lyrics_lrc() {
    for c in cases("lyrics_lrc") {
        let want: Vec<(f64, String)> = c["lines"].as_array().unwrap().iter().map(|l| (l[0].as_f64().unwrap(), s(&l[1]).to_string())).collect();
        assert_eq!(lyrics::parse_lrc(s(&c["lrc"])), want, "{}", s(&c["lrc"]));
    }
}
