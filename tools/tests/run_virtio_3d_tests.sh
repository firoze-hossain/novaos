#!/bin/sh
# Month 4 - Graphics & DRM: host-side tests for VirtIO-GPU 3D Hardware Acceleration & Virgl Mesa Bridge
# 1. Virgl 3D command header packing (opcode, object type, length).
# 2. 3D bounding box dimensions and depth slicing.
# 3. Viewport transformation and hardware clear command stream generation.
# 4. TGSI vertex and fragment shader creation and DRAW_VBO primitive emission.
# 5. 3D Context lifecycle management (create, attach resources, detach, destroy).
# 6. 3D GPU Resource creation (Render Targets, Vertex Buffers, Textures).
# 7. Asynchronous 3D command submission and GPU synchronization fence retirement.
# 8. Userland Mesa / Gallium bridge IOCTL dispatcher (WAIT_FENCE, SUBMIT_3D, TRANSFER_3D).
# 9. In-kernel 3D pipeline self-test suite (run_virtio_3d_selftest).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_virtio_3d_tests.sh      (or: make virgl3d-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/drivers/gpu/virgl_cmd.rs host tests (Virgl 3D Command Buffer Encoder) ---"
    rustc --edition 2021 --test -o "$OUT/virgl_cmd_test" kernel/src/drivers/gpu/virgl_cmd.rs
    "$OUT/virgl_cmd_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/virgl_cmd_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/gpu/virgl_cmd.rs tests succeeded."

    echo "--- kernel/src/drivers/gpu/virtio_3d.rs host tests (VirtIO-GPU 3D & Mesa Bridge) ---"
    rustc --edition 2021 --test -o "$OUT/virtio_3d_test" kernel/src/drivers/gpu/virtio_3d.rs
    "$OUT/virtio_3d_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/virtio_3d_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/gpu/virtio_3d.rs tests succeeded."

    echo "[PASS] All VirtIO-GPU 3D Acceleration & Virgl Mesa Bridge host tests succeeded."
else
    echo "virgl3d-test: rustc not found - skipping the Rust suite"
fi
