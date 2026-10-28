#!/bin/sh
# Month 6: host-side tests for Graphical Crash Reporter UI:
# 1. Crash report parser (userland/crashrep-rs/parser.rs): text & metadata parsing,
#    exception vector classification, fault analysis, symbol hints, and hex formatting.
# 2. Ring-3 Crash Reporter App (userland/crashrep-rs/crashrep.rs):
#    initialization, tab navigation state machine, record clearing,
#    BufferSurface32 graphical rasterization, and selftest verification.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_crashrep_tests.sh      (or: make crashrep-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/crashrep-rs host tests (NovaOS Graphical Crash Reporter) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/crashrep_test" userland/crashrep-rs/crashrep.rs
    "$OUT/crashrep_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/crashrep_test" --test-threads=1 >/dev/null
    echo "[PASS] All userland/crashrep-rs host tests succeeded."
else
    echo "crashrep-test: rustc not found - skipping the Rust suite"
fi
