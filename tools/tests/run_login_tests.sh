#!/bin/sh
# Interface Track / Month 3 - Core desktop apps: host-side tests for the NovaOS
# Graphical Login Screen (userland/login-rs): verifying persistent account parsing
# (USERS.CFG / SYSTEM.CFG), credential verification via SYS_LOGIN, identity
# transition & isolation (persisted uid 700, admin uid 800), password buffer
# handling, masking, hero digital clock, session selector, and 32-bit BufferSurface32 rasterization.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_login_tests.sh      (or: make login-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/login-rs host tests (NovaOS Graphical Login Screen) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/login_test" userland/login-rs/login.rs
    "$OUT/login_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/login_test" --test-threads=1 >/dev/null
    echo "[PASS] All userland/login-rs host tests succeeded."
else
    echo "login-test: rustc not found - skipping the Rust suite"
fi
