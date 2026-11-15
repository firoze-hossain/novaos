#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for Next-Gen File Explorer (nova-files)
# 1. Synthesis of Windows File Explorer (ribbon, breadcrumbs, tree pane, properties)
#    and macOS Finder (Miller columns, QuickLook, tags, tabs, drag-and-drop).
# 2. Four distinct view modes:
#    - Icons View (2D grid, icon badges, tag dots, multi-select).
#    - List View (compact horizontal rows, metadata, size).
#    - Columns View (macOS Miller columns cascading hierarchical browser).
#    - Details View (sortable table headers, zebra striping, selection).
# 3. Path Breadcrumbs & Navigation Bar:
#    - Segmented path crumbs with parent-child navigation.
#    - Back/Forward history traversal and direct path editing.
#    - Instant search filter with live result counter.
# 4. Navigation Sidebar Tree Pane:
#    - Quick Access Favorites (Home, Documents, Downloads, Pictures, Music, System).
#    - Drives & Storage Volumes (NovaFS, FAT32, ext2, Snapshots).
#    - 7 macOS colored tags filter.
# 5. Spacebar QuickLook Floating Acrylic Preview Modal:
#    - Instant preview for Text/Code, Images, Audio waveforms, Packages, and Binaries.
#    - Level 4 elevation drop shadow and translucent backdrop scrim.
# 6. NovaFS Snapshot Integration & File Version History:
#    - Snapshot timeline listing with timestamp, diff summary, and SHA-256 checksums.
#    - Zero-cost instant file rollback to previous snapshot versions.
# 7. File Operations Engine:
#    - Clipboard operations: Copy, Cut, Paste, Duplicate.
#    - In-place Rename, Delete (Trash vs Permanent), and Folder creation.
#    - Drag-and-Drop state machine with destination folder targeting.
#    - Package & Archive extraction.
# 8. Tabbed Browsing:
#    - Multiple independent tabs with isolated history, view mode, and paths.
# 9. Detailed Properties Inspector Pane:
#    - POSIX octal permissions, NovaFS SHA-256 block integrity shield, and tag picker.
# 10. BufferSurface graphical rasterization with zero errors.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_files_tests.sh      (or: make nova-files-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/apps/nova-files host tests (Next-Gen File Explorer) ---"
    (cd userland/desktop/apps/nova-files && cargo test --quiet)
    (cd userland/desktop/apps/nova-files && cargo run --quiet -- --selftest)
    echo "[PASS] All 11 Next-Gen File Explorer (nova-files) tests passed successfully."
else
    echo "nova-files-test: cargo not found - skipping the suite"
fi
