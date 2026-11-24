//! userland/desktop/apps/nova-clip/src/lib.rs
//!
//! # NovaOS Clipboard Application Library (nova-clip)
//!
//! Sovereign, pure-Rust default Clipboard Manager for NovaOS, synthesizing
//! macOS Maccy (acrylic glass card, fuzzy search header, inline color swatches,
//! multiline return indicators, shortcuts 1..9, flyout preview popovers, and privacy filtering).

pub mod model;
pub mod ui;

use model::{ClipHistory, ClipItem, FuzzyMatcher, ItemKind, MnemosHub, NovaClipboard, ClipboardBackend};
use ui::{MnemosRenderer, OwnedBufferSurface, Surface};

/// Executes the comprehensive autonomous qualification suite for NovaOS Clipboard.
pub fn run_nova_clip_selftest() -> bool {
    // 1. Models & inline metadata detection
    let color_item = ClipItem::new(1, "#284058", "Figma", 1000);
    if color_item.detected_color != Some((40, 64, 88)) || color_item.kind != ItemKind::ColorCode {
        eprintln!("[FAIL] Hex color #284058 should be parsed as RGB (40, 64, 88)");
        return false;
    }

    let file_item = ClipItem::new(2, "file:///Users/dev/Screenshot.png", "Finder", 1001);
    if file_item.kind != ItemKind::FileUrls || file_item.source_app != "Finder" {
        eprintln!("[FAIL] File URL should be parsed as ItemKind::FileUrls");
        return false;
    }

    let multi_item = ClipItem::new(3, "line1\nline2", "Terminal", 1002);
    if !multi_item.has_newlines {
        eprintln!("[FAIL] Multiline item should set has_newlines = true");
        return false;
    }

    // 2. Deduplication, Timestamps & FIFO eviction
    let mut hist = ClipHistory::default();
    hist.push_text("cargo test", "Terminal", 100);
    hist.push_text("cargo test", "Terminal", 200);
    if hist.items.len() != 1 || hist.items[0].copy_count != 2 || hist.items[0].last_copied_at != 200 {
        eprintln!("[FAIL] Deduplication failed to merge identical clips");
        return false;
    }

    // Password manager filter
    hist.push_text("super_secret", "1Password", 300);
    if hist.items.len() != 1 {
        eprintln!("[FAIL] Password manager item should be ignored");
        return false;
    }

    // Pinning & clear unpinned
    hist.push_text("unpinned", "Notes", 400);
    hist.toggle_pin(1); // Pin the first item
    hist.clear_unpinned();
    if !hist.items.iter().all(|i| i.pinned) {
        eprintln!("[FAIL] Clear unpinned should preserve pinned items");
        return false;
    }

    // 3. Search Engine
    let score_exact = FuzzyMatcher::score("novaos", "novaos").unwrap_or(0);
    let score_prefix = FuzzyMatcher::score("nova", "novaos operating system").unwrap_or(0);
    let score_fuzzy = FuzzyMatcher::score("nvs", "novaos").unwrap_or(0);
    if score_exact < 5000 || score_prefix < 2000 || score_fuzzy == 0 {
        eprintln!("[FAIL] Fuzzy matching scoring hierarchy violated");
        return false;
    }

    // 4. Hub navigation & shortcuts
    let mut hub = MnemosHub::with_maccy_demo();
    if hub.history.items.is_empty() {
        eprintln!("[FAIL] Demo history should not be empty");
        return false;
    }
    hub.move_down();
    if hub.selected_idx != 1 {
        eprintln!("[FAIL] Down arrow should select index 1");
        return false;
    }
    let shortcut_item = hub.select_shortcut(3);
    if shortcut_item.is_none() || hub.selected_idx != 2 {
        eprintln!("[FAIL] Shortcut 3 should select index 2");
        return false;
    }

    // 5. UI Rasterization & popover
    let mut surface = OwnedBufferSurface::new(800, 600);
    MnemosRenderer::render(&mut surface, &hub, 50, 50);
    if surface.get_pixel(100, 80).to_u32() == 0 {
        eprintln!("[FAIL] Acrylic card surface should contain rendered pixels");
        return false;
    }

    // 6. NovaOS Ring-3 IPC
    let mut nova_clip = NovaClipboard::new();
    if !nova_clip.write_text("Ring-3 System Call") || nova_clip.poll_changes() != Some("Ring-3 System Call".to_string()) {
        eprintln!("[FAIL] NovaOS Ring-3 clipboard IPC failed");
        return false;
    }

    true
}
