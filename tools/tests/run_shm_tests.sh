#!/bin/sh
# Phase 83: host-side tests for shared-memory IPC. Two independent suites:
#
#  * kernel/rust/shm.rs - the kernel subsystem, run against a mock MMU that
#    PANICS on a double free, a free of a never-allocated frame, mapping a
#    frame that was not zeroed, or mapping over an existing page-table
#    entry; a hand-written model checks every permission outcome of 30,000
#    random operations; failure is injected at every frame allocation and
#    every page mapping.
#  * tools/tests/novashm_test.c - the userland triple buffer
#    (userland/libc/include/novashm_chan.h): an exhaustive model check of
#    every reachable state of the protocol (which must also FLAG two
#    deliberately broken protocols), a two-thread stress test with torn-frame
#    detection, and validation of everything attach() reads from shared
#    memory. Also built under ThreadSanitizer and UBSan/ASan.
#
# No QEMU needed; well under a minute. The in-OS conformance test is
# SHMTEST.ELF, run by `make test`.
#
#   ./tools/tests/run_shm_tests.sh      (or: make shm-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/rust/shm.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/shm_rust" kernel/rust/shm.rs
    "$OUT/shm_rust" 2>&1 | grep -E "^test result|FAILED|panicked" || true
    "$OUT/shm_rust" >/dev/null 2>&1
else
    echo "shm-test: rustc not found - skipping the Rust suite"
fi

if command -v gcc >/dev/null 2>&1; then
    # -idirafter, NOT -I: the NovaOS libc has headers named like the host's
    # (stdlib.h, stdio.h, ...) and the host's must win for everything except
    # the NovaOS-only one this test needs (novashm_chan.h).
    CFLAGS="-Wall -Wextra -idirafter userland/libc/include"
    echo "--- novashm_test (native, -O2) ---"
    gcc -O2 $CFLAGS -pthread -o "$OUT/nv" tools/tests/novashm_test.c
    "$OUT/nv"

    echo "--- novashm_test (UBSan + ASan) ---"
    gcc -O1 -g $CFLAGS -pthread -fsanitize=undefined,address -fno-sanitize-recover=undefined \
        -o "$OUT/nv_asan" tools/tests/novashm_test.c
    "$OUT/nv_asan"

    # ThreadSanitizer: the stress test hands pixel data between threads using
    # nothing but the header's atomic exchanges, so any missing acquire/
    # release shows up here as a data race even though x86 would hide it.
    # Not every toolchain ships the runtime, so absence is a skip, not a failure.
    if echo 'int main(void){return 0;}' | gcc -x c -fsanitize=thread -o "$OUT/tsan_probe" - 2>/dev/null; then
        echo "--- novashm_test (ThreadSanitizer) ---"
        gcc -O1 -g $CFLAGS -pthread -fsanitize=thread -o "$OUT/nv_tsan" tools/tests/novashm_test.c
        "$OUT/nv_tsan"
    else
        echo "--- ThreadSanitizer not available here - skipped ---"
    fi
else
    echo "shm-test: gcc not found - skipping the C suite"
fi
