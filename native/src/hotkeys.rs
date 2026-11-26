//! Global hotkeys: work even when the Wavy window isn't focused (in a game).
//! Combinations are stored as a string like "Ctrl+Alt+P" (settings: action → combination, empty — disabled).
use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;
use std::collections::HashMap;

/// (action, label, default combination)
pub const ACTIONS: &[(&str, &str, &str)] = &[
    ("toggle", "Пауза / играть", "Ctrl+Alt+P"),
    ("next", "Следующий трек", "Ctrl+Alt+Right"),
    ("prev", "Предыдущий трек", "Ctrl+Alt+Left"),
    ("volUp", "Громче", "Ctrl+Alt+Up"),
    ("volDown", "Тише", "Ctrl+Alt+Down"),
    ("like", "Лайк текущему", "Ctrl+Alt+L"),
    ("skip", "Пропустить опенинг (+85 с)", "Ctrl+Alt+S"),
    ("mini", "Мини-плеер", "Ctrl+Alt+M"),
    ("overlay", "Оверлей поверх игры", "Ctrl+Shift+O"),
    ("lock", "Закрепить / двигать оверлей", "Ctrl+Shift+L"),
];

/// Combination for an action, taking settings into account.
pub fn combo(map: &HashMap<String, String>, action: &str) -> String {
    map.get(action).cloned().unwrap_or_else(|| ACTIONS.iter().find(|a| a.0 == action).map(|a| a.2.to_string()).unwrap_or_default())
}

fn code(key: &str) -> Option<Code> {
    use Code::*;
    let k = key.to_ascii_uppercase();
    if k.len() == 1 {
        let c = k.chars().next()?;
        const L: [Code; 26] = [KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ];
        const D: [Code; 10] = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
        if c.is_ascii_uppercase() { return Some(L[(c as u8 - b'A') as usize]); }
        if c.is_ascii_digit() { return Some(D[(c as u8 - b'0') as usize]); }
    }
    const F: [Code; 12] = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    if let Some(n) = k.strip_prefix('F').and_then(|n| n.parse::<usize>().ok()) { return F.get(n.wrapping_sub(1)).copied(); }
    Some(match k.as_str() {
        "LEFT" => ArrowLeft, "RIGHT" => ArrowRight, "UP" => ArrowUp, "DOWN" => ArrowDown,
        "SPACE" => Space, "PAGEUP" => PageUp, "PAGEDOWN" => PageDown, "HOME" => Home, "END" => End,
        "INSERT" => Insert, "DELETE" => Delete, "PAUSE" => Pause, "[" => BracketLeft, "]" => BracketRight,
        "," => Comma, "." => Period, "-" => Minus, "=" => Equal, "/" => Slash, ";" => Semicolon,
        _ => return None,
    })
}

/// "Ctrl+Alt+P" → HotKey. Not accepted without a modifier: otherwise the key would stop working everywhere.
pub fn parse(s: &str) -> Option<HotKey> {
    let parts: Vec<&str> = s.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let (key, mods) = parts.split_last()?;
    let mut m = Modifiers::empty();
    for p in mods {
        m |= match p.to_ascii_lowercase().as_str() { "ctrl" | "control" => Modifiers::CONTROL, "alt" => Modifiers::ALT, "shift" => Modifiers::SHIFT, "win" | "super" => Modifiers::SUPER, _ => return None };
    }
    if m.is_empty() { return None; }
    Some(HotKey::new(Some(m), code(key)?))
}

/// A key pressed in the window (for recording a combination) → "Ctrl+Alt+P".
pub fn from_egui(key: egui::Key, m: egui::Modifiers) -> Option<String> {
    use egui::Key::*;
    let k = match key {
        ArrowLeft => "Left".to_string(), ArrowRight => "Right".into(), ArrowUp => "Up".into(), ArrowDown => "Down".into(),
        Space => "Space".into(), PageUp => "PageUp".into(), PageDown => "PageDown".into(), Home => "Home".into(), End => "End".into(),
        Insert => "Insert".into(), Delete => "Delete".into(), OpenBracket => "[".into(), CloseBracket => "]".into(),
        Comma => ",".into(), Period => ".".into(), Minus => "-".into(), Equals => "=".into(), Slash => "/".into(), Semicolon => ";".into(),
        k => { let n = k.name(); if n.len() == 1 || (n.starts_with('F') && n[1..].parse::<u8>().is_ok()) { n.to_string() } else { return None } }
    };
    let mut s = String::new();
    if m.ctrl || m.command { s += "Ctrl+"; }
    if m.alt { s += "Alt+"; }
    if m.shift { s += "Shift+"; }
    if s.is_empty() { return None; }
    Some(s + &k)
}

/// Registered combinations: event id → action.
#[derive(Default)]
pub struct Keys { pub ids: HashMap<u32, String>, registered: Vec<HotKey>, pub failed: Vec<String> }

impl Keys {
    /// Re-register everything from settings. `failed` — combinations taken by another program.
    pub fn apply(&mut self, mgr: &GlobalHotKeyManager, map: &HashMap<String, String>) {
        let _ = mgr.unregister_all(&self.registered);
        self.registered.clear();
        self.ids.clear();
        self.failed.clear();
        for (action, _, _) in ACTIONS {
            let c = combo(map, action);
            let Some(hk) = parse(&c) else { continue };
            if self.registered.iter().any(|r| r.id() == hk.id()) { continue; }
            match mgr.register(hk) {
                Ok(()) => { self.ids.insert(hk.id(), action.to_string()); self.registered.push(hk); }
                Err(_) => self.failed.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        assert_eq!(parse("Ctrl+Alt+P"), Some(HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyP)));
        assert_eq!(parse("Ctrl+Shift+Right"), Some(HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::ArrowRight)));
        assert_eq!(parse("Alt+F5"), Some(HotKey::new(Some(Modifiers::ALT), Code::F5)));
        assert_eq!(parse("Ctrl+7"), Some(HotKey::new(Some(Modifiers::CONTROL), Code::Digit7)));
        assert_eq!(parse("P"), None);
        assert_eq!(parse(""), None);
        assert_eq!(from_egui(egui::Key::P, egui::Modifiers { ctrl: true, alt: true, ..Default::default() }).as_deref(), Some("Ctrl+Alt+P"));
        assert_eq!(from_egui(egui::Key::ArrowRight, egui::Modifiers { ctrl: true, ..Default::default() }).as_deref(), Some("Ctrl+Right"));
        assert_eq!(from_egui(egui::Key::P, egui::Modifiers::default()), None);
        for (a, _, d) in ACTIONS { assert!(parse(d).is_some(), "{a}"); }
    }
}
