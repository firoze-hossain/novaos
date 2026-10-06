#!/bin/sh
# Phase 87: host-side tests for mandatory access control (kernel/rust/mac.rs):
# the profile parser (every error class, the exact size and rule limits, 30,000
# random and mutated inputs that must never panic), the pattern matcher
# (against a naive reference on 40,000 random cases, and against a pattern
# built to blow up a naive matcher), the evaluation of a profile STACK, complain
# mode, failing closed, the profile table (dedupe, versions, full, frozen), and
# a model-based test that renders 400 random profiles to text, loads them
# through the real parser, and compares 80,000 decisions with an independent
# reference. It also parses kernel/arch/x86/cpu/syscall.h and fails if the
# profile syscall table ever disagrees with it.
#
# No QEMU needed; a few seconds. The in-OS conformance test is MACTEST.ELF,
# run by `make test`.
#
#   ./tools/tests/run_mac_tests.sh      (or: make mac-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/rust/mac.rs host tests ---"
    rustc --edition 2021 --test -o "$OUT/mac_rust" kernel/rust/mac.rs
    "$OUT/mac_rust" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/mac_rust" >/dev/null 2>&1
else
    echo "mac-test: rustc not found - skipping the Rust suite"
fi
