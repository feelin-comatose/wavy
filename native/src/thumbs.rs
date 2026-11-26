//! Cover cache: load and downscale in the background, keep a limited number of textures.
//! This is the main memory saving: 500 covers at 64px ≈ 8 MB instead of hundreds of MB.
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};

const MAX: usize = 300;

pub struct Thumbs {
    tex: HashMap<(String, u32), TextureHandle>,
    order: VecDeque<(String, u32)>,
    pending: HashSet<(String, u32)>,
    failed: HashSet<(String, u32)>,
    req: Sender<(String, u32)>,
    res: Receiver<((String, u32), Option<ColorImage>)>,
}

impl Thumbs {
    pub fn new(ctx: &egui::Context) -> Self {
        let (req_tx, req_rx) = channel::<(String, u32)>();
        let (res_tx, res_rx) = channel();
        let req_rx = std::sync::Arc::new(std::sync::Mutex::new(req_rx));
        for i in 0..3 {
            let (rx, tx, ctx) = (req_rx.clone(), res_tx.clone(), ctx.clone());
            std::thread::Builder::new().name(format!("thumbs-{i}")).spawn(move || loop {
                let Ok(key) = rx.lock().unwrap().recv() else { break };
                let img = load(&key.0, key.1);
                if tx.send((key, img)).is_err() { break; }
                ctx.request_repaint();
            }).unwrap();
        }
        Self { tex: HashMap::new(), order: VecDeque::new(), pending: HashSet::new(), failed: HashSet::new(), req: req_tx, res: res_rx }
    }

    /// Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, img)) = self.res.try_recv() {
            self.pending.remove(&key);
            match img {
                Some(img) => {
                    let h = ctx.load_texture(format!("{}@{}", key.0, key.1), img, TextureOptions::LINEAR);
                    self.tex.insert(key.clone(), h);
                    self.order.push_back(key);
                    while self.order.len() > MAX {
                        if let Some(old) = self.order.pop_front() { self.tex.remove(&old); }
                    }
                }
                None => { self.failed.insert(key); }
            }
        }
    }

    pub fn get(&mut self, src: &str, size: u32) -> Option<TextureHandle> { self.request(src.to_string(), size) }

    /// Whole poster, without the square crop (size — by the longer side).
    pub fn poster(&mut self, src: &str, size: u32) -> Option<TextureHandle> { self.request(format!("P|{src}"), size) }

    fn request(&mut self, src: String, size: u32) -> Option<TextureHandle> {
        let key = (src, size);
        if let Some(t) = self.tex.get(&key) { return Some(t.clone()); }
        if !self.pending.contains(&key) && !self.failed.contains(&key) {
            self.pending.insert(key.clone());
            let _ = self.req.send(key);
        }
        None
    }
}

fn load(src: &str, size: u32) -> Option<ColorImage> {
    let (src, whole) = match src.strip_prefix("P|") { Some(s) => (s, true), None => (src, false) };
    let bytes = if src.starts_with("http") {
        let mut r = wavy_core::http().get(src).call().ok()?;
        r.body_mut().with_config().limit(10 * 1024 * 1024).read_to_vec().ok()?
    } else {
        std::fs::read(src).ok()?
    };
    let img = image::load_from_memory(&bytes).ok()?;
    if whole {
        let img = img.thumbnail(size, size).to_rgba8();
        return Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()));
    }
    // center square crop (YouTube 4:3 thumbnails with black bars)
    let (w, h) = (img.width(), img.height());
    let s = w.min(h);
    let img = if src.contains("ytimg.com") && w > h {
        let s2 = (h as f32 * 0.75) as u32; // hq/default thumbnails have bars at the top and bottom
        img.crop_imm((w - s2) / 2, (h - s2) / 2, s2, s2)
    } else {
        img.crop_imm((w - s) / 2, (h - s) / 2, s, s)
    };
    let img = img.thumbnail(size, size).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}
