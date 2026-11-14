#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for macOS-Style Persistent Unified Top Menu Bar (nova-shell)
# 1. TopMenuBar geometry: persistent 28px height, acrylic blur background, delicate bottom border.
# 2. Dynamic App Menu Tree: system menu logo, active window focus synchronizer, dynamic category tree over IPC.
# 3. Interactive Menu Scrubbing: click-to-open, mouse hover scrubbing between adjacent categories, click outside/re-click dismiss.
# 4. Keyboard Shortcuts: accelerator dispatch table (⌘N, ⌃⌘Q, etc.) matching active window and system actions.
# 5. Right-Aligned System Status Indicators:
#    - Clock / Calendar flyout with live time updates
#    - Network indicator (Wi-Fi/Ethernet status pill and IP switcher)
#    - Audio volume slider and mute toggle
#    - Battery level percentage and charging state indicator
#    - User account pill (avatar badge and session controls)
# 6. Flyout Popover Rendering: Level 3 elevation shadows, rounded acrylic cards, and subpixel positioning.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_shell_tests.sh      (or: make nova-shell-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/nova-shell host tests (macOS-Style Persistent Unified Top Menu Bar) ---"
    (cd userland/desktop/nova-shell && cargo test --quiet)
    (cd userland/desktop/nova-shell && cargo run --quiet -- --selftest)
    echo "[PASS] All macOS-Style Persistent Unified Top Menu Bar (nova-shell) tests passed successfully."
else
    echo "nova-shell-test: cargo not found - skipping the suite"
fi
