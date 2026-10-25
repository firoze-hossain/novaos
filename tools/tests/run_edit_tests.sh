#!/bin/sh
# Interface Track / Month 3 - Core desktop apps: host-side tests for the NovaOS
# Graphical Text Editor App (userland/edit-rs): verifying text buffer model,
# character insertion, line splitting, backspace line merging, cursor navigation,
# VFS file save and load roundtrip, multi-size font modes (Mono, Sans, Small, Large),
# mouse hit-testing, clipboard copy/paste, and 32-bit BufferSurface32 rasterization.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_edit_tests.sh      (or: make edit-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/edit-rs host tests (NovaOS Graphical Text Editor App) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/edit_test" userland/edit-rs/editor.rs
    "$OUT/edit_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/edit_test" >/dev/null 2>&1
    echo "[PASS] All userland/edit-rs host tests succeeded."
else
    echo "edit-test: rustc not found - skipping the Rust suite"
fi
