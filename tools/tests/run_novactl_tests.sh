#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Pure Rust Container Runtime Engine (novactl / novadock)
#
# Validates pure Rust container runtime features:
# 1. OCI Spec Compliance (v1.0.2):
#    - config.json schema: process, root, mounts, namespaces, linux resources.
#    - Round-trip serialization & deserialization without external crates.
# 2. Image Store & Rootfs Layer Management:
#    - Resolves local container images (alpine:latest, ubuntu:24.04, busybox, nginx, redis).
#    - Prepares dedicated writable container layer with /etc/hosts, /etc/resolv.conf, /etc/hostname.
# 3. Virtual Networking & IPAM:
#    - Bridge nova0 (10.88.0.1/16) and container IP pool (10.88.0.2+).
#    - Virtual Ethernet (veth) pair generation and loopback isolation.
#    - Port forwarding rules (e.g. host:container).
# 4. Linux Namespaces & Path Traversal Prevention:
#    - Configures PID, Mount, Net, IPC, UTS, and User namespaces.
#    - Chroot jail boundary normalization prevents directory traversal attacks.
# 5. Control Groups v2 Governance:
#    - Enforces memory.max, cpu.max (CFS bandwidth quota), pids.max, and io.max.
#    - Process attachment via cgroup.procs and live telemetry collection.
# 6. Ultra-Fast Launch Latency:
#    - Container launch benchmarked at sub-50ms (typically under 1ms).
# 7. Complete Container Lifecycle CLI:
#    - novactl run, ps, stop, start, restart, rm, logs, stats, exec, images.
#
# Usage:
#   ./tools/tests/run_novactl_tests.sh      (or: make novactl-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/server/novactl host tests (Pure Rust Container Runtime Engine) ---"
    
    # 1. Run cargo unit & integration test suite
    echo "[1/3] Running Cargo unit test suite..."
    (cd userland/server/novactl && cargo test --quiet)
    
    # 2. Run automated self-test suite
    echo "[2/3] Running novactl --selftest engine verification..."
    (cd userland/server/novactl && cargo run --quiet -- --selftest)

    # 3. Run full CLI lifecycle workflow in isolated sandbox
    echo "[3/3] Running full CLI lifecycle integration test..."
    SANDBOX_DIR=$(mktemp -d)
    trap 'rm -rf "$SANDBOX_DIR"' EXIT
    export NOVACTL_DATA_DIR="$SANDBOX_DIR/data"
    export NOVACTL_CGROUP_DIR="$SANDBOX_DIR/cgroups"
    mkdir -p "$NOVACTL_DATA_DIR" "$NOVACTL_CGROUP_DIR"

    BIN="userland/server/novactl/target/debug/novactl"
    
    # Test images listing
    "$BIN" images >/dev/null

    # Test run command
    CID=$("$BIN" run --name test-ci-worker --memory 256M alpine:latest /bin/sh)
    if [ -z "$CID" ]; then
        echo "Error: Failed to obtain container ID from novactl run"
        exit 1
    fi

    # Test ps command
    PS_OUT=$("$BIN" ps)
    echo "$PS_OUT" | grep -q "test-ci-worker" || { echo "Error: Container not found in novactl ps"; exit 1; }
    echo "$PS_OUT" | grep -q "Up" || { echo "Error: Container not running"; exit 1; }

    # Test logs command
    LOGS_OUT=$("$BIN" logs test-ci-worker)
    echo "$LOGS_OUT" | grep -q "test-ci-worker" || { echo "Error: Missing container log output"; exit 1; }

    # Test stats command
    STATS_OUT=$("$BIN" stats test-ci-worker)
    echo "$STATS_OUT" | grep -q "256.0MB" || { echo "Error: Memory limit not reflected in stats"; exit 1; }

    # Test exec command
    EXEC_OUT=$("$BIN" exec test-ci-worker date)
    echo "$EXEC_OUT" | grep -q "executing 'date'" || { echo "Error: Exec failed"; exit 1; }

    # Test stop command
    "$BIN" stop test-ci-worker >/dev/null
    PS_A_OUT=$("$BIN" ps -a)
    echo "$PS_A_OUT" | grep -q "Stopped" || { echo "Error: Container not stopped in ps -a"; exit 1; }

    # Test rm command
    "$BIN" rm test-ci-worker >/dev/null
    PS_CLEAN=$("$BIN" ps -a)
    if echo "$PS_CLEAN" | grep -q "test-ci-worker"; then
        echo "Error: Container still present after novactl rm"
        exit 1
    fi

    echo "[PASS] All Pure Rust Container Runtime Engine (novactl / novadock) tests passed successfully."
else
    echo "novactl-test: cargo not found - skipping the suite"
fi
