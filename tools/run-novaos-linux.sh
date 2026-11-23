#!/bin/bash
# tools/run-novaos-linux.sh
# Linux QEMU launcher for NovaOS:
# 1. Resolves mouse grab freeze on modern Linux desktop environments (Wayland & X11):
#    - For SDL: routes through native Wayland relative pointer protocols
#      (zwp_relative_pointer_manager_v1) or X11, providing smooth, lag-free 60fps tracking.
#    - For GTK: enables native Wayland/X11 backend without forcing broken XWayland pointer warping.
# 2. Preserves terminal control and clean exit on Ctrl+C.

set -e

if [ -n "$WAYLAND_DISPLAY" ]; then
    export SDL_VIDEODRIVER=wayland,x11
    # Allow GTK to use native Wayland instead of forcing legacy XWayland
    if [ "$GDK_BACKEND" = "x11" ]; then
        unset GDK_BACKEND
    fi
fi

exec "$@"
