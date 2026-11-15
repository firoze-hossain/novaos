//! userland/desktop/apps/nova-settings/src/panels/apps.rs
//!
//! Apps Panel: Default applications, Startup apps, and app permissions.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

use super::SettingsPanel;
use crate::backend::{NovadCommand, SystemBackend};
use crate::types::{PanelCategory, SettingItem, SettingsAction};

pub struct AppsPanel {
    subgroups: &'static [&'static str],
}

impl Default for AppsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl AppsPanel {
    pub fn new() -> Self {
        Self {
            subgroups: &["Default Applications", "Startup Apps", "App Permissions & Sandbox"],
        }
    }
}

impl SettingsPanel for AppsPanel {
    fn category(&self) -> PanelCategory {
        PanelCategory::Apps
    }

    fn title(&self) -> &'static str {
        "Apps"
    }

    fn subgroups(&self) -> &'static [&'static str] {
        self.subgroups
    }

    fn items(&self, backend: &SystemBackend) -> Vec<SettingItem> {
        backend
            .settings_store
            .values()
            .filter(|i| i.category == PanelCategory::Apps)
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
                if key.starts_with("apps.") {
                    backend.set_setting(key, new_value.clone())?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            SettingsAction::TriggerAction(act) => {
                if let Some(service_name) = act.strip_prefix("toggle_startup:") {
                    let is_enabled = backend
                        .startup_services
                        .iter()
                        .find(|s| s.name == service_name)
                        .map(|s| !s.enabled)
                        .unwrap_or(true);
                    backend.dispatch_novad_ipc(NovadCommand::ToggleStartupService {
                        name: service_name.into(),
                        enabled: is_enabled,
                    });
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            _ => Ok(false),
        }
    }
}
