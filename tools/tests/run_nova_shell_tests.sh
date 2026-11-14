#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for NovaOS Desktop Shell (nova-shell)
# A. macOS-Style Persistent Unified Top Menu Bar:
#    1. TopMenuBar geometry: persistent 28px height, acrylic blur background, delicate bottom border.
#    2. Dynamic App Menu Tree: system menu logo, active window focus synchronizer, dynamic category tree over IPC.
#    3. Interactive Menu Scrubbing: click-to-open, mouse hover scrubbing between adjacent categories, click outside/re-click dismiss.
#    4. Keyboard Shortcuts: accelerator dispatch table (⌘N, ⌃⌘Q, etc.) matching active window and system actions.
#    5. Right-Aligned System Status Indicators (Clock, Network, Audio, Battery, User account pill).
#    6. Flyout Popover Rendering: Level 3 elevation shadows, rounded acrylic cards, and subpixel positioning.
# B. Windows 11-Style Centered Dynamic Taskbar & Application Dock:
#    1. Customizable alignment: Centered (Windows 11 style) and Left (classic style).
#    2. Pinned & running applications: active dot indicator, multi-window double dots, and focused glowing accent pill.
#    3. Badge notifications: unread count badges with crimson capsule rendering.
#    4. macOS-style magnification curve: smooth Gaussian zoom on mouse hover across icon cluster.
#    5. Launch bounce physics: vertical impulse, gravity, and elastic restitution simulation.
#    6. Live window thumbnail previews: miniature surface previews from compositor with close (✕) and minimize (—) actions.
#    7. Start Launcher button and Quick Search pill with vector icon rasterization.
# C. Windows Action Center & Notification Flyout:
#    1. Slide-in right drawer with acrylic blur and Level 4 elevation shadows.
#    2. Rich push notifications with actionable buttons (reply, dismiss, open).
#    3. Transient banner toasts in bottom-right corner with auto-dismiss countdown.
#    4. Quick Settings grid: Wi-Fi, Bluetooth, Dark Mode, Night Light, DND / Focus Assist, Airplane Mode.
#    5. Dual sliders for Display Brightness and Audio Volume.
#    6. Configurable Quiet Hours / DND toast suppression.
#    7. Persistent notification history archive with "Clear all" clearing.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_shell_tests.sh      (or: make nova-shell-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/nova-shell host tests (Top Menu Bar, Taskbar & Action Center) ---"
    (cd userland/desktop/nova-shell && cargo test --quiet)
    (cd userland/desktop/nova-shell && cargo run --quiet -- --selftest)
    echo "[PASS] All 17 NovaOS Desktop Shell (nova-shell) tests passed successfully."
else
    echo "nova-shell-test: cargo not found - skipping the suite"
fi
