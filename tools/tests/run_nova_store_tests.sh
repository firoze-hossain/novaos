#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for Graphical Software Store & Package Manager (nova-store)
# 1. Synthesis of Ubuntu Software Center and macOS App Store into a unified software ecosystem.
# 2. Seven store categories:
#    - Discover & Featured (Hero spotlight banner, curated packages).
#    - Developer Tools (Code editors, Git visualizers, native libraries).
#    - Productivity & Office (Calculators, Markdown notebooks).
#    - System Utilities (Activity monitor, Archive managers).
#    - Internet & Networking (Modern web browsers, encrypted chat clients).
#    - Installed Applications (Package inventory with version and size).
#    - Available Updates (Pending updates notification badge).
# 3. Cryptographic Signature Verification:
#    - Pure #![no_std] Ed25519 digital signature and SHA-256 payload integrity check.
#    - Rejection of tampered binaries, forged headers, and altered payloads.
# 4. Sandbox Permission Declaration:
#    - Granular capability declarations: Network, Filesystem, Audio, Video, GPU, Background daemons.
#    - Security boundary enforcement and isolated sandbox status validation.
# 5. Dependency Tree Resolution & Safe Removal:
#    - Recursive dependency discovery and topological install sequence calculation.
#    - Reverse dependency checking preventing removal of libraries depended on by active apps.
# 6. One-Click Package Lifecycle:
#    - GET / INSTALL, UPDATE, OPEN, and REMOVE actions.
# 7. UI Shell & BufferSurface Rasterization:
#    - Windows 11 Fluent mica background + macOS rounded card aesthetic.
#    - Left navigation sidebar with category badges and search box.
#    - 2-column app cards grid, hero spotlight banner, and app details inspector view.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_store_tests.sh      (or: make nova-store-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/apps/nova-store host tests (Graphical Software Store & Package Manager) ---"
    (cd userland/desktop/apps/nova-store && cargo test --quiet)
    (cd userland/desktop/apps/nova-store && cargo run --quiet -- --selftest)
    echo "[PASS] All 11 Graphical Software Store (nova-store) tests passed successfully."
else
    echo "nova-store-test: cargo not found - skipping the suite"
fi
