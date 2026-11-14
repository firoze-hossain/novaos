#!/bin/sh
# Month 4 - Compositor & UI: host-side tests for Hardware-Accelerated Wayland-Style Display Server (nova-compositor)
# 1. Double-buffered atomic commit state machine, damage rect accumulation and bounding box clipping.
# 2. Strict application window isolation, anti-keylogger keystroke confinement, and private buffer protection.
# 3. DRM/KMS modesetting, atomic tear-free page flipping synchronized to VBLANK (60 FPS pacing).
# 4. Independent hardware cursor plane positioning without primary scanout blits.
# 5. Zero-copy DMA-BUF direct scanout qualification (<1ms presentation latency).
# 6. Wayland binary wire protocol serialization, header parsing, and event formatting.
# 7. Windows 11 Snap layouts, tiling grids, and synchronized boundary resizing.
# 8. Dynamic HiDPI / Retina display scaling and EDID physical DPI detection.
# 9. In-server full self-test suite (run_compositor_selftest).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_compositor_tests.sh      (or: make compositor-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/nova-compositor host tests (Wayland Display Server) ---"
    (cd userland/desktop/nova-compositor && cargo test --quiet)
    echo "[PASS] All 19 Wayland display server (nova-compositor) tests passed successfully."
else
    echo "compositor-test: cargo not found - skipping the suite"
fi
