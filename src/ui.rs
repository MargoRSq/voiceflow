use crate::store::{self, Entry, Stats};
use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use eframe::egui::{
    self, Color32, CornerRadius, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};
use rusqlite::Connection;

const BG: Color32 = Color32::from_rgb(12, 13, 16);
const RAIL: Color32 = Color32::from_rgb(16, 18, 22);
const CARD: Color32 = Color32::from_rgb(22, 24, 29);
const CARD_HOVER: Color32 = Color32::from_rgb(28, 31, 38);
const LINE: Color32 = Color32::from_rgb(36, 39, 47);
const TEXT: Color32 = Color32::from_rgb(242, 244, 248);
const MUTED: Color32 = Color32::from_rgb(138, 144, 160);
const FAINT: Color32 = Color32::from_rgb(90, 95, 110);
const ACCENT: Color32 = Color32::from_rgb(110, 168, 255);
const MINT: Color32 = Color32::from_rgb(123, 224, 192);

const RAIL_W: f32 = 216.0;
const PAGE_PAD: f32 = 28.0;
const LIMIT: usize = 500;
const RELOAD_EVERY: f64 = 2.0;

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    History,
    Stats,
}

pub struct App {
    conn: Connection,
    tab: Tab,
    query: String,
    entries: Vec<Entry>,
    stats: Stats,
    daily: Vec<(NaiveDate, i64)>,
    chart_days: i64,
    last_load: f64,
    shown: bool,
    toast: Option<(String, f64)>,
}

impl App {
    pub fn new(conn: Connection, tab: Tab) -> Self {
        let mut app = Self {
            conn,
            tab,
            query: String::new(),
            entries: vec![],
            stats: Stats::default(),
            daily: vec![],
            chart_days: 7,
            last_load: f64::NEG_INFINITY,
            shown: false,
            toast: None,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        match store::list(&self.conn, &self.query, LIMIT) {
            Ok(e) => self.entries = e,
            Err(e) => eprintln!("ui: list failed: {e}"),
        }
        match store::stats(&self.conn) {
            Ok(s) => self.stats = s,
            Err(e) => eprintln!("ui: stats failed: {e}"),
        }
        match store::daily_words(&self.conn, self.chart_days) {
            Ok(d) => self.daily = d,
            Err(e) => eprintln!("ui: daily failed: {e}"),
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        BG.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        crate::shot::tick(&ctx);

        // Mutter hands XWayland clients back as iconified here, so the window
        // never appears until it asks for itself.
        if !self.shown {
            self.shown = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // Pick up dictations made while this window is open.
        if now - self.last_load > RELOAD_EVERY {
            self.last_load = now;
            self.reload();
        }

        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, BG);

        let rail = Rect::from_min_size(full.min, vec2(RAIL_W, full.height()));
        let page = Rect::from_min_max(pos2(rail.right(), full.top()), full.max);

        self.rail(ui, rail);
        self.page(ui, page, now);
        self.toast(ui, full, now);
    }
}

impl App {
    fn rail(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter();
        p.rect_filled(rect, 0.0, RAIL);
        p.line_segment(
            [rect.right_top(), rect.right_bottom()],
            Stroke::new(1.0, LINE),
        );

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(16.0, 22.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );

        child.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.5, ACCENT);
            ui.add_space(2.0);
            ui.label(RichText::new("voiceflow").size(19.0).color(TEXT).strong());
        });
        child.add_space(2.0);
        child.label(RichText::new("голос в текст").size(11.5).color(FAINT));
        child.add_space(26.0);

        for (tab, name, hint) in [
            (Tab::History, "История", "все диктовки"),
            (Tab::Stats, "Статистика", "цифры и график"),
        ] {
            if nav_item(&mut child, self.tab == tab, name, hint) {
                self.tab = tab;
                self.last_load = f64::NEG_INFINITY;
            }
            child.add_space(6.0);
        }

        // Today's summary pinned to the bottom of the rail.
        let s = self.stats.clone();
        let bottom = Rect::from_min_max(
            pos2(rect.left() + 16.0, rect.bottom() - 138.0),
            pos2(rect.right() - 16.0, rect.bottom() - 20.0),
        );
        let mut b = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bottom)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        egui::Frame::NONE
            .fill(CARD)
            .corner_radius(12.0)
            .inner_margin(14)
            .stroke(Stroke::new(1.0, LINE))
            .show(&mut b, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("СЕГОДНЯ").size(10.0).color(FAINT));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(group(s.words_today))
                        .size(26.0)
                        .color(TEXT)
                        .strong(),
                );
                ui.label(RichText::new("слов").size(11.5).color(MUTED));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("{} диктовок · серия {}", s.sessions_today, s.streak))
                        .size(11.5)
                        .color(MUTED),
                );
            });
    }

    fn page(&mut self, ui: &mut egui::Ui, rect: Rect, now: f64) {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink(PAGE_PAD))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        match self.tab {
            Tab::History => self.history(&mut child, now),
            Tab::Stats => self.stats_page(&mut child),
        }
    }

    fn history(&mut self, ui: &mut egui::Ui, now: f64) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("История").size(24.0).color(TEXT).strong());
            ui.add_space(10.0);
            ui.label(
                RichText::new(group(self.stats.total))
                    .size(13.0)
                    .color(FAINT),
            );
        });
        ui.add_space(16.0);

        egui::Frame::NONE
            .fill(CARD)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .stroke(Stroke::new(1.0, LINE))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (ic, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    let c = ic.center() - vec2(1.0, 1.0);
                    ui.painter().circle_stroke(c, 5.0, Stroke::new(1.4, FAINT));
                    ui.painter().line_segment(
                        [c + vec2(3.6, 3.6), c + vec2(7.0, 7.0)],
                        Stroke::new(1.4, FAINT),
                    );
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .frame(egui::Frame::NONE)
                            .desired_width(ui.available_width() - 26.0)
                            .hint_text(RichText::new("поиск по тексту").color(FAINT)),
                    );
                    if r.changed() {
                        self.last_load = f64::NEG_INFINITY;
                    }
                    if !self.query.is_empty()
                        && ui
                            .add(egui::Button::new(RichText::new("×").color(MUTED)).frame(false))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                    {
                        self.query.clear();
                        self.last_load = f64::NEG_INFINITY;
                    }
                });
            });
        ui.add_space(14.0);

        if self.entries.is_empty() {
            empty(
                ui,
                if self.query.is_empty() {
                    ("Пока пусто", "Нажми Alt+Shift и продиктуй что-нибудь")
                } else {
                    ("Ничего не нашлось", "Попробуй другой запрос")
                },
            );
            return;
        }

        let mut to_delete = None;
        let mut to_copy = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut last_day: Option<NaiveDate> = None;
                for e in &self.entries {
                    let day = e.local().date_naive();
                    if last_day != Some(day) {
                        if last_day.is_some() {
                            ui.add_space(16.0);
                        }
                        day_header(ui, day);
                        ui.add_space(8.0);
                        last_day = Some(day);
                    }
                    match entry_card(ui, e) {
                        Action::Copy => to_copy = Some(e.text.clone()),
                        Action::Delete => to_delete = Some(e.id),
                        Action::None => {}
                    }
                    ui.add_space(7.0);
                }
                ui.add_space(10.0);
            });

        if let Some(text) = to_copy {
            self.toast = Some(match crate::inject::to_clipboard(&text) {
                Ok(()) => ("Скопировано".into(), now),
                Err(e) => (format!("Не скопировалось: {e}"), now),
            });
        }
        if let Some(id) = to_delete {
            if let Err(e) = store::delete(&self.conn, id) {
                eprintln!("ui: delete failed: {e}");
            }
            self.last_load = f64::NEG_INFINITY;
        }
    }

    fn stats_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Статистика").size(24.0).color(TEXT).strong());
        if self.stats.total == 0 {
            empty(
                ui,
                ("Считать пока нечего", "Появится после первой диктовки"),
            );
            return;
        }
        let s = self.stats.clone();
        ui.add_space(16.0);
        let w = ui.max_rect().width() - 16.0; // leave room for the scrollbar

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                hero(ui, w, &s);
                ui.add_space(12.0);

                let gap = ui.spacing().item_spacing.x;
                let tile_w = (w - gap * 2.0) / 3.0;
                ui.horizontal(|ui| {
                    tile(ui, tile_w, "Всего слов", &group(s.total_words), &format!("+{} сегодня", s.words_today), ACCENT);
                    tile(ui, tile_w, "Диктовок", &group(s.total), &format!("в среднем {:.0} слов", s.avg_words), ACCENT);
                    tile(ui, tile_w, "Серия дней", &group(s.streak), &format!("рекорд {}", s.best_streak), MINT);
                });

                ui.add_space(12.0);
                self.chart(ui, w);
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    tile(ui, tile_w, "Пик активности", &s.peak_hour.map_or("—".into(), |h| format!("{h:02}:00")), "чаще всего диктуешь", MUTED);
                    tile(ui, tile_w, "Самая длинная", &group(s.longest_words), "слов за раз", MUTED);
                    tile(ui, tile_w, "Лучший день", &group(s.most_words_day), &format!("слов, {} диктовок", s.most_sessions_day), MUTED);
                });
                ui.add_space(18.0);
            });
    }

    fn chart(&mut self, ui: &mut egui::Ui, w: f32) {
        egui::Frame::NONE
            .fill(CARD)
            .corner_radius(14.0)
            .inner_margin(18)
            .stroke(Stroke::new(1.0, LINE))
            .show(ui, |ui| {
                ui.set_width(w - 36.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Слова по дням").size(15.0).color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(112.0, 26.0), Sense::hover());
                        ui.painter().rect_filled(rect, 8.0, BG);
                        for (i, d) in [7i64, 30].into_iter().enumerate() {
                            let seg = Rect::from_min_size(
                                rect.left_top() + vec2(3.0 + i as f32 * 53.0, 3.0),
                                vec2(53.0, 20.0),
                            );
                            let on = self.chart_days == d;
                            let resp = ui
                                .interact(seg, egui::Id::new(("seg", d)), Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if on {
                                ui.painter().rect_filled(seg, 6.0, CARD_HOVER);
                            }
                            ui.painter().text(
                                seg.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("{d} дн"),
                                egui::FontId::proportional(11.5),
                                if on { TEXT } else { MUTED },
                            );
                            if resp.clicked() {
                                self.chart_days = d;
                                self.last_load = f64::NEG_INFINITY;
                            }
                        }
                    });
                });
                ui.add_space(12.0);

                let h = 148.0;
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
                let p = ui.painter();
                let plot = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 20.0));
                let max = self.daily.iter().map(|(_, w)| *w).max().unwrap_or(0).max(1);

                for k in 0..=3 {
                    let y = plot.bottom() - plot.height() * k as f32 / 3.0;
                    p.line_segment(
                        [pos2(plot.left(), y), pos2(plot.right(), y)],
                        Stroke::new(1.0, Color32::from_white_alpha(8)),
                    );
                }
                p.text(
                    pos2(plot.left(), plot.top() - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    group(max),
                    egui::FontId::proportional(10.0),
                    FAINT,
                );

                let n = self.daily.len().max(1);
                let pitch = plot.width() / n as f32;
                let bar_w = (pitch - 6.0).clamp(3.0, 30.0);
                let peak = self
                    .daily
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, (_, w))| *w)
                    .map(|(i, _)| i);

                for (i, (date, words)) in self.daily.iter().enumerate() {
                    let frac = *words as f32 / max as f32;
                    let bh = (frac * plot.height()).max(if *words > 0 { 4.0 } else { 2.0 });
                    let cx = plot.left() + (i as f32 + 0.5) * pitch;
                    let bar =
                        Rect::from_min_size(pos2(cx - bar_w / 2.0, plot.bottom() - bh), vec2(bar_w, bh));

                    if *words == 0 {
                        p.rect_filled(bar, 2.0, LINE);
                    } else {
                        let tone = if peak == Some(i) { MINT } else { ACCENT };
                        // Stacked bands fake a vertical gradient; a mesh would
                        // need its own rounded-corner handling for one bar.
                        const BANDS: usize = 5;
                        for b in 0..BANDS {
                            let frac = 1.0 - b as f32 / BANDS as f32;
                            let seg = Rect::from_min_size(
                                bar.min,
                                vec2(bar.width(), (bar.height() * frac).max(4.0)),
                            );
                            // Later bands are shorter and sit on top, so they
                            // must be the bright end of the ramp.
                            let lit = 0.40 + 0.60 * (b as f32 / (BANDS - 1) as f32);
                            p.rect_filled(seg, CornerRadius::same(4), tone.gamma_multiply(lit));
                        }
                    }

                    let step = (n as f32 / 7.0).ceil().max(1.0) as usize;
                    if i % step == 0 || peak == Some(i) {
                        p.text(
                            pos2(cx, rect.bottom() - 8.0),
                            egui::Align2::CENTER_CENTER,
                            format!("{}.{:02}", date.day(), date.month()),
                            egui::FontId::proportional(10.0),
                            if peak == Some(i) { MUTED } else { FAINT },
                        );
                    }
                }

                let total: i64 = self.daily.iter().map(|(_, w)| w).sum();
                let active = self.daily.iter().filter(|(_, w)| *w > 0).count();
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "{} слов за период · активных дней {}",
                        group(total),
                        active
                    ))
                    .size(11.5)
                    .color(MUTED),
                );
            });
    }

    fn toast(&mut self, ui: &mut egui::Ui, full: Rect, now: f64) {
        let Some((msg, at)) = self.toast.clone() else {
            return;
        };
        if now - at > 2.0 {
            self.toast = None;
            return;
        }
        let fade = (((2.0 - (now - at)) / 0.4) as f32).clamp(0.0, 1.0);
        let rect =
            Rect::from_center_size(pos2(full.center().x, full.bottom() - 52.0), vec2(230.0, 40.0));
        let p = ui.painter();
        p.rect_filled(rect, 12.0, CARD_HOVER.gamma_multiply(fade));
        p.rect_stroke(
            rect,
            12.0,
            Stroke::new(1.0, LINE.gamma_multiply(fade)),
            StrokeKind::Inside,
        );
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            msg,
            egui::FontId::proportional(13.5),
            TEXT.gamma_multiply(fade),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(60));
    }
}

fn nav_item(ui: &mut egui::Ui, active: bool, name: &str, hint: &str) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 46.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();

    if active {
        p.rect_filled(rect, 10.0, CARD_HOVER);
        p.rect_filled(
            Rect::from_min_size(rect.left_top() + vec2(0.0, 11.0), vec2(3.0, 24.0)),
            2.0,
            ACCENT,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, 10.0, CARD);
    }

    p.text(
        pos2(rect.left() + 14.0, rect.center().y - 7.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(14.0),
        if active { TEXT } else { MUTED },
    );
    p.text(
        pos2(rect.left() + 14.0, rect.center().y + 9.0),
        egui::Align2::LEFT_CENTER,
        hint,
        egui::FontId::proportional(10.5),
        FAINT,
    );
    resp.clicked()
}

enum Action {
    None,
    Copy,
    Delete,
}

/// The card background is painted after the content so it can react to hover —
/// egui's placeholder-shape trick.
fn entry_card(ui: &mut egui::Ui, e: &Entry) -> Action {
    let bg = ui.painter().add(Shape::Noop);
    let mut action = Action::None;

    let inner = egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(e.local().format("%H:%M").to_string())
                        .size(12.5)
                        .color(MUTED),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "{} слов · {:.1} с",
                        e.words,
                        e.duration_ms as f64 / 1000.0
                    ))
                    .size(11.5)
                    .color(FAINT),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Удалить").size(12.0).color(FAINT))
                                .frame(false),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        action = Action::Delete;
                    }
                    ui.add_space(6.0);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Копировать").size(12.0).color(ACCENT),
                            )
                            .frame(false),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        action = Action::Copy;
                    }
                });
            });
            ui.add_space(5.0);
            ui.add(
                egui::Label::new(RichText::new(&e.text).size(14.5).color(TEXT))
                    .selectable(true)
                    .wrap(),
            );
        });

    let rect = inner.response.rect;
    let hovered = ui.rect_contains_pointer(rect);
    ui.painter().set(
        bg,
        Shape::rect_filled(
            rect,
            CornerRadius::same(12),
            if hovered { CARD_HOVER } else { CARD },
        ),
    );
    action
}

fn day_header(ui: &mut egui::Ui, day: NaiveDate) {
    const MONTHS: [&str; 12] = [
        "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября",
        "октября", "ноября", "декабря",
    ];
    let today = Local::now().date_naive();
    let label = match (today - day).num_days() {
        0 => "Сегодня".to_string(),
        1 => "Вчера".to_string(),
        _ => format!("{} {}", day.day(), MONTHS[day.month0() as usize]),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(MUTED).strong());
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().line_segment(
            [pos2(r.left() + 6.0, r.center().y), r.right_center()],
            Stroke::new(1.0, LINE),
        );
    });
}

fn hero(ui: &mut egui::Ui, w: f32, s: &Stats) {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(16.0)
        .inner_margin(22)
        .stroke(Stroke::new(1.0, LINE))
        .show(ui, |ui| {
            ui.set_width(w - 44.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("СЭКОНОМЛЕНО").size(10.5).color(FAINT));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(fmt_dur(s.time_saved_secs()))
                            .size(42.0)
                            .color(TEXT)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "против набора на {:.0} слов/мин · наговорено {}",
                            store::TYPING_WPM,
                            fmt_dur(s.speaking_secs)
                        ))
                        .size(12.0)
                        .color(MUTED),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(104.0, 104.0), Sense::hover());
                    let p = ui.painter();
                    // Today measured against the best day on record.
                    let frac = if s.most_words_day > 0 {
                        (s.words_today as f32 / s.most_words_day as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    ring(p, r.center(), 38.0, 7.0, frac);
                    p.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{:.0}%", frac * 100.0),
                        egui::FontId::proportional(15.0),
                        TEXT,
                    );
                    p.text(
                        pos2(r.center().x, r.bottom() - 2.0),
                        egui::Align2::CENTER_CENTER,
                        "от лучшего дня",
                        egui::FontId::proportional(10.0),
                        FAINT,
                    );
                });
            });
        });
}

fn ring(p: &egui::Painter, c: egui::Pos2, r: f32, w: f32, frac: f32) {
    let steps = 72;
    let arc = |from: usize, to: usize| -> Vec<egui::Pos2> {
        (from..=to)
            .map(|i| {
                let a =
                    std::f32::consts::TAU * i as f32 / steps as f32 - std::f32::consts::FRAC_PI_2;
                pos2(c.x + r * a.cos(), c.y + r * a.sin())
            })
            .collect()
    };
    p.add(Shape::line(arc(0, steps), Stroke::new(w, LINE)));
    let lit = (steps as f32 * frac).round() as usize;
    if lit > 0 {
        p.add(Shape::line(arc(0, lit), Stroke::new(w, ACCENT)));
    }
}

fn tile(ui: &mut egui::Ui, w: f32, title: &str, value: &str, sub: &str, accent: Color32) {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(14.0)
        .inner_margin(16)
        .stroke(Stroke::new(1.0, LINE))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(w - 34.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(3.0, 12.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, accent);
                    ui.add_space(2.0);
                    ui.label(RichText::new(title.to_uppercase()).size(10.0).color(FAINT));
                });
                ui.add_space(6.0);
                ui.label(RichText::new(value).size(28.0).color(TEXT).strong());
                ui.add_space(2.0);
                ui.label(RichText::new(sub).size(11.5).color(MUTED));
            });
        });
}

fn empty(ui: &mut egui::Ui, (title, hint): (&str, &str)) {
    ui.vertical_centered(|ui| {
        ui.add_space(110.0);
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter()
            .circle_stroke(r.center(), 22.0, Stroke::new(2.0, LINE));
        ui.painter().circle_filled(r.center(), 5.0, LINE);
        ui.add_space(14.0);
        ui.label(RichText::new(title).size(16.0).color(MUTED));
        ui.add_space(4.0);
        ui.label(RichText::new(hint).size(12.5).color(FAINT));
    });
}

fn fmt_dur(secs: f64) -> String {
    let s = secs.max(0.0) as i64;
    match (s / 3600, (s % 3600) / 60, s % 60) {
        (0, 0, sec) => format!("{sec} с"),
        (0, m, _) => format!("{m} мин"),
        (h, m, _) => format!("{h} ч {m} мин"),
    }
}

/// 12345 -> "12 345"
fn group(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

pub fn run(tab: &str) -> Result<()> {
    let conn = store::open()?;
    let tab = if tab == "stats" {
        Tab::Stats
    } else {
        Tab::History
    };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(Vec2::new(1040.0, 700.0))
            .with_min_inner_size(Vec2::new(760.0, 480.0))
            .with_title("voiceflow"),
        ..Default::default()
    };
    eframe::run_native(
        "voiceflow-ui",
        opts,
        Box::new(move |cc| {
            crate::overlay::install_fonts(&cc.egui_ctx);
            let mut v = egui::Visuals::dark();
            v.panel_fill = BG;
            v.window_fill = BG;
            v.extreme_bg_color = BG;
            v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
            cc.egui_ctx.set_visuals(v);
            Ok(Box::new(App::new(conn, tab)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_thousands() {
        assert_eq!(group(7), "7");
        assert_eq!(group(1234), "1\u{202f}234");
        assert_eq!(group(1234567), "1\u{202f}234\u{202f}567");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(fmt_dur(42.0), "42 с");
        assert_eq!(fmt_dur(600.0), "10 мин");
        assert_eq!(fmt_dur(3900.0), "1 ч 5 мин");
        assert_eq!(fmt_dur(-5.0), "0 с");
    }
}
