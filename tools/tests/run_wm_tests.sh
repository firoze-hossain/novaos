#!/bin/sh
# Interface Track / Month 2 - Compositor: host-side tests for the NovaOS Framebuffer
# Window Manager (userland/wm-rs): verifying window management logic (dragging,
# z-order, taskbar switching, left/right half snapping, un-snapping), high-resolution
# display geometry layout, and in-memory 32-bit BufferSurface32 rendering.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_wm_tests.sh      (or: make wm-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/wm-rs host tests (NovaOS Framebuffer Window Manager) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/wm_test" userland/wm-rs/wm.rs
    "$OUT/wm_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/wm_test" >/dev/null 2>&1
    echo "[PASS] All userland/wm-rs host tests succeeded."
else
    echo "wm-test: rustc not found - skipping the Rust suite"
fi
