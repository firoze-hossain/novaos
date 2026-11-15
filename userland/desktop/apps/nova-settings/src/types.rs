//! userland/desktop/apps/nova-settings/src/types.rs
//!
//! Core data types, enums, settings values, actions, and search representations
//! for the NovaOS Unified System Settings & Control Hub.

#[cfg(not(test))]
use alloc::{string::{String, ToString}, vec::Vec};

#[cfg(test)]
use std::{string::{String, ToString}, vec::Vec};

/// The 6 primary categories synthesizing Windows 11 Settings and macOS System Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelCategory {
    System,
    Network,
    Personalization,
    Apps,
    Accounts,
    Updates,
}

impl PanelCategory {
    pub const fn id(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Network => "network",
            Self::Personalization => "personalization",
            Self::Apps => "apps",
            Self::Accounts => "accounts",
            Self::Updates => "updates",
        }
    }

    pub const fn title(&self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Network => "Network & Internet",
            Self::Personalization => "Personalization",
            Self::Apps => "Apps",
            Self::Accounts => "Accounts",
            Self::Updates => "System Updates",
        }
    }

    pub const fn icon(&self) -> &'static str {
        match self {
            Self::System => "💻",
            Self::Network => "🌐",
            Self::Personalization => "🎨",
            Self::Apps => "📱",
            Self::Accounts => "👤",
            Self::Updates => "🔄",
        }
    }

    pub const fn summary(&self) -> &'static str {
        match self {
            Self::System => "Display, sound, power, sleep, and device specifications",
            Self::Network => "Wi-Fi, Ethernet, VPN, proxy, and network adapter settings",
            Self::Personalization => "Wallpaper, dark mode, accent colors, and UI scaling",
            Self::Apps => "Default applications, startup apps, and system permissions",
            Self::Accounts => "Your account, multi-user management, passwords, and privileges",
            Self::Updates => "NovaOS updates, dual-slot A/B status, and system recovery",
        }
    }

    pub const fn all() -> &'static [PanelCategory] {
        &[
            Self::System,
            Self::Network,
            Self::Personalization,
            Self::Apps,
            Self::Accounts,
            Self::Updates,
        ]
    }
}

/// Generic value container for settings toggles, sliders, text inputs, and choices.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Float(f32),
    Text(String),
    Choice {
        selected: usize,
        options: Vec<String>,
    },
    ColorRgb(u8, u8, u8),
}

impl SettingValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f32> {
        match self {
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn choice_str(&self) -> Option<&str> {
        match self {
            Self::Choice { selected, options } => {
                options.get(*selected).map(|s| s.as_str())
            }
            _ => None,
        }
    }

    pub fn set_choice(&mut self, idx: usize) -> bool {
        match self {
            Self::Choice { selected, options } => {
                if idx < options.len() {
                    *selected = idx;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

/// A discrete setting toggle, slider, picker, or status row with search metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingItem {
    pub key: String,
    pub category: PanelCategory,
    pub subgroup: String,
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub value: SettingValue,
    pub enabled: bool,
}

impl SettingItem {
    pub fn new(
        key: &str,
        category: PanelCategory,
        subgroup: &str,
        title: &str,
        description: &str,
        keywords: &[&str],
        value: SettingValue,
    ) -> Self {
        Self {
            key: key.to_string(),
            category,
            subgroup: subgroup.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            value,
            enabled: true,
        }
    }
}

/// Actions dispatched by UI events or IPC commands.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsAction {
    SelectCategory(PanelCategory),
    ChangeSetting {
        key: String,
        new_value: SettingValue,
    },
    SearchQueryChanged(String),
    SelectSearchResult(usize),
    TriggerAction(String),
}

/// Search result representation for quick navigation.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchResult {
    pub item_key: String,
    pub category: PanelCategory,
    pub subgroup: String,
    pub title: String,
    pub description: String,
    pub breadcrumb: String,
    pub score: u32,
}
