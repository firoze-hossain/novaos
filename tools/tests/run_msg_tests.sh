#!/bin/sh
# Phase 84: host-side tests for app-to-app messaging: kernel/rust/msg.rs run
# against a mock process table (liveness, uids, the clock) and an
# INDEPENDENT model - a deliberately simple second statement of the rules
# that predicts the exact outcome (result code, delivered bytes, sender,
# type, tag, counters) of every operation in a 100,000-step random run that
# includes processes dying WITHOUT their exit hook having run. Also reads
# userland/libc/include/nova_msg_abi.h and fails if any limit or error code
# disagrees with the kernel's constant.
#
# No QEMU needed; a few seconds. The in-OS conformance test is MSGTEST.ELF,
# run by `make test`.
#
#   ./tools/tests/run_msg_tests.sh      (or: make msg-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/rust/msg.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/msg_rust" kernel/rust/msg.rs
    "$OUT/msg_rust" 2>&1 | grep -E "^test result|FAILED|panicked" || true
    "$OUT/msg_rust" >/dev/null 2>&1
else
    echo "msg-test: rustc not found - skipping the Rust suite"
fi
