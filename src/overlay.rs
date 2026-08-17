use crate::{State, inject};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use std::sync::{Arc, Mutex};

// Modelled on FluidVoice's "Medium" overlay preset: fixed width, wrapped
// preview text, height that grows in steps as lines are added.
const CONTENT_W: f32 = 400.0;
const MARGIN: f32 = 10.0; // room outside the card for the drop shadow
const PAD: f32 = 16.0;
const CORNER: f32 = 18.0;
const HEADER_H: f32 = 24.0;
const ROW_GAP: f32 = 12.0;
const FONT_SIZE: f32 = 15.0;
const MAX_LINES: usize = 4;
/// FluidVoice debounces its own content resizes by the same amount.
const RESIZE_DEBOUNCE: f64 = 0.08;

const BAR_W: f32 = 3.0;
const BAR_GAP: f32 = 3.5;
const DOT_R: f32 = 5.0;
const DOT_GAP: f32 = 14.0;

/// The meter fills whatever the dot leaves, so the bar count follows the card
/// width rather than being picked by hand.
const METER_W: f32 = CONTENT_W - PAD * 2.0 - DOT_R * 2.0 - DOT_GAP;
const BARS: usize = (METER_W / (BAR_W + BAR_GAP)) as usize;

const BODY: Color32 = Color32::from_rgb(17, 17, 21);
const TEXT: Color32 = Color32::from_rgb(242, 243, 247);
const MUTED: Color32 = Color32::from_rgb(122, 126, 138);
const ACCENT: Color32 = Color32::from_rgb(126, 200, 255);
const REC: Color32 = Color32::from_rgb(236, 79, 79);

pub const WINDOW_W: f32 = CONTENT_W + MARGIN * 2.0;

fn line_h() -> f32 {
    (FONT_SIZE * 1.35).round()
}

/// Height for a card holding `lines` rows of preview text.
fn window_h(lines: usize) -> f32 {
    let lines = lines.clamp(1, MAX_LINES) as f32;
    MARGIN * 2.0 + PAD * 2.0 + HEADER_H + ROW_GAP + lines * line_h()
}

pub fn install_fonts(ctx: &egui::Context) {
    const FACES: [(&str, &str); 2] = [
        ("inter", "/usr/share/fonts/opentype/inter/Inter-Medium.otf"),
        ("inter-sb", "/usr/share/fonts/opentype/inter/Inter-SemiBold.otf"),
    ];

    let mut fonts = egui::FontDefinitions::default();
    let mut installed = vec![];
    for (name, path) in FACES {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                name.to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            installed.push(name.to_owned());
        }
    }
    if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        for name in installed.into_iter().rev() {
            fam.insert(0, name);
        }
    }
    ctx.set_fonts(fonts);
}

pub struct Overlay {
    state: Arc<Mutex<State>>,
    visible: Option<bool>,
    /// Bottom-left corner, root-relative. Authoritative: the card grows upward
    /// so the edge nearest the screen bottom stays put as lines are added.
    anchor: Pos2,
    t: f32,
    frames: u64,
    h: f32,
    sent_h: f32,
    last_resize: f64,
    /// (pointer at press, anchor at press), both root-relative.
    grab: Option<(Pos2, Pos2)>,
    was_down: bool,
    pointer: Option<crate::pointer::Pointer>,
    /// Smoothed copy of the meter so bars glide instead of snapping.
    bars: [f32; BARS],
}

impl Overlay {
    pub fn new(state: Arc<Mutex<State>>, anchor: Pos2) -> Self {
        let h = window_h(1);
        Self {
            state,
            visible: None,
            anchor,
            t: 0.0,
            frames: 0,
            h,
            sent_h: h,
            last_resize: 0.0,
            grab: None,
            was_down: false,
            pointer: crate::pointer::Pointer::new(),
            bars: [0.0; BARS],
        }
    }

    fn origin(&self) -> Pos2 {
        pos2(self.anchor.x, self.anchor.y - self.h)
    }
}

impl eframe::App for Overlay {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.t += dt;
        self.frames += 1;

        let (recording, partial, done, levels) = {
            let mut st = self.state.lock().unwrap();
            (
                st.recording,
                st.partial.clone(),
                st.final_text.take(),
                st.recent_levels(BARS),
            )
        };

        if let Some(text) = done {
            std::thread::spawn(move || {
                if let Err(e) = inject::paste(&text) {
                    eprintln!("inject: {e}");
                }
            });
        }

        if self.visible != Some(recording) {
            self.visible = Some(recording);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(recording));
            if !recording {
                self.bars = [0.0; BARS];
            }
        }
        if !recording {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
            return;
        }

        let root = ui.max_rect();
        let card = Rect::from_min_max(
            root.min + vec2(MARGIN, MARGIN),
            pos2(root.min.x + MARGIN + CONTENT_W, root.max.y - MARGIN),
        );
        let text_w = card.width() - PAD * 2.0;

        let idle = partial.trim().is_empty();
        let galley = self.lay_out(ui, &partial, text_w, idle);
        self.resize(&ctx, galley.rows.len());
        self.drag(&ctx);

        let p = ui.painter();
        for i in (1..=4).rev() {
            let k = i as f32;
            p.rect_filled(
                card.expand(k * 2.0),
                CORNER + k * 2.0,
                Color32::from_black_alpha((14.0 / k) as u8),
            );
        }
        p.rect_filled(card, CORNER, BODY);
        p.rect_stroke(
            card,
            CORNER,
            Stroke::new(1.0, Color32::from_white_alpha(20)),
            egui::StrokeKind::Inside,
        );

        let header_y = card.top() + PAD + HEADER_H / 2.0;
        let dot = pos2(card.left() + PAD + DOT_R, header_y);
        let pulse = 0.55 + 0.45 * (self.t * 4.5).sin();
        p.circle_filled(dot, DOT_R, REC.gamma_multiply(0.45 + 0.55 * pulse));

        self.draw_meter(
            p,
            Rect::from_min_size(
                pos2(dot.x + DOT_R + DOT_GAP, header_y - HEADER_H / 2.0),
                vec2(card.right() - PAD - (dot.x + DOT_R + DOT_GAP), HEADER_H),
            ),
            &levels,
            dt,
        );

        p.galley(
            pos2(card.left() + PAD, card.top() + PAD + HEADER_H + ROW_GAP),
            galley,
            if idle { MUTED } else { TEXT },
        );

        crate::shot::tick(&ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

impl Overlay {
    /// Wraps the preview to `width` and keeps only the newest `MAX_LINES` rows,
    /// binary-searching the longest suffix that still fits.
    fn lay_out(
        &self,
        ui: &egui::Ui,
        text: &str,
        width: f32,
        idle: bool,
    ) -> std::sync::Arc<egui::Galley> {
        let font = FontId::proportional(FONT_SIZE);
        let color = if idle { MUTED } else { TEXT };
        let lay = |s: &str| {
            ui.painter()
                .layout(s.to_owned(), font.clone(), color, width)
        };

        if idle {
            return lay("Hard work beats talent...");
        }

        let full = lay(text);
        if full.rows.len() <= MAX_LINES {
            return full;
        }

        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let suffix: String = chars[chars.len() - mid..].iter().collect();
            if lay(&suffix).rows.len() <= MAX_LINES {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let suffix: String = chars[chars.len() - lo..].iter().collect();
        // Dropping a partial word looks like a glitch; start at the next one.
        let trimmed = match suffix.find(' ') {
            Some(i) if !suffix.starts_with(' ') => &suffix[i + 1..],
            _ => suffix.trim_start(),
        };
        lay(trimmed)
    }

    /// Height follows the wrapped line count in discrete steps, debounced so a
    /// word landing on a line boundary cannot make the card flicker.
    fn resize(&mut self, ctx: &egui::Context, lines: usize) {
        let target = window_h(lines);
        if (target - self.sent_h).abs() < 0.5 {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now - self.last_resize < RESIZE_DEBOUNCE {
            return;
        }
        self.last_resize = now;
        self.sent_h = target;
        self.h = target;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(WINDOW_W, target)));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(self.origin()));
    }

    /// Drag driven entirely by root-relative pointer state from X — see
    /// [`crate::pointer::Pointer`] for why egui's own pointer cannot do this.
    fn drag(&mut self, ctx: &egui::Context) {
        let Some(s) = self.pointer.as_ref().and_then(|p| p.sample()) else {
            return;
        };

        // Only a press that *starts* on the card grabs it. Without the edge
        // check, a drag begun elsewhere that merely passes over the overlay
        // would hijack it and also overwrite the saved position on release.
        let pressed = s.primary_down && !self.was_down;
        self.was_down = s.primary_down;

        match (self.grab, s.primary_down) {
            (None, true) => {
                let rect = Rect::from_min_size(self.origin(), vec2(WINDOW_W, self.h));
                if pressed && rect.contains(s.pos) {
                    self.grab = Some((s.pos, self.anchor));
                }
            }
            (Some((grab_ptr, origin)), true) => {
                let want = origin + (s.pos - grab_ptr);
                if want != self.anchor {
                    self.anchor = want;
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(self.origin()));
                }
            }
            (Some(_), false) => {
                self.grab = None;
                crate::save_pos(self.anchor);
            }
            _ => {}
        }
    }

    fn draw_meter(&mut self, p: &egui::Painter, area: Rect, levels: &[f32], dt: f32) {
        let mid = area.center().y;
        let max_h = area.height() * 0.5;
        let k = (dt * 22.0).min(1.0);
        // Spread over the real area instead of a fixed pitch, so the last bar
        // lands exactly on the right padding edge whatever the card width is.
        let pitch = area.width() / BARS as f32;

        for i in 0..BARS {
            let target = levels.get(i).copied().unwrap_or(0.0);
            self.bars[i] += (target - self.bars[i]) * k;
            let v = self.bars[i];

            let h = (2.5 + v * (max_h * 2.0 - 2.5)).min(max_h * 2.0);
            let x = area.left() + (i as f32 + 0.5) * pitch;
            let bar = Rect::from_center_size(pos2(x, mid), vec2(BAR_W, h));

            // Newest sample sits at the right and is the brightest.
            let age = i as f32 / BARS as f32;
            p.rect_filled(
                bar,
                BAR_W / 2.0,
                ACCENT.gamma_multiply(0.35 + 0.65 * age * (0.35 + 0.65 * v)),
            );
        }
    }
}

pub fn viewport(anchor: Pos2) -> egui::ViewportBuilder {
    let h = window_h(1);
    egui::ViewportBuilder::default()
        .with_inner_size(Vec2::new(WINDOW_W, h))
        .with_position(pos2(anchor.x, anchor.y - h))
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_window_type(egui::X11WindowType::Notification)
        // Unmanaged by Mutter: it can never hand us keyboard focus, so clicking
        // the card to drag it cannot steal focus from the app being dictated into.
        .with_override_redirect(true)
        .with_resizable(false)
        .with_taskbar(false)
        .with_active(false)
        .with_visible(false)
}
