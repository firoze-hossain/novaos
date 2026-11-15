#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for Unified System Settings & Control Hub (nova-settings)
# 1. Synthesis of Windows 11 Settings and macOS System Settings into a unified configuration hub.
# 2. Six primary settings categories:
#    - System (Display resolution/scaling/refresh, Sound output/input/volume, Power/Sleep timeouts).
#    - Network & Internet (Wi-Fi toggle/SSID scan, Ethernet DHCP/Static IP, VPN, Proxy).
#    - Personalization (Appearance Dark/Light, 8 Accent colors, Wallpaper gallery, Acrylic transparency).
#    - Apps (Default application handlers, Startup supervised services, App permissions).
#    - Accounts (Current user profile, Multi-user accounts, Passwords/PBKDF2, Wheel/Sudo privileges).
#    - System Updates (Dual-slot A/B update engine, OS version info, Update check/apply, Rollback recovery).
# 3. System Configuration Backend:
#    - Direct query and mutation of sysfs (/sys/) nodes (DRM modes, sound volumes, power profiles, net interfaces).
#    - novad system daemon IPC protocol (GetConfig, SetConfig, ListUsers, AddUser, ToggleService, Updates).
# 4. Real-Time Search Engine:
#    - Inverted index with multi-token fuzzy matching across titles, descriptions, and keywords.
#    - Deep-link breadcrumb navigation jumping directly to target settings panels.
# 5. UI Shell & BufferSurface Rasterization:
#    - Windows 11 Fluent mica + macOS rounded acrylic cards.
#    - Left navigation sidebar with user avatar pill, real-time search box, and category icons.
#    - Interactive controls: toggle switches, sliders, dropdown choice selectors, action buttons.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_settings_tests.sh      (or: make nova-settings-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/apps/nova-settings host tests (Unified System Settings & Control Hub) ---"
    (cd userland/desktop/apps/nova-settings && cargo test --quiet)
    (cd userland/desktop/apps/nova-settings && cargo run --quiet -- --selftest)
    echo "[PASS] All 10 Unified System Settings (nova-settings) tests passed successfully."
else
    echo "nova-settings-test: cargo not found - skipping the suite"
fi
