//! userland/desktop/apps/nova-settings/src/panels/mod.rs
//!
//! Pluggable Settings Panels interface for NovaOS Unified System Settings.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

pub mod system;
pub mod network;
pub mod personalization;
pub mod apps;
pub mod accounts;
pub mod updates;
pub mod display;

pub use system::SystemPanel;
pub use network::NetworkPanel;
pub use personalization::PersonalizationPanel;
pub use apps::AppsPanel;
pub use accounts::AccountsPanel;
pub use updates::UpdatesPanel;
pub use display::DisplayPanel;

use crate::backend::SystemBackend;
use crate::types::{PanelCategory, SettingItem, SettingsAction};

/// Common trait implemented by all settings category panels.
pub trait SettingsPanel {
    /// Category identifier.
    fn category(&self) -> PanelCategory;

    /// Panel display title.
    fn title(&self) -> &'static str;

    /// List of subgroups/cards in this panel.
    fn subgroups(&self) -> &'static [&'static str];

    /// Collects setting items dynamically from backend.
    fn items(&self, backend: &SystemBackend) -> Vec<SettingItem>;

    /// Handles a user action or setting mutation.
    fn handle_action(
        &mut self,
        action: &SettingsAction,
        backend: &mut SystemBackend,
    ) -> Result<bool, &'static str>;
}
