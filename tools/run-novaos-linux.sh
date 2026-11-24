#!/bin/bash
# tools/run-novaos-linux.sh
# Linux QEMU launcher for NovaOS:
# 1. Resolves mouse grab freeze and modal popups on Wayland desktop environments:
#    - Native Wayland client grabbing causes GNOME Mutter to display an intrusive
#      "Allow inhibiting shortcuts" modal dialog, which breaks pointer capture and drops
#      relative pointer locks, permanently freezing the mouse.
#    - Routing through XWayland (GDK_BACKEND=x11, SDL_VIDEODRIVER=x11) ensures standard
#      X11 pointer grabbing (XGrabPointer) and warping (XWarpPointer) operate smoothly
#      without triggering portal dialogs or losing relative pointer grab.
#    - Sets GTK_USE_PORTAL=0 to prevent shortcut inhibitor portal prompts.
# 2. Preserves terminal control and clean exit on Ctrl+C.

set -e

if [ -n "$WAYLAND_DISPLAY" ] && [ -n "$DISPLAY" ]; then
    export GDK_BACKEND=x11
    export SDL_VIDEODRIVER=x11
    export GTK_USE_PORTAL=0
fi

exec "$@"
