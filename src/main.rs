mod asr;
mod audio;
mod hotkey;
mod inject;
mod overlay;
mod pointer;
mod shot;
mod store;
mod ui;

use anyhow::{Context, Result};
use eframe::egui::{self, Pos2, pos2};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

const LEVELS_CAP: usize = 128;

#[derive(Default)]
pub struct State {
    pub recording: bool,
    pub partial: String,
    pub final_text: Option<String>,
    /// When the current recording began, for the history duration.
    pub started_at: Option<std::time::Instant>,
    levels: VecDeque<f32>,
}

impl State {
    pub fn push_level(&mut self, v: f32) {
        if self.levels.len() == LEVELS_CAP {
            self.levels.pop_front();
        }
        self.levels.push_back(v);
    }

    /// Oldest first, newest last, zero-padded when we have not filled up yet.
    pub fn recent_levels(&self, n: usize) -> Vec<f32> {
        let have = self.levels.len().min(n);
        let mut out = vec![0.0; n - have];
        out.extend(self.levels.iter().rev().take(have).rev());
        out
    }
}

fn data_dir() -> String {
    format!(
        "{}/.local/share/voiceflow",
        std::env::var("HOME").unwrap_or_default()
    )
}

fn pos_file() -> String {
    format!("{}/overlay-pos", data_dir())
}

fn load_pos() -> Pos2 {
    let raw = std::fs::read_to_string(pos_file()).unwrap_or_default();
    let mut it = raw.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
    let saved = match (it.next(), it.next()) {
        (Some(x), Some(y)) => Some(pos2(x, y)),
        _ => None,
    };
    clamp_to_screen(saved)
}

/// The overlay is only reachable by dragging it, so a position off the current
/// screen — a smaller monitor than last time, or a stale saved file — would be
/// unrecoverable without deleting the file by hand.
fn clamp_to_screen(saved: Option<Pos2>) -> Pos2 {
    let screen = pointer::Pointer::new()
        .and_then(|p| p.screen_size())
        .unwrap_or(egui::vec2(1920.0, 1080.0));

    let default = pos2(
        (screen.x / 2.0 + overlay::WINDOW_W / 2.0).min(screen.x - 40.0),
        screen.y - 80.0,
    );
    let p = saved.unwrap_or(default);
    pos2(
        p.x.clamp(overlay::WINDOW_W * 0.25, screen.x - 40.0),
        p.y.clamp(120.0, screen.y),
    )
}

pub fn save_pos(p: Pos2) {
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(pos_file(), format!("{} {}", p.x, p.y));
}

/// The overlay needs X11: its override-redirect trick has no Wayland analogue.
/// winit picks Wayland whenever WAYLAND_DISPLAY is set, so the daemon drops it
/// for itself. The history window keeps Wayland — as a normal managed window it
/// gains nothing from X11, and Mutter refuses to map our XWayland windows at all
/// (they come up iconified and ignore both Minimized(false) and Focus).
fn force_x11() {
    if let Ok(v) = std::env::var("WAYLAND_DISPLAY") {
        // `wl-copy` and `ydotool` inherit our environment; without this they
        // would fall back to the default `wayland-0` socket and break in any
        // session that uses another one.
        unsafe {
            std::env::set_var(WAYLAND_FALLBACK, v);
            std::env::remove_var("WAYLAND_DISPLAY");
        }
    }
}

/// Where [`force_x11`] parks the session's Wayland socket name.
pub const WAYLAND_FALLBACK: &str = "VOICEFLOW_WAYLAND_DISPLAY";

fn socket_path() -> String {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    format!("{dir}/voiceflow.sock")
}

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("toggle") => {
            let mut s = UnixStream::connect(socket_path()).context("daemon not running")?;
            s.write_all(b"toggle")?;
            Ok(())
        }
        Some("preview") => preview(),
        Some("ui") => ui::run(std::env::args().nth(2).unwrap_or_default().as_str()),
        _ => daemon(),
    }
}

/// Renders the overlay with canned data so the layout can be reviewed without
/// speaking into it. Pair with `VOICEFLOW_SHOT=out.png`.
fn preview() -> Result<()> {
    force_x11();
    let state = Arc::new(Mutex::new(State::default()));
    {
        let mut st = state.lock().unwrap();
        st.recording = true;
        st.partial = std::env::var("VOICEFLOW_PREVIEW_TEXT")
            .unwrap_or_else(|_| "распознавание идёт прямо сейчас".into());
        for i in 0..LEVELS_CAP {
            let x = i as f32 / 7.0;
            st.push_level(((x.sin() * 0.5 + 0.5) * (x * 0.37).cos().abs()).clamp(0.05, 1.0));
        }
    }

    let pos = load_pos();
    let opts = eframe::NativeOptions {
        viewport: overlay::viewport(pos),
        ..Default::default()
    };
    eframe::run_native(
        "voiceflow-overlay",
        opts,
        Box::new(move |cc| {
            overlay::install_fonts(&cc.egui_ctx);
            Ok(Box::new(overlay::Overlay::new(state, pos)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

fn daemon() -> Result<()> {
    // Before anything expensive: a second daemon would spend 2.5 s loading a
    // 2.4 GB model only to find the socket taken.
    if UnixStream::connect(socket_path()).is_ok() {
        anyhow::bail!("another voiceflow daemon is already running");
    }
    force_x11();
    let lang = std::env::var("VOICEFLOW_LANG").unwrap_or_else(|_| "ru-RU".into());
    let state = Arc::new(Mutex::new(State::default()));

    let (asr_tx, asr_rx) = std::sync::mpsc::channel::<asr::Msg>();
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<audio::Cmd>();

    asr::spawn(state.clone(), asr_rx, lang)?;
    audio::spawn(asr_tx.clone(), cmd_rx, state.clone())?;

    spawn_control(toggler(state.clone(), asr_tx.clone(), cmd_tx.clone()))?;

    // Modifier-only chords are impossible via GNOME keybindings, so this one
    // is read straight off evdev. Not fatal if it fails: the socket still works.
    if let Err(e) = hotkey::spawn(toggler(state.clone(), asr_tx, cmd_tx)) {
        eprintln!("hotkey: disabled ({e})");
    }

    let pos = load_pos();
    let opts = eframe::NativeOptions {
        viewport: overlay::viewport(pos),
        ..Default::default()
    };

    eframe::run_native(
        "voiceflow-overlay",
        opts,
        Box::new(move |cc| {
            overlay::install_fonts(&cc.egui_ctx);
            Ok(Box::new(overlay::Overlay::new(state, pos)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Flips recording on/off. Shared by the socket and the evdev hotkey.
fn toggler(
    state: Arc<Mutex<State>>,
    asr_tx: Sender<asr::Msg>,
    cmd_tx: Sender<audio::Cmd>,
) -> impl Fn() + Send + 'static {
    move || {
        let now = {
            let mut st = state.lock().unwrap();
            st.recording = !st.recording;
            if st.recording {
                st.partial.clear();
                st.final_text = None;
                st.levels.clear();
                st.started_at = Some(std::time::Instant::now());
            }
            st.recording
        };

        if now {
            let _ = asr_tx.send(asr::Msg::Start);
            let _ = cmd_tx.send(audio::Cmd::Start);
        } else {
            let _ = cmd_tx.send(audio::Cmd::Stop);
            let _ = asr_tx.send(asr::Msg::Finish);
        }
        eprintln!("toggle: recording={now}");
    }
}

/// Listens on the unix socket that `voiceflow toggle` talks to.
fn spawn_control(toggle: impl Fn() + Send + 'static) -> Result<()> {
    let path = socket_path();
    // Nothing answered on it during the startup check, so it is stale.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("bind {path}"))?;
    eprintln!("control: listening on {path}");

    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let mut buf = String::new();
            let _ = conn.read_to_string(&mut buf);
            if buf.trim() == "toggle" {
                toggle();
            }
        }
    });
    Ok(())
}
