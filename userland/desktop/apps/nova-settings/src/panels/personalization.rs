//! userland/desktop/apps/nova-settings/src/panels/personalization.rs
//!
//! Personalization Panel: Wallpapers, Accent colors, Dark/Light mode, and visual effects.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

use super::SettingsPanel;
use crate::backend::SystemBackend;
use crate::types::{PanelCategory, SettingItem, SettingsAction};

pub struct PersonalizationPanel {
    subgroups: &'static [&'static str],
}

impl Default for PersonalizationPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl PersonalizationPanel {
    pub fn new() -> Self {
        Self {
            subgroups: &["Appearance", "Colors", "Wallpaper", "Effects & Scaling"],
        }
    }
}

impl SettingsPanel for PersonalizationPanel {
    fn category(&self) -> PanelCategory {
        PanelCategory::Personalization
    }

    fn title(&self) -> &'static str {
        "Personalization"
    }

    fn subgroups(&self) -> &'static [&'static str] {
        self.subgroups
    }

    fn items(&self, backend: &SystemBackend) -> Vec<SettingItem> {
        backend
            .settings_store
            .values()
            .filter(|i| i.category == PanelCategory::Personalization)
            .cloned()
            .collect()
    }

    fn handle_action(
        &mut self,
        action: &SettingsAction,
        backend: &mut SystemBackend,
    ) -> Result<bool, &'static str> {
        match action {
            SettingsAction::ChangeSetting { key, new_value } => {
                if key.starts_with("personalization.") {
                    backend.set_setting(key, new_value.clone())?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            _ => Ok(false),
        }
    }
}
