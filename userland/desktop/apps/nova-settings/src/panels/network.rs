//! userland/desktop/apps/nova-settings/src/panels/network.rs
//!
//! Network Panel: Wi-Fi, Ethernet, VPN, and Proxy configuration.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

use super::SettingsPanel;
use crate::backend::SystemBackend;
use crate::types::{PanelCategory, SettingItem, SettingsAction};

pub struct NetworkPanel {
    subgroups: &'static [&'static str],
}

impl Default for NetworkPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkPanel {
    pub fn new() -> Self {
        Self {
            subgroups: &["Wi-Fi", "Ethernet", "VPN", "Proxy"],
        }
    }
}

impl SettingsPanel for NetworkPanel {
    fn category(&self) -> PanelCategory {
        PanelCategory::Network
    }

    fn title(&self) -> &'static str {
        "Network & Internet"
    }

    fn subgroups(&self) -> &'static [&'static str] {
        self.subgroups
    }

    fn items(&self, backend: &SystemBackend) -> Vec<SettingItem> {
        backend
            .settings_store
            .values()
            .filter(|i| i.category == PanelCategory::Network)
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
                if key.starts_with("network.") {
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
