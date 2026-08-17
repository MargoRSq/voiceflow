#!/usr/bin/env bash
# Registers the history window as a launchable app. Paths are expanded at
# install time so nothing user-specific is committed.
set -euo pipefail

BIN="${VOICEFLOW_BIN:-$HOME/.local/bin/voiceflow}"
DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
FILE="$DIR/voiceflow.desktop"

[ -x "$BIN" ] || { echo "no voiceflow binary at $BIN" >&2; exit 1; }

mkdir -p "$DIR"
cat > "$FILE" <<EOF
[Desktop Entry]
Type=Application
Name=voiceflow
GenericName=Voice dictation history
Comment=История диктовок и статистика
Exec=$BIN ui
Icon=audio-input-microphone
Terminal=false
Categories=Utility;AudioVideo;
StartupWMClass=voiceflow-ui
EOF

update-desktop-database "$DIR" 2>/dev/null || true
echo "installed $FILE"
