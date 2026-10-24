#!/bin/sh
# Interface Track / Month 1 - Prep: host-side tests for the NovaOS Unified
# Design System (userland/ui-rs): colors, spacing, typography metrics,
# widget styling contracts, clipping, surface rendering, and self-test.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_ui_tests.sh      (or: make ui-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/ui-rs host tests (NovaOS Unified Design System) ---"
    rustc --edition 2021 --test -o "$OUT/ui_rust" userland/ui-rs/src/lib.rs
    "$OUT/ui_rust" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/ui_rust" >/dev/null 2>&1
    echo "[PASS] All userland/ui-rs host tests succeeded."
else
    echo "ui-test: rustc not found - skipping the Rust suite"
fi
