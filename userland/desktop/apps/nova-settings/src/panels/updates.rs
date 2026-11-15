//! userland/desktop/apps/nova-settings/src/panels/updates.rs
//!
//! Updates Panel: Dual-slot A/B updates, OS versioning, and rollback recovery.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

use super::SettingsPanel;
use crate::backend::{NovadCommand, SystemBackend};
use crate::types::{PanelCategory, SettingItem, SettingsAction};

pub struct UpdatesPanel {
    subgroups: &'static [&'static str],
}

impl Default for UpdatesPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdatesPanel {
    pub fn new() -> Self {
        Self {
            subgroups: &["System Information", "A/B Dual-Slot Engine", "Update Channel", "Update Actions"],
        }
    }
}

impl SettingsPanel for UpdatesPanel {
    fn category(&self) -> PanelCategory {
        PanelCategory::Updates
    }

    fn title(&self) -> &'static str {
        "System Updates"
    }

    fn subgroups(&self) -> &'static [&'static str] {
        self.subgroups
    }

    fn items(&self, backend: &SystemBackend) -> Vec<SettingItem> {
        backend
            .settings_store
            .values()
            .filter(|i| i.category == PanelCategory::Updates)
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
                if key.starts_with("updates.") {
                    backend.set_setting(key, new_value.clone())?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            SettingsAction::TriggerAction(act) => {
                match act.as_str() {
                    "check_updates" => {
                        backend.dispatch_novad_ipc(NovadCommand::CheckUpdate);
                        Ok(true)
                    }
                    "apply_update" => {
                        backend.dispatch_novad_ipc(NovadCommand::ApplyUpdate);
                        Ok(true)
                    }
                    "confirm_healthy" => {
                        backend.dispatch_novad_ipc(NovadCommand::ConfirmSlotHealthy);
                        Ok(true)
                    }
                    "rollback_slot" => {
                        backend.dispatch_novad_ipc(NovadCommand::RollbackSlot);
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            _ => Ok(false),
        }
    }
}
