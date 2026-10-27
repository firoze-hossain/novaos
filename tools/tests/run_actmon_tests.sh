#!/bin/sh
# Month 5: host-side tests for Activity & Performance Monitor:
# 1. Kernel subsystem (kernel/rust/procman.rs): system activity metrics,
#    physical memory calculation, process table iteration, PID lookup.
# 2. Ring-3 Activity Monitor App (userland/actmon-rs/actmon.rs): UI state,
#    tab switching, sorting (PID, CPU, Memory), filtering, and BufferSurface32 rendering.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_actmon_tests.sh      (or: make actmon-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/rust/procman.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/procman_rust" kernel/rust/procman.rs
    "$OUT/procman_rust" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/procman_rust" >/dev/null 2>&1
    echo "[PASS] All kernel/rust/procman.rs host tests succeeded."

    echo "--- userland/actmon-rs host tests (NovaOS Activity Monitor) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/actmon_app" userland/actmon-rs/actmon.rs
    "$OUT/actmon_app" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/actmon_app" --test-threads=1 >/dev/null
    echo "[PASS] All userland/actmon-rs host tests succeeded."
else
    echo "actmon-test: rustc not found - skipping the Rust suite"
fi
