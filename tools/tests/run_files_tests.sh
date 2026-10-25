#!/bin/sh
# Interface Track / Month 3 - Core desktop apps: host-side tests for the NovaOS
# Graphical File Manager (userland/files-rs): verifying VFS directory parsing,
# metadata classification, mouse selection, preview loading, move & delete operations,
# and in-memory 32-bit BufferSurface32 rendering.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_files_tests.sh      (or: make files-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/files-rs host tests (NovaOS Graphical File Manager) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/files_test" userland/files-rs/files.rs
    "$OUT/files_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/files_test" >/dev/null 2>&1
    echo "[PASS] All userland/files-rs host tests succeeded."
else
    echo "files-test: rustc not found - skipping the Rust suite"
fi
