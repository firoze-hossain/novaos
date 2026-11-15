//! userland/desktop/apps/nova-settings/src/panels/system.rs
//!
//! System Panel: Display resolution/scaling, Sound output/input, Power & Sleep.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

use super::SettingsPanel;
use crate::backend::SystemBackend;
use crate::types::{PanelCategory, SettingItem, SettingsAction};

pub struct SystemPanel {
    subgroups: &'static [&'static str],
}

impl Default for SystemPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemPanel {
    pub fn new() -> Self {
        Self {
            subgroups: &["Display", "Sound", "Power & Battery", "About System"],
        }
    }
}

impl SettingsPanel for SystemPanel {
    fn category(&self) -> PanelCategory {
        PanelCategory::System
    }

    fn title(&self) -> &'static str {
        "System"
    }

    fn subgroups(&self) -> &'static [&'static str] {
        self.subgroups
    }

    fn items(&self, backend: &SystemBackend) -> Vec<SettingItem> {
        backend
            .settings_store
            .values()
            .filter(|i| i.category == PanelCategory::System)
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
                if key.starts_with("system.") {
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
