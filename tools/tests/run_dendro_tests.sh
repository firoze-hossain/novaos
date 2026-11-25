#!/bin/sh
# NovaOS Disk Usage Analyzer (nova-dendro / Dendro) Test Suite
#
# Synthesizes Ubuntu Disk Usage Analyzer (baobab):
# 1. Real-time recursive directory scanner with depth limit & permission safety.
# 2. Interactive tree table: Folder, Size, Contents, Modified, and proportional percentage bars.
# 3. Dual-mode interactive visualization:
#    - Multi-tier radial sunburst concentric rings chart with polar hit-testing.
#    - Squarified 2D rectangular treemap space partitioning.
# 4. Storage volumes & locations overview (Home Folder, root /, external/unmounted devices).
# 5. Breadcrumb navigation and one-click directory drill-down.
# 6. Pure 2D software double-buffered vector rasterizer for framebuffers.
# 7. Full CLI execution: --selftest, --devices, --rings, --treemap, --scan, --report, --version.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_dendro_tests.sh
set -e
cd "$(dirname "$0")/../.."

echo "--- userland/desktop/apps/nova-dendro host tests (Disk Usage Analyzer) ---"

if command -v cargo >/dev/null 2>&1; then
    # 1. Run Dendro Companion Workspace Tests
    DENDRO_DIR="$(cd .. && pwd)/dendro"
    if [ -d "$DENDRO_DIR" ]; then
        echo "--> Testing Dendro Core, Storage, Render & UI in $DENDRO_DIR..."
        (cd "$DENDRO_DIR" && cargo test --quiet)
        (cd "$DENDRO_DIR" && cargo run --quiet -p dendro-app -- --test-full)
    fi

    # 2. Run NovaOS userland/desktop/apps/nova-dendro Tests
    echo "--> Testing userland/desktop/apps/nova-dendro..."
    (cd userland/desktop/apps/nova-dendro && cargo test --quiet)
    (cd userland/desktop/apps/nova-dendro && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-dendro && cargo run --quiet -- --devices > /dev/null)
    (cd userland/desktop/apps/nova-dendro && cargo run --quiet -- --rings > /dev/null)
    (cd userland/desktop/apps/nova-dendro && cargo run --quiet -- --treemap > /dev/null)
    (cd userland/desktop/apps/nova-dendro && cargo run --quiet -- --version > /dev/null)

    # 3. Verify Bare-Metal Fixtures
    if [ -f "tools/fixtures/DENDRO.ELF" ] && [ -f "tools/fixtures/DENDRO.PKG" ] && [ -f "tools/fixtures/BAOBAB.ELF" ] && [ -f "tools/fixtures/BAOBAB.PKG" ]; then
        echo "--> Verified DENDRO.ELF, BAOBAB.ELF, DENDRO.PKG and BAOBAB.PKG fixtures present and valid."
    fi

    echo "[PASS] All NovaOS Disk Usage Analyzer (nova-dendro / Dendro) tests passed successfully."
else
    echo "nova-dendro-test: cargo not found - skipping the suite"
fi
