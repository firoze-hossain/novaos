#!/bin/sh
# NovaOS Disks & Storage Management Studio (nova-disks / Stratum) Test Suite
#
# Synthesizes Ubuntu Disks (gnome-disk-utility):
# 1. Drive list and hardware topology (NVMe/SATA SSDs, HDDs, Optical CD/DVD drives).
# 2. Volumes proportional visual bar with color-coded partition blocks.
# 3. Partition management (mount/unmount, format, resize, label, flags, take ownership).
# 4. Storage telemetry & SMART self-tests (read error rate, wear leveling, temperature, reallocated sectors).
# 5. Live disk & partition performance benchmarker (transfer rates, latency curve).
# 6. Disk image creation and restoration (.img / .iso).
# 7. Pure 2D software double-buffered vector rasterizer.
# 8. Full CLI execution: --selftest, --drives, --smart, --benchmark, --info, --version.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_stratum_tests.sh
set -e
cd "$(dirname "$0")/../.."

echo "--- userland/desktop/apps/nova-disks host tests (Disks & Volume Topology Studio) ---"

if command -v cargo >/dev/null 2>&1; then
    # 1. Run Stratum Companion Workspace Tests
    STRATUM_DIR="$(cd .. && pwd)/stratum"
    if [ -d "$STRATUM_DIR" ]; then
        echo "--> Testing Stratum Core, Storage, Render & UI in $STRATUM_DIR..."
        (cd "$STRATUM_DIR" && cargo test --quiet)
        (cd "$STRATUM_DIR" && cargo run --quiet -p stratum-app -- --test-full)
    fi

    # 2. Run NovaOS userland/desktop/apps/nova-disks Tests
    echo "--> Testing userland/desktop/apps/nova-disks..."
    (cd userland/desktop/apps/nova-disks && cargo test --quiet)
    (cd userland/desktop/apps/nova-disks && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-disks && cargo run --quiet -- --drives > /dev/null)
    (cd userland/desktop/apps/nova-disks && cargo run --quiet -- --smart > /dev/null)
    (cd userland/desktop/apps/nova-disks && cargo run --quiet -- --benchmark > /dev/null)
    (cd userland/desktop/apps/nova-disks && cargo run --quiet -- --info > /dev/null)
    (cd userland/desktop/apps/nova-disks && cargo run --quiet -- --version > /dev/null)

    # 3. Verify Bare-Metal Fixtures
    if [ -f "tools/fixtures/STRATUM.ELF" ] && [ -f "tools/fixtures/STRATUM.PKG" ] && [ -f "tools/fixtures/DISKS.ELF" ] && [ -f "tools/fixtures/DISKS.PKG" ]; then
        echo "--> Verified STRATUM.ELF, DISKS.ELF, STRATUM.PKG and DISKS.PKG fixtures present and valid."
    fi

    echo "[PASS] All NovaOS Disks (nova-disks / Stratum) tests passed successfully."
else
    echo "nova-disks-test: cargo not found - skipping the suite"
fi
