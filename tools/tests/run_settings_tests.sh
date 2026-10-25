#!/bin/sh
# Interface Track / Month 3 - Core desktop apps: host-side tests for the NovaOS
# Graphical Settings App (userland/settings-rs): verifying plugin panel registration,
# SYSTEM.CFG serialization, USERS.CFG serialization, SERVICES.CFG parsing,
# mouse hit-testing, action dispatch, and in-memory 32-bit BufferSurface32 rendering.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_settings_tests.sh      (or: make settings-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/settings-rs host tests (NovaOS Graphical Settings App) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/settings_test" userland/settings-rs/settings.rs
    "$OUT/settings_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/settings_test" >/dev/null 2>&1
    echo "[PASS] All userland/settings-rs host tests succeeded."
else
    echo "settings-test: rustc not found - skipping the Rust suite"
fi
