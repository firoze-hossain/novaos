//! userland/desktop/apps/nova-settings/src/lib.rs
//!
//! # NovaOS Unified System Settings & Control Hub (nova-settings)
//!
//! Synthesizes Windows 11 Settings and macOS System Settings into a unified,
//! searchable configuration hub.
//!
//! Categories:
//! - System (Display resolution/scaling, Sound output/input, Power/Sleep)
//! - Network & Internet (Wi-Fi, Ethernet, VPN, Proxy)
//! - Personalization (Wallpapers, Accent colors, Dark/Light mode)
//! - Apps (Default applications, Startup apps, Permissions)
//! - Accounts (Multi-user, Passwords, Privileges & wheel)
//! - Updates (Dual-slot A/B updates, OS version, Rollback recovery)
//!
//! Subsystems:
//! - `backend.rs`: Sysfs (/sys/) queries/mutations and `novad` IPC message protocol.
//! - `search.rs`: Real-time instant search with inverted index scoring and deep links.
//! - `view.rs`: Windows 11 Fluent mica + macOS rounded cards graphical view.
//! - `panels/*.rs`: Independent modular panels for each category.

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
extern crate alloc;

#[cfg(not(test))]
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};

pub mod types;
pub mod backend;
pub mod search;
pub mod view;
pub mod panels;

use backend::{BootSlot, NovadCommand, NovadResponse, SlotHealth, SystemBackend};
use nova_ui::prelude::{BufferSurface, Point};
use panels::{
    AccountsPanel, AppsPanel, NetworkPanel, PersonalizationPanel, SettingsPanel, SystemPanel,
    UpdatesPanel,
};
use search::SettingsSearchEngine;
use types::{PanelCategory, SearchResult, SettingItem, SettingValue};
use view::{HitTarget, SettingsView};

/// Top-level application controller for Nova Settings.
pub struct NovaSettingsApp {
    pub active_category: PanelCategory,
    pub backend: SystemBackend,
    pub search_engine: SettingsSearchEngine,
    pub search_query: String,
    pub search_results: Vec<SearchResult>,
    pub view: SettingsView,
    pub panels: Vec<Box<dyn SettingsPanel>>,
}

impl Default for NovaSettingsApp {
    fn default() -> Self {
        Self::new()
    }
}

impl NovaSettingsApp {
    pub fn new() -> Self {
        let backend = SystemBackend::new();
        let mut search_engine = SettingsSearchEngine::new();
        search_engine.rebuild_index(&backend.settings_store);

        let panels: Vec<Box<dyn SettingsPanel>> = vec![
            Box::new(SystemPanel::new()),
            Box::new(NetworkPanel::new()),
            Box::new(PersonalizationPanel::new()),
            Box::new(AppsPanel::new()),
            Box::new(AccountsPanel::new()),
            Box::new(UpdatesPanel::new()),
        ];

        Self {
            active_category: PanelCategory::System,
            backend,
            search_engine,
            search_query: String::new(),
            search_results: Vec::new(),
            view: SettingsView::default(),
            panels,
        }
    }

    /// Selects an active category.
    pub fn select_category(&mut self, cat: PanelCategory) {
        self.active_category = cat;
        self.search_query.clear();
        self.search_results.clear();
    }

    /// Updates search query and triggers real-time search index query.
    pub fn set_search_query(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.search_results = self.search_engine.search(query);
    }

    /// Clears search filter.
    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.search_results.clear();
    }

    /// Navigates directly to the panel containing the selected search result.
    pub fn select_search_result(&mut self, idx: usize) -> bool {
        if let Some(res) = self.search_results.get(idx) {
            self.active_category = res.category;
            self.search_query.clear();
            self.search_results.clear();
            true
        } else {
            false
        }
    }

    /// Toggles a boolean setting.
    pub fn toggle_setting(&mut self, key: &str) -> bool {
        if let Some(item) = self.backend.get_setting(key) {
            if let SettingValue::Bool(b) = item.value {
                let new_val = SettingValue::Bool(!b);
                let ok = self.backend.set_setting(key, new_val).is_ok();
                if ok {
                    self.search_engine.rebuild_index(&self.backend.settings_store);
                }
                return ok;
            }
        }
        false
    }

    /// Cycles a choice setting to the next available option.
    pub fn cycle_choice(&mut self, key: &str) -> bool {
        if let Some(item) = self.backend.get_setting(key) {
            if let SettingValue::Choice { selected, options } = &item.value {
                let next_idx = (selected + 1) % options.len();
                let mut updated = item.value.clone();
                updated.set_choice(next_idx);
                let ok = self.backend.set_setting(key, updated).is_ok();
                if ok {
                    self.search_engine.rebuild_index(&self.backend.settings_store);
                }
                return ok;
            }
        }
        false
    }

    /// Sets an integer slider value from 0.0 to 1.0 ratio.
    pub fn set_slider_ratio(&mut self, key: &str, ratio: f32) -> bool {
        if let Some(item) = self.backend.get_setting(key) {
            if let SettingValue::Int(_) = item.value {
                let val = (ratio.clamp(0.0, 1.0) * 100.0) as i64;
                let ok = self.backend.set_setting(key, SettingValue::Int(val)).is_ok();
                if ok {
                    self.search_engine.rebuild_index(&self.backend.settings_store);
                }
                return ok;
            }
        }
        false
    }

    /// Handles mouse click / tap event.
    pub fn handle_click(&mut self, pt: Point) -> bool {
        let items: Vec<SettingItem> = self
            .backend
            .settings_store
            .values()
            .filter(|i| i.category == self.active_category)
            .cloned()
            .collect();

        if let Some(target) = self.view.hit_test(
            pt,
            self.active_category,
            &self.search_query,
            &self.search_results,
            &items,
        ) {
            match target {
                HitTarget::Category(cat) => {
                    self.select_category(cat);
                    true
                }
                HitTarget::SearchBox => true,
                HitTarget::ClearSearch => {
                    self.clear_search();
                    true
                }
                HitTarget::SearchResult(idx) => self.select_search_result(idx),
                HitTarget::ToggleSwitch(key) => self.toggle_setting(&key),
                HitTarget::NextChoice(key) => self.cycle_choice(&key),
                HitTarget::SliderDrag { key, ratio } => self.set_slider_ratio(&key, ratio),
                HitTarget::ActionButton(_) => true,
            }
        } else {
            false
        }
    }

    /// Renders the complete settings view to the provided buffer surface.
    pub fn render(&self, surface: &mut BufferSurface) {
        let items: Vec<SettingItem> = self
            .backend
            .settings_store
            .values()
            .filter(|i| i.category == self.active_category)
            .cloned()
            .collect();

        self.view.render(
            surface,
            self.active_category,
            &self.search_query,
            &self.search_results,
            &items,
        );
    }
}

// ============================================================================
// Comprehensive Self-Test Suite
// ============================================================================

/// Runs the 10-point comprehensive self-test suite for nova-settings.
pub fn run_nova_settings_selftest() -> bool {
    let mut app = NovaSettingsApp::new();

    // 1. Verify Category Registration and Synthesis
    let categories = PanelCategory::all();
    if categories.len() != 6 {
        return false;
    }
    if categories[0] != PanelCategory::System || categories[5] != PanelCategory::Updates {
        return false;
    }

    // 2. Verify Sysfs Interface & Backend Synchronization
    // Initial read
    if app.backend.sysfs.read("/sys/class/drm/card0/mode") != Some("1920x1080@60".into()) {
        return false;
    }
    if app.backend.sysfs.read("/sys/class/sound/pcm0p/volume") != Some("75".into()) {
        return false;
    }
    // Mutation via backend
    if app.backend.set_setting("system.sound.master_volume", SettingValue::Int(90)).is_err() {
        return false;
    }
    if app.backend.sysfs.read("/sys/class/sound/pcm0p/volume") != Some("90".into()) {
        return false;
    }

    // 3. Verify Real-Time Search & Inverted Index Scoring
    // Search for "resolution" -> should find Display Resolution in System
    app.set_search_query("resolution");
    if app.search_results.is_empty() {
        return false;
    }
    if app.search_results[0].item_key != "system.display.resolution" {
        return false;
    }
    if app.search_results[0].category != PanelCategory::System {
        return false;
    }

    // Search for "wifi" -> should find Wi-Fi in Network
    app.set_search_query("wifi");
    if app.search_results.is_empty() {
        return false;
    }
    if app.search_results[0].category != PanelCategory::Network {
        return false;
    }

    // Search for "dark mode" -> should find Dark / Light Mode in Personalization
    app.set_search_query("dark mode");
    if app.search_results.is_empty() {
        return false;
    }
    if app.search_results[0].category != PanelCategory::Personalization {
        return false;
    }

    // 4. Verify Deep Link Search Navigation
    app.set_search_query("wifi");
    if !app.select_search_result(0) {
        return false;
    }
    if app.active_category != PanelCategory::Network || !app.search_query.is_empty() {
        return false;
    }

    // 5. Verify Network Panel State & Mutators
    if !app.toggle_setting("network.wifi.enabled") {
        return false;
    }
    if app.backend.get_setting("network.wifi.enabled").unwrap().value != SettingValue::Bool(false) {
        return false;
    }
    // Sysfs sync
    if app.backend.sysfs.read("/sys/class/net/wlan0/operstate") != Some("down".into()) {
        return false;
    }

    // 6. Verify Personalization Panel & Accent Colors
    app.select_category(PanelCategory::Personalization);
    let accent_setting = app.backend.get_setting("personalization.appearance.accent_color").unwrap();
    if let SettingValue::Choice { options, .. } = &accent_setting.value {
        if options.len() != 8 {
            return false;
        }
    } else {
        return false;
    }
    if !app.cycle_choice("personalization.appearance.accent_color") {
        return false;
    }
    let updated_accent = app.backend.get_setting("personalization.appearance.accent_color").unwrap();
    if let SettingValue::Choice { selected, .. } = &updated_accent.value {
        if *selected != 1 {
            return false;
        }
    }

    // 7. Verify Apps Panel & Startup Service IPC
    app.select_category(PanelCategory::Apps);
    let svc_resp = app.backend.dispatch_novad_ipc(NovadCommand::ListStartupServices);
    if let NovadResponse::ServiceList(svcs) = svc_resp {
        if svcs.len() < 4 {
            return false;
        }
    } else {
        return false;
    }
    // Toggle startup service
    app.backend.dispatch_novad_ipc(NovadCommand::ToggleStartupService {
        name: "nova-search-indexer".into(),
        enabled: false,
    });
    if app.backend.startup_services.iter().find(|s| s.name == "nova-search-indexer").unwrap().enabled {
        return false;
    }

    // 8. Verify Accounts Panel & User Management
    app.select_category(PanelCategory::Accounts);
    let user_count_before = app.backend.users.len();
    app.backend.dispatch_novad_ipc(NovadCommand::AddUser {
        username: "testuser".into(),
        full_name: "Test User".into(),
        is_admin: false,
    });
    if app.backend.users.len() != user_count_before + 1 {
        return false;
    }
    // Elevate admin
    app.backend.dispatch_novad_ipc(NovadCommand::SetUserAdmin {
        username: "testuser".into(),
        is_admin: true,
    });
    let test_user = app.backend.users.iter().find(|u| u.username == "testuser").unwrap();
    if !test_user.is_admin || !test_user.groups.contains(&"wheel".to_string()) {
        return false;
    }

    // 9. Verify System Updates Panel & Dual-Slot A/B Engine
    app.select_category(PanelCategory::Updates);
    if app.backend.update_state.active_slot != BootSlot::SlotA {
        return false;
    }
    // Apply update -> transitions to Slot B Testing
    app.backend.dispatch_novad_ipc(NovadCommand::ApplyUpdate);
    if app.backend.update_state.active_slot != BootSlot::SlotB
        || app.backend.update_state.slot_health != SlotHealth::Testing
    {
        return false;
    }
    // Confirm healthy
    app.backend.dispatch_novad_ipc(NovadCommand::ConfirmSlotHealthy);
    if app.backend.update_state.slot_health != SlotHealth::Healthy {
        return false;
    }
    // Rollback
    app.backend.dispatch_novad_ipc(NovadCommand::RollbackSlot);
    if app.backend.update_state.active_slot != BootSlot::SlotA
        || app.backend.update_state.slot_health != SlotHealth::RolledBack
    {
        return false;
    }

    // 10. Verify BufferSurface Graphical Rendering & Hit Testing
    let mut surface = BufferSurface::new(960, 640);
    app.render(&mut surface);

    // Verify non-zero pixels rendered across sidebar and content
    let non_zero_pixels = surface
        .pixels
        .iter()
        .filter(|&&p| p != 0)
        .count();
    if non_zero_pixels < 10000 {
        return false;
    }

    // Verify sidebar category hit testing
    let hit_network = app.handle_click(Point::new(100, 175)); // Network category button
    if !hit_network || app.active_category != PanelCategory::Network {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_settings_selftest() {
        assert!(run_nova_settings_selftest(), "All nova-settings self-tests pass");
    }

    #[test]
    fn test_category_metadata() {
        for cat in PanelCategory::all() {
            assert!(!cat.id().is_empty());
            assert!(!cat.title().is_empty());
            assert!(!cat.icon().is_empty());
            assert!(!cat.summary().is_empty());
        }
    }

    #[test]
    fn test_search_engine_ranking_and_tokens() {
        let app = NovaSettingsApp::new();
        // Single token search
        let results = app.search_engine.search("audio");
        assert!(!results.is_empty(), "Should find audio settings");
        assert_eq!(results[0].category, PanelCategory::System);

        // Multi-token search: "display scale"
        let scale_results = app.search_engine.search("display scale");
        assert!(!scale_results.is_empty(), "Should find display scaling");
        assert_eq!(scale_results[0].item_key, "system.display.scaling");

        // Keyword search: "4k"
        let k4_results = app.search_engine.search("4k");
        assert!(!k4_results.is_empty(), "Should find 4K resolution options");
    }

    #[test]
    fn test_sysfs_synchronization() {
        let mut app = NovaSettingsApp::new();
        assert_eq!(app.backend.sysfs.read("/sys/class/drm/card0/mode").unwrap(), "1920x1080@60");
        let mut val = app.backend.get_setting("system.display.resolution").unwrap().value.clone();
        val.set_choice(2); // Select 3840x2160 (4K UHD)
        app.backend.set_setting("system.display.resolution", val).unwrap();
        assert_eq!(app.backend.sysfs.read("/sys/class/drm/card0/mode").unwrap(), "3840x2160@60");

        // Test power profile sysfs sync
        let mut pwr = app.backend.get_setting("system.power.power_mode").unwrap().value.clone();
        pwr.set_choice(1); // Best Performance
        app.backend.set_setting("system.power.power_mode", pwr).unwrap();
        assert_eq!(app.backend.sysfs.read("/sys/class/power/profile").unwrap(), "performance");
    }

    #[test]
    fn test_novad_ipc_user_management() {
        let mut app = NovaSettingsApp::new();
        let initial_count = app.backend.users.len();
        app.backend.dispatch_novad_ipc(NovadCommand::AddUser {
            username: "guest".into(),
            full_name: "Guest User".into(),
            is_admin: false,
        });
        assert_eq!(app.backend.users.len(), initial_count + 1);
        assert!(app.backend.users.iter().any(|u| u.username == "guest"));

        // Elevate to admin
        app.backend.dispatch_novad_ipc(NovadCommand::SetUserAdmin {
            username: "guest".into(),
            is_admin: true,
        });
        let guest = app.backend.users.iter().find(|u| u.username == "guest").unwrap();
        assert!(guest.is_admin);
        assert!(guest.groups.contains(&"wheel".to_string()));
    }

    #[test]
    fn test_novad_ipc_startup_services() {
        let mut app = NovaSettingsApp::new();
        let res = app.backend.dispatch_novad_ipc(NovadCommand::ListStartupServices);
        if let NovadResponse::ServiceList(svcs) = res {
            assert!(svcs.iter().any(|s| s.name == "nova-compositor"));
            assert!(svcs.iter().any(|s| s.name == "nova-shell"));
        } else {
            panic!("Expected ServiceList response");
        }

        app.backend.dispatch_novad_ipc(NovadCommand::ToggleStartupService {
            name: "nova-audio".into(),
            enabled: false,
        });
        let audio_svc = app.backend.startup_services.iter().find(|s| s.name == "nova-audio").unwrap();
        assert!(!audio_svc.enabled);
    }

    #[test]
    fn test_dual_slot_ab_update_engine() {
        let mut app = NovaSettingsApp::new();
        assert_eq!(app.backend.update_state.active_slot, BootSlot::SlotA);
        assert_eq!(app.backend.update_state.slot_health, SlotHealth::Healthy);

        // Apply update -> transitions to Slot B Testing
        app.backend.dispatch_novad_ipc(NovadCommand::ApplyUpdate);
        assert_eq!(app.backend.update_state.active_slot, BootSlot::SlotB);
        assert_eq!(app.backend.update_state.slot_health, SlotHealth::Testing);
        assert_eq!(app.backend.update_state.download_progress_pct, 100);

        // Confirm healthy
        app.backend.dispatch_novad_ipc(NovadCommand::ConfirmSlotHealthy);
        assert_eq!(app.backend.update_state.slot_health, SlotHealth::Healthy);

        // Rollback
        app.backend.dispatch_novad_ipc(NovadCommand::RollbackSlot);
        assert_eq!(app.backend.update_state.active_slot, BootSlot::SlotA);
        assert_eq!(app.backend.update_state.slot_health, SlotHealth::RolledBack);
    }

    #[test]
    fn test_controls_manipulation() {
        let mut app = NovaSettingsApp::new();
        // Slider
        assert!(app.set_slider_ratio("system.sound.master_volume", 0.42));
        assert_eq!(
            app.backend.get_setting("system.sound.master_volume").unwrap().value,
            SettingValue::Int(42)
        );

        // Toggle
        let initial_wifi = app.backend.get_setting("network.wifi.enabled").unwrap().value.as_bool().unwrap();
        assert!(app.toggle_setting("network.wifi.enabled"));
        assert_eq!(
            app.backend.get_setting("network.wifi.enabled").unwrap().value.as_bool().unwrap(),
            !initial_wifi
        );

        // Choice cycle
        let initial_choice = app.backend.get_setting("personalization.wallpaper.preset").unwrap().value.choice_str().unwrap().to_string();
        assert!(app.cycle_choice("personalization.wallpaper.preset"));
        let next_choice = app.backend.get_setting("personalization.wallpaper.preset").unwrap().value.choice_str().unwrap().to_string();
        assert_ne!(initial_choice, next_choice);
    }

    #[test]
    fn test_deep_link_search_navigation() {
        let mut app = NovaSettingsApp::new();
        app.select_category(PanelCategory::System);

        // Search for proxy in Network
        app.set_search_query("proxy");
        assert!(!app.search_results.is_empty());
        assert_eq!(app.search_results[0].category, PanelCategory::Network);

        // Navigate via search result
        assert!(app.select_search_result(0));
        assert_eq!(app.active_category, PanelCategory::Network);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn test_view_rasterization_and_hit_testing() {
        let mut app = NovaSettingsApp::new();
        let mut surf = BufferSurface::new(960, 640);
        app.render(&mut surf);

        let non_zero = surf.pixels.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 15000, "Should rasterize UI elements across surface");

        // Hit test category button (Network is item 1, y ~ 170)
        let clicked = app.handle_click(Point::new(100, 175));
        assert!(clicked);
        assert_eq!(app.active_category, PanelCategory::Network);
    }
}
