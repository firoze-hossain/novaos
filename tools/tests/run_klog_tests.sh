#!/bin/sh
# Phase 89 & Month 4: host-side tests for structured kernel logging:
# 1. Kernel subsystem (kernel/rust/klog.rs): ring-buffer storage, wrap-around,
#    monotonic sequence numbers, incremental streaming, and filtering.
# 2. Ring-3 Log Viewer App (userland/klog-rs/klog.rs): UI state, filtering,
#    search highlighting, inspector, and BufferSurface32 rendering.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_klog_tests.sh      (or: make klog-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/rust/klog.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/klog_rust" kernel/rust/klog.rs
    "$OUT/klog_rust" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/klog_rust" >/dev/null 2>&1
    echo "[PASS] All kernel/rust/klog.rs host tests succeeded."

    echo "--- userland/klog-rs host tests (NovaOS Graphical Log Viewer) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/klog_app" userland/klog-rs/klog.rs
    "$OUT/klog_app" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/klog_app" --test-threads=1 >/dev/null
    echo "[PASS] All userland/klog-rs host tests succeeded."
else
    echo "klog-test: rustc not found - skipping the Rust suite"
fi
