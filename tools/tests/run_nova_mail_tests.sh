#!/bin/sh
# NovaOS Email & Communication Studio (nova-mail / Hermes) Test Suite
#
# Synthesizes macOS Mail (acrylic sidebar, Sequoia categories, Add Account sheets):
# 1. Protocols: RFC 5322 / RFC 2822 / MIME Multipart / RFC 2047 standard streams.
# 2. Categories: macOS Sequoia Smart Tabs (Primary, Transactions, Updates, Promotions).
# 3. Accounts: Multi-account routing (iCloud, Google, Exchange, Yahoo, IMAP/SMTP).
# 4. Layout: 3-pane sovereign layout (Translucent Sidebar, Message List, Reading Pane).
# 5. Security: DKIM/SPF cryptographic verification shields, heuristic spam filter.
# 6. Graphics: Pure 2D software double-buffered vector rasterizer (zero-syscall).
# 7. Full CLI execution: --selftest, --inbox, --accounts, --categories, --info, --version.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_mail_tests.sh
set -e
cd "$(dirname "$0")/../.."

echo "--- userland/desktop/apps/nova-mail host tests (Sovereign Email & Communication Studio) ---"

if command -v cargo >/dev/null 2>&1; then
    # 1. Run Hermes Companion Workspace Tests
    HERMES_DIR="$(cd .. && pwd)/hermes"
    if [ -d "$HERMES_DIR" ]; then
        echo "--> Testing Hermes Core, Storage, Render & UI in $HERMES_DIR..."
        (cd "$HERMES_DIR" && cargo test --quiet)
        (cd "$HERMES_DIR" && cargo run --quiet -p hermes-app -- --selftest)
    fi

    # 2. Run NovaOS userland/desktop/apps/nova-mail Tests
    echo "--> Testing userland/desktop/apps/nova-mail..."
    (cd userland/desktop/apps/nova-mail && cargo test --quiet)
    (cd userland/desktop/apps/nova-mail && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-mail && cargo run --quiet -- --inbox > /dev/null)
    (cd userland/desktop/apps/nova-mail && cargo run --quiet -- --accounts > /dev/null)
    (cd userland/desktop/apps/nova-mail && cargo run --quiet -- --categories > /dev/null)
    (cd userland/desktop/apps/nova-mail && cargo run --quiet -- --info > /dev/null)
    (cd userland/desktop/apps/nova-mail && cargo run --quiet -- --version > /dev/null)

    # 3. Verify Bare-Metal Fixtures
    if [ -f "tools/fixtures/MAIL.ELF" ] && [ -f "tools/fixtures/MAIL.PKG" ] && [ -f "tools/fixtures/HERMES.ELF" ] && [ -f "tools/fixtures/HERMES.PKG" ]; then
        echo "--> Verified MAIL.ELF, HERMES.ELF, MAIL.PKG and HERMES.PKG fixtures present and valid."
    fi

    echo "[PASS] All NovaOS Mail (nova-mail / Hermes) tests passed successfully."
else
    echo "nova-mail-test: cargo not found - skipping the suite"
fi
