#!/bin/sh
# Month 2 - Storage & Async I/O: host-side tests for Asynchronous I/O Ring Engine (nova_uring)
# 1. 64-byte SQE and 16-byte CQE layouts matching standard hardware cache lines.
# 2. Lockless single-producer single-consumer circular ring index arithmetic.
# 3. Batch request submission into shared memory submission queue (SQ).
# 4. Zero-syscall completion harvesting from shared memory completion queue (CQ).
# 5. Async operations: NOP, READ, WRITE, RECV, SEND, FSYNC, TIMEOUT.
# 6. Ring wrap-around correctness across multiple circular passes.
# 7. Sequential linked SQE chain execution (URING_SQE_LINK).
# 8. Kernel SQPOLL worker polling simulation without userland context switches.
# 9. Robust error handling (invalid file descriptors and unsupported opcodes return negative errno).
# 10. High-throughput userland library (nova-io) scaling to 100,000+ operations/sec.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_uring_tests.sh      (or: make uring-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/io/uring.rs host tests (Kernel Async Ring Engine) ---"
    rustc --edition 2021 --test -o "$OUT/uring_test" kernel/src/io/uring.rs
    "$OUT/uring_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/uring_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/io/uring.rs tests succeeded."

    echo "--- userland/libs/nova-io/src/lib.rs host tests (Userland nova-io Library) ---"
    rustc --edition 2021 --test -o "$OUT/nova_io_test" userland/libs/nova-io/src/lib.rs
    "$OUT/nova_io_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/nova_io_test" >/dev/null 2>&1
    echo "[PASS] All userland/libs/nova-io/src/lib.rs tests succeeded."

    echo "[PASS] All Asynchronous I/O Ring Engine (nova_uring & nova-io) host tests succeeded."
else
    echo "uring-test: rustc not found - skipping the Rust suite"
fi
