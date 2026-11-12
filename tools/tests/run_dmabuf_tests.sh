#!/bin/sh
# Month 4 - Graphics & DRM: host-side tests for Zero-Copy DMA-BUF Framebuffer Sharing & GPU Memory Allocator
# 1. Bit-packed GPU physical memory allocator (contiguous scanout runs, scatter-gather fallback, zero-leak reclaim).
# 2. Client-to-Compositor export/import FD capability handoff & ACL isolation.
# 3. Simultaneous multi-process MMU mapping with identical 4KB physical frame proof + trailing guard pages.
# 4. Cache coherency barriers (SYS_DMABUF_SYNC START/END) & device engine attachments (DRM scanout, VirtIO 3D).
# 5. Crash-safe refcounted teardown (cleanup_process) & full syscall dispatcher (SYS_DMABUF_*).
# 6. In-kernel DMA-BUF self-test suite (run_dmabuf_selftest).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_dmabuf_tests.sh      (or: make dmabuf-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/drivers/gpu/dmabuf.rs host tests (GPU Memory Allocator & Zero-Copy Sharing) ---"
    rustc --edition 2021 --test -o "$OUT/dmabuf_test" kernel/src/drivers/gpu/dmabuf.rs
    "$OUT/dmabuf_test" --test-threads=1 2>&1 | grep -E "^test |^test result|FAILED|panicked|failed" || true
    "$OUT/dmabuf_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/gpu/dmabuf.rs tests succeeded."

    echo "--- kernel/src/syscall/drm.rs host tests (SYS_DMABUF_* Syscall Dispatcher & Compositor Swapchain) ---"
    rustc --edition 2021 --test -o "$OUT/drm_syscall_test" kernel/src/syscall/drm.rs
    "$OUT/drm_syscall_test" --test-threads=1 2>&1 | grep -E "^test |^test result|FAILED|panicked|failed" || true
    "$OUT/drm_syscall_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/syscall/drm.rs tests succeeded."

    echo "[PASS] All Zero-Copy DMA-BUF & GPU Memory Allocator host tests succeeded."
else
    echo "dmabuf-test: rustc not found - skipping the Rust suite"
fi
