#!/usr/bin/env bash
# Record N seconds from the default source and transcribe.
# usage: scripts/try.sh [seconds] [lang]
set -euo pipefail

SECS="${1:-8}"
LANG_TAG="${2:-ru-RU}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${TMPDIR:-/tmp}/voiceflow_try.wav"

SRC="$(pactl get-default-source)"
echo "source: $SRC"
echo "recording ${SECS}s — SPEAK NOW"
timeout "$SECS" parecord --device="$SRC" --format=s16le --rate=16000 \
  --channels=1 --file-format=wav "$OUT" || true
echo "recorded $(stat -c%s "$OUT") bytes"
echo

exec "$ROOT/target/release/examples/asr_spike" "$OUT" "$LANG_TAG"
