//! Опенинги и эндинги серий из AniSkip (открытая база, без ключа). Тайтл — по id MyAnimeList,
//! он совпадает с id Шикимори. Ответы кэшируются: серия смотрится много раз, а база меняется редко.
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Skip {
    /// op | ed | mixed-op | mixed-ed | recap
    pub kind: String,
    pub start: f64,
    pub end: f64,
}

impl Skip {
    pub fn is_ed(&self) -> bool { self.kind.ends_with("ed") }
    pub fn label(&self) -> &'static str {
        match self.kind.as_str() { "op" | "mixed-op" => "опенинг", "ed" | "mixed-ed" => "эндинг", "recap" => "пересказ", _ => "заставку" }
    }
}

fn parse(v: &Value) -> Vec<Skip> {
    let mut out: Vec<Skip> = v["results"].as_array().map(|a| a.iter().filter_map(|r| Some(Skip {
        kind: r["skipType"].as_str()?.to_string(),
        start: r["interval"]["startTime"].as_f64()?,
        end: r["interval"]["endTime"].as_f64()?,
    })).filter(|s| s.end > s.start + 5.0).collect()).unwrap_or_default();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// Отрезки серии `ep` тайтла `mal_id` (длина серии `length` — сервер подгоняет время под нашу версию).
pub fn fetch(mal_id: u64, ep: u32, length: f64, cache: &Path) -> Result<Vec<Skip>, String> {
    let file = cache.join(format!("{mal_id}_{ep}.json"));
    if let Some(v) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) { return Ok(parse(&v)); }
    let url = format!("https://api.aniskip.com/v2/skip-times/{mal_id}/{ep}?types=op&types=ed&types=mixed-op&types=mixed-ed&types=recap&episodeLength={}", length.round().max(0.0));
    let v: Value = match crate::http().get(&url).call() {
        Ok(mut r) => r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null),
        Err(ureq::Error::StatusCode(404)) => Value::Null,
        Err(e) => return Err(format!("AniSkip недоступен: {e}")),
    };
    let _ = std::fs::create_dir_all(cache);
    let _ = std::fs::write(&file, v.to_string());
    Ok(parse(&v))
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let v: serde_json::Value = serde_json::from_str(r#"{"found":true,"results":[{"interval":{"startTime":1295.1,"endTime":1385.1},"skipType":"ed"},{"interval":{"startTime":5.6,"endTime":95.6},"skipType":"op"}]}"#).unwrap();
        let s = super::parse(&v);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].kind, "op");
        assert!(s[1].is_ed());
        assert_eq!(super::parse(&serde_json::Value::Null), vec![]);
    }
}
