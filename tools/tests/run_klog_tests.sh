#!/bin/sh
# Phase 89: host-side tests for the structured, queryable kernel logging
# subsystem (kernel/rust/klog.rs): ring-buffer storage, wrap-around and gap
# detection, monotonic sequence numbers, incremental streaming, severity
# filtering, facility filtering, and concurrency safety.
#
# No QEMU needed; a few seconds. The in-OS conformance test is KLOGTEST.ELF,
# run by `make test`.
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
else
    echo "klog-test: rustc not found - skipping the Rust suite"
fi
