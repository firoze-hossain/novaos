#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Kernel Namespaces (PID, Mount, Net, IPC, UTS, User)
#
# Validates comprehensive Linux-grade isolation namespaces in pure Rust:
# 1. PID Namespace:
#    - Multi-level hierarchical process tree and translation mapping ((ns_id, local_pid) <-> host_pid).
#    - Dedicated PID 1 (container init) per namespace with cascading kill on init exit.
#    - Visibility boundaries (child cannot see or signal parent/sibling processes).
# 2. Mount Namespace:
#    - Isolated mount tables (/proc, /sys, /dev, /tmp).
#    - Strict chroot jail boundary preventing directory traversal (../) escape.
#    - Mount propagation (MS_PRIVATE, MS_SHARED, MS_SLAVE) and pivot_root semantics.
# 3. Network Namespace:
#    - Dedicated virtual loopback interface (lo 127.0.0.1/8) per network namespace.
#    - Virtual Ethernet pairs (veth) bridging container to host.
#    - Isolated port binding (multiple namespaces can each bind port 80 without conflict).
# 4. IPC Namespace:
#    - Isolated shared memory (shm), message queues (mq), and semaphores (sem).
#    - Complete key/id separation across containers.
# 5. UTS Namespace:
#    - Isolated hostname (nodename) and domainname.
#    - Independent sethostname / setdomainname without host pollution.
# 6. User Namespace:
#    - Bidirectional UID / GID mapping tables (inside_id <-> outside_id).
#    - Unprivileged host users mapping to root (UID 0) inside container namespaces.
#    - Confined container root capabilities (CAP_CHROOT, CAP_NET_ADMIN, CAP_SYS_ADMIN).
# 7. Syscalls:
#    - sys_unshare(flags) and sys_setns(fd, nstype).
#    - clone(CLONE_NEW*) integration into process manager.
#
# Usage:
#   ./tools/tests/run_namespace_tests.sh      (or: make namespace-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/security/namespace/ host tests (Linux-Grade Isolation Namespaces) ---"
    rustc --edition 2021 --test -o "$OUT/namespace_test" kernel/src/security/namespace/mod.rs
    "$OUT/namespace_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/namespace_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/security/namespace/ tests succeeded."

    echo "[PASS] All Kernel Namespaces (PID, Mount, Net, IPC, UTS, User) tests passed successfully."
else
    echo "namespace-test: rustc not found - skipping the Rust suite"
fi
