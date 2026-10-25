#!/bin/sh
# Interface Track / Month 3 - Core desktop apps: host-side tests for the NovaOS
# In-Window Terminal App (userland/term-rs): verifying VT grid geometry, dynamic resize,
# character flow, line wrapping, line scrolling, ANSI escape sequence parsing,
# interactive line editing, command history, shell command dispatch, and 32-bit
# BufferSurface32 rasterization.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_term_tests.sh      (or: make term-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/term-rs host tests (NovaOS In-Window Terminal App) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/term_test" userland/term-rs/term.rs
    "$OUT/term_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/term_test" >/dev/null 2>&1
    echo "[PASS] All userland/term-rs host tests succeeded."
else
    echo "term-test: rustc not found - skipping the Rust suite"
fi
