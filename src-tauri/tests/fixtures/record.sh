#!/usr/bin/env bash
# Record a fixture for e2e_voice tests. Saves 16kHz mono 16-bit PCM WAV —
# same format Whisper consumes — so no conversion is needed later.
#
# Usage:
#   ./record.sh <fixture_name> [duration_seconds]
# Example:
#   ./record.sh short_sentence 4
#
# Creates tests/fixtures/<fixture_name>/audio.wav. Edit
# tests/fixtures/<fixture_name>/expect.txt afterward to list substrings the
# polished transcript MUST contain (one per line). Prefix with `!` to assert
# absence. Blank lines and `#` comments are ignored.

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "usage: $0 <fixture_name> [duration_seconds]" >&2
    exit 1
fi

NAME="$1"
DURATION="${2:-5}"
DIR="$(cd "$(dirname "$0")" && pwd)/$NAME"
mkdir -p "$DIR"
OUT="$DIR/audio.wav"

if ! command -v ffmpeg >/dev/null; then
    echo "ffmpeg not found. Install: brew install ffmpeg" >&2
    exit 1
fi

# List devices once so the user knows what's available. The default mic is
# usually index 0; override with AVFOUNDATION_INDEX if needed.
echo "Available input devices:"
ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 | grep -E 'AVFoundation (input|audio)' -A 20 | grep -E '\[[0-9]+\]' || true
echo

INDEX="${AVFOUNDATION_INDEX:-:0}"
echo "Recording $DURATION seconds to $OUT from device $INDEX ..."
echo "Speak after the beep."
sleep 1
printf '\a'  # terminal bell as a crude "beep"

ffmpeg -hide_banner -loglevel warning \
    -f avfoundation -i "$INDEX" \
    -t "$DURATION" \
    -ac 1 -ar 16000 -sample_fmt s16 \
    -y "$OUT"

echo
echo "Wrote $OUT"
if [[ ! -f "$DIR/expect.txt" ]]; then
    cat > "$DIR/expect.txt" <<EOF
# Substrings the polished transcript MUST contain (one per line).
# Prefix with ! to assert absence. Blank lines and # comments ignored.
# Example:
#   Kubernetes
#   !um
EOF
    echo "Wrote template $DIR/expect.txt — edit it with expected terms."
fi
