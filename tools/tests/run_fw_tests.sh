#!/bin/sh
# Phase 88: host-side tests for the stateful firewall (kernel/rust/firewall.rs):
# rule and configuration parsing (every error class, the exact limits), packet
# header parsing (truncated and inconsistent headers), the TCP connection state
# machine through every state, UDP and ICMP tracking, ICMP errors as RELATED,
# the TFTP helper (a reply from a port other than the one asked), timeouts to
# the exact tick and across the 32-bit tick counter's wrap, the connection-table
# and half-open limits, the administrative lock, a model-based test (every
# legitimate session accepted, every stray packet dropped) and fuzzing.
#
# No QEMU needed; a few seconds. The in-OS conformance test is FWTEST.ELF, run
# by `make test`.
#
#   ./tools/tests/run_fw_tests.sh      (or: make fw-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/rust/firewall.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/fw_rust" kernel/rust/firewall.rs
    "$OUT/fw_rust" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/fw_rust" >/dev/null 2>&1
else
    echo "fw-test: rustc not found - skipping the Rust suite"
fi
