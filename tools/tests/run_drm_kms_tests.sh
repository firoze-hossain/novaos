#!/bin/sh
# Month 4 - Graphics & DRM: host-side tests for DRM/KMS Kernel Mode-Setting & Display Plane Pipeline
# 1. KMS display mode timing and resolution validation (1024x768, 1280x720, 1920x1080@60Hz, 2560x1440@144Hz).
# 2. KMS device topology (Connectors, Encoders, CRTCs, Hardware Planes).
# 3. Linear dumb buffer allocation, sizing, pitch alignment, and framebuffer registration.
# 4. Atomic tear-free page flipping synchronized to VBLANK interrupt counter and timestamps.
# 5. Hardware cursor plane positioning (x, y) without modifying desktop primary framebuffer pixels.
# 6. Overlay hardware plane with multi-format support (XRGB8888, ARGB8888, NV12) and alpha blending.
# 7. Linux DRM UAPI version (novadrm v2.0.0) and driver description handshake.
# 8. DRM capability queries (DRM_CAP_DUMB_BUFFER, DRM_CAP_PAGE_FLIP, DRM_CAP_CURSOR_WIDTH, etc.).
# 9. DRM resource discovery via DRM_IOCTL_MODE_GETRESOURCES, GETCONNECTOR, GETENCODER, GETPLANES.
# 10. Dynamic modesetting via DRM_IOCTL_MODE_SETCRTC.
# 11. Page flip submission and completion event harvesting via /dev/dri/card0 read().
# 12. Full in-kernel self-test suite (run_drm_kms_selftest).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_drm_kms_tests.sh      (or: make drm-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/drivers/gpu/kms.rs host tests (KMS Display Pipeline & Page Flip) ---"
    rustc --edition 2021 --test -o "$OUT/kms_test" kernel/src/drivers/gpu/kms.rs
    "$OUT/kms_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/kms_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/gpu/kms.rs tests succeeded."

    echo "--- kernel/src/drivers/gpu/drm.rs host tests (DRM Character Device & IOCTLs) ---"
    rustc --edition 2021 --test -o "$OUT/drm_test" kernel/src/drivers/gpu/drm.rs
    "$OUT/drm_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/drm_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/gpu/drm.rs tests succeeded."

    echo "[PASS] All DRM/KMS Kernel Mode-Setting & Display Plane Pipeline host tests succeeded."
else
    echo "drm-test: rustc not found - skipping the Rust suite"
fi
