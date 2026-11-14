//! userland/desktop/nova-shell/src/lib.rs
//!
//! NovaOS Desktop Shell Library.
//! Exposes top menu bar, status indicators, and desktop environment components.

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::vec;

pub mod menubar;

pub use menubar::*;
use nova_ui::prelude::*;

/// Self-test routine for NovaOS Desktop Shell
pub fn run_nova_shell_selftest() -> bool {
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
            vec![menubar::MenuItem::new("New Tab", 1, Some("⌘T"))],
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
    let act = bar.handle_keyboard_shortcut("⌘T");
    if act != Some(1) {
        return false;
    }

    // BufferSurface rendering verification
    let mut surface = BufferSurface::new(1024, 100);
    bar.render(&mut surface);

    true
}
