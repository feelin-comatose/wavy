//! Итоги картинкой (1080×1350, как пост в соцсетях): часы музыки, прослушивания, топ треков и артистов.
//! Чёрно-белая, как и сам Wavy. Текст рисуется Inter; иероглифы — системным шрифтом (как в интерфейсе).
use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};
use std::path::Path;

pub struct Entry { pub title: String, pub sub: String, pub cover: Option<std::path::PathBuf> }

pub struct Recap {
    pub period: String,
    pub hours: String,
    pub plays: u32,
    pub top_artist: Option<String>,
    pub tracks: Vec<Entry>,
    pub artists: Vec<(String, u32)>,
}

const W: u32 = 1080;
const H: u32 = 1350;
const BG: Rgba<u8> = Rgba([10, 10, 12, 255]);
const CARD: Rgba<u8> = Rgba([24, 24, 28, 255]);
const TEXT: Rgba<u8> = Rgba([244, 244, 245, 255]);
const MUTED: Rgba<u8> = Rgba([122, 122, 134, 255]);

struct Fonts { regular: FontArc, semi: FontArc, bold: FontArc, cjk: Option<FontArc> }

impl Fonts {
    fn load(cjk: Option<Vec<u8>>) -> Self {
        let f = |b: &'static [u8]| FontArc::try_from_slice(b).expect("шрифт");
        Self {
            regular: f(include_bytes!("../assets/Inter-Regular.otf")),
            semi: f(include_bytes!("../assets/Inter-SemiBold.otf")),
            bold: f(include_bytes!("../assets/Inter-Bold.otf")),
            cjk: cjk.and_then(|b| FontVec::try_from_vec_and_index(b, 0).ok()).map(FontArc::new),
        }
    }
}

fn blend(img: &mut RgbaImage, x: i32, y: i32, c: Rgba<u8>, a: f32) {
    if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 || a <= 0.0 { return; }
    let p = img.get_pixel_mut(x as u32, y as u32);
    let a = a.min(1.0);
    for i in 0..3 { p.0[i] = (p.0[i] as f32 * (1.0 - a) + c.0[i] as f32 * a) as u8; }
}

fn rect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, r: i32, c: Rgba<u8>) {
    for yy in y..y + h {
        for xx in x..x + w {
            // скруглённые углы
            let dx = if xx < x + r { x + r - xx } else if xx >= x + w - r { xx - (x + w - r - 1) } else { 0 };
            let dy = if yy < y + r { y + r - yy } else if yy >= y + h - r { yy - (y + h - r - 1) } else { 0 };
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            let a = if dx > 0 && dy > 0 { (r as f32 - d + 0.5).clamp(0.0, 1.0) } else { 1.0 };
            blend(img, xx, yy, c, a);
        }
    }
}

/// Текст в строку, обрезается многоточием по ширине `max`. Возвращает ширину.
fn text(img: &mut RgbaImage, fonts: &Fonts, font: &FontArc, size: f32, x: f32, y: f32, s: &str, c: Rgba<u8>, max: f32) -> f32 {
    let scale = PxScale::from(size);
    let pick = |ch: char| -> &FontArc { if font.glyph_id(ch).0 != 0 { font } else { fonts.cjk.as_ref().filter(|f| f.glyph_id(ch).0 != 0).unwrap_or(font) } };
    // обрезка по ширине
    let width = |s: &str| s.chars().map(|ch| pick(ch).as_scaled(scale).h_advance(pick(ch).glyph_id(ch))).sum::<f32>();
    let mut s = s.to_string();
    if width(&s) > max {
        while !s.is_empty() && width(&format!("{s}…")) > max { s.pop(); }
        s = format!("{}…", s.trim_end());
    }
    let ascent = font.as_scaled(scale).ascent();
    let mut pen = x;
    for ch in s.chars() {
        let f = pick(ch);
        let sf = f.as_scaled(scale);
        let g = f.glyph_id(ch).with_scale_and_position(scale, ab_glyph::point(pen, y + ascent));
        pen += sf.h_advance(f.glyph_id(ch));
        if let Some(o) = f.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, a| blend(img, b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, c, a));
        }
    }
    pen - x
}

fn cover(img: &mut RgbaImage, path: Option<&Path>, x: i32, y: i32, size: u32) {
    rect(img, x, y, size as i32, size as i32, 12, CARD);
    let Some(src) = path.and_then(|p| image::open(p).ok()) else { return };
    let s = src.width().min(src.height());
    let sq = src.crop_imm((src.width() - s) / 2, (src.height() - s) / 2, s, s).resize_exact(size, size, image::imageops::FilterType::Triangle).to_rgba8();
    for (px, py, p) in sq.enumerate_pixels() {
        // скругление углов картинки
        let (dx, dy) = ((px as i32 - size as i32 / 2).abs(), (py as i32 - size as i32 / 2).abs());
        let edge = size as i32 / 2 - 12;
        if dx > edge && dy > edge && (((dx - edge).pow(2) + (dy - edge).pow(2)) as f32).sqrt() > 12.0 { continue; }
        blend(img, x + px as i32, y + py as i32, Rgba([p.0[0], p.0[1], p.0[2], 255]), p.0[3] as f32 / 255.0);
    }
}

pub fn render(r: &Recap, cjk: Option<Vec<u8>>) -> RgbaImage {
    let f = Fonts::load(cjk);
    let mut img = RgbaImage::from_pixel(W, H, BG);
    let m = 72.0;
    text(&mut img, &f, &f.semi, 26.0, m, 64.0, l!("WAVY · ИТОГИ", "WAVY · RECAP"), MUTED, 900.0);
    text(&mut img, &f, &f.bold, 84.0, m, 100.0, &r.period, TEXT, 940.0);
    // три плитки с цифрами
    let tiles = [(r.hours.clone(), l!("музыки", "of music")), (r.plays.to_string(), l!("прослушиваний", "plays")), (r.top_artist.clone().unwrap_or_else(|| "—".into()), l!("артист №1", "top artist"))];
    let tw = (W as f32 - 2.0 * m - 2.0 * 20.0) / 3.0;
    for (i, (big, small)) in tiles.iter().enumerate() {
        let x = m + i as f32 * (tw + 20.0);
        rect(&mut img, x as i32, 232, tw as i32, 150, 22, CARD);
        text(&mut img, &f, &f.bold, if big.chars().count() > 9 { 34.0 } else { 46.0 }, x + 26.0, 262.0, big, TEXT, tw - 52.0);
        text(&mut img, &f, &f.regular, 24.0, x + 26.0, 330.0, small, MUTED, tw - 52.0);
    }
    // топ треков
    text(&mut img, &f, &f.semi, 32.0, m, 430.0, l!("Топ треков", "Top tracks"), TEXT, 900.0);
    for (i, e) in r.tracks.iter().take(5).enumerate() {
        let y = 490.0 + i as f32 * 104.0;
        text(&mut img, &f, &f.bold, 30.0, m, y + 24.0, &(i + 1).to_string(), MUTED, 40.0);
        cover(&mut img, e.cover.as_deref(), (m + 48.0) as i32, y as i32, 84);
        text(&mut img, &f, &f.semi, 31.0, m + 156.0, y + 8.0, &e.title, TEXT, W as f32 - m - (m + 156.0));
        text(&mut img, &f, &f.regular, 24.0, m + 156.0, y + 50.0, &e.sub, MUTED, W as f32 - m - (m + 156.0));
    }
    // топ артистов — полосками
    let ay = 1032.0;
    text(&mut img, &f, &f.semi, 32.0, m, ay, l!("Топ артистов", "Top artists"), TEXT, 900.0);
    let max = r.artists.first().map(|a| a.1).unwrap_or(1).max(1) as f32;
    for (i, (a, n)) in r.artists.iter().take(4).enumerate() {
        let y = ay + 56.0 + i as f32 * 52.0;
        text(&mut img, &f, &f.regular, 26.0, m, y, a, TEXT, 420.0);
        let bx = m + 440.0;
        let bw = W as f32 - m - bx - 70.0;
        rect(&mut img, bx as i32, (y + 12.0) as i32, bw as i32, 8, 4, CARD);
        rect(&mut img, bx as i32, (y + 12.0) as i32, (bw * *n as f32 / max).max(8.0) as i32, 8, 4, TEXT);
        text(&mut img, &f, &f.regular, 24.0, bx + bw + 16.0, y + 2.0, &n.to_string(), MUTED, 60.0);
    }
    text(&mut img, &f, &f.bold, 26.0, m, H as f32 - 70.0, "wavy", MUTED, 200.0);
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders() {
        let r = Recap {
            period: "2026".into(), hours: "142 ч".into(), plays: 2318, top_artist: Some("LiSA".into()),
            tracks: vec![
                Entry { title: "Gurenge".into(), sub: "LiSA · 128 раз".into(), cover: None },
                Entry { title: "アイドル (Idol)".into(), sub: "YOASOBI · 97 раз".into(), cover: None },
                Entry { title: "Очень длинное название трека, которое не влезает в строку целиком никак".into(), sub: "Кто-то".into(), cover: None },
            ],
            artists: vec![("LiSA".into(), 300), ("YOASOBI".into(), 210), ("Eve".into(), 120)],
        };
        let cjk = ["/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"].iter().find_map(|p| std::fs::read(p).ok());
        let img = render(&r, cjk);
        assert_eq!(img.dimensions(), (W, H));
        // что-то нарисовано светлым
        assert!(img.pixels().filter(|p| p.0[0] > 200).count() > 5000);
        if let Ok(p) = std::env::var("WAVY_RECAP_OUT") { img.save(p).unwrap(); }
    }
}
