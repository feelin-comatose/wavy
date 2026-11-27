//! "For you" tab: YouTube-style rows of covers with recommendations (data: wavy_core::recs).
//! Rows load in the background, each on its own; the page is kept for a few hours and
//! loads again when opened after that (or with "Refresh").
use super::*;
use wavy_core::recs::{self, Card, CardKind};

/// How long the loaded rows stay before the tab loads new ones on opening.
const FRESH: Duration = Duration::from_secs(3 * 3600);
/// Card (cover) size.
const CW: f32 = 150.0;
const SLOT_WAVE: u32 = 10;
const SLOT_NEW: u32 = 20;
const SLOT_CHARTS: u32 = 30;
/// scroll position key of the mood row
const SLOT_MOOD: u32 = 40;

pub struct Row {
    pub slot: u32,
    pub title: String,
    pub hint: String,
    /// None — still loading
    pub cards: Option<Result<Vec<Card>, String>>,
}

#[derive(Default)]
pub struct ForYou {
    /// generation: answers to an older refresh are ignored
    pub gen: u64,
    pub at: Option<Instant>,
    pub rows: Vec<Row>,
    /// picked mood (index in recs::MOODS) and its playlists (None — loading)
    pub mood: Option<usize>,
    pub mood_cards: Option<Result<Vec<Card>, String>>,
    /// album or playlist being fetched to play or save (card id)
    pub opening: Option<String>,
    /// row slot → (scroll offset, content width, visible width); arrow clicks → where to scroll
    scroll: HashMap<u32, (f32, f32, f32)>,
    jump: HashMap<u32, f32>,
}

impl App {
    /// Opening the tab: load the rows if there are none yet or they're old.
    pub(crate) fn foryou_open(&mut self, ctx: &egui::Context) {
        if self.foryou.at.is_none_or(|t| t.elapsed() > FRESH) { self.foryou_refresh(ctx); }
    }

    pub(crate) fn foryou_refresh(&mut self, ctx: &egui::Context) {
        let ru = !wavy_core::i18n::en();
        self.foryou.gen += 1;
        let gen = self.foryou.gen;
        self.foryou.at = Some(Instant::now());
        self.foryou.rows.clear();
        self.foryou.opening = None;
        self.foryou.scroll.clear();
        let (seeds, exclude) = {
            let s = self.store.lock().unwrap();
            (recs::seeds(&s.lib, wavy_core::now_ms(), 3, gen - 1), s.lib.tracks.values().filter_map(|t| t.video_id.clone()).collect::<HashSet<String>>())
        };
        let exclude = Arc::new(exclude);
        let spawn = |slot: u32, job: Box<dyn FnOnce() -> Result<Vec<Card>, String> + Send>, tx: Sender<Bg>, c: egui::Context| {
            std::thread::spawn(move || { let _ = tx.send(Bg::Rec(gen, slot, job())); c.request_repaint(); });
        };
        for (i, seed) in seeds.into_iter().enumerate() {
            let name = if seed.artist.is_empty() { seed.title.clone() } else { format!("{} — {}", seed.artist, seed.title) };
            self.foryou.rows.push(Row { slot: i as u32, title: lf!("Похоже на «{}»", "Because you listened to “{}”", seed.title), hint: lf!("Похожие песни на {name}, которых нет в библиотеке", "Songs similar to {name} that aren't in your library"), cards: None });
            let (yt, ex) = (self.yt.clone(), exclude.clone());
            spawn(i as u32, Box::new(move || {
                let vid = yt.seed_video(&seed).ok_or_else(|| l!("не нашёл трек на YouTube", "couldn't find the track on YouTube").to_string())?;
                Ok(yt.similar(&seed, &vid, &ex, 20)?.into_iter().map(Card::track).collect())
            }), self.tx.clone(), ctx.clone());
        }
        if self.acc == AccState::On {
            self.foryou.rows.push(Row { slot: SLOT_WAVE, title: l!("Миксы для тебя", "Mixed for you").into(), hint: l!("Начало «Моей волны»: по лайкам и истории YouTube Music", "The start of “My wave”: from your YouTube Music likes and history").into(), cards: None });
            let (yt, ex) = (self.yt.clone(), exclude.clone());
            spawn(SLOT_WAVE, Box::new(move || Ok(yt.wave(&[], &ex, 20)?.into_iter().map(Card::track).collect())), self.tx.clone(), ctx.clone());
        }
        self.foryou.rows.push(Row { slot: SLOT_NEW, title: l!("Новинки", "New releases").into(), hint: l!("Новые песни недели на YouTube Music", "New songs of the week on YouTube Music").into(), cards: None });
        let yt = self.yt.clone();
        spawn(SLOT_NEW, Box::new(move || yt.new_releases(ru)), self.tx.clone(), ctx.clone());
        self.foryou.rows.push(Row { slot: SLOT_CHARTS, title: l!("Чарты", "Charts").into(), hint: l!("Хит-парады YouTube: топ клипов, тренды, жанры", "YouTube charts: top videos, trending, genres").into(), cards: None });
        let yt = self.yt.clone();
        spawn(SLOT_CHARTS, Box::new(move || yt.charts(ru)), self.tx.clone(), ctx.clone());
        if let Some(m) = self.foryou.mood { self.foryou_mood(m, ctx); }
    }

    pub(crate) fn foryou_mood(&mut self, i: usize, ctx: &egui::Context) {
        let Some((_, q)) = recs::MOODS.get(i) else { return };
        self.foryou.mood = Some(i);
        self.foryou.mood_cards = None;
        self.foryou.jump.insert(SLOT_MOOD, 0.0);
        let (yt, tx, c, gen, ru) = (self.yt.clone(), self.tx.clone(), ctx.clone(), self.foryou.gen, !wavy_core::i18n::en());
        std::thread::spawn(move || { let _ = tx.send(Bg::RecMood(gen, i, yt.mood_playlists(q, ru))); c.request_repaint(); });
    }

    /// Tracks from the rows can be played, queued and added like search results.
    pub(crate) fn foryou_pool(&mut self, cards: &[Card]) {
        let list: Vec<Track> = cards.iter().filter_map(|c| if let CardKind::Track(t) = &c.kind { Some((**t).clone()) } else { None }).collect();
        if list.is_empty() { return; }
        let ids: HashSet<&str> = list.iter().map(|t| t.id.as_str()).collect();
        self.radio.retain(|t| !ids.contains(t.id.as_str()));
        self.radio.extend(list);
    }

    pub(crate) fn foryou_row_done(&mut self, gen: u64, slot: u32, r: Result<Vec<Card>, String>) {
        if gen != self.foryou.gen { return; }
        if let Err(e) = &r { wavy_core::log(&format!("для тебя, ряд {slot}: {e}")); }
        if let Ok(cards) = &r { let cards = cards.clone(); self.foryou_pool(&cards); }
        if let Some(row) = self.foryou.rows.iter_mut().find(|x| x.slot == slot) { row.cards = Some(r); }
    }

    pub(crate) fn foryou_mood_done(&mut self, gen: u64, i: usize, r: Result<Vec<Card>, String>) {
        if gen != self.foryou.gen || self.foryou.mood != Some(i) { return; }
        self.foryou.mood_cards = Some(r);
    }

    /// Album or playlist fetched: play it or save it as a playlist (synced by its link).
    pub(crate) fn foryou_opened(&mut self, id: String, save: bool, r: Result<(String, String, Vec<Track>), String>, ctx: &egui::Context) {
        if self.foryou.opening.as_deref() == Some(id.as_str()) { self.foryou.opening = None; }
        match r {
            Ok((_, _, tracks)) if tracks.is_empty() => self.toast(l!("Там пусто", "It's empty"), true),
            Ok((name, link, tracks)) if save => {
                let pid = save_ytm_playlist(&self.store, &link, &name, tracks);
                self.toast(lf!("Плейлист «{name}» сохранён", "Playlist “{name}” saved"), false);
                self.apply(Act::View(View::Playlist(pid)), ctx);
            }
            Ok((_, _, tracks)) => {
                let cards: Vec<Card> = tracks.into_iter().map(Card::track).collect();
                self.foryou_pool(&cards);
                let ids = cards.into_iter().map(|c| c.id).collect();
                self.play_list(ids, 0, false, ctx);
                self.wave_on = false;
            }
            Err(e) => self.toast(e, true),
        }
    }

    pub(crate) fn foryou_page(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("YOUTUBE MUSIC").size(11.0).color(th().muted));
                ui.label(RichText::new(l!("Для тебя", "For you")).size(34.0).family(bold()).color(th().text));
                ui.label(RichText::new(l!("Подборки по тому, что ты слушаешь, новинки и чарты", "Picks based on what you listen to, new releases and charts")).color(th().muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let busy = self.foryou.rows.iter().any(|r| r.cards.is_none());
                if ui.add_enabled(!busy, pill_btn(if busy { l!("Загружаю…", "Loading…") } else { l!("\u{E036}  Обновить", "\u{E036}  Refresh") }, false)).clicked() { acts.push(Act::ForYou); }
            });
        });
        ui.add_space(10.0);
        let pls: Vec<(String, String)> = self.store.lock().unwrap().lib.playlists.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        egui::ScrollArea::vertical().id_salt("foryou").auto_shrink([false, false]).show(ui, |ui| {
            if !self.foryou.rows.iter().any(|r| r.slot < SLOT_WAVE) {
                egui::Frame::new().fill(th().bg3).corner_radius(12).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(l!("Послушай что-нибудь из библиотеки — и здесь появятся подборки по твоему вкусу", "Play something from your library, and picks based on your taste will show up here")).color(th().muted));
                });
                ui.add_space(6.0);
            }
            let rows = std::mem::take(&mut self.foryou.rows);
            for row in &rows {
                // a seed row that found nothing new is just left out
                if row.slot < SLOT_WAVE && matches!(&row.cards, Some(Ok(v)) if v.is_empty()) { continue; }
                let wave = row.slot == SLOT_WAVE;
                self.rec_row(ui, row.slot, &row.title, &row.hint, row.cards.as_ref(), wave, &pls, acts);
            }
            self.foryou.rows = rows;
            // moods and genres
            ui.add_space(18.0);
            ui.label(RichText::new(l!("Настроения и жанры", "Moods and genres")).size(19.0).family(bold()).color(th().text));
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                for (i, (name, _)) in recs::MOODS.iter().enumerate() {
                    let on = self.foryou.mood == Some(i);
                    let b = egui::Button::new(RichText::new(wavy_core::i18n::tr(name)).color(if on { th().on_accent } else { th().text })).corner_radius(99).min_size(Vec2::new(0.0, 30.0));
                    let b = if on { b.fill(th().accent).stroke(Stroke::NONE) } else { b.fill(th().bg3).stroke(Stroke::new(1.0, th().line)) };
                    if ui.add(b).clicked() { acts.push(Act::RecMood(i)); }
                }
            });
            if let Some(m) = self.foryou.mood {
                let cards = self.foryou.mood_cards.clone();
                let title = wavy_core::i18n::tr(recs::MOODS[m].0).to_string();
                self.rec_row(ui, SLOT_MOOD, &title, "", cards.as_ref(), false, &pls, acts);
            }
            ui.add_space(24.0);
        });
    }

    /// A row: title, arrows, covers scrolled sideways.
    #[allow(clippy::too_many_arguments)]
    fn rec_row(&mut self, ui: &mut egui::Ui, slot: u32, title: &str, hint: &str, cards: Option<&Result<Vec<Card>, String>>, wave: bool, pls: &[(String, String)], acts: &mut Vec<Act>) {
        let tracks: Vec<Track> = cards.and_then(|r| r.as_ref().ok()).map(|v| v.iter().filter_map(|c| if let CardKind::Track(t) = &c.kind { Some((**t).clone()) } else { None }).collect()).unwrap_or_default();
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            let r = ui.label(RichText::new(title).size(19.0).family(bold()).color(th().text));
            if !hint.is_empty() { r.on_hover_text(hint); }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (off, content, view) = self.foryou.scroll.get(&slot).copied().unwrap_or((0.0, 0.0, 0.0));
                let step = (view - CW).max(CW);
                if ui.add_enabled(off + view < content - 1.0, egui::Button::new("\u{E13A}").corner_radius(99).min_size(Vec2::splat(30.0))).clicked() { self.foryou.jump.insert(slot, (off + step).min(content - view)); }
                if ui.add_enabled(off > 1.0, egui::Button::new("\u{E138}").corner_radius(99).min_size(Vec2::splat(30.0))).clicked() { self.foryou.jump.insert(slot, (off - step).max(0.0)); }
                if !tracks.is_empty() {
                    let label = if wave { l!("\u{E3D0}  Включить волну", "\u{E3D0}  Start the wave") } else { l!("\u{E3D0}  Слушать всё", "\u{E3D0}  Play all") };
                    if pill(ui, label, wave).clicked() { acts.push(Act::RecPlay(tracks.clone(), 0, wave)); }
                }
            });
        });
        ui.add_space(8.0);
        let cards = match cards {
            None => {
                // loading: grey placeholders in place of the covers
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    let n = ((ui.available_width() + 16.0) / (CW + 16.0)).floor().max(1.0) as usize;
                    for _ in 0..n {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(CW, CW + 44.0), Sense::hover());
                        ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::splat(CW)), 10, th().bg3);
                        ui.painter().rect_filled(Rect::from_min_size(r.min + Vec2::new(0.0, CW + 10.0), Vec2::new(CW * 0.8, 10.0)), 4, th().bg3);
                        ui.painter().rect_filled(Rect::from_min_size(r.min + Vec2::new(0.0, CW + 28.0), Vec2::new(CW * 0.5, 9.0)), 4, th().bg3);
                    }
                });
                return;
            }
            Some(Err(e)) => { ui.label(RichText::new(e).color(th().muted)); return; }
            Some(Ok(v)) if v.is_empty() => { ui.label(RichText::new(l!("Ничего не нашлось", "Nothing found")).color(th().muted)); return; }
            Some(Ok(v)) => v,
        };
        let cur = self.current.as_ref().map(|t| t.id.clone());
        let mut area = egui::ScrollArea::horizontal().id_salt(("rec-row", slot)).auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        if let Some(x) = self.foryou.jump.remove(&slot) { area = area.horizontal_scroll_offset(x); }
        let out = area.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let mut ti = 0;
                for c in cards {
                    let idx = if matches!(c.kind, CardKind::Track(_)) { ti += 1; Some(ti - 1) } else { None };
                    self.rec_card(ui, c, idx, &tracks, cur.as_deref(), wave, pls, acts);
                }
            });
        });
        self.foryou.scroll.insert(slot, (out.state.clamped_offset().x, out.content_size.x, out.inner_rect.width()));
    }

    #[allow(clippy::too_many_arguments)]
    fn rec_card(&mut self, ui: &mut egui::Ui, c: &Card, idx: Option<usize>, tracks: &[Track], cur: Option<&str>, wave: bool, pls: &[(String, String)], acts: &mut Vec<Act>) {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(CW, CW + 44.0), Sense::click());
        if !ui.is_rect_visible(r) { return; } // covers off screen aren't loaded
        let cr = Rect::from_min_size(r.min, Vec2::splat(CW));
        match c.cover.as_deref().and_then(|u| self.thumbs.get(u, 160)) {
            Some(tex) => { egui::Image::new(&tex).corner_radius(10).paint_at(ui, cr); }
            None => {
                ui.painter().rect_filled(cr, 10, th().bg3);
                let icon = match c.kind { CardKind::Track(_) => "\u{E33C}", _ => "\u{E340}" };
                ui.painter().text(cr.center(), Align2::CENTER_CENTER, icon, FontId::proportional(40.0), th().muted);
            }
        }
        let playing = matches!(&c.kind, CardKind::Track(t) if cur == Some(t.id.as_str()));
        let opening = self.foryou.opening.as_deref() == Some(c.id.as_str());
        if resp.hovered() || playing || opening {
            ui.painter().rect_filled(cr, 10, Color32::from_black_alpha(if resp.hovered() { 110 } else { 70 }));
            let b = Rect::from_center_size(cr.right_bottom() - Vec2::splat(26.0), Vec2::splat(36.0));
            ui.painter().circle_filled(b.center(), 18.0, th().accent);
            if opening {
                egui::Spinner::new().size(18.0).color(th().on_accent).paint_at(ui, Rect::from_center_size(b.center(), Vec2::splat(18.0)));
            } else {
                glyph(ui.painter(), b.center(), if playing && !resp.hovered() { "\u{E340}" } else { "\u{E3D0}" }, FontId::new(16.0, egui::FontFamily::Name("fill".into())), th().on_accent);
            }
        }
        if resp.hovered() { ui.painter().rect_stroke(cr, 10, Stroke::new(2.0, th().text), egui::StrokeKind::Outside); }
        ui.painter().text(cr.left_bottom() + Vec2::new(0.0, 8.0), Align2::LEFT_TOP, ellipsize(c.short_title(), CW - 4.0, 13.5), FontId::new(13.5, semi()), if playing { th().accent } else { th().text });
        let kind = match c.kind { CardKind::Album => l!("Альбом", "Album"), CardKind::Playlist => l!("Плейлист", "Playlist"), CardKind::Track(_) => "" };
        let sub = match (kind.is_empty(), c.subtitle.is_empty()) { (true, _) => c.subtitle.clone(), (false, true) => kind.to_string(), (false, false) => format!("{kind} · {}", c.subtitle) };
        ui.painter().text(cr.left_bottom() + Vec2::new(0.0, 27.0), Align2::LEFT_TOP, ellipsize(wavy_core::i18n::tr(&sub), CW - 4.0, 12.0), FontId::proportional(12.0), th().muted);
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if c.subtitle.is_empty() { c.title.clone() } else { format!("{}\n{}", c.title, c.subtitle) });
        match &c.kind {
            CardKind::Track(t) => {
                if resp.clicked() { acts.push(Act::RecPlay(tracks.to_vec(), idx.unwrap_or(0), wave)); }
                let in_lib = self.store.lock().unwrap().lib.tracks.contains_key(&t.id);
                let t = (**t).clone();
                resp.context_menu(|ui| {
                    let ids = vec![t.id.clone()];
                    if ui.button(l!("Играть далее", "Play next")).clicked() { acts.push(Act::PlayNext(ids.clone())); ui.close(); }
                    if ui.button(l!("Добавить в очередь", "Add to queue")).clicked() { acts.push(Act::Enqueue(ids.clone())); ui.close(); }
                    ui.separator();
                    if ui.add_enabled(!in_lib, egui::Button::new(if in_lib { l!("Уже в библиотеке", "Already in library") } else { l!("+ В библиотеку", "+ To library") })).clicked() { acts.push(Act::AddTracks(vec![t.clone()])); ui.close(); }
                    ui.menu_button(l!("Добавить в плейлист", "Add to playlist"), |ui| {
                        if ui.button(l!("+ Новый плейлист…", "+ New playlist…")).clicked() { acts.push(Act::Modal(Modal::NewPlaylist { name: String::new(), ids: ids.clone() })); ui.close(); }
                        if !pls.is_empty() { ui.separator(); }
                        for (id, name) in pls { if ui.button(name).clicked() { acts.push(Act::AddTo(id.clone(), ids.clone())); ui.close(); } }
                    });
                    if ui.button(l!("Радио по треку", "Track radio")).on_hover_text(l!("Похожие песни с YouTube — в очередь", "Similar songs from YouTube — into the queue")).clicked() { acts.push(Act::Radio(t.id.clone())); ui.close(); }
                    if let Some(u) = &t.url { if ui.button(l!("Открыть в браузере", "Open in browser")).clicked() { acts.push(Act::OpenUrl(u.clone())); ui.close(); } }
                });
            }
            _ => {
                if resp.clicked() && !opening { acts.push(Act::RecOpen(c.clone(), false)); }
                let c = c.clone();
                resp.context_menu(|ui| {
                    if ui.button(l!("Слушать", "Play")).clicked() { acts.push(Act::RecOpen(c.clone(), false)); ui.close(); }
                    if ui.button(l!("Сохранить как плейлист", "Save as a playlist")).on_hover_text(l!("Появится в плейлистах, обновляется кнопкой «Синхронизировать»", "Shows up among your playlists; the Sync button updates it")).clicked() { acts.push(Act::RecOpen(c.clone(), true)); ui.close(); }
                    let url = match c.kind { CardKind::Album => format!("https://music.youtube.com/browse/{}", c.id), _ => format!("https://music.youtube.com/playlist?list={}", c.id) };
                    if ui.button(l!("Открыть в браузере", "Open in browser")).clicked() { acts.push(Act::OpenUrl(url)); ui.close(); }
                });
            }
        }
    }
}
