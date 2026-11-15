#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for Pro Developer Terminal (nova-term)
# 1. High-Performance GPU-Accelerated Terminal Emulator in pure Rust.
# 2. 24-bit TrueColor support (\x1b[38;2;r;g;bm and \x1b[48;2;r;g;bm) & xterm 256-color palette.
# 3. ANSI / VT100 / xterm escape sequence state machine (CSI, OSC, SGR, DEC private modes).
# 4. Monospaced character cell grid with primary & alternate screen buffers (for htop, vim, cargo).
# 5. Scrollback history buffer (up to 5,000 lines) with DECSTBM scrolling margins.
# 6. Multi-tab session management with tab bar metrics, close buttons, and shortcuts (Ctrl+Shift+T/W).
# 7. Split horizontal & vertical panes (Ctrl+Shift+D/E) with recursive binary tree and focus cycling.
# 8. 6 Developer Themes: Dracula, Nord, Solarized Dark, Monokai, One Dark, and Nova Dark.
# 9. GPU Glyph texture cache & vector blitter with Block, Underline, and Beam cursor styles.
# 10. Smart URL detection with click-to-open and hover underlines.
# 11. Clipboard integration (Ctrl+Shift+C, Ctrl+Shift+V, Linear & Block text selection).
# 12. PTY master session connection with SIGWINCH window resize signal dispatch.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_term_tests.sh      (or: make nova-term-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/apps/nova-term host tests (Pro Developer Terminal) ---"
    (cd userland/desktop/apps/nova-term && cargo test --quiet)
    (cd userland/desktop/apps/nova-term && cargo run --quiet -- --selftest)
    echo "[PASS] All 11 Pro Developer Terminal (nova-term) tests passed successfully."
else
    echo "nova-term-test: cargo not found - skipping the suite"
fi
