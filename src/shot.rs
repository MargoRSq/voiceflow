use eframe::egui;

/// `VOICEFLOW_SHOT=path` makes a window grab its own framebuffer and quit.
/// GNOME refuses external capture (`org.gnome.Shell.Screenshot` returns
/// AccessDenied, and `import` on a redirected window yields an empty image),
/// so this is the only way to review a layout without a human looking at it.
///
/// `VOICEFLOW_SHOT_AFTER=<seconds>` delays the grab, leaving room to drive the
/// window first and capture the resulting state. Seconds rather than frames:
/// an unfocused window repaints far below 60 Hz, which makes frame counts a
/// useless clock.
pub fn tick(ctx: &egui::Context) {
    let Ok(path) = std::env::var("VOICEFLOW_SHOT") else {
        return;
    };
    let after: f64 = std::env::var("VOICEFLOW_SHOT_AFTER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);

    let shot = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    if let Some(img) = shot {
        let (w, h) = (img.size[0] as u32, img.size[1] as u32);
        let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
        let _ = image::save_buffer(&path, &bytes, w, h, image::ColorType::Rgba8);
        eprintln!("wrote {path} {w}x{h}");
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        return;
    }

    if ctx.input(|i| i.time) >= after {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}
