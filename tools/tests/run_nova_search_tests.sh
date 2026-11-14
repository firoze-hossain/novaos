#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for macOS Spotlight-Style Universal Instant Search (nova-search)
# 1. Spotlight Overlay lifecycle: show, hide, toggle, and centered floating card geometry.
# 2. Multi-category indexing: Applications, Documents, Settings Panels, and Shell Commands.
# 3. Sub-millisecond fuzzy subsequence matching with prefix and word-boundary bonuses (<10ms).
# 4. Real-time inline math calculation: arithmetic, functions (sqrt, abs), and parentheses.
# 5. Natural unit conversions: digital storage (KB/MB/GB), temperature (°F/°C), and length (km/miles/in/cm).
# 6. Keyboard navigation: Arrow Up/Down selection cycle and Enter action dispatch.
# 7. BufferSurface acrylic rendering: Level 4 elevation shadows, translucent backdrop scrim, and subpixel text.
# 8. In-daemon full self-test suite (run_nova_search_selftest).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_search_tests.sh      (or: make nova-search-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/nova-search host tests (Spotlight Instant Search & Calculator) ---"
    (cd userland/desktop/nova-search && cargo test --quiet)
    (cd userland/desktop/nova-search && cargo run --quiet -- --selftest)
    echo "[PASS] All 6 macOS Spotlight Instant Search (nova-search) tests passed successfully."
else
    echo "nova-search-test: cargo not found - skipping the suite"
fi
