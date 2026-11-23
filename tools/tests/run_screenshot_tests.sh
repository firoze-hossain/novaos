#!/bin/sh
# Host tests for NovaOS Sovereign Screenshot & Screen Recording Utility
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/screenshot-rs host tests (Sovereign Screenshot & Screen Recorder) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/screenshot_test" userland/screenshot-rs/screenshot.rs
    "$OUT/screenshot_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/screenshot_test" >/dev/null 2>&1
    echo "[PASS] All userland/screenshot-rs host tests succeeded."
else
    echo "screenshot-test: rustc not found - skipping test suite"
fi
