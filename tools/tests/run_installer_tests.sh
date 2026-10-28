#!/bin/sh
# Month 6: host-side tests for Graphical Installer Flow:
# 1. Userland crypto engine (userland/installer-rs/crypto.rs): SHA-256,
#    PBKDF2-HMAC-SHA256 RFC vectors, salt derivation, constant-time equality.
# 2. Ring-3 Installer Wizard App (userland/installer-rs/installer.rs):
#    wizard state machine, disk selection, partition layout calculations,
#    user account credential synthesis (USERS.CFG/SYSTEM.CFG), installation pipeline,
#    and zero-syscall BufferSurface32 rasterization.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_installer_tests.sh      (or: make installer-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/installer-rs/crypto.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/crypto_test" userland/installer-rs/crypto.rs
    "$OUT/crypto_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/crypto_test" >/dev/null 2>&1
    echo "[PASS] All userland/installer-rs/crypto.rs host tests succeeded."

    echo "--- userland/installer-rs host tests (NovaOS Graphical Installer) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/installer_app" userland/installer-rs/installer.rs
    "$OUT/installer_app" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/installer_app" --test-threads=1 >/dev/null
    echo "[PASS] All userland/installer-rs host tests succeeded."
else
    echo "installer-test: rustc not found - skipping the Rust suite"
fi
