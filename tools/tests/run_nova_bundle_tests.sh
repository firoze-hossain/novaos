#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for Self-Contained Application Bundle Architecture (.app Bundles)
# 1. Elimination of Dependency Hell:
#    - Self-contained directory hierarchy (AppName.app).
#    - Private shared dependencies in lib/ prioritized ahead of system /lib in LD_LIBRARY_PATH.
# 2. Manifest & Capability Declaration:
#    - Manifest.json containing reverse-DNS bundle_id, version, executable, icon, categories, and author.
#    - Declarative sandboxing: network, audio, camera, gpu, filesystem read/write paths, background services.
# 3. Directory Layout & Component Discovery:
#    - bin/ containing 64-bit ELF binary.
#    - assets/ containing high-res icons and UI assets.
#    - lib/ containing private shared libraries.
# 4. Drag-and-Drop Installation & Atomic Uninstallation:
#    - Installing an app requires simply copying or dropping its .app folder into /Applications.
#    - Uninstalling requires simply deleting the .app folder (reclaiming 100% of bytes, zero orphaned files).
# 5. Application Launcher & Spotlight Integration (nova-shell):
#    - Stock application bundle discovery (Files.app, Terminal.app, Editor.app, Calculator.app, Settings.app, Store.app).
#    - Real-time search query matching and category tab filtering.
#    - Drag-and-drop bundle drop installation handling.
#    - Sandbox execution profile generation and BufferSurface rasterization.
#
# No QEMU needed; executes in under three seconds.
#
#   ./tools/tests/run_nova_bundle_tests.sh      (or: make nova-bundle-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/libs/nova-bundle & userland/desktop/nova-shell host tests (Application Bundle Architecture) ---"
    (cd userland/libs/nova-bundle && cargo test --quiet)
    (cd userland/desktop/nova-shell && cargo test --quiet)
    (cd userland/desktop/nova-shell && cargo run --quiet -- --selftest)
    echo "[PASS] All Self-Contained Application Bundle Architecture (.app) tests passed successfully."
else
    echo "nova-bundle-test: cargo not found - skipping the suite"
fi
