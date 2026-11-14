//! userland/desktop/nova-shell/src/action_center.rs
//!
//! Windows 11 & macOS Hybrid Action Center & Notification Flyout.
//!
//! Features:
//! - Slide-in notification drawer from the right edge with acrylic blur and Level 4 elevation.
//! - Rich push notification queue with actionable buttons (reply, dismiss, open).
//! - Transient banner toasts in the bottom-right corner with auto-dismiss countdown.
//! - Persistent notification history archive with per-app grouping and "Clear All".
//! - Quick Settings grid (Wi-Fi, Bluetooth, Dark Mode, Night Light, Do Not Disturb, Airplane Mode).
//! - Dual slider controls: Display Brightness and Audio Volume.
//! - Configurable Quiet Hours / Do Not Disturb toast suppression.

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

use nova_ui::prelude::*;

pub const ACTION_CENTER_WIDTH: i32 = 360;
pub const TOAST_WIDTH: i32 = 320;
pub const TOAST_HEIGHT: i32 = 76;
pub const DEFAULT_TOAST_TTL_TICKS: u32 = 300; // ~5 seconds at 60Hz

// ============================================================================
// Data Models
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationUrgency {
    Low,
    Normal,
    Critical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
    pub bounds: Rect,
}

impl NotificationAction {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: String::from(id),
            label: String::from(label),
            bounds: Rect::new(0, 0, 0, 0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotificationItem {
    pub id: u64,
    pub app_id: String,
    pub app_name: String,
    pub icon_glyph: String,
    pub title: String,
    pub body: String,
    pub timestamp_str: String,
    pub urgency: NotificationUrgency,
    pub actions: Vec<NotificationAction>,
    pub is_read: bool,
    pub toast_ttl_ticks: u32,
    pub is_toast_active: bool,
    pub toast_bounds: Rect,
    pub close_bounds: Rect,
}

impl NotificationItem {
    pub fn new(
        id: u64,
        app_id: &str,
        app_name: &str,
        icon_glyph: &str,
        title: &str,
        body: &str,
        timestamp_str: &str,
        urgency: NotificationUrgency,
        actions: Vec<NotificationAction>,
    ) -> Self {
        Self {
            id,
            app_id: String::from(app_id),
            app_name: String::from(app_name),
            icon_glyph: String::from(icon_glyph),
            title: String::from(title),
            body: String::from(body),
            timestamp_str: String::from(timestamp_str),
            urgency,
            actions,
            is_read: false,
            toast_ttl_ticks: DEFAULT_TOAST_TTL_TICKS,
            is_toast_active: true,
            toast_bounds: Rect::new(0, 0, TOAST_WIDTH, TOAST_HEIGHT),
            close_bounds: Rect::new(0, 0, 16, 16),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickSettingsState {
    pub wifi_enabled: bool,
    pub bluetooth_enabled: bool,
    pub dark_mode: bool,
    pub night_light: bool,
    pub do_not_disturb: bool,
    pub airplane_mode: bool,
    pub brightness_percent: u32,
    pub volume_percent: u32,
}

impl Default for QuickSettingsState {
    fn default() -> Self {
        Self {
            wifi_enabled: true,
            bluetooth_enabled: true,
            dark_mode: true,
            night_light: false,
            do_not_disturb: false,
            airplane_mode: false,
            brightness_percent: 85,
            volume_percent: 75,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionCenterOutcome {
    None,
    ToggleWifi(bool),
    ToggleBluetooth(bool),
    ToggleDarkMode(bool),
    ToggleNightLight(bool),
    ToggleDnd(bool),
    ToggleAirplane(bool),
    SetBrightness(u32),
    SetVolume(u32),
    ClearAllNotifications,
    DismissNotification(u64),
    TriggerNotificationAction {
        notification_id: u64,
        action_id: String,
    },
    OpenNotification(u64),
}

// ============================================================================
// Action Center Drawer Component
// ============================================================================

pub struct ActionCenterDrawer {
    pub screen_w: i32,
    pub screen_h: i32,
    pub drawer_w: i32,
    pub is_open: bool,
    pub slide_offset: f32, // 0.0 (closed) to 1.0 (open)
    pub quick_settings: QuickSettingsState,
    pub notifications: Vec<NotificationItem>,
    pub next_notification_id: u64,
    pub theme_mode: ThemeMode,
    pub tokens: ColorTokens,
    pub elevation: [ElevationTier; 6],
    pub corners: CornerTokens,

    // Cached hit-testing button bounds
    pub btn_wifi: Rect,
    pub btn_bluetooth: Rect,
    pub btn_dark_mode: Rect,
    pub btn_night_light: Rect,
    pub btn_dnd: Rect,
    pub btn_airplane: Rect,
    pub slider_brightness: Rect,
    pub slider_volume: Rect,
    pub btn_clear_all: Rect,
}

impl ActionCenterDrawer {
    pub fn new(screen_w: i32, screen_h: i32, mode: ThemeMode, accent: AccentPalette) -> Self {
        let tokens = ColorTokens::build(mode, accent);
        let elevation = get_elevation_tiers(mode);
        let corners = CornerTokens::default();

        let mut ac = Self {
            screen_w,
            screen_h,
            drawer_w: ACTION_CENTER_WIDTH,
            is_open: false,
            slide_offset: 0.0,
            quick_settings: QuickSettingsState::default(),
            notifications: Vec::new(),
            next_notification_id: 1,
            theme_mode: mode,
            tokens,
            elevation,
            corners,
            btn_wifi: Rect::new(0, 0, 0, 0),
            btn_bluetooth: Rect::new(0, 0, 0, 0),
            btn_dark_mode: Rect::new(0, 0, 0, 0),
            btn_night_light: Rect::new(0, 0, 0, 0),
            btn_dnd: Rect::new(0, 0, 0, 0),
            btn_airplane: Rect::new(0, 0, 0, 0),
            slider_brightness: Rect::new(0, 0, 0, 0),
            slider_volume: Rect::new(0, 0, 0, 0),
            btn_clear_all: Rect::new(0, 0, 0, 0),
        };

        ac.relayout_drawer();
        ac
    }

    pub fn toggle_open(&mut self) {
        self.is_open = !self.is_open;
    }

    pub fn open(&mut self) {
        self.is_open = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    /// Posts a new push notification
    pub fn post_notification(
        &mut self,
        app_id: &str,
        app_name: &str,
        icon_glyph: &str,
        title: &str,
        body: &str,
        urgency: NotificationUrgency,
        actions: Vec<NotificationAction>,
    ) -> u64 {
        let id = self.next_notification_id;
        self.next_notification_id += 1;

        let mut item = NotificationItem::new(
            id,
            app_id,
            app_name,
            icon_glyph,
            title,
            body,
            "Just now",
            urgency,
            actions,
        );

        // If Do Not Disturb is active, suppress banner toast popup
        if self.quick_settings.do_not_disturb && urgency != NotificationUrgency::Critical {
            item.is_toast_active = false;
        }

        self.notifications.insert(0, item);
        self.relayout_toasts();
        id
    }

    /// Dismiss a specific notification by ID
    pub fn dismiss_notification(&mut self, id: u64) {
        self.notifications.retain(|n| n.id != id);
        self.relayout_toasts();
    }

    /// Clear all notifications from history
    pub fn clear_all_notifications(&mut self) {
        self.notifications.clear();
        self.relayout_toasts();
    }

    /// Advances slide-in and toast countdown animation ticks
    pub fn tick_animations(&mut self, dt_factor: f32) -> bool {
        let mut active = false;

        // 1. Drawer slide-in easing
        let target_slide = if self.is_open { 1.0 } else { 0.0 };
        let slide_diff = target_slide - self.slide_offset;
        if slide_diff.abs() > 0.015 {
            self.slide_offset += slide_diff * 0.35 * dt_factor;
            active = true;
        } else {
            self.slide_offset = target_slide;
        }

        // 2. Toast countdown
        for item in self.notifications.iter_mut() {
            if item.is_toast_active {
                if item.toast_ttl_ticks > 0 {
                    item.toast_ttl_ticks = item.toast_ttl_ticks.saturating_sub(1);
                    active = true;
                } else {
                    item.is_toast_active = false;
                }
            }
        }

        self.relayout_drawer();
        self.relayout_toasts();
        active
    }

    /// Computes drawer bounding rectangle based on slide offset
    pub fn drawer_bounds(&self) -> Rect {
        let drawer_x = self.screen_w - (self.drawer_w as f32 * self.slide_offset) as i32;
        let drawer_y = 28; // Below menubar
        let drawer_h = self.screen_h - 28 - 48; // Above taskbar
        Rect::new(drawer_x, drawer_y, self.drawer_w, drawer_h)
    }

    fn relayout_drawer(&mut self) {
        let d = self.drawer_bounds();
        let pad = 16;
        let gap = 10;
        let btn_w = (self.drawer_w - pad * 2 - gap * 2) / 3;
        let btn_h = 44;

        let row1_y = d.y + pad + 24; // Below Quick Settings header
        let row2_y = row1_y + btn_h + gap;

        // Row 1: Wi-Fi, Bluetooth, Dark Mode
        self.btn_wifi = Rect::new(d.x + pad, row1_y, btn_w, btn_h);
        self.btn_bluetooth = Rect::new(d.x + pad + btn_w + gap, row1_y, btn_w, btn_h);
        self.btn_dark_mode = Rect::new(d.x + pad + (btn_w + gap) * 2, row1_y, btn_w, btn_h);

        // Row 2: Night Light, DND, Airplane Mode
        self.btn_night_light = Rect::new(d.x + pad, row2_y, btn_w, btn_h);
        self.btn_dnd = Rect::new(d.x + pad + btn_w + gap, row2_y, btn_w, btn_h);
        self.btn_airplane = Rect::new(d.x + pad + (btn_w + gap) * 2, row2_y, btn_w, btn_h);

        // Sliders
        let slider_y1 = row2_y + btn_h + 16;
        let slider_y2 = slider_y1 + 32;
        let slider_w = self.drawer_w - pad * 2;
        self.slider_brightness = Rect::new(d.x + pad, slider_y1, slider_w, 20);
        self.slider_volume = Rect::new(d.x + pad, slider_y2, slider_w, 20);

        // Clear All button in Notifications section
        let notif_header_y = slider_y2 + 48;
        self.btn_clear_all = Rect::new(d.x + self.drawer_w - 90, notif_header_y, 74, 22);
    }

    fn relayout_toasts(&mut self) {
        let margin_right = 16;
        let margin_bottom = 60; // Above 48px taskbar
        let toast_x = self.screen_w - TOAST_WIDTH - margin_right;
        let mut cur_bottom_y = self.screen_h - margin_bottom;

        // Stack up to 3 active toasts
        let mut active_count = 0;
        for item in self.notifications.iter_mut() {
            if item.is_toast_active && active_count < 3 {
                let toast_y = cur_bottom_y - TOAST_HEIGHT;
                item.toast_bounds = Rect::new(toast_x, toast_y, TOAST_WIDTH, TOAST_HEIGHT);
                item.close_bounds = Rect::new(toast_x + TOAST_WIDTH - 22, toast_y + 6, 16, 16);

                // Relayout action buttons inside toast
                let action_count = item.actions.len();
                if action_count > 0 {
                    let act_w = 64;
                    let act_h = 22;
                    let mut act_x = toast_x + TOAST_WIDTH - 12 - (action_count as i32 * (act_w + 6));
                    let act_y = toast_y + TOAST_HEIGHT - act_h - 8;
                    for action in item.actions.iter_mut() {
                        action.bounds = Rect::new(act_x, act_y, act_w, act_h);
                        act_x += act_w + 6;
                    }
                }

                cur_bottom_y -= TOAST_HEIGHT + 10;
                active_count += 1;
            }
        }
    }

    /// Handles mouse click interaction across Quick Settings, drawer, and banner toasts
    pub fn handle_mouse_click(&mut self, pos: Point) -> ActionCenterOutcome {
        // 1. Toast banner interactions (when drawer is closed or open)
        for item in self.notifications.iter_mut() {
            if item.is_toast_active {
                if item.close_bounds.contains(pos) {
                    let id = item.id;
                    item.is_toast_active = false;
                    return ActionCenterOutcome::DismissNotification(id);
                }
                for act in &item.actions {
                    if act.bounds.contains(pos) {
                        let nid = item.id;
                        let aid = act.id.clone();
                        item.is_toast_active = false;
                        return ActionCenterOutcome::TriggerNotificationAction {
                            notification_id: nid,
                            action_id: aid,
                        };
                    }
                }
                if item.toast_bounds.contains(pos) {
                    let nid = item.id;
                    item.is_toast_active = false;
                    item.is_read = true;
                    return ActionCenterOutcome::OpenNotification(nid);
                }
            }
        }

        // If drawer is closed, return
        if !self.is_open && self.slide_offset <= 0.05 {
            return ActionCenterOutcome::None;
        }

        let d = self.drawer_bounds();
        if !d.contains(pos) {
            // Click outside drawer closes it
            self.close();
            return ActionCenterOutcome::None;
        }

        // 2. Quick Settings toggles
        if self.btn_wifi.contains(pos) {
            self.quick_settings.wifi_enabled = !self.quick_settings.wifi_enabled;
            if self.quick_settings.wifi_enabled && self.quick_settings.airplane_mode {
                self.quick_settings.airplane_mode = false;
            }
            return ActionCenterOutcome::ToggleWifi(self.quick_settings.wifi_enabled);
        }
        if self.btn_bluetooth.contains(pos) {
            self.quick_settings.bluetooth_enabled = !self.quick_settings.bluetooth_enabled;
            if self.quick_settings.bluetooth_enabled && self.quick_settings.airplane_mode {
                self.quick_settings.airplane_mode = false;
            }
            return ActionCenterOutcome::ToggleBluetooth(self.quick_settings.bluetooth_enabled);
        }
        if self.btn_dark_mode.contains(pos) {
            self.quick_settings.dark_mode = !self.quick_settings.dark_mode;
            self.theme_mode = if self.quick_settings.dark_mode { ThemeMode::Dark } else { ThemeMode::Light };
            self.tokens = ColorTokens::build(self.theme_mode, AccentPalette::FluentBlue);
            return ActionCenterOutcome::ToggleDarkMode(self.quick_settings.dark_mode);
        }
        if self.btn_night_light.contains(pos) {
            self.quick_settings.night_light = !self.quick_settings.night_light;
            return ActionCenterOutcome::ToggleNightLight(self.quick_settings.night_light);
        }
        if self.btn_dnd.contains(pos) {
            self.quick_settings.do_not_disturb = !self.quick_settings.do_not_disturb;
            return ActionCenterOutcome::ToggleDnd(self.quick_settings.do_not_disturb);
        }
        if self.btn_airplane.contains(pos) {
            self.quick_settings.airplane_mode = !self.quick_settings.airplane_mode;
            if self.quick_settings.airplane_mode {
                self.quick_settings.wifi_enabled = false;
                self.quick_settings.bluetooth_enabled = false;
            }
            return ActionCenterOutcome::ToggleAirplane(self.quick_settings.airplane_mode);
        }

        // 3. Sliders click
        if self.slider_brightness.contains(pos) {
            let relative_x = (pos.x - self.slider_brightness.x).max(0) as u32;
            let percent = ((relative_x * 100) / self.slider_brightness.width as u32).min(100);
            self.quick_settings.brightness_percent = percent;
            return ActionCenterOutcome::SetBrightness(percent);
        }
        if self.slider_volume.contains(pos) {
            let relative_x = (pos.x - self.slider_volume.x).max(0) as u32;
            let percent = ((relative_x * 100) / self.slider_volume.width as u32).min(100);
            self.quick_settings.volume_percent = percent;
            return ActionCenterOutcome::SetVolume(percent);
        }

        // 4. Clear All Notifications
        if self.btn_clear_all.contains(pos) {
            self.clear_all_notifications();
            return ActionCenterOutcome::ClearAllNotifications;
        }

        ActionCenterOutcome::None
    }

    /// Render Action Center drawer and active toast banners
    pub fn render(&self, surface: &mut BufferSurface) {
        // 1. Render active transient toast banners in bottom-right corner
        for item in &self.notifications {
            if item.is_toast_active {
                self.render_toast_card(surface, item);
            }
        }

        // 2. Render Slide-In Drawer if visible
        if self.slide_offset > 0.01 {
            let d = self.drawer_bounds();

            // Level 4 elevation shadow & acrylic background
            render_elevated_card(
                surface,
                d,
                self.corners.medium,
                4,
                &self.elevation,
                &self.tokens,
            );
            surface.apply_gaussian_blur(d, self.tokens.acrylic_blur_radius / 2);
            surface.fill_rounded_rect(d, self.corners.medium, self.tokens.popup_background);

            // Left delicate border
            surface.fill_rect(Rect::new(d.x, d.y, 1, d.height), self.tokens.border_subtle);

            // Quick Settings Title
            surface.draw_text("Quick Settings", d.x + 16, d.y + 14, self.tokens.text_primary);

            // Render Quick Settings grid buttons
            self.render_quick_tile(surface, self.btn_wifi, "Wi-Fi", self.quick_settings.wifi_enabled);
            self.render_quick_tile(surface, self.btn_bluetooth, "Bluetooth", self.quick_settings.bluetooth_enabled);
            self.render_quick_tile(surface, self.btn_dark_mode, "Dark Mode", self.quick_settings.dark_mode);
            self.render_quick_tile(surface, self.btn_night_light, "Night Light", self.quick_settings.night_light);
            self.render_quick_tile(surface, self.btn_dnd, "Focus / DND", self.quick_settings.do_not_disturb);
            self.render_quick_tile(surface, self.btn_airplane, "Airplane", self.quick_settings.airplane_mode);

            // Render Brightness Slider
            self.render_slider(surface, self.slider_brightness, "Brightness", self.quick_settings.brightness_percent);
            // Render Volume Slider
            self.render_slider(surface, self.slider_volume, "Sound Volume", self.quick_settings.volume_percent);

            // Divider between Quick Settings and Notifications
            let div_y = self.slider_volume.y + 36;
            surface.fill_rect(Rect::new(d.x + 16, div_y, d.width - 32, 1), self.tokens.divider);

            // Notifications Header
            let notif_y = div_y + 12;
            surface.draw_text("Notifications", d.x + 16, notif_y + 2, self.tokens.text_primary);

            // Clear All Button
            surface.fill_rounded_rect(self.btn_clear_all, self.corners.small, self.tokens.card_hover);
            surface.draw_text("Clear all", self.btn_clear_all.x + 10, self.btn_clear_all.y + 4, self.tokens.text_secondary);

            // Notification List
            let mut cur_card_y = notif_y + 30;
            if self.notifications.is_empty() {
                surface.draw_text("No new notifications", d.x + 16, cur_card_y + 10, self.tokens.text_muted);
            } else {
                for item in self.notifications.iter().take(5) {
                    let card_rect = Rect::new(d.x + 14, cur_card_y, d.width - 28, 64);
                    surface.fill_rounded_rect(card_rect, self.corners.small, self.tokens.card_background);

                    // App icon & title
                    surface.draw_text(&item.icon_glyph, card_rect.x + 8, card_rect.y + 8, self.tokens.accent_primary);
                    surface.draw_text(&item.title, card_rect.x + 28, card_rect.y + 8, self.tokens.text_primary);
                    surface.draw_text(&item.body, card_rect.x + 28, card_rect.y + 26, self.tokens.text_muted);

                    cur_card_y += 72;
                }
            }
        }
    }

    fn render_quick_tile(&self, surface: &mut BufferSurface, bounds: Rect, label: &str, is_on: bool) {
        let (bg, text_col) = if is_on {
            (self.tokens.accent_primary, self.tokens.text_on_accent)
        } else {
            (self.tokens.card_background, self.tokens.text_secondary)
        };

        surface.fill_rounded_rect(bounds, self.corners.small, bg);
        surface.draw_text(label, bounds.x + 8, bounds.y + 14, text_col);
    }

    fn render_slider(&self, surface: &mut BufferSurface, bounds: Rect, label: &str, percent: u32) {
        // Label
        surface.draw_text(label, bounds.x, bounds.y - 12, self.tokens.text_secondary);

        // Background Track
        surface.fill_rounded_rect(bounds, 3, self.tokens.card_hover);

        // Active Track
        let active_w = ((bounds.width as u32 * percent) / 100) as i32;
        let active_rect = Rect::new(bounds.x, bounds.y, active_w, bounds.height);
        surface.fill_rounded_rect(active_rect, 3, self.tokens.accent_primary);
    }

    fn render_toast_card(&self, surface: &mut BufferSurface, item: &NotificationItem) {
        let b = item.toast_bounds;

        // Level 3 elevation shadow & card
        render_elevated_card(
            surface,
            b,
            self.corners.medium,
            3,
            &self.elevation,
            &self.tokens,
        );
        surface.fill_rounded_rect(b, self.corners.medium, self.tokens.popup_background);
        surface.fill_rect(Rect::new(b.x, b.y, b.width, 1), self.tokens.border_subtle);

        // Urgency color stripe on left border (e.g. Critical = Crimson)
        let stripe_color = match item.urgency {
            NotificationUrgency::Critical => Color::rgb(232, 17, 35),
            _ => self.tokens.accent_primary,
        };
        surface.fill_rounded_rect(Rect::new(b.x, b.y, 4, b.height), 2, stripe_color);

        // App Icon & Title
        surface.draw_text(&item.icon_glyph, b.x + 12, b.y + 10, self.tokens.accent_primary);
        surface.draw_text(&item.title, b.x + 32, b.y + 10, self.tokens.text_primary);

        // Body text
        surface.draw_text(&item.body, b.x + 32, b.y + 28, self.tokens.text_secondary);

        // Close button (✕)
        surface.fill_rect(item.close_bounds, self.tokens.card_hover);
        for d in 0..6 {
            surface.set_pixel(item.close_bounds.x + 5 + d, item.close_bounds.y + 5 + d, self.tokens.text_muted);
            surface.set_pixel(item.close_bounds.x + 10 - d, item.close_bounds.y + 5 + d, self.tokens.text_muted);
        }

        // Action Buttons
        for act in &item.actions {
            surface.fill_rounded_rect(act.bounds, self.corners.small, self.tokens.card_hover);
            surface.draw_text(&act.label, act.bounds.x + 8, act.bounds.y + 4, self.tokens.accent_primary);
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
    fn test_action_center_drawer_toggle_and_slide() {
        let mut ac = ActionCenterDrawer::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        assert!(!ac.is_open);
        assert_eq!(ac.slide_offset, 0.0);

        ac.open();
        assert!(ac.is_open);

        // Tick animation
        ac.tick_animations(1.0);
        assert!(ac.slide_offset > 0.0);

        // Advance to full open
        for _ in 0..15 {
            ac.tick_animations(1.0);
        }
        assert_eq!(ac.slide_offset, 1.0);
        let bounds = ac.drawer_bounds();
        assert_eq!(bounds.x, 1024 - ACTION_CENTER_WIDTH);
    }

    #[test]
    fn test_push_notifications_and_toasts() {
        let mut ac = ActionCenterDrawer::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);

        let actions = vec![NotificationAction::new("open", "Open"), NotificationAction::new("dismiss", "Dismiss")];
        let id = ac.post_notification(
            "terminal",
            "Terminal",
            "💻",
            "Build Complete",
            "kernel/build: success in 1.4s",
            NotificationUrgency::Normal,
            actions,
        );

        assert_eq!(ac.notifications.len(), 1);
        assert!(ac.notifications[0].is_toast_active);

        // Action button click
        let act_pos = Point::new(ac.notifications[0].actions[0].bounds.x + 4, ac.notifications[0].actions[0].bounds.y + 4);
        let outcome = ac.handle_mouse_click(act_pos);
        assert_eq!(
            outcome,
            ActionCenterOutcome::TriggerNotificationAction {
                notification_id: id,
                action_id: String::from("open"),
            }
        );
    }

    #[test]
    fn test_do_not_disturb_toast_suppression() {
        let mut ac = ActionCenterDrawer::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        ac.quick_settings.do_not_disturb = true;

        // Normal notification during DND -> toast suppressed!
        ac.post_notification(
            "chat",
            "Chat",
            "💬",
            "Alice",
            "Hey, check out the new build",
            NotificationUrgency::Normal,
            vec![],
        );
        assert_eq!(ac.notifications.len(), 1);
        assert!(!ac.notifications[0].is_toast_active); // suppressed

        // Critical notification -> overrides DND!
        ac.post_notification(
            "system",
            "System",
            "⚠️",
            "Low Battery",
            "10% battery remaining",
            NotificationUrgency::Critical,
            vec![],
        );
        assert_eq!(ac.notifications.len(), 2);
        assert!(ac.notifications[0].is_toast_active); // critical toast shown
    }

    #[test]
    fn test_quick_settings_grid_toggles() {
        let mut ac = ActionCenterDrawer::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        ac.is_open = true;
        ac.slide_offset = 1.0;
        ac.relayout_drawer();

        // Toggle Wi-Fi
        let wifi_click = Point::new(ac.btn_wifi.x + 4, ac.btn_wifi.y + 4);
        let out = ac.handle_mouse_click(wifi_click);
        assert_eq!(out, ActionCenterOutcome::ToggleWifi(false));
        assert!(!ac.quick_settings.wifi_enabled);

        // Toggle Airplane Mode -> disables Wi-Fi and Bluetooth
        let air_click = Point::new(ac.btn_airplane.x + 4, ac.btn_airplane.y + 4);
        let out2 = ac.handle_mouse_click(air_click);
        assert_eq!(out2, ActionCenterOutcome::ToggleAirplane(true));
        assert!(ac.quick_settings.airplane_mode);
        assert!(!ac.quick_settings.wifi_enabled);
        assert!(!ac.quick_settings.bluetooth_enabled);

        // Volume slider adjust
        let vol_click = Point::new(ac.slider_volume.x + (ac.slider_volume.width / 2), ac.slider_volume.y + 5);
        let out3 = ac.handle_mouse_click(vol_click);
        assert!(matches!(out3, ActionCenterOutcome::SetVolume(_)));
    }

    #[test]
    fn test_action_center_rendering() {
        let mut ac = ActionCenterDrawer::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        ac.open();
        ac.slide_offset = 1.0;
        ac.post_notification("app", "App", "🔔", "Hello", "Test alert", NotificationUrgency::Normal, vec![]);

        let mut surface = BufferSurface::new(1024, 768);
        surface.clear(Color::rgb(20, 20, 20));

        ac.render(&mut surface);

        // Drawer background pixels should be painted
        let d = ac.drawer_bounds();
        assert_ne!(surface.get_pixel(d.x + 50, d.y + 50), Color::rgb(20, 20, 20));
    }
}
