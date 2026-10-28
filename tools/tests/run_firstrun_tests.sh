#!/bin/sh
# Month 6: host-side tests for First-Run Setup Wizard:
# 1. Userland crypto engine (userland/firstrun-rs/crypto.rs): SHA-256,
#    PBKDF2-HMAC-SHA256 RFC vectors, salt derivation, constant-time equality.
# 2. Ring-3 First-Run Setup Wizard App (userland/firstrun-rs/firstrun.rs):
#    wizard state machine, hardware info, machine identity & hostname (SYSTEM.CFG),
#    administrator account creation (USERS.CFG), network configuration (FIREWALL.CFG),
#    visual preferences & accessibility (ACCESS.CFG), deployment pipeline,
#    and zero-syscall BufferSurface32 rasterization.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_firstrun_tests.sh      (or: make firstrun-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/firstrun-rs/crypto.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/crypto_test" userland/firstrun-rs/crypto.rs
    "$OUT/crypto_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/crypto_test" >/dev/null 2>&1
    echo "[PASS] All userland/firstrun-rs/crypto.rs host tests succeeded."

    echo "--- userland/firstrun-rs host tests (NovaOS First-Run Setup Wizard) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/firstrun_app" userland/firstrun-rs/firstrun.rs
    "$OUT/firstrun_app" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/firstrun_app" --test-threads=1 >/dev/null
    echo "[PASS] All userland/firstrun-rs host tests succeeded."
else
    echo "firstrun-test: rustc not found - skipping the Rust suite"
fi
