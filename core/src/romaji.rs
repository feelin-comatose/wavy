//! Romaji for Japanese lyrics: kana by Hepburn rules, kanji through a compact dictionary built from JMdict
//! (common words with their readings, see scripts/build-romaji-dict.py; CC BY-SA 4.0, EDRDG).
//! It's an approximation: a dictionary can't always know which reading a word takes in context,
//! and kanji missing from it are left as they are.
use std::collections::HashMap;
use std::sync::OnceLock;

const DICT_GZ: &[u8] = include_bytes!("../assets/romaji-dict.tsv.gz");

/// kanji spelling → (word reading, stem reading)
struct Dict { map: HashMap<String, (Option<String>, Option<String>)>, longest: usize }

/// Loaded on first use (~2 MB in memory), only if lyrics with Japanese are shown with romaji on.
fn dict() -> &'static Dict {
    static D: OnceLock<Dict> = OnceLock::new();
    D.get_or_init(|| {
        use std::io::Read;
        let mut text = String::new();
        let _ = flate2::read::GzDecoder::new(DICT_GZ).read_to_string(&mut text);
        let mut map: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        let mut longest = 1;
        for l in text.lines() {
            let mut p = l.split('\t');
            let (Some(k), Some(kind), Some(r)) = (p.next(), p.next(), p.next()) else { continue };
            longest = longest.max(k.chars().count());
            let e = map.entry(k.to_string()).or_default();
            if kind == "s" { e.1 = Some(r.to_string()) } else { e.0 = Some(r.to_string()) }
        }
        Dict { map, longest: longest.min(10) }
    })
}

fn is_hiragana(c: char) -> bool { ('\u{3041}'..='\u{309F}').contains(&c) }
fn is_katakana(c: char) -> bool { ('\u{30A1}'..='\u{30FF}').contains(&c) }
fn is_kana(c: char) -> bool { is_hiragana(c) || is_katakana(c) }
fn is_kanji(c: char) -> bool { ('\u{4E00}'..='\u{9FFF}').contains(&c) || c == '々' }

/// Whether a line has Japanese in it (kana or kanji).
pub fn has_japanese(s: &str) -> bool { s.chars().any(|c| is_kana(c) || is_kanji(c)) }

/// Katakana → hiragana (ー stays: it lengthens the vowel).
fn to_hira(c: char) -> char {
    if ('\u{30A1}'..='\u{30F6}').contains(&c) { char::from_u32(c as u32 - 0x60).unwrap_or(c) } else { c }
}

fn base(c: char) -> Option<&'static str> {
    Some(match c {
        'あ' => "a", 'い' => "i", 'う' => "u", 'え' => "e", 'お' => "o",
        'か' => "ka", 'き' => "ki", 'く' => "ku", 'け' => "ke", 'こ' => "ko",
        'さ' => "sa", 'し' => "shi", 'す' => "su", 'せ' => "se", 'そ' => "so",
        'た' => "ta", 'ち' => "chi", 'つ' => "tsu", 'て' => "te", 'と' => "to",
        'な' => "na", 'に' => "ni", 'ぬ' => "nu", 'ね' => "ne", 'の' => "no",
        'は' => "ha", 'ひ' => "hi", 'ふ' => "fu", 'へ' => "he", 'ほ' => "ho",
        'ま' => "ma", 'み' => "mi", 'む' => "mu", 'め' => "me", 'も' => "mo",
        'や' => "ya", 'ゆ' => "yu", 'よ' => "yo",
        'ら' => "ra", 'り' => "ri", 'る' => "ru", 'れ' => "re", 'ろ' => "ro",
        'わ' => "wa", 'ゐ' => "i", 'ゑ' => "e", 'を' => "wo", 'ん' => "n",
        'が' => "ga", 'ぎ' => "gi", 'ぐ' => "gu", 'げ' => "ge", 'ご' => "go",
        'ざ' => "za", 'じ' => "ji", 'ず' => "zu", 'ぜ' => "ze", 'ぞ' => "zo",
        'だ' => "da", 'ぢ' => "ji", 'づ' => "zu", 'で' => "de", 'ど' => "do",
        'ば' => "ba", 'び' => "bi", 'ぶ' => "bu", 'べ' => "be", 'ぼ' => "bo",
        'ぱ' => "pa", 'ぴ' => "pi", 'ぷ' => "pu", 'ぺ' => "pe", 'ぽ' => "po",
        'ぁ' => "a", 'ぃ' => "i", 'ぅ' => "u", 'ぇ' => "e", 'ぉ' => "o", 'ゔ' => "vu",
        'ゃ' => "ya", 'ゅ' => "yu", 'ょ' => "yo", 'ゎ' => "wa",
        _ => return None,
    })
}

/// Kana (hiragana or katakana) → Hepburn romaji.
pub fn kana(s: &str) -> String {
    let cs: Vec<char> = s.chars().map(to_hira).collect();
    let mut out = String::new();
    let mut double = false; // っ: double the next consonant
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == 'っ' { double = true; i += 1; continue; }
        if c == 'ー' {
            if let Some(v) = out.chars().rev().find(|c| "aeiou".contains(*c)) { out.push(v); }
            i += 1;
            continue;
        }
        let next = cs.get(i + 1).copied();
        let mut syl: String = match base(c) { Some(b) => b.into(), None => { out.push(c); i += 1; continue; } };
        // combinations with a small kana: きゃ kya, しゃ sha, ちゃ cha, じゃ ja; ティ ti, ファ fa, ウィ wi …
        if let Some(n) = next {
            let combo: Option<String> = match (c, n) {
                (_, 'ゃ' | 'ゅ' | 'ょ') if syl.ends_with('i') && syl.len() > 1 => {
                    let v = &base(n).unwrap()[1..];
                    Some(match syl.as_str() { "shi" => format!("sh{v}"), "chi" => format!("ch{v}"), "ji" => format!("j{v}"), s => format!("{}y{v}", &s[..s.len() - 1]) })
                }
                ('て', 'ぃ') => Some("ti".into()), ('で', 'ぃ') => Some("di".into()), ('と', 'ぅ') => Some("tu".into()), ('ど', 'ぅ') => Some("du".into()),
                ('ふ', 'ぁ' | 'ぃ' | 'ぇ' | 'ぉ') => Some(format!("f{}", base(n).unwrap())),
                ('う', 'ぃ' | 'ぇ' | 'ぉ') => Some(format!("w{}", base(n).unwrap())),
                ('ゔ', 'ぁ' | 'ぃ' | 'ぇ' | 'ぉ') => Some(format!("v{}", base(n).unwrap())),
                ('し', 'ぇ') => Some("she".into()), ('ち', 'ぇ') => Some("che".into()), ('じ', 'ぇ') => Some("je".into()),
                _ => None,
            };
            if let Some(c2) = combo { syl = c2; i += 1; }
        }
        if double {
            double = false;
            if syl.starts_with("ch") { out.push('t'); } else if let Some(f) = syl.chars().next().filter(|c| !"aeiou".contains(*c)) { out.push(f); }
        }
        out.push_str(&syl);
        i += 1;
    }
    out
}

const PARTICLES: &[char] = &['の', 'を', 'が', 'に', 'は', 'で', 'と', 'も', 'へ', 'や', 'か'];
/// Particles that mark the kanji before them as a noun (か and や also start verb endings: 行かない).
const NOUN_PARTICLES: &[char] = &['の', 'を', 'が', 'に', 'は', 'で', 'と', 'も', 'へ'];

const LONG_PARTICLES: &[&str] = &["まで", "から", "より", "だけ", "しか", "ばかり", "など"];
fn starts_particle(rest: &[char]) -> bool { LONG_PARTICLES.iter().any(|p| rest.iter().take(p.chars().count()).copied().eq(p.chars())) }

fn particle(c: char) -> String { match c { 'は' => "wa".into(), 'へ' => "e".into(), 'を' => "wo".into(), c => kana(&c.to_string()) } }

/// One lyric line → romaji, or None if it has no Japanese.
pub fn line(s: &str) -> Option<String> {
    if !has_japanese(s) { return None; }
    let d = dict();
    let cs: Vec<char> = s.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if is_kanji(c) {
            // longest dictionary spelling starting here
            let mut hit = None;
            for len in (1..=d.longest.min(cs.len() - i)).rev() {
                let key: String = cs[i..i + len].iter().collect();
                let Some(e) = d.map.get(&key) else { continue };
                // "今日は" is also a greeting (konnichiwa), but in lyrics it's 今日 + は: prefer the word without a trailing particle
                let last = cs[i + len - 1];
                if len > 1 && PARTICLES.contains(&last) && d.map.get(&cs[i..i + len - 1].iter().collect::<String>()).is_some_and(|e| e.0.is_some()) { continue; }
                hit = Some((len, e));
                break;
            }
            let Some((len, (word, stem))) = hit else { words.push(c.to_string()); i += 1; continue };
            // kana right after a kanji: an inflection (言って → stem) — unless it's a particle (空が → the noun)
            let next = cs.get(i + len).copied();
            let inflected = next.is_some_and(|c| is_hiragana(c) && !NOUN_PARTICLES.contains(&c));
            let (reading, is_stem) = match (word, stem) {
                (_, Some(st)) if inflected => (st, true),
                (Some(w), _) => (w, false),
                (None, Some(st)) => (st, true),
                _ => unreachable!(),
            };
            let mut w = kana(reading);
            i += len;
            // okurigana / inflection stays attached (言って → itte); after a full word a particle starts a new word
            while i < cs.len() && is_hiragana(cs[i]) {
                if !is_stem && PARTICLES.contains(&cs[i]) { break; }
                // longer particles start a new word too (夜明けまで → yoake made, 一度だけ → ichido dake)
                if !is_stem && starts_particle(&cs[i..]) { break; }
                let start = i;
                while i < cs.len() && is_hiragana(cs[i]) && (is_stem || !PARTICLES.contains(&cs[i]) || i == start) { i += 1; if !is_stem { break; } }
                w.push_str(&kana(&cs[start..i].iter().collect::<String>()));
            }
            words.push(w);
        } else if is_kana(c) {
            let start = i;
            // a particle right after a kanji word stands alone
            if is_hiragana(c) && PARTICLES.contains(&c) && i > 0 && is_kanji(cs[i - 1]) && !cs.get(i + 1).is_some_and(|n| matches!(n, 'ゃ' | 'ゅ' | 'ょ')) {
                words.push(particle(c));
                i += 1;
                continue;
            }
            while i < cs.len() && is_kana(cs[i]) { i += 1; }
            words.push(kana(&cs[start..i].iter().collect::<String>()));
        } else {
            let p = match c { '、' => ",", '。' => ".", '！' => "!", '？' => "?", '「' | '」' | '『' | '』' => "\"", '　' => " ", '〜' | '～' => "~", _ => "" };
            if !p.is_empty() { words.push(p.to_string()); } else { words.push(c.to_string()); }
            i += 1;
        }
    }
    // join with single spaces; punctuation sticks to the word before it
    let mut out = String::new();
    for w in words {
        let w = w.trim();
        if w.is_empty() { continue; }
        if !out.is_empty() && !matches!(w, "," | "." | "!" | "?") && !out.ends_with(' ') { out.push(' '); }
        out.push_str(w);
    }
    Some(out.split_whitespace().collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kana_rules() {
        assert_eq!(kana("ありがとう"), "arigatou");
        assert_eq!(kana("ちょっと"), "chotto");
        assert_eq!(kana("きゃりーぱみゅぱみゅ"), "kyariipamyupamyu");
        assert_eq!(kana("カタカナ"), "katakana");
        assert_eq!(kana("ファンタジー"), "fantajii");
        assert_eq!(kana("パーティー"), "paatii");
        assert_eq!(kana("まっちゃ"), "matcha");
        assert_eq!(kana("しゃしん"), "shashin");
    }

    #[test]
    fn lyric_lines() {
        assert_eq!(line("君の名前を呼ぶ").unwrap(), "kimi no namae wo yobu");
        assert_eq!(line("今日は空がきれい").unwrap(), "kyou wa sora ga kirei");
        assert_eq!(line("走って行こう").unwrap(), "hashitte ikou");
        assert_eq!(line("東京タワー").unwrap(), "toukyou tawaa");
        assert_eq!(line("夢を見た、あの夜").unwrap(), "yume wo mita, ano yoru");
        assert_eq!(line("夜明けまで").unwrap(), "yoake made");
        assert_eq!(line("もう一度だけ").unwrap(), "mou ichido dake");
        assert_eq!(line("I love you"), None); // no Japanese
        assert!(has_japanese("Hello 世界") && !has_japanese("Hello"));
    }
}
