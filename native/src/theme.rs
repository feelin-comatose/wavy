//! Interface color themes. The palette is global (read by every widget each frame) and changes at
//! runtime when the user picks another theme or accent in settings.
//! The default is "dark", the original black-and-white look.
use eframe::egui::Color32;
use std::sync::RwLock;

#[derive(Clone, Copy, PartialEq)]
pub struct Pal {
    /// whether the theme is dark (egui base visuals and a few contrast decisions depend on it)
    pub dark: bool,
    pub bg: Color32,
    pub bg2: Color32,
    pub bg3: Color32,
    pub line: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    /// softer accent for secondary highlights
    pub accent_soft: Color32,
    /// text and icons drawn on top of an accent fill (play button, primary buttons)
    pub on_accent: Color32,
    pub hover: Color32,
    pub hover2: Color32,
    pub active: Color32,
    /// empty part of sliders and progress bars
    pub track: Color32,
}

const fn rgb(v: u32) -> Color32 { Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8) }

#[allow(clippy::too_many_arguments)]
const fn pal(dark: bool, bg: u32, bg2: u32, bg3: u32, line: u32, text: u32, muted: u32, accent: u32, soft: u32, hover: u32, hover2: u32, active: u32, track: u32) -> Pal {
    let on_accent = if dark { rgb(bg) } else { rgb(0xffffff) };
    Pal { dark, bg: rgb(bg), bg2: rgb(bg2), bg3: rgb(bg3), line: rgb(line), text: rgb(text), muted: rgb(muted), accent: rgb(accent), accent_soft: rgb(soft), on_accent, hover: rgb(hover), hover2: rgb(hover2), active: rgb(active), track: rgb(track) }
}

/// (id, Russian name, palette). Russian names go through `wavy_core::i18n::tr` for display.
pub const THEMES: &[(&str, &str, Pal)] = &[
    ("dark", "Тёмная", pal(true, 0x0a0a0c, 0x101013, 0x18181c, 0x232329, 0xe4e4e8, 0x7a7a86, 0xf4f4f5, 0xa1a1aa, 0x202026, 0x2a2a30, 0x34343a, 0x2c2c33)),
    ("amoled", "Чёрная (AMOLED)", pal(true, 0x000000, 0x07070a, 0x101014, 0x1c1c22, 0xe8e8ec, 0x74747e, 0xffffff, 0xa1a1aa, 0x18181d, 0x222228, 0x2c2c33, 0x26262c)),
    ("light", "Светлая", pal(false, 0xf7f7f8, 0xffffff, 0xececef, 0xdcdce1, 0x16161a, 0x6b6b76, 0x111114, 0x55555e, 0xe2e2e7, 0xd8d8de, 0xcdcdd4, 0xd0d0d6)),
    ("ocean", "Океан", pal(true, 0x0a0f1a, 0x0f1624, 0x162033, 0x22304a, 0xe2e8f0, 0x7487a3, 0x60a5fa, 0x93c5fd, 0x1b2840, 0x22324f, 0x2a3c5e, 0x24324c)),
    ("sakura", "Сакура", pal(true, 0x140b10, 0x1b1016, 0x25161e, 0x37212d, 0xf5e6ee, 0xa07f90, 0xf472b6, 0xf9a8d4, 0x2c1a24, 0x36202c, 0x422836, 0x3a2430)),
    ("forest", "Лес", pal(true, 0x0a110d, 0x0f1812, 0x15221a, 0x213426, 0xe3efe6, 0x7d9785, 0x4ade80, 0x86efac, 0x1a2b20, 0x213527, 0x294230, 0x22362a)),
    ("amber", "Янтарь", pal(true, 0x120e08, 0x18130b, 0x211a10, 0x33281a, 0xf3ece0, 0x9c8c74, 0xfbbf24, 0xfcd34d, 0x2a2114, 0x342918, 0x3f321d, 0x382c1b)),
];

static CURRENT: RwLock<Pal> = RwLock::new(THEMES[0].2);

/// The current palette.
pub fn p() -> Pal { *CURRENT.read().unwrap() }

/// Palette for the in-game overlay window: it's always a dark card over the game, so a light theme
/// falls back to the dark one there (otherwise its near-black accent would vanish on the card).
pub fn overlay() -> Pal { let c = p(); if c.dark { c } else { THEMES[0].2 } }

/// "#ff8800" / "ff8800" → color.
pub fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 { return None; }
    u32::from_str_radix(h, 16).ok().map(rgb)
}

pub fn to_hex(c: Color32) -> String { format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()) }

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Relative luminance (sRGB, 0..1).
fn luminance(c: Color32) -> f32 {
    let f = |v: u8| { let v = v as f32 / 255.0; if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
    0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
}

/// Palette for a theme id plus an optional custom accent ("#rrggbb"; empty — the theme's own).
pub fn resolve(theme: &str, accent: &str) -> Pal {
    let mut p = THEMES.iter().find(|t| t.0 == theme).unwrap_or(&THEMES[0]).2;
    if let Some(a) = parse_hex(accent) {
        p.accent = a;
        p.accent_soft = mix(a, p.text, 0.35);
        // text on the accent: whichever of black/white contrasts more
        p.on_accent = if luminance(a) > 0.45 { Color32::from_rgb(0x0a, 0x0a, 0x0c) } else { Color32::WHITE };
    }
    p
}

/// Switch the palette. Returns true if it changed (then the egui visuals need refreshing).
pub fn set(theme: &str, accent: &str) -> bool {
    let new = resolve(theme, accent);
    let mut cur = CURRENT.write().unwrap();
    let changed = *cur != new;
    *cur = new;
    changed
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_custom_accent() {
        assert_eq!(THEMES[0].0, "dark");
        assert!(THEMES.iter().all(|t| t.2.dark != (t.0 == "light")));
        // unknown theme falls back to dark
        assert_eq!(resolve("nope", "").bg, THEMES[0].2.bg);
        // custom accent: bright → dark text on it, dark → white text
        assert_eq!(resolve("dark", "#ffee00").on_accent, Color32::from_rgb(0x0a, 0x0a, 0x0c));
        assert_eq!(resolve("dark", "1d4ed8").on_accent, Color32::WHITE);
        assert_eq!(resolve("dark", "#12345").accent, THEMES[0].2.accent); // malformed — ignored
        assert_eq!(to_hex(parse_hex("#A1b2C3").unwrap()), "#a1b2c3");
        // every preset keeps readable text: text vs background contrast ratio ≥ 7 (WCAG AAA)
        for (id, _, p) in THEMES {
            let (a, b) = (luminance(p.text), luminance(p.bg));
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(ratio >= 7.0, "{id}: {ratio}");
        }
    }
}
