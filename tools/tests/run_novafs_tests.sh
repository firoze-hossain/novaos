#!/bin/sh
# Month 2 - Storage & Async I/O: host-side tests for Pure Rust Enterprise Filesystem (NovaFS)
# 1. 64-bit B-Tree indexing with composite keys (object_id, item_type, offset).
# 2. Pure #![no_std] SHA-256 block checksumming and automatic bit-rot detection.
# 3. 64-bit extent allocation and payload verification.
# 4. Copy-on-Write (CoW) node duplication and history preservation.
# 5. Dual redundant superblocks (primary and backup) with generation tracking.
# 6. Write-Ahead Journaling (WAL) for atomic multi-block crash consistency.
# 7. Instantaneous O(1) zero-cost atomic snapshots (sub-second creation).
# 8. Subvolume branching (writable container roots and overlays).
# 9. CoW tree divergence and snapshot immutability.
# 10. Instantaneous snapshot rollback restoring historical state.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_novafs_tests.sh      (or: make novafs-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/fs/novafs/btree.rs host tests (B-Tree Indexing & SHA-256 Extents) ---"
    rustc --edition 2021 --test -o "$OUT/btree_test" kernel/src/fs/novafs/btree.rs
    "$OUT/btree_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/btree_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/fs/novafs/btree.rs tests succeeded."

    echo "--- kernel/src/fs/novafs/mod.rs host tests (Superblock, WAL, Snapshots & Engine) ---"
    rustc --edition 2021 --test -o "$OUT/novafs_test" kernel/src/fs/novafs/mod.rs
    "$OUT/novafs_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/novafs_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/fs/novafs/mod.rs tests succeeded."

    echo "[PASS] All Pure Rust Enterprise Filesystem (NovaFS) host tests succeeded."
else
    echo "novafs-test: rustc not found - skipping the Rust suite"
fi
