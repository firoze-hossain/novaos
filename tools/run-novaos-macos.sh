#!/bin/bash
# tools/run-novaos-macos.sh
# macOS QEMU launcher for NovaOS:
# 1. Runs QEMU with Cocoa backend and zoom-to-fit enabled so the window is freely resizable by dragging.
# 2. Automatically sizes the initial window to large (Image 2: 1024x640) on high-DPI Retina screens.
# 3. Preserves foreground terminal control and clean exit on Ctrl+C.

BIN_NAME="$(basename "$1")"

# If not in fullscreen mode, launch a background watcher to enlarge window to 1024x640
if [[ "$*" != *"-full-screen"* ]]; then
    (
        for i in {1..40}; do
            if osascript -e "tell application \"System Events\" to exists (window 1 of process \"$BIN_NAME\")" 2>/dev/null | grep -q "true"; then
                osascript -e "
                    tell application \"System Events\"
                        tell process \"$BIN_NAME\"
                            set size of window 1 to {1024, 640}
                            set position of window 1 to {160, 100}
                        end tell
                    end tell" 2>/dev/null
                break
            fi
            sleep 0.1
        done
    ) &
    WATCHER_PID=$!
fi

# Run QEMU in foreground
"$@"
STATUS=$?

if [ -n "$WATCHER_PID" ]; then
    kill "$WATCHER_PID" 2>/dev/null || true
fi

exit $STATUS
