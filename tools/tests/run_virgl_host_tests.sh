#!/bin/sh
# Phase 82: the host-side tests for kernel/rust/virgl.rs - the virgl
# command-stream encoder, the virtio-gpu 3D control builders, the capset
# parser (checked against a real capset captured from a live host), and
# the whole orchestrator run against a mock GPU with a software
# rasterizer: success, a failure injected at EVERY command (teardown
# must leave nothing behind), and every submission silently dropped in
# turn (the real host's failure mode - it answers OK to a stream it
# rejected).
#
# Standalone `rustc --test`, the same convention as the other kernel
# Rust modules (see PROGRESS.md). Needs no QEMU and no GL; a few
# seconds.
#
#   ./tools/tests/run_virgl_host_tests.sh      (or: make virgl-test)
set -e
cd "$(dirname "$0")/../.."
if ! command -v rustc >/dev/null 2>&1; then
    echo "virgl-test: SKIPPED - rustc not found"
    exit 0
fi
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
rustc --edition 2021 --test -o "$OUT/virgl_tests" kernel/rust/virgl.rs
"$OUT/virgl_tests"
