//! userland/desktop/nova-shell/src/menubar.rs
//!
//! macOS-Style Persistent Unified Top Menu Bar for NovaOS.
//! Features:
//! - Persistent 28px acrylic blurred top bar
//! - Dynamic application menu tree updating on window focus changes
//! - NovaOS System Menu ([NovaOS] logo)
//! - Menu dropdown flyouts with macOS-style scrubbing and keyboard shortcuts
//! - Right-aligned status indicators:
//!   - Clock / Calendar flyout
//!   - Network indicator (Wi-Fi/Ethernet status & IP switcher)
//!   - Audio volume slider & mute toggle
//!   - Battery level percentage & charging status
//!   - User account pill (avatar, lock, sign-out)

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{string::String, vec, vec::Vec};

#[cfg(test)]
use std::{string::String, vec, vec::Vec};

use nova_ui::prelude::*;

pub const MENUBAR_HEIGHT: i32 = 28;

// ============================================================================
// Menu Item & Tree Definitions
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: Option<String>,
    pub action_id: u32,
    pub enabled: bool,
    pub separator_after: bool,
}

impl MenuItem {
    pub fn new(label: &str, action_id: u32, shortcut: Option<&str>) -> Self {
        Self {
            label: String::from(label),
            shortcut: shortcut.map(String::from),
            action_id,
            enabled: true,
            separator_after: false,
        }
    }

    pub fn with_separator(mut self) -> Self {
        self.separator_after = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuCategory {
    pub title: String,
    pub items: Vec<MenuItem>,
    pub bounds: Rect,
}

impl MenuCategory {
    pub fn new(title: &str, items: Vec<MenuItem>) -> Self {
        Self {
            title: String::from(title),
            items,
            bounds: Rect::zero(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppMenuTree {
    pub app_name: String,
    pub window_id: u32,
    pub categories: Vec<MenuCategory>,
}

impl AppMenuTree {
    pub fn default_desktop() -> Self {
        Self {
            app_name: String::from("Finder"),
            window_id: 0,
            categories: vec![
                MenuCategory::new("File", vec![
                    MenuItem::new("New Window", 101, Some("⌘N")),
                    MenuItem::new("New Folder", 102, Some("⇧⌘N")).with_separator(),
                    MenuItem::new("Close Window", 103, Some("⌘W")),
                ]),
                MenuCategory::new("Edit", vec![
                    MenuItem::new("Undo", 201, Some("⌘Z")),
                    MenuItem::new("Redo", 202, Some("⇧⌘Z")).with_separator(),
                    MenuItem::new("Cut", 203, Some("⌘X")),
                    MenuItem::new("Copy", 204, Some("⌘C")),
                    MenuItem::new("Paste", 205, Some("⌘V")),
                ]),
                MenuCategory::new("View", vec![
                    MenuItem::new("as Icons", 301, Some("⌘1")),
                    MenuItem::new("as List", 302, Some("⌘2")),
                    MenuItem::new("as Columns", 303, Some("⌘3")).with_separator(),
                    MenuItem::new("Toggle Full Screen", 304, Some("⌃⌘F")),
                ]),
                MenuCategory::new("Window", vec![
                    MenuItem::new("Minimize", 401, Some("⌘M")),
                    MenuItem::new("Zoom", 402, None).with_separator(),
                    MenuItem::new("Tile Window to Left", 403, None),
                    MenuItem::new("Tile Window to Right", 404, None),
                ]),
                MenuCategory::new("Help", vec![
                    MenuItem::new("NovaOS Help", 501, Some("⌘?")),
                    MenuItem::new("Documentation", 502, None),
                ]),
            ],
        }
    }
}

// ============================================================================
// Status Indicators
// ============================================================================

#[derive(Clone, Debug)]
pub struct ClockIndicator {
    pub time_str: String,
    pub date_str: String,
    pub bounds: Rect,
}

#[derive(Clone, Debug)]
pub struct NetworkIndicator {
    pub connected: bool,
    pub is_wifi: bool,
    pub signal_bars: u8,
    pub ssid: String,
    pub ip_address: String,
    pub bounds: Rect,
}

#[derive(Clone, Debug)]
pub struct AudioIndicator {
    pub volume: u8, // 0..100
    pub muted: bool,
    pub bounds: Rect,
}

#[derive(Clone, Debug)]
pub struct BatteryIndicator {
    pub percentage: u8, // 0..100
    pub is_charging: bool,
    pub time_remaining: String,
    pub bounds: Rect,
}

#[derive(Clone, Debug)]
pub struct UserAccountIndicator {
    pub username: String,
    pub display_name: String,
    pub initials: String,
    pub bounds: Rect,
}

// ============================================================================
// Active Flyout State Machine
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveFlyout {
    None,
    SystemMenu,
    AppMenu(usize),
    Clock,
    Network,
    Audio,
    Battery,
    User,
}

// ============================================================================
// macOS-Style Top Menu Bar
// ============================================================================

pub struct TopMenuBar {
    pub width: i32,
    pub height: i32,
    pub system_menu: MenuCategory,
    pub app_menu: AppMenuTree,
    pub clock: ClockIndicator,
    pub network: NetworkIndicator,
    pub audio: AudioIndicator,
    pub battery: BatteryIndicator,
    pub user: UserAccountIndicator,
    pub active_flyout: ActiveFlyout,
    pub theme_mode: ThemeMode,
    pub tokens: ColorTokens,
    pub elevation: [ElevationTier; 6],
    pub corners: CornerTokens,
}

impl TopMenuBar {
    pub fn new(width: i32, mode: ThemeMode, accent: AccentPalette) -> Self {
        let system_menu = MenuCategory::new("NovaOS", vec![
            MenuItem::new("About This Mac / NovaOS", 1, None).with_separator(),
            MenuItem::new("System Settings...", 2, Some("⌘,")),
            MenuItem::new("App Store...", 3, None).with_separator(),
            MenuItem::new("Sleep", 4, None),
            MenuItem::new("Restart...", 5, None),
            MenuItem::new("Shut Down...", 6, None).with_separator(),
            MenuItem::new("Lock Screen", 7, Some("⌃⌘Q")),
            MenuItem::new("Log Out admin...", 8, Some("⇧⌘Q")),
        ]);

        let mut bar = Self {
            width,
            height: MENUBAR_HEIGHT,
            system_menu,
            app_menu: AppMenuTree::default_desktop(),
            clock: ClockIndicator {
                time_str: String::from("12:34 PM"),
                date_str: String::from("Wed Oct 24"),
                bounds: Rect::zero(),
            },
            network: NetworkIndicator {
                connected: true,
                is_wifi: true,
                signal_bars: 4,
                ssid: String::from("NovaNet-5G"),
                ip_address: String::from("192.168.1.105"),
                bounds: Rect::zero(),
            },
            audio: AudioIndicator {
                volume: 85,
                muted: false,
                bounds: Rect::zero(),
            },
            battery: BatteryIndicator {
                percentage: 94,
                is_charging: true,
                time_remaining: String::from("4:12 remaining"),
                bounds: Rect::zero(),
            },
            user: UserAccountIndicator {
                username: String::from("admin"),
                display_name: String::from("Firoze Hossain"),
                initials: String::from("FH"),
                bounds: Rect::zero(),
            },
            active_flyout: ActiveFlyout::None,
            theme_mode: mode,
            tokens: ColorTokens::build(mode, accent),
            elevation: get_elevation_tiers(mode),
            corners: CornerTokens::default(),
        };

        bar.layout_items();
        bar
    }

    /// Computes exact hit-testing bounds for all left and right elements.
    pub fn layout_items(&mut self) {
        // 1. Left side: NovaOS Apple logo + App Name + Categories
        let mut cur_x = 12;

        // [NovaOS] System Logo item
        let logo_w = 64;
        self.system_menu.bounds = Rect::new(cur_x, 0, logo_w, self.height);
        cur_x += logo_w + 4;

        // App Name (Bold Header)
        let app_name_w = (self.app_menu.app_name.len() as i32 * 8) + 16;
        cur_x += app_name_w + 4;

        // Categories (File, Edit, View, Window, Help)
        for cat in self.app_menu.categories.iter_mut() {
            let cat_w = (cat.title.len() as i32 * 8) + 18;
            cat.bounds = Rect::new(cur_x, 0, cat_w, self.height);
            cur_x += cat_w;
        }

        // 2. Right side: User Pill <- Battery <- Audio <- Network <- Clock
        let mut right_x = self.width - 12;

        // User Pill: 36px wide avatar/capsule
        let user_w = 32;
        right_x -= user_w;
        self.user.bounds = Rect::new(right_x, (self.height - 20) / 2, user_w, 20);
        right_x -= 12;

        // Clock: "Wed Oct 24 12:34 PM" ~ 80px
        let clock_w = (self.clock.time_str.len() as i32 * 8) + 8;
        right_x -= clock_w;
        self.clock.bounds = Rect::new(right_x, 0, clock_w, self.height);
        right_x -= 12;

        // Battery: "94%" + icon ~ 54px
        let battery_w = 54;
        right_x -= battery_w;
        self.battery.bounds = Rect::new(right_x, 0, battery_w, self.height);
        right_x -= 12;

        // Audio: speaker icon + "85%" ~ 48px
        let audio_w = 48;
        right_x -= audio_w;
        self.audio.bounds = Rect::new(right_x, 0, audio_w, self.height);
        right_x -= 12;

        // Network: Wi-Fi signal icon ~ 24px
        let net_w = 28;
        right_x -= net_w;
        self.network.bounds = Rect::new(right_x, 0, net_w, self.height);
    }

    /// Handles dynamic window focus event from nova-compositor over IPC.
    pub fn update_active_app(
        &mut self,
        window_id: u32,
        app_name: &str,
        categories: Vec<MenuCategory>,
    ) {
        self.app_menu.window_id = window_id;
        self.app_menu.app_name = String::from(app_name);
        if !categories.is_empty() {
            self.app_menu.categories = categories;
        } else {
            self.app_menu.categories = AppMenuTree::default_desktop().categories;
        }
        self.active_flyout = ActiveFlyout::None;
        self.layout_items();
    }

    /// Handles mouse movements, supporting macOS top bar scrubbing between open menus.
    pub fn handle_mouse_move(&mut self, pos: Point) -> bool {
        if self.active_flyout == ActiveFlyout::None {
            return false;
        }

        if pos.y >= 0 && pos.y <= self.height {
            // Check system menu
            if self.system_menu.bounds.contains(pos) {
                if self.active_flyout != ActiveFlyout::SystemMenu {
                    self.active_flyout = ActiveFlyout::SystemMenu;
                    return true;
                }
            }
            // Check app categories
            for (i, cat) in self.app_menu.categories.iter().enumerate() {
                if cat.bounds.contains(pos) {
                    if self.active_flyout != ActiveFlyout::AppMenu(i) {
                        self.active_flyout = ActiveFlyout::AppMenu(i);
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Handles mouse click on top bar, opening/closing flyouts or invoking actions.
    pub fn handle_mouse_click(&mut self, pos: Point) -> Option<u32> {
        if pos.y >= 0 && pos.y <= self.height {
            // 1. Left items: System Menu & Categories
            if self.system_menu.bounds.contains(pos) {
                self.active_flyout = if self.active_flyout == ActiveFlyout::SystemMenu {
                    ActiveFlyout::None
                } else {
                    ActiveFlyout::SystemMenu
                };
                return None;
            }

            for (i, cat) in self.app_menu.categories.iter().enumerate() {
                if cat.bounds.contains(pos) {
                    self.active_flyout = if self.active_flyout == ActiveFlyout::AppMenu(i) {
                        ActiveFlyout::None
                    } else {
                        ActiveFlyout::AppMenu(i)
                    };
                    return None;
                }
            }

            // 2. Right items: Status flyouts
            if self.clock.bounds.contains(pos) {
                self.active_flyout = if self.active_flyout == ActiveFlyout::Clock {
                    ActiveFlyout::None
                } else {
                    ActiveFlyout::Clock
                };
                return None;
            }
            if self.network.bounds.contains(pos) {
                self.active_flyout = if self.active_flyout == ActiveFlyout::Network {
                    ActiveFlyout::None
                } else {
                    ActiveFlyout::Network
                };
                return None;
            }
            if self.audio.bounds.contains(pos) {
                self.active_flyout = if self.active_flyout == ActiveFlyout::Audio {
                    ActiveFlyout::None
                } else {
                    ActiveFlyout::Audio
                };
                return None;
            }
            if self.battery.bounds.contains(pos) {
                self.active_flyout = if self.active_flyout == ActiveFlyout::Battery {
                    ActiveFlyout::None
                } else {
                    ActiveFlyout::Battery
                };
                return None;
            }
            if self.user.bounds.contains(pos) {
                self.active_flyout = if self.active_flyout == ActiveFlyout::User {
                    ActiveFlyout::None
                } else {
                    ActiveFlyout::User
                };
                return None;
            }
        } else if self.active_flyout != ActiveFlyout::None {
            // Click outside bar: check if inside open dropdown card
            let flyout_bounds = self.current_flyout_bounds();
            if let Some(fb) = flyout_bounds {
                if fb.contains(pos) {
                    // Click inside flyout card: find item clicked
                    let rel_y = pos.y - fb.y;
                    let item_idx = (rel_y / 24) as usize;

                    let action = match self.active_flyout {
                        ActiveFlyout::SystemMenu => {
                            self.system_menu.items.get(item_idx).map(|it| it.action_id)
                        }
                        ActiveFlyout::AppMenu(cat_idx) => {
                            self.app_menu.categories.get(cat_idx)
                                .and_then(|cat| cat.items.get(item_idx))
                                .map(|it| it.action_id)
                        }
                        _ => None,
                    };
                    self.active_flyout = ActiveFlyout::None;
                    return action;
                }
            }
            // Dismiss on outside click
            self.active_flyout = ActiveFlyout::None;
        }

        None
    }

    /// Evaluates keyboard shortcuts (e.g. ⌘N, ⌘S, ⌘Q) across active menus.
    pub fn handle_keyboard_shortcut(&self, shortcut: &str) -> Option<u32> {
        // Search active application menu
        for cat in &self.app_menu.categories {
            for item in &cat.items {
                if let Some(sc) = &item.shortcut {
                    if sc == shortcut && item.enabled {
                        return Some(item.action_id);
                    }
                }
            }
        }
        // Search system menu
        for item in &self.system_menu.items {
            if let Some(sc) = &item.shortcut {
                if sc == shortcut && item.enabled {
                    return Some(item.action_id);
                }
            }
        }
        None
    }

    /// Computes the bounding rectangle of the currently opened dropdown card.
    pub fn current_flyout_bounds(&self) -> Option<Rect> {
        match self.active_flyout {
            ActiveFlyout::SystemMenu => {
                let h = (self.system_menu.items.len() as i32 * 24) + 12;
                Some(Rect::new(self.system_menu.bounds.x, self.height, 220, h))
            }
            ActiveFlyout::AppMenu(idx) => {
                let cat = self.app_menu.categories.get(idx)?;
                let h = (cat.items.len() as i32 * 24) + 12;
                Some(Rect::new(cat.bounds.x, self.height, 200, h))
            }
            ActiveFlyout::Clock => {
                Some(Rect::new(self.clock.bounds.x - 120, self.height, 200, 160))
            }
            ActiveFlyout::Network => {
                Some(Rect::new(self.network.bounds.x - 140, self.height, 220, 140))
            }
            ActiveFlyout::Audio => {
                Some(Rect::new(self.audio.bounds.x - 100, self.height, 180, 70))
            }
            ActiveFlyout::Battery => {
                Some(Rect::new(self.battery.bounds.x - 100, self.height, 180, 80))
            }
            ActiveFlyout::User => {
                Some(Rect::new(self.user.bounds.x - 120, self.height, 160, 100))
            }
            ActiveFlyout::None => None,
        }
    }

    /// Renders the complete persistent top menu bar into the target BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface) {
        let bar_rect = Rect::new(0, 0, self.width, self.height);

        // 1. Acrylic blurred bar background
        surface.apply_gaussian_blur(bar_rect, self.tokens.acrylic_blur_radius / 2);
        surface.fill_rect(bar_rect, self.tokens.acrylic_tint);

        // 2. Delicate 1px bottom border
        let border_rect = Rect::new(0, self.height - 1, self.width, 1);
        surface.fill_rect(border_rect, self.tokens.border_subtle);

        // 3. Render Left Items
        // NovaOS Logo / System Menu
        let is_sys_active = self.active_flyout == ActiveFlyout::SystemMenu;
        if is_sys_active {
            surface.fill_rounded_rect(self.system_menu.bounds, self.corners.small, self.tokens.accent_subtle);
        }
        surface.draw_text("[NovaOS]", self.system_menu.bounds.x + 6, 6, self.tokens.text_primary);

        // Bold App Name
        let app_x = self.system_menu.bounds.x + self.system_menu.bounds.width + 6;
        surface.draw_text(&self.app_menu.app_name, app_x, 6, self.tokens.text_primary);

        // Categories
        for (i, cat) in self.app_menu.categories.iter().enumerate() {
            let is_cat_active = self.active_flyout == ActiveFlyout::AppMenu(i);
            if is_cat_active {
                surface.fill_rounded_rect(cat.bounds, self.corners.small, self.tokens.accent_subtle);
            }
            surface.draw_text(&cat.title, cat.bounds.x + 8, 6, if is_cat_active { WHITE } else { self.tokens.text_secondary });
        }

        // 4. Render Right Items
        // Clock
        surface.draw_text(&self.clock.time_str, self.clock.bounds.x + 4, 6, self.tokens.text_primary);

        // Battery: percentage + status
        let bat_text = if self.battery.is_charging {
            String::from("⚡94%")
        } else {
            String::from("94%")
        };
        surface.draw_text(&bat_text, self.battery.bounds.x + 4, 6, self.tokens.text_secondary);

        // Audio: speaker indicator
        let vol_text = if self.audio.muted { String::from("🔇") } else { String::from("🔊85%") };
        surface.draw_text(&vol_text, self.audio.bounds.x + 2, 6, self.tokens.text_secondary);

        // Network: Wi-Fi status
        surface.draw_text("📶", self.network.bounds.x + 4, 6, self.tokens.text_primary);

        // User Avatar Pill
        surface.fill_rounded_rect(self.user.bounds, 10, self.tokens.accent_primary);
        surface.draw_text(&self.user.initials, self.user.bounds.x + 8, self.user.bounds.y + 2, WHITE);

        // 5. Render Active Dropdown Flyout Card (Level 3 Elevation)
        if let Some(card_rect) = self.current_flyout_bounds() {
            render_elevated_card(
                surface,
                card_rect,
                self.corners.card,
                3, // Elevation Level 3 Popover
                &self.elevation,
                &self.tokens,
            );

            match self.active_flyout {
                ActiveFlyout::SystemMenu => {
                    self.render_menu_items(surface, card_rect, &self.system_menu.items);
                }
                ActiveFlyout::AppMenu(idx) => {
                    if let Some(cat) = self.app_menu.categories.get(idx) {
                        self.render_menu_items(surface, card_rect, &cat.items);
                    }
                }
                ActiveFlyout::Audio => {
                    // Quick volume control flyout
                    surface.draw_text("Sound Volume", card_rect.x + 12, card_rect.y + 10, self.tokens.text_primary);
                    // Mini volume track
                    let track = Rect::new(card_rect.x + 12, card_rect.y + 36, card_rect.width - 24, 6);
                    surface.fill_rounded_rect(track, 3, Color::rgb(70, 70, 70));
                    let active = Rect::new(track.x, track.y, (track.width * 85) / 100, 6);
                    surface.fill_rounded_rect(active, 3, self.tokens.accent_primary);
                }
                ActiveFlyout::Network => {
                    surface.draw_text("Wi-Fi: Connected", card_rect.x + 12, card_rect.y + 10, self.tokens.text_primary);
                    surface.draw_text(&self.network.ssid, card_rect.x + 12, card_rect.y + 34, self.tokens.accent_primary);
                    surface.draw_text(&self.network.ip_address, card_rect.x + 12, card_rect.y + 58, self.tokens.text_muted);
                }
                ActiveFlyout::Clock => {
                    surface.draw_text(&self.clock.date_str, card_rect.x + 12, card_rect.y + 10, self.tokens.text_primary);
                    surface.draw_text(&self.clock.time_str, card_rect.x + 12, card_rect.y + 34, self.tokens.accent_primary);
                }
                _ => {}
            }
        }
    }

    fn render_menu_items(&self, surface: &mut BufferSurface, card: Rect, items: &[MenuItem]) {
        let mut cur_y = card.y + 6;
        for item in items {
            let item_color = if item.enabled { self.tokens.text_primary } else { self.tokens.text_muted };
            surface.draw_text(&item.label, card.x + 14, cur_y + 4, item_color);

            if let Some(sc) = &item.shortcut {
                let sc_w = sc.len() as i32 * 8;
                let sc_x = card.x + card.width - sc_w - 14;
                surface.draw_text(sc, sc_x, cur_y + 4, self.tokens.text_muted);
            }

            if item.separator_after {
                let sep_rect = Rect::new(card.x + 8, cur_y + 23, card.width - 16, 1);
                surface.fill_rect(sep_rect, self.tokens.divider);
            }
            cur_y += 24;
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menubar_initialization_and_layout() {
        let bar = TopMenuBar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);
        assert_eq!(bar.height, 28);
        assert_eq!(bar.width, 1024);
        assert!(!bar.app_menu.categories.is_empty());

        // Left items bounds
        assert_eq!(bar.system_menu.bounds.x, 12);
        assert!(bar.app_menu.categories[0].bounds.x > bar.system_menu.bounds.x);

        // Right items bounds
        assert!(bar.clock.bounds.x > 800);
        assert!(bar.user.bounds.x > bar.clock.bounds.x);
    }

    #[test]
    fn test_dynamic_window_focus_change() {
        let mut bar = TopMenuBar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);
        assert_eq!(bar.app_menu.app_name, "Finder");

        // Focus switches to Text Editor
        let editor_menu = vec![
            MenuCategory::new("File", vec![MenuItem::new("Save", 10, Some("⌘S"))]),
            MenuCategory::new("Format", vec![MenuItem::new("Bold", 20, Some("⌘B"))]),
        ];
        bar.update_active_app(42, "Text Editor", editor_menu);

        assert_eq!(bar.app_menu.app_name, "Text Editor");
        assert_eq!(bar.app_menu.window_id, 42);
        assert_eq!(bar.app_menu.categories.len(), 2);
        assert_eq!(bar.app_menu.categories[1].title, "Format");
    }

    #[test]
    fn test_menu_scrubbing_and_click_toggle() {
        let mut bar = TopMenuBar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);
        assert_eq!(bar.active_flyout, ActiveFlyout::None);

        // Click system menu to open
        let sys_click = Point::new(bar.system_menu.bounds.x + 5, 10);
        bar.handle_mouse_click(sys_click);
        assert_eq!(bar.active_flyout, ActiveFlyout::SystemMenu);

        // Mouse scrubs to first app category (File)
        let cat_pos = Point::new(bar.app_menu.categories[0].bounds.x + 5, 10);
        let changed = bar.handle_mouse_move(cat_pos);
        assert!(changed);
        assert_eq!(bar.active_flyout, ActiveFlyout::AppMenu(0));

        // Click again on same category closes it
        bar.handle_mouse_click(cat_pos);
        assert_eq!(bar.active_flyout, ActiveFlyout::None);
    }

    #[test]
    fn test_keyboard_accelerators() {
        let bar = TopMenuBar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);

        // File -> New Window (⌘N)
        let action = bar.handle_keyboard_shortcut("⌘N");
        assert_eq!(action, Some(101));

        // System -> Lock Screen (⌃⌘Q)
        let lock_action = bar.handle_keyboard_shortcut("⌃⌘Q");
        assert_eq!(lock_action, Some(7));

        // Non-existent shortcut
        assert_eq!(bar.handle_keyboard_shortcut("⌘K"), None);
    }

    #[test]
    fn test_menubar_rendering() {
        let mut bar = TopMenuBar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);
        let mut surface = BufferSurface::new(1024, 200);
        surface.clear(Color::rgb(20, 20, 20));

        // Open audio flyout
        bar.active_flyout = ActiveFlyout::Audio;
        bar.render(&mut surface);

        // Pixels within the 28px bar should be populated
        assert_ne!(surface.get_pixel(50, 10), Color::rgb(20, 20, 20));
        // Popover pixels should also be rendered
        assert_ne!(surface.get_pixel(bar.audio.bounds.x - 20, 45), Color::rgb(20, 20, 20));
    }
}
