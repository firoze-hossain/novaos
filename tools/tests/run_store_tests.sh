#!/bin/sh
# Interface Track / Month 3 - Core desktop apps: host-side tests for the NovaOS
# Graphical Software Store & App Launcher (userland/store-rs): verifying package
# manifest parsing, HMAC-SHA256 signature computation & verification, install
# database state management (install/remove), repository catalog search and
# category filtering, in-kernel HTTP package repository telemetry (SYS_PKGREPO_CTL),
# and 32-bit BufferSurface32 rasterization.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_store_tests.sh      (or: make store-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/store-rs host tests (NovaOS Graphical Software Store & Launcher) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/store_test" userland/store-rs/store.rs
    "$OUT/store_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/store_test" >/dev/null 2>&1
    echo "[PASS] All userland/store-rs host tests succeeded."
else
    echo "store-test: rustc not found - skipping the Rust suite"
fi
