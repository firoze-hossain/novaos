#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for Code & Text Editor (nova-edit)
# 1. Piece-table text buffer data structure for O(1) mutations on multi-megabyte files.
#    - Immutable original text and append-only add text buffers.
#    - Multi-level undo/redo transaction history stack.
#    - O(1) byte-to-line/column translation and line slicing.
# 2. Multi-language syntax highlighting:
#    - Rust, C, Python, Markdown, Shell, and JSON.
#    - Tokenizer with Keyword, Type, StringLiteral, NumberLiteral, Comment, Operator, Heading, and Text tokens.
#    - ANSI terminal and 32-bit RGB color palette mapping.
# 3. Multi-cursor editing:
#    - Simultaneous carets with synchronized typing, line jumping, and text selection.
#    - Caret normalization, deduplication, and bounds clamping.
# 4. Code folding engine:
#    - Block scope discovery for curly braces and indentation.
#    - Fold/unfold toggle state, gutter marker indicators ([-] / [+]), and line visibility filtering.
# 5. Bracket matching & auto-closing delimiters:
#    - Bidirectional matching for (), [], and {}.
#    - Auto-pairing closing delimiter injection on typing.
# 6. Find & Replace engine:
#    - Case-sensitive and case-insensitive forward/backward search.
#    - In-place bulk find and replace with piece table modification.
# 7. Project directory sidebar:
#    - Hierarchical tree navigation with folder expand/collapse.
#    - File type extension icon badges and visible node flattening.
# 8. Multi-file tabbed document management:
#    - Tab switching, closing, dirty tracking, and auto-save.
# 9. UI Shell & BufferSurface Rasterization:
#    - Dark acrylic theme with tab bar, toolbar, gutter, syntax canvas, carets, and status bar.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_edit_tests.sh      (or: make nova-edit-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/apps/nova-edit host tests (Code & Text Editor) ---"
    (cd userland/desktop/apps/nova-edit && cargo test --quiet)
    (cd userland/desktop/apps/nova-edit && cargo run --quiet -- --selftest)
    echo "[PASS] All 11 Code & Text Editor (nova-edit) tests passed successfully."
else
    echo "nova-edit-test: cargo not found - skipping the suite"
fi
