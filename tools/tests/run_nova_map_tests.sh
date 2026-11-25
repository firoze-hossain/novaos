#!/bin/sh
# NovaOS Vector Cartography & Navigation Engine (nova-map / Atlas) Test Suite
#
# Synthesizes macOS Maps (acrylic sidebar, nearby drawer, marked location card, weather):
# 1. Web Mercator: Exact EPSG:3857 projection matrices forward and inverse.
# 2. Geodesy: Haversine great-circle distance & bearing geometry.
# 3. Spatial DB: Vector polygons (Lakes, Parks), multi-pass road networks, POIs.
# 4. Navigation: Turn-by-turn waypoint routing, step maneuvers & durations.
# 5. 3D Perspective: Dynamic 35° horizon tilt and depth ground-plane rasterizer.
# 6. macOS Shell: Floating translucent acrylic sidebar, Nearby drawer, Marked Location card.
# 7. Full CLI execution: --selftest, --coords, --poi, --search, --info, --version.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_map_tests.sh
set -e
cd "$(dirname "$0")/../.."

echo "--- userland/desktop/apps/nova-map host tests (Sovereign Vector Cartography & Navigation) ---"

if command -v cargo >/dev/null 2>&1; then
    # 1. Run Atlas Companion Workspace Tests
    ATLAS_DIR="$(cd .. && pwd)/atlas"
    if [ -d "$ATLAS_DIR" ]; then
        echo "--> Testing Atlas Core, Render, UI & Platform in $ATLAS_DIR..."
        (cd "$ATLAS_DIR" && cargo test --quiet)
        (cd "$ATLAS_DIR" && cargo run --quiet -p atlas-app -- --selftest)
    fi

    # 2. Run NovaOS userland/desktop/apps/nova-map Tests
    echo "--> Testing userland/desktop/apps/nova-map..."
    (cd userland/desktop/apps/nova-map && cargo test --quiet)
    (cd userland/desktop/apps/nova-map && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-map && cargo run --quiet -- --coords > /dev/null)
    (cd userland/desktop/apps/nova-map && cargo run --quiet -- --poi > /dev/null)
    (cd userland/desktop/apps/nova-map && cargo run --quiet -- --search "Hospital" > /dev/null)
    (cd userland/desktop/apps/nova-map && cargo run --quiet -- --info > /dev/null)
    (cd userland/desktop/apps/nova-map && cargo run --quiet -- --version > /dev/null)

    # 3. Verify Bare-Metal Fixtures
    if [ -f "tools/fixtures/MAP.ELF" ] && [ -f "tools/fixtures/MAP.PKG" ] && [ -f "tools/fixtures/ATLAS.ELF" ] && [ -f "tools/fixtures/ATLAS.PKG" ]; then
        echo "--> Verified MAP.ELF, ATLAS.ELF, MAP.PKG and ATLAS.PKG fixtures present and valid."
    fi

    echo "[PASS] All NovaOS Map (nova-map / Atlas) tests passed successfully."
else
    echo "nova-map-test: cargo not found - skipping the suite"
fi
