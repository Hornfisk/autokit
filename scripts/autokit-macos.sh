#!/bin/bash
# Launch Autokit standalone on macOS with a safe buffer size.
# CoreAudio may deliver buffers larger than 512, which crashes
# nih-plug's CPAL backend. Using 1024 avoids this.
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/autokit-standalone" --buffer-size 1024 "$@"
