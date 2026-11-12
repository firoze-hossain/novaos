#!/bin/sh
# Month 2 - Storage & Async I/O: host-side tests for Kernel Memory Page Cache & Anonymous Swap Subsystem
# 1. Unified Page Cache reading and writing 4KB memory pages in unallocated RAM.
# 2. Hash bucket indexing for fast O(1) page lookup by (inode_id, page_index).
# 3. Dual-List LRU (Active and Inactive) page reclaim algorithm with promotion & demotion.
# 4. Dirty page tracking (PAGE_DIRTY) and asynchronous flush engine to underlying storage.
# 5. Clean page eviction from inactive list tail without disk I/O.
# 6. Pure #![no_std] Rust LZ4 block compressor and decompressor with 100% roundtrip fidelity.
# 7. Anonymous swap subsystem with zswap in-RAM compressed memory pool.
# 8. High memory pressure watermark (85%) triggering page reclaim and swap out.
# 9. Incompressible page fallback to swap disk partition.
# 10. Swap slot freeing without memory leaks on unmap or process exit.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_pagecache_swap_tests.sh      (or: make pagecache-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/mm/pagecache.rs host tests (Unified Page Cache & 2-List LRU) ---"
    rustc --edition 2021 --test -o "$OUT/pagecache_test" kernel/src/mm/pagecache.rs
    "$OUT/pagecache_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/pagecache_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/mm/pagecache.rs tests succeeded."

    echo "--- kernel/src/mm/swap.rs host tests (Anonymous Swap, zswap & Pure Rust LZ4) ---"
    rustc --edition 2021 --test -o "$OUT/swap_test" kernel/src/mm/swap.rs
    "$OUT/swap_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/swap_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/mm/swap.rs tests succeeded."

    echo "[PASS] All Page Cache & Anonymous Swap Subsystem host tests succeeded."
else
    echo "pagecache-test: rustc not found - skipping the Rust suite"
fi
