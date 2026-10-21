#!/bin/sh
# Phase 90: host-side tests for the package-repository network protocol
# support, server side (kernel/rust/pkgrepo.rs): HTTP request parsing,
# package verification, response formatting, status reporting, and server lifecycle.
#
# No QEMU needed; a few seconds. The in-OS conformance test is PKGREPOTEST.ELF,
# run by `make test`.
#
#   ./tools/tests/run_pkgrepo_tests.sh      (or: make pkgrepo-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/rust/pkgrepo.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/pkgrepo_rust" kernel/rust/pkgrepo.rs
    "$OUT/pkgrepo_rust" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/pkgrepo_rust" >/dev/null 2>&1
    echo "[PASS] All kernel/rust/pkgrepo.rs host tests succeeded."
else
    echo "pkgrepo-test: rustc not found - skipping the Rust suite"
fi
