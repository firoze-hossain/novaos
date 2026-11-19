#!/bin/bash
# tools/run-novaos-linux.sh
# Linux QEMU launcher for NovaOS:
# 1. Fixes the Wayland mouse freeze issue in QEMU:
#    Under native Wayland (Ubuntu/GNOME/KDE default), GTK prevents client-side
#    pointer warping (gdk_device_warp is a no-op under Wayland). When QEMU
#    emulates a relative PS/2 mouse, moving to the bottom of the screen (the dock /
#    menu options) causes the host cursor to hit the window border. Upon clicking,
#    Wayland loses pointer grab or stops reporting motion deltas because the host
#    cursor cannot warp back to window center, causing the mouse cursor in QEMU to
#    freeze.
#    By routing through XWayland (GDK_BACKEND=x11) when WAYLAND_DISPLAY and DISPLAY
#    are set, XWarpPointer and XGrabPointer function fully, providing smooth,
#    reliable mouse tracking identical to macOS Cocoa.
# 2. Preserves terminal control and clean exit on Ctrl+C.

set -e

if [ -n "$WAYLAND_DISPLAY" ] && [ -n "$DISPLAY" ] && [ -z "$GDK_BACKEND" ]; then
    export GDK_BACKEND=x11
fi

exec "$@"
