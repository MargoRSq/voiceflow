use eframe::egui::{Pos2, pos2};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, KeyButMask};
use x11rb::rust_connection::RustConnection;

/// Root-relative pointer state read straight from X.
///
/// egui's own pointer position is window-relative and only refreshes on real
/// motion events, so while we are moving the window it reports stale
/// coordinates and a drag either runs away or stalls. Root coordinates are
/// immune to that. The button state comes from the same query because egui can
/// also miss the release once the pointer leaves the window mid-drag.
pub struct Pointer {
    conn: RustConnection,
    root: u32,
}

pub struct Sample {
    pub pos: Pos2,
    pub primary_down: bool,
}

impl Pointer {
    pub fn new() -> Option<Self> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        Some(Self { conn, root })
    }

    /// Root window size, used to keep the overlay on screen.
    pub fn screen_size(&self) -> Option<eframe::egui::Vec2> {
        let g = self.conn.get_geometry(self.root).ok()?.reply().ok()?;
        Some(eframe::egui::vec2(g.width as f32, g.height as f32))
    }

    pub fn sample(&self) -> Option<Sample> {
        let r = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some(Sample {
            pos: pos2(r.root_x as f32, r.root_y as f32),
            primary_down: r.mask.contains(KeyButMask::BUTTON1),
        })
    }
}
