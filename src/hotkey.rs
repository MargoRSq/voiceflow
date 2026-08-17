use anyhow::{Context, Result};
use evdev::{Device, EventSummary, KeyCode};

/// Toshy (xwaykeyz) grabs the physical keyboards and re-emits remapped events
/// on its own virtual device, so that is what we listen to by default.
const PREFERRED: &str = "XWayKeyz (virtual) Keyboard";

/// Never listen to our own injected Ctrl+V.
const EXCLUDE: &str = "ydotoold virtual device";

fn is_alt(k: KeyCode) -> bool {
    k == KeyCode::KEY_LEFTALT || k == KeyCode::KEY_RIGHTALT
}

fn is_shift(k: KeyCode) -> bool {
    k == KeyCode::KEY_LEFTSHIFT || k == KeyCode::KEY_RIGHTSHIFT
}

fn pick() -> Result<(String, Device)> {
    let explicit = std::env::var("VOICEFLOW_KBD").ok();
    let want = explicit.clone().unwrap_or_else(|| PREFERRED.into());
    let mut fallback = None;

    for (_, dev) in evdev::enumerate() {
        let name = dev.name().unwrap_or_default().to_string();
        if name.contains(EXCLUDE) && explicit.is_none() {
            continue;
        }
        let has_alt = dev
            .supported_keys()
            .is_some_and(|k| k.contains(KeyCode::KEY_LEFTALT));
        if !has_alt {
            continue;
        }
        if name.contains(&want) {
            return Ok((name, dev));
        }
        fallback.get_or_insert((name, dev));
    }

    fallback.context("no keyboard device with Alt found")
}

/// Fires on a *tap* of Alt+Shift: both held, then released without any other
/// key in between. Holding Alt+Shift+X stays available to other apps.
pub fn spawn(on_trigger: impl Fn() + Send + 'static) -> Result<()> {
    let (name, mut dev) = pick()?;
    eprintln!("hotkey: listening on {name:?} for Alt+Shift tap");

    std::thread::spawn(move || {
        let mut chord = Chord::default();

        loop {
            let events = match dev.fetch_events() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("hotkey: {e}");
                    return;
                }
            };

            for ev in events {
                let EventSummary::Key(_, key, value) = ev.destructure() else {
                    continue;
                };
                if chord.feed(key, value) {
                    on_trigger();
                }
            }
        }
    });
    Ok(())
}

#[derive(Default)]
struct Chord {
    alt: bool,
    shift: bool,
    armed: bool,
    /// Set once any other key is pressed while the chord is held, and cleared
    /// only when both modifiers are back up. Without it, Alt+Shift+Tab followed
    /// by a second Shift press (Alt still held — ordinary window cycling)
    /// re-arms the chord and the final Shift release fires a dictation.
    used: bool,
}

impl Chord {
    /// Returns true when a clean Alt+Shift tap completes.
    fn feed(&mut self, key: KeyCode, value: i32) -> bool {
        if value == 2 {
            return false; // autorepeat
        }
        let down = value == 1;

        if is_alt(key) {
            self.alt = down;
        } else if is_shift(key) {
            self.shift = down;
        } else {
            if down {
                self.used = true;
                self.armed = false;
            }
            return false;
        }

        if !self.alt && !self.shift {
            let fire = self.armed && !self.used;
            self.armed = false;
            self.used = false;
            return fire;
        }

        if self.alt && self.shift && !self.used {
            self.armed = true;
        } else if self.armed && self.used {
            self.armed = false;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: KeyCode = KeyCode::KEY_LEFTALT;
    const SHIFT: KeyCode = KeyCode::KEY_LEFTSHIFT;
    const TAB: KeyCode = KeyCode::KEY_TAB;

    /// Feeds a sequence and reports how many times the chord fired.
    fn run(seq: &[(KeyCode, i32)]) -> usize {
        let mut c = Chord::default();
        seq.iter().filter(|(k, v)| c.feed(*k, *v)).count()
    }

    #[test]
    fn clean_tap_fires_once() {
        assert_eq!(run(&[(ALT, 1), (SHIFT, 1), (SHIFT, 0), (ALT, 0)]), 1);
    }

    #[test]
    fn reverse_order_also_fires() {
        assert_eq!(run(&[(SHIFT, 1), (ALT, 1), (ALT, 0), (SHIFT, 0)]), 1);
    }

    #[test]
    fn chord_used_as_a_modifier_does_not_fire() {
        assert_eq!(
            run(&[(ALT, 1), (SHIFT, 1), (TAB, 1), (TAB, 0), (SHIFT, 0), (ALT, 0)]),
            0
        );
    }

    #[test]
    fn re_pressing_shift_after_alt_shift_tab_does_not_fire() {
        // Window cycling: Alt stays down while Shift is tapped again.
        assert_eq!(
            run(&[
                (ALT, 1),
                (SHIFT, 1),
                (TAB, 1),
                (TAB, 0),
                (SHIFT, 0),
                (SHIFT, 1),
                (SHIFT, 0),
                (ALT, 0),
            ]),
            0
        );
    }

    #[test]
    fn single_modifier_does_not_fire() {
        assert_eq!(run(&[(ALT, 1), (ALT, 0)]), 0);
        assert_eq!(run(&[(SHIFT, 1), (SHIFT, 0)]), 0);
    }

    #[test]
    fn autorepeat_is_ignored() {
        assert_eq!(
            run(&[(ALT, 1), (SHIFT, 1), (SHIFT, 2), (ALT, 2), (SHIFT, 0), (ALT, 0)]),
            1
        );
    }

    #[test]
    fn two_taps_fire_twice() {
        assert_eq!(
            run(&[
                (ALT, 1),
                (SHIFT, 1),
                (SHIFT, 0),
                (ALT, 0),
                (ALT, 1),
                (SHIFT, 1),
                (SHIFT, 0),
                (ALT, 0),
            ]),
            2
        );
    }
}
