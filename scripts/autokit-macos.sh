#!/bin/bash
# Launch Autokit standalone on macOS with a safe buffer size.
# CoreAudio delivers variable-sized buffers that can exceed the
# configured size, crashing nih-plug's CPAL backend assertion.
# 4096 is large enough to accommodate CoreAudio's actual delivery.
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/autokit-standalone" --period-size 4096 "$@"
