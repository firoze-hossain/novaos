//! userland/desktop/apps/nova-settings/src/panels/accounts.rs
//!
//! Accounts Panel: Multi-user accounts, Passwords, Privileges, and groups.

#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

use super::SettingsPanel;
use crate::backend::{NovadCommand, SystemBackend};
use crate::types::{PanelCategory, SettingItem, SettingsAction};

pub struct AccountsPanel {
    subgroups: &'static [&'static str],
}

impl Default for AccountsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountsPanel {
    pub fn new() -> Self {
        Self {
            subgroups: &["Your Info", "Multi-User Accounts", "Sign-in Options", "Privileges & Groups"],
        }
    }
}

impl SettingsPanel for AccountsPanel {
    fn category(&self) -> PanelCategory {
        PanelCategory::Accounts
    }

    fn title(&self) -> &'static str {
        "Accounts"
    }

    fn subgroups(&self) -> &'static [&'static str] {
        self.subgroups
    }

    fn items(&self, backend: &SystemBackend) -> Vec<SettingItem> {
        backend
            .settings_store
            .values()
            .filter(|i| i.category == PanelCategory::Accounts)
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
                if key.starts_with("accounts.") {
                    backend.set_setting(key, new_value.clone())?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            SettingsAction::TriggerAction(act) => {
                if let Some(params) = act.strip_prefix("add_user:") {
                    // params = username:fullname:is_admin
                    let parts: Vec<&str> = params.split(':').collect();
                    if parts.len() >= 3 {
                        let username = String::from(parts[0]);
                        let fullname = String::from(parts[1]);
                        let is_admin = parts[2] == "true" || parts[2] == "1";
                        backend.dispatch_novad_ipc(NovadCommand::AddUser {
                            username,
                            full_name: fullname,
                            is_admin,
                        });
                        return Ok(true);
                    }
                } else if let Some(params) = act.strip_prefix("update_password:") {
                    // params = username:new_hash
                    let parts: Vec<&str> = params.split(':').collect();
                    if parts.len() >= 2 {
                        let username = String::from(parts[0]);
                        let new_hash = String::from(parts[1]);
                        backend.dispatch_novad_ipc(NovadCommand::UpdatePassword {
                            username,
                            new_hash,
                        });
                        return Ok(true);
                    }
                } else if let Some(params) = act.strip_prefix("set_admin:") {
                    // params = username:is_admin
                    let parts: Vec<&str> = params.split(':').collect();
                    if parts.len() >= 2 {
                        let username = String::from(parts[0]);
                        let is_admin = parts[1] == "true" || parts[1] == "1";
                        backend.dispatch_novad_ipc(NovadCommand::SetUserAdmin {
                            username,
                            is_admin,
                        });
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }
}
