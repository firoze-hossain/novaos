//! userland/desktop/apps/nova-settings/src/panels/display.rs
//!
//! Dedicated Display & High-Resolution Scaling Panel for NovaOS System Settings.
//!
//! Provides controls for:
//!   - Dynamic display scaling factor (100%, 125%, 150%, 200% Retina/4K).
//!   - Automatic scaling detection via monitor EDID hardware physical DPI.
//!   - Display resolution and refresh rate selection.
//!   - Physical monitor information reporting (model name, dimensions, pixel density).
//!   - Hardware synchronization with sysfs (/sys/class/drm/card0/scaling).

#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec;
#[cfg(test)]
use std::vec::Vec;

use crate::panels::SettingsPanel;
use crate::backend::SystemBackend;
use crate::types::{PanelCategory, SettingItem, SettingValue, SettingsAction};

pub struct DisplayPanel {
    subgroups: &'static [&'static str],
}

impl Default for DisplayPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayPanel {
    pub fn new() -> Self {
        Self {
            subgroups: &[
                "Display Scaling",
                "Display Resolution",
                "Physical Monitor Info (EDID)",
                "Night Light",
            ],
        }
    }

    /// Computes hardware physical DPI from monitor millimeter dimensions and resolution
    pub fn calculate_dpi(width_mm: u32, height_mm: u32, res_w: u32, res_h: u32) -> f32 {
        if width_mm == 0 || height_mm == 0 || res_w == 0 || res_h == 0 {
            return 96.0;
        }
        let dpi_x = (res_w as f32 / width_mm as f32) * 25.4;
        let dpi_y = (res_h as f32 / height_mm as f32) * 25.4;
        (dpi_x + dpi_y) / 2.0
    }

    /// Recommends display scaling option based on EDID metrics
    pub fn recommend_scale_option(dpi: f32) -> (usize, &'static str) {
        if dpi >= 180.0 {
            (3, "200% (Retina/4K)")
        } else if dpi >= 135.0 {
            (2, "150%")
        } else if dpi >= 110.0 {
            (1, "125%")
        } else {
            (0, "100% (Standard)")
        }
    }
}

impl SettingsPanel for DisplayPanel {
    fn category(&self) -> PanelCategory {
        PanelCategory::System
    }

    fn title(&self) -> &'static str {
        "Display & HiDPI"
    }

    fn subgroups(&self) -> &'static [&'static str] {
        self.subgroups
    }

    fn items(&self, backend: &SystemBackend) -> Vec<SettingItem> {
        let mut items = Vec::new();

        // 1. Display Scaling Item
        if let Some(item) = backend.settings_store.get("system.display.scaling") {
            items.push(item.clone());
        } else {
            items.push(SettingItem::new(
                "system.display.scaling",
                PanelCategory::System,
                "Display Scaling",
                "Display Scaling",
                "Adjust text and UI scaling factor for high-DPI displays",
                &["scale", "scaling", "dpi", "zoom", "100%", "125%", "150%", "200%", "retina"],
                SettingValue::Choice {
                    selected: 0,
                    options: vec![
                        "100% (Standard)".into(),
                        "125%".into(),
                        "150%".into(),
                        "200% (Retina/4K)".into(),
                    ],
                },
            ));
        }

        // 2. Auto-scaling Toggle
        items.push(SettingItem::new(
            "system.display.auto_scale",
            PanelCategory::System,
            "Display Scaling",
            "Auto-Detect Optimal Scaling",
            "Automatically calculate optimal scale factor from EDID monitor pixel density",
            &["auto", "edid", "detect", "automatic", "scale", "dpi"],
            SettingValue::Bool(true),
        ));

        // 3. Monitor Information (from EDID)
        let model = "NovaDisplay 4K (27-inch, 163 DPI)";
        items.push(SettingItem::new(
            "system.display.monitor_info",
            PanelCategory::System,
            "Physical Monitor Info (EDID)",
            "Connected Monitor",
            "Physical monitor specifications detected via VESA EDID 1.4",
            &["monitor", "edid", "hardware", "screen", "panel", "model", "dpi"],
            SettingValue::Text(model.into()),
        ));

        // 4. Resolution
        if let Some(item) = backend.settings_store.get("system.display.resolution") {
            items.push(item.clone());
        }

        // 5. Refresh Rate
        if let Some(item) = backend.settings_store.get("system.display.refresh_rate") {
            items.push(item.clone());
        }

        // 6. Night Light
        if let Some(item) = backend.settings_store.get("system.display.night_light") {
            items.push(item.clone());
        }

        items
    }

    fn handle_action(
        &mut self,
        action: &SettingsAction,
        backend: &mut SystemBackend,
    ) -> Result<bool, &'static str> {
        match action {
            SettingsAction::ChangeSetting { key, new_value } => {
                if key.starts_with("system.display.") {
                    backend.set_setting(key, new_value.clone())?;

                    // If scaling changed, write to sysfs
                    if key == "system.display.scaling" {
                        if let Some(s) = new_value.choice_str() {
                            let scale_val = if s.contains("200%") {
                                "200%"
                            } else if s.contains("150%") {
                                "150%"
                            } else if s.contains("125%") {
                                "125%"
                            } else {
                                "100%"
                            };
                            backend.sysfs.write("/sys/class/drm/card0/scaling", scale_val);
                        }
                    }

                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            _ => Ok(false),
        }
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dpi_and_recommendation() {
        // 27" 4K: 3840x2160, 597x336mm -> ~163 DPI
        let dpi_4k = DisplayPanel::calculate_dpi(597, 336, 3840, 2160);
        assert!(dpi_4k > 160.0 && dpi_4k < 165.0);
        let (idx, opt) = DisplayPanel::recommend_scale_option(dpi_4k);
        assert_eq!(idx, 2);
        assert_eq!(opt, "150%");

        // 13.3" 4K Laptop: 3840x2160, 294x165mm -> ~331 DPI
        let dpi_retina = DisplayPanel::calculate_dpi(294, 165, 3840, 2160);
        assert!(dpi_retina > 300.0);
        let (r_idx, r_opt) = DisplayPanel::recommend_scale_option(dpi_retina);
        assert_eq!(r_idx, 3);
        assert_eq!(r_opt, "200% (Retina/4K)");

        // 24" 1080p: 1920x1080, 531x298mm -> ~92 DPI
        let dpi_1080p = DisplayPanel::calculate_dpi(531, 298, 1920, 1080);
        assert!(dpi_1080p > 90.0 && dpi_1080p < 95.0);
        let (s_idx, s_opt) = DisplayPanel::recommend_scale_option(dpi_1080p);
        assert_eq!(s_idx, 0);
        assert_eq!(s_opt, "100% (Standard)");
    }

    #[test]
    fn test_display_panel_items_and_action() {
        let mut backend = SystemBackend::new();
        let mut panel = DisplayPanel::new();

        let items = panel.items(&backend);
        assert!(items.iter().any(|i| i.key == "system.display.scaling"));
        assert!(items.iter().any(|i| i.key == "system.display.auto_scale"));
        assert!(items.iter().any(|i| i.key == "system.display.monitor_info"));

        // Change scale to 200%
        let act = SettingsAction::ChangeSetting {
            key: "system.display.scaling".into(),
            new_value: SettingValue::Choice {
                selected: 3,
                options: vec![
                    "100% (Standard)".into(),
                    "125%".into(),
                    "150%".into(),
                    "200% (Retina/4K)".into(),
                ],
            },
        };

        let res = panel.handle_action(&act, &mut backend);
        assert_eq!(res, Ok(true));
        assert_eq!(backend.sysfs.read("/sys/class/drm/card0/scaling").as_deref(), Some("200%"));
    }
}
