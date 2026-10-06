#!/bin/sh
# Phase 85: host-side tests for audio mixing: kernel/rust/mixer.rs run against
# a mock that SIMULATES the AC97 DMA engine - time passes only when the test
# says so, the engine plays one descriptor after another at 48kHz, HALTS after
# finishing the last valid index, and every sample the DAC would play is
# recorded - so the tests check what would be HEARD, including a stale period
# replayed after the system fell behind. Also an INDEPENDENT oracle for the
# mixer (a 60,000-operation random run compared sample by sample), resampler
# property tests (a constant stays constant, a tone keeps its pitch), and a
# check that reads userland/libc/include/nova_audio_abi.h and fails if any
# limit, error code or stat-array index disagrees with the kernel's.
#
# No QEMU needed; a few seconds. The in-OS conformance test is AUDIOTEST.ELF,
# run by `make test`.
#
#   ./tools/tests/run_audio_tests.sh      (or: make audio-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/rust/mixer.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/mixer_rust" kernel/rust/mixer.rs
    "$OUT/mixer_rust" 2>&1 | grep -E "^test result|FAILED|panicked" || true
    "$OUT/mixer_rust" >/dev/null 2>&1
else
    echo "audio-test: rustc not found - skipping the Rust suite"
fi
