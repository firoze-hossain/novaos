#!/bin/sh
# Phase 86: host-side tests for per-process resource limits:
# kernel/task/rlimit_policy.c, the PURE rules and arithmetic (window
# accounting, throttle and kill verdicts, who may change what, the page-count
# check, inheritance, the process-count rule), compiled standalone on the
# host and checked at every boundary, against a per-tick reference model
# after every step of a random run, in a simulated scheduler loop (a 20%-capped
# process must get EXACTLY 20 ticks in every one of fifty windows), across the
# wrap of the 32-bit tick counter, and with a fork-bomb simulation. Also built
# under UBSan/ASan.
#
# No QEMU needed; a few seconds. The in-OS conformance test is RLIMTEST.ELF,
# run by `make test`.
#
#   ./tools/tests/run_rlimit_tests.sh      (or: make rlimit-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
if command -v gcc >/dev/null 2>&1; then
    echo "--- rlimit_policy.c host tests (native, -O2) ---"
    gcc -O2 -Wall -Wextra -o "$OUT/rl" tools/tests/rlimit_test.c
    "$OUT/rl"
    echo "--- rlimit_policy.c host tests (UBSan + ASan) ---"
    gcc -O1 -g -Wall -Wextra -fsanitize=undefined,address -fno-sanitize-recover=undefined \
        -o "$OUT/rl_asan" tools/tests/rlimit_test.c
    "$OUT/rl_asan"
else
    echo "rlimit-test: gcc not found - skipping"
fi
