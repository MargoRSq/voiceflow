use anyhow::{Result, bail};
use std::io::Write;
use std::process::{Command, Stdio};

/// The daemon clears `WAYLAND_DISPLAY` for winit's sake (see `force_x11`), so
/// wayland clients we spawn need it handed back explicitly.
fn wayland_cmd(program: &str) -> Command {
    let mut c = Command::new(program);
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        if let Some(v) = std::env::var_os(crate::WAYLAND_FALLBACK) {
            c.env("WAYLAND_DISPLAY", v);
        }
    }
    c
}

pub fn to_clipboard(text: &str) -> Result<()> {
    let mut wl = wayland_cmd("wl-copy").stdin(Stdio::piped()).spawn()?;
    wl.stdin.as_mut().unwrap().write_all(text.as_bytes())?;
    let st = wl.wait()?;
    if !st.success() {
        bail!("wl-copy failed: {st}");
    }
    Ok(())
}

/// Clipboard + Ctrl+V. `ydotool type` mangles non-Latin layouts, so paste is
/// the only reliable path for Cyrillic.
pub fn paste(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    to_clipboard(text)?;
    std::thread::sleep(std::time::Duration::from_millis(60));

    // keycodes: 29 = leftctrl, 47 = v
    let st = wayland_cmd("ydotool")
        .args(["key", "29:1", "47:1", "47:0", "29:0"])
        .status()?;
    if !st.success() {
        bail!("ydotool failed: {st}");
    }
    Ok(())
}
