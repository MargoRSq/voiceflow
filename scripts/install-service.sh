#!/usr/bin/env bash
# Installs and starts the user service. %h in the unit expands to $HOME, so
# nothing user-specific is committed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
BIN="${VOICEFLOW_BIN:-$HOME/.local/bin/voiceflow}"

[ -x "$BIN" ] || { echo "no voiceflow binary at $BIN — run: cargo build --release && install -Dm755 target/release/voiceflow $BIN" >&2; exit 1; }

mkdir -p "$UNIT_DIR"
install -m644 "$ROOT/systemd/voiceflow.service" "$UNIT_DIR/voiceflow.service"

# The overlay talks X11 and the session bus; the user manager needs those in
# its environment or the daemon starts blind.
systemctl --user import-environment DISPLAY XAUTHORITY WAYLAND_DISPLAY XDG_SESSION_TYPE 2>/dev/null || true

systemctl --user daemon-reload
systemctl --user enable --now voiceflow.service

echo "--- status"
systemctl --user --no-pager status voiceflow.service | head -12
