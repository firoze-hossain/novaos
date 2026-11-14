//! userland/desktop/nova-shell/src/lib.rs
//!
//! NovaOS Desktop Shell Library.
//! Exposes top menu bar, status indicators, centered taskbar, and application dock.

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::vec;

pub mod menubar;
pub mod dock;
pub mod taskbar;

pub use menubar::*;
pub use dock::*;
pub use taskbar::*;

use nova_ui::prelude::*;

/// Self-test routine for NovaOS Desktop Shell
pub fn run_nova_shell_selftest() -> bool {
    // 1. Top Menu Bar Verification
    let mut bar = menubar::TopMenuBar::new(
        1024,
        ThemeMode::Dark,
        AccentPalette::FluentBlue,
    );
    if bar.height != menubar::MENUBAR_HEIGHT {
        return false;
    }

    // Switch focus
    bar.update_active_app(
        10,
        "Terminal",
        vec![menubar::MenuCategory::new(
            "Shell",
            vec![menubar::MenuItem::new("New Tab", 1, Some("\u{2318}T"))],
        )],
    );

    if bar.app_menu.app_name != "Terminal" {
        return false;
    }

    // Hit-testing / scrubbing
    let click = Point::new(bar.system_menu.bounds.x + 2, 10);
    bar.handle_mouse_click(click);
    if bar.active_flyout != menubar::ActiveFlyout::SystemMenu {
        return false;
    }

    // Accelerators
    let act = bar.handle_keyboard_shortcut("\u{2318}T");
    if act != Some(1) {
        return false;
    }

    // Menu Bar BufferSurface rendering
    let mut surface = BufferSurface::new(1024, 100);
    bar.render(&mut surface);

    // 2. Centered Dynamic Taskbar & Dock Verification
    let mut taskbar = taskbar::Taskbar::new(
        1024,
        ThemeMode::Dark,
        AccentPalette::FluentBlue,
    );
    if taskbar.height != taskbar::TASKBAR_HEIGHT {
        return false;
    }
    if taskbar.alignment != taskbar::TaskbarAlignment::Centered {
        return false;
    }

    // Window surface capture & thumbnail sync
    let thumb = dock::WindowThumbnail::create_test_thumbnail(
        42,
        "Code Editor",
        160,
        100,
        Color::rgb(0, 120, 215),
    );
    taskbar.sync_window_surface("edit", thumb);
    taskbar.set_active_window(Some(42));

    if !taskbar.items.iter().any(|it| it.app_id == "edit" && it.is_running() && it.is_focused()) {
        return false;
    }

    // Badge notification test
    taskbar.set_badge_count("term", Some(5));

    // Mouse hover live thumbnail preview activation
    let edit_pos = Point::new(taskbar.items[2].base_bounds.x + 10, 20);
    taskbar.handle_mouse_move(edit_pos);
    if taskbar.hover_preview.is_none() {
        return false;
    }

    // Taskbar BufferSurface rendering
    let mut tb_surface = BufferSurface::new(1024, 48);
    taskbar.render(&mut tb_surface);

    true
}
