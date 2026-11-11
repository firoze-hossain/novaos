#!/bin/sh
# Month 2 - Storage & Async I/O: host-side tests for Concurrent VFS with Lockless Dcache & Async Inotify
# 1. 64-partition striped Directory Entry Cache (dcache) hash lookups, hits, and misses.
# 2. Negative dentry caching to accelerate lookups for non-existent paths.
# 3. LRU dentry eviction and invalidation on unlink/rmdir.
# 4. Asynchronous inotify instance creation, watch descriptors, and event masks.
# 5. Inotify non-blocking event queues and dispatch on mutating VFS operations.
# 6. Global mount table resolution across /, /dev, /proc, /sys, and /mnt.
# 7. Hierarchical path resolution with . and .. normalization.
# 8. POSIX DAC permission enforcement across owner, group, other, and root override.
# 9. Symbolic link resolution and 16-hop recursion loop detection (ELOOP).
# 10. Hard link creation and inode refcounting (nlink).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_vfs_tests.sh      (or: make vfs-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/fs/vfs/dcache.rs host tests (Lockless Dentry Cache) ---"
    rustc --edition 2021 --test -o "$OUT/dcache_test" kernel/src/fs/vfs/dcache.rs
    "$OUT/dcache_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/dcache_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/fs/vfs/dcache.rs tests succeeded."

    echo "--- kernel/src/fs/vfs/inotify.rs host tests (Async Inotify Engine) ---"
    rustc --edition 2021 --test -o "$OUT/inotify_test" kernel/src/fs/vfs/inotify.rs
    "$OUT/inotify_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/inotify_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/fs/vfs/inotify.rs tests succeeded."

    echo "--- kernel/src/fs/vfs/mod.rs host tests (Concurrent VFS & Path Resolution) ---"
    rustc --edition 2021 --test -o "$OUT/vfs_test" kernel/src/fs/vfs/mod.rs
    "$OUT/vfs_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/vfs_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/fs/vfs/mod.rs tests succeeded."

    echo "[PASS] All Concurrent VFS, Dcache & Inotify host tests succeeded."
else
    echo "vfs-test: rustc not found - skipping the Rust suite"
fi
