#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Control Groups v2 (cgroups v2)
#
# Validates comprehensive Linux-grade resource governance in pure Rust:
# 1. Unified hierarchy:
#    - Root cgroup (id 0, path /) with all controllers available and subtree control.
#    - Subtree control propagation to children (+cpu +memory +io +pids).
#    - Creation of child cgroups (e.g. /container-alpha).
# 2. PIDs Controller (pids.max, pids.current, pids.events):
#    - Process limit capping and fork-bomb prevention (EAGAIN on limit hit).
# 3. CPU Bandwidth Controller (cpu.max, cpu.weight, cpu.stat):
#    - CFS bandwidth quota/period enforcement and automatic throttling.
#    - Automatic unthrottling on period rollover.
# 4. Memory Controller (memory.max, memory.high, memory.current, memory.stat, memory.oom.group):
#    - Memory allocation charging and watermark tracking.
#    - Soft throttle warning at memory.high.
#    - Hard ceiling enforcement and automatic OOM killer execution at memory.max.
# 5. I/O Throttling Controller (io.max, io.weight, io.stat):
#    - Per-device bandwidth (rbps/wbps) and IOPS (riops/wiops) limits.
# 6. Cgroup Freezing (cgroup.freeze, cgroup.events):
#    - Atomic freezing of tasks within cgroup tree.
# 7. CgroupFS virtual filesystem:
#    - Virtual read/write and mkdir/rmdir operations.
#
# Usage:
#   ./tools/tests/run_cgroups_tests.sh      (or: make cgroups-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/security/cgroups/ host tests (Control Groups v2 Unified Hierarchy) ---"
    rustc --edition 2021 --test -o "$OUT/cgroups_test" kernel/src/security/cgroups/mod.rs
    "$OUT/cgroups_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/cgroups_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/security/cgroups/ tests succeeded."

    echo "[PASS] All Control Groups v2 (cgroups v2) resource governance tests passed successfully."
else
    echo "cgroups-test: rustc not found - skipping the Rust suite"
fi
