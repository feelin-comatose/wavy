//! Иконка в трее (Windows): показать окно, пауза, следующий, выход.
//! «Свернуть в трей» — окно сворачивается и прячется с панели задач (стиль tool window), но остаётся
//! «видимым» для Windows — поэтому плеер продолжает работать (переключать треки, ловить горячие клавиши).
use eframe::egui;
use std::sync::mpsc::Receiver;

#[cfg_attr(not(windows), allow(dead_code))]
pub enum Cmd { Show, Toggle, Next, Prev, Quit }

#[cfg(windows)]
mod imp {
    use super::Cmd;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::mpsc::{channel, Receiver};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    static HWND_: AtomicIsize = AtomicIsize::new(0);

    pub struct Tray { _icon: TrayIcon }

    fn hwnd() -> windows_sys::Win32::Foundation::HWND { HWND_.load(Ordering::Relaxed) as _ }

    /// Вернуть окно: с панели задач, развернуть, на передний план.
    pub fn show_now() {
        let h = hwnd();
        if h.is_null() { return; }
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(h, GWL_EXSTYLE, (ex & !(WS_EX_TOOLWINDOW as isize)) | WS_EX_APPWINDOW as isize);
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
        }
    }

    /// Свернуть и убрать с панели задач.
    pub fn hide_now() {
        let h = hwnd();
        if h.is_null() { return; }
        unsafe {
            ShowWindow(h, SW_MINIMIZE);
            ShowWindow(h, SW_HIDE); // смена стиля применяется к скрытому окну
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(h, GWL_EXSTYLE, (ex & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize);
            ShowWindow(h, SW_SHOWMINNOACTIVE);
        }
    }

    pub fn new(cc: &eframe::CreationContext) -> Option<(Tray, Receiver<Cmd>)> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = cc.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() { HWND_.store(w.hwnd.get(), Ordering::Relaxed); }
        }
        let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.resize(32, 32, image::imageops::FilterType::Triangle).to_rgba8();
        let icon = tray_icon::Icon::from_rgba(img.to_vec(), img.width(), img.height()).ok()?;
        let menu = Menu::new();
        let show = MenuItem::new("Показать Wavy", true, None);
        let toggle = MenuItem::new("Пауза / играть", true, None);
        let next = MenuItem::new("Следующий трек", true, None);
        let prev = MenuItem::new("Предыдущий трек", true, None);
        let quit = MenuItem::new("Выход", true, None);
        menu.append_items(&[&show, &toggle, &next, &prev, &PredefinedMenuItem::separator(), &quit]).ok()?;
        let tray = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Wavy").with_icon(icon).build().ok()?;
        let (tx, rx) = channel();
        let ctx = cc.egui_ctx.clone();
        let ids = (show.id().clone(), toggle.id().clone(), next.id().clone(), prev.id().clone(), quit.id().clone());
        let (t2, c2) = (tx.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let cmd = if e.id == ids.0 { show_now(); Cmd::Show } else if e.id == ids.1 { Cmd::Toggle } else if e.id == ids.2 { Cmd::Next } else if e.id == ids.3 { Cmd::Prev } else if e.id == ids.4 { show_now(); Cmd::Quit } else { return };
            let _ = t2.send(cmd);
            c2.request_repaint();
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, .. } = e { show_now(); let _ = tx.send(Cmd::Show); ctx.request_repaint(); }
        }));
        Some((Tray { _icon: tray }, rx))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Cmd;
    use std::sync::mpsc::Receiver;
    pub struct Tray;
    pub fn show_now() {}
    pub fn hide_now() {}
    pub fn new(_cc: &eframe::CreationContext) -> Option<(Tray, Receiver<Cmd>)> { None }
}

pub use imp::Tray;

pub fn new(cc: &eframe::CreationContext) -> Option<(Tray, Receiver<Cmd>)> { imp::new(cc) }

/// Свернуть в трей (если трея нет — просто свернуть).
pub fn hide(ctx: &egui::Context) {
    if cfg!(windows) { imp::hide_now(); } else { ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
}

pub fn show(ctx: &egui::Context) {
    if cfg!(windows) { imp::show_now(); } else {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
}
