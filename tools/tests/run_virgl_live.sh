#!/bin/sh
# Phase 82: boots the kernel against a REAL virglrenderer and runs the
# whole 3D self-test (capsets, context, clear, shaded triangle in both Y
# conventions, constant buffer + alpha blend, depth test with a control
# run, texture + index buffer, scanout), plus every ordinary assertion,
# on the virgl-capable device.
#
# What this needs, and why it is a separate target from `make test`:
# virgl runs OpenGL on the HOST, so QEMU needs a GL-capable display. The
# one that works headlessly here is `-display gtk,gl=on` on a virtual X
# server (Xvfb) with Mesa's software rasterizer. (Found by trying them:
# `egl-headless` needs a DRM render node that containers do not have, and
# SDL's GL path aborts inside epoxy.) If those are not installed this
# script says so and exits 0 - the same stance as `make libc-test` on a
# machine that cannot run its binaries - rather than failing a build for
# lack of a GPU stack.
#
#   ./tools/tests/run_virgl_live.sh [test_runner.py args...]   (or: make test-3d)
set -e
cd "$(dirname "$0")/../.."

missing=""
command -v Xvfb >/dev/null 2>&1 || missing="$missing Xvfb"
qemu-system-x86_64 -device help 2>&1 | grep -q 'virtio-gpu-gl-pci' || missing="$missing qemu-virtio-gpu-gl"
qemu-system-x86_64 -display help 2>&1 | grep -q '^gtk' || missing="$missing qemu-gtk-display"
if [ -n "$missing" ]; then
    echo "test-3d: SKIPPED - needs:$missing (and Mesa software GL). On Debian/Ubuntu:"
    echo "         apt-get install xvfb qemu-system-gui qemu-system-modules-opengl libvirglrenderer1"
    exit 0
fi

XVFB_PID=""
if [ -z "$DISPLAY" ]; then
    DISP=":$((90 + $$ % 9))"
    Xvfb "$DISP" -screen 0 1280x1024x24 +extension GLX +render -noreset >build/xvfb.log 2>&1 &
    XVFB_PID=$!
    export DISPLAY="$DISP"
    sleep 2
fi
cleanup() { [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true; }
trap cleanup EXIT

LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
    python3 tools/python/test_runner.py --boot --virgl "$@"
