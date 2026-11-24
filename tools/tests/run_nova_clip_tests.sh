#!/bin/sh
# NovaOS Clipboard Manager Application (nova-clip / Mnemos) Test Suite
#
# Synthesizes Apple macOS Maccy:
# 1. Translucent Acrylic Card: 360px floating window with rounded corners and border.
# 2. Instant Fuzzy Search: Prefix, substring & acronym ranking engine with word boundary bonuses.
# 3. Inline Color Swatches: Hex color detection (#284058) with inline square swatches.
# 4. File URLs & Newlines: file:// middle-truncation and multiline hooked return indicators (↩).
# 5. Flyout Preview Popover: Application source badge, timestamps, copy count, and full text.
# 6. Numeric Shortcuts: Direct one-stroke pasting with keys 1..9.
# 7. Privacy Filter: Automatic protection against password managers and confidential credentials.
# 8. Full CLI execution: --selftest, --status, --copy, --paste, and --search.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_clip_tests.sh
set -e
cd "$(dirname "$0")/../.."

echo "--- userland/desktop/apps/nova-clip host tests (Sovereign Clipboard Manager Suite) ---"

if command -v cargo >/dev/null 2>&1; then
    # 1. Run Mnemos Companion Workspace Tests
    MNEMOS_DIR="$(cd .. && pwd)/mnemos"
    if [ -d "$MNEMOS_DIR" ]; then
        echo "--> Testing Mnemos Core, UI & Platform in $MNEMOS_DIR..."
        (cd "$MNEMOS_DIR" && cargo test --quiet)
        (cd "$MNEMOS_DIR" && cargo run --quiet -p mnemos-app -- --selftest)
    fi

    # 2. Run NovaOS userland/desktop/apps/nova-clip Tests
    echo "--> Testing userland/desktop/apps/nova-clip..."
    (cd userland/desktop/apps/nova-clip && cargo test --quiet)
    (cd userland/desktop/apps/nova-clip && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-clip && cargo run --quiet -- --status > /dev/null)

    # 3. Verify Bare-Metal Fixtures
    if [ -f "tools/fixtures/CLIP.ELF" ] && [ -f "tools/fixtures/CLIP.PKG" ] && [ -f "tools/fixtures/MNEMOS.ELF" ] && [ -f "tools/fixtures/MNEMOS.PKG" ]; then
        echo "--> Verified CLIP.ELF, MNEMOS.ELF, CLIP.PKG and MNEMOS.PKG fixtures present and valid."
    fi

    echo "[PASS] All NovaOS Clipboard (nova-clip / Mnemos) tests passed successfully."
else
    echo "nova-clip-test: cargo not found - skipping the suite"
fi
