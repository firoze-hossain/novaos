//! userland/desktop/apps/nova-settings/src/backend.rs
//!
//! System Configuration Backend interfacing with NovaOS `sysfs` (/sys/)
//! and `novad` system daemon IPC protocol.
//! Provides unified queries and mutations for hardware, services, users, and updates.

#[cfg(not(test))]
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::types::{PanelCategory, SettingItem, SettingValue};

/// Mock/Virtual sysfs key-value file node.
#[derive(Clone, Debug, PartialEq)]
pub struct SysfsNode {
    pub path: String,
    pub content: String,
    pub readable: bool,
    pub writable: bool,
}

/// Sysfs interface abstraction.
#[derive(Clone, Debug)]
pub struct SysfsInterface {
    nodes: BTreeMap<String, SysfsNode>,
}

impl Default for SysfsInterface {
    fn default() -> Self {
        Self::new()
    }
}

impl SysfsInterface {
    pub fn new() -> Self {
        let mut iface = Self {
            nodes: BTreeMap::new(),
        };
        // Populate standard NovaOS sysfs nodes
        iface.register("/sys/class/drm/card0/mode", "1920x1080@60", true, true);
        iface.register("/sys/class/drm/card0/scaling", "100%", true, true);
        iface.register("/sys/class/sound/pcm0p/volume", "75", true, true);
        iface.register("/sys/class/sound/pcm0p/mute", "0", true, true);
        iface.register("/sys/class/power_supply/battery/capacity", "88", true, false);
        iface.register("/sys/class/power_supply/battery/status", "Discharging", true, false);
        iface.register("/sys/class/power/profile", "balanced", true, true);
        iface.register("/sys/class/net/eth0/operstate", "up", true, false);
        iface.register("/sys/class/net/eth0/speed", "1000", true, false);
        iface.register("/sys/class/net/eth0/address", "52:54:00:12:34:56", true, false);
        iface.register("/sys/class/net/wlan0/operstate", "up", true, true);
        iface.register("/sys/class/net/wlan0/ssid", "Nova_Corporate_5G", true, false);
        iface
    }

    pub fn register(&mut self, path: &str, initial: &str, readable: bool, writable: bool) {
        self.nodes.insert(
            path.to_string(),
            SysfsNode {
                path: path.to_string(),
                content: initial.to_string(),
                readable,
                writable,
            },
        );
    }

    pub fn read(&self, path: &str) -> Option<String> {
        self.nodes.get(path).and_then(|n| {
            if n.readable {
                Some(n.content.clone())
            } else {
                None
            }
        })
    }

    pub fn write(&mut self, path: &str, val: &str) -> bool {
        if let Some(node) = self.nodes.get_mut(path) {
            if node.writable {
                node.content = val.to_string();
                return true;
            }
        }
        false
    }
}

/// User account record managed via novad IPC.
#[derive(Clone, Debug, PartialEq)]
pub struct UserProfile {
    pub uid: u32,
    pub username: String,
    pub full_name: String,
    pub is_admin: bool,
    pub password_hash: String,
    pub groups: Vec<String>,
}

/// Dual-boot slot status for A/B updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootSlot {
    SlotA,
    SlotB,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotHealth {
    Healthy,
    Testing,
    RolledBack,
}

/// Update metadata report from novad update daemon.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateMetadata {
    pub current_version: String,
    pub available_version: Option<String>,
    pub release_notes: String,
    pub active_slot: BootSlot,
    pub slot_health: SlotHealth,
    pub download_progress_pct: u8,
}

/// Supervised daemon service entry.
#[derive(Clone, Debug, PartialEq)]
pub struct StartupServiceEntry {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub impact: &'static str,
}

/// IPC commands delivered to `novad` system supervisor.
#[derive(Clone, Debug, PartialEq)]
pub enum NovadCommand {
    GetConfig { key: String },
    SetConfig { key: String, value: SettingValue },
    ListUsers,
    AddUser { username: String, full_name: String, is_admin: bool },
    UpdatePassword { username: String, new_hash: String },
    SetUserAdmin { username: String, is_admin: bool },
    CheckUpdate,
    ApplyUpdate,
    RollbackSlot,
    ConfirmSlotHealthy,
    ListStartupServices,
    ToggleStartupService { name: String, enabled: bool },
}

/// IPC response payload from `novad`.
#[derive(Clone, Debug, PartialEq)]
pub enum NovadResponse {
    Ack,
    ConfigValue(Option<SettingValue>),
    UserList(Vec<UserProfile>),
    UpdateInfo(UpdateMetadata),
    ServiceList(Vec<StartupServiceEntry>),
    Error(String),
}

/// Combined system configuration backend interfacing sysfs and novad.
#[derive(Clone, Debug)]
pub struct SystemBackend {
    pub sysfs: SysfsInterface,
    pub settings_store: BTreeMap<String, SettingItem>,
    pub users: Vec<UserProfile>,
    pub update_state: UpdateMetadata,
    pub startup_services: Vec<StartupServiceEntry>,
    pub mutation_log: Vec<String>,
}

impl Default for SystemBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemBackend {
    pub fn new() -> Self {
        let mut backend = Self {
            sysfs: SysfsInterface::new(),
            settings_store: BTreeMap::new(),
            users: vec![
                UserProfile {
                    uid: 1000,
                    username: "admin".to_string(),
                    full_name: "Nova Administrator".to_string(),
                    is_admin: true,
                    password_hash: "sha256$pbkdf2$e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
                    groups: vec!["wheel".into(), "audio".into(), "video".into(), "netdev".into()],
                },
                UserProfile {
                    uid: 1001,
                    username: "developer".to_string(),
                    full_name: "Core Developer".to_string(),
                    is_admin: false,
                    password_hash: "sha256$pbkdf2$5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8".to_string(),
                    groups: vec!["developer".into(), "audio".into(), "video".into()],
                },
            ],
            update_state: UpdateMetadata {
                current_version: "2.0.0".to_string(),
                available_version: Some("2.1.0 LTS".to_string()),
                release_notes: "Includes hardware DRM acceleration updates, high-throughput NVMe DMA, and Fish-shell integration.".to_string(),
                active_slot: BootSlot::SlotA,
                slot_health: SlotHealth::Healthy,
                download_progress_pct: 0,
            },
            startup_services: vec![
                StartupServiceEntry {
                    name: "novad-session".to_string(),
                    description: "System Session Coordinator".to_string(),
                    enabled: true,
                    impact: "Low",
                },
                StartupServiceEntry {
                    name: "nova-compositor".to_string(),
                    description: "Wayland Display Server".to_string(),
                    enabled: true,
                    impact: "High",
                },
                StartupServiceEntry {
                    name: "nova-shell".to_string(),
                    description: "Taskbar, Menu & Dock".to_string(),
                    enabled: true,
                    impact: "Medium",
                },
                StartupServiceEntry {
                    name: "nova-audio".to_string(),
                    description: "HDA Sound Server".to_string(),
                    enabled: true,
                    impact: "Low",
                },
                StartupServiceEntry {
                    name: "nova-search-indexer".to_string(),
                    description: "Instant Search Background Indexer".to_string(),
                    enabled: true,
                    impact: "Low",
                },
            ],
            mutation_log: Vec::new(),
        };

        backend.populate_default_settings();
        backend
    }

    /// Registers standard system settings across all 6 categories.
    fn populate_default_settings(&mut self) {
        // --- 1. SYSTEM ---
        self.register_setting(SettingItem::new(
            "system.display.resolution",
            PanelCategory::System,
            "Display",
            "Display Resolution",
            "Select display pixel dimensions for the primary monitor",
            &["resolution", "screen", "display", "monitor", "1080p", "4k", "pixels"],
            SettingValue::Choice {
                selected: 0,
                options: vec![
                    "1920x1080 (Recommended)".into(),
                    "2560x1440 (2K QHD)".into(),
                    "3840x2160 (4K UHD)".into(),
                    "1024x768 (Legacy SVGA)".into(),
                ],
            },
        ));

        self.register_setting(SettingItem::new(
            "system.display.refresh_rate",
            PanelCategory::System,
            "Display",
            "Refresh Rate",
            "Set the monitor refresh frequency in Hertz",
            &["refresh", "hertz", "hz", "60hz", "120hz", "144hz", "vblank"],
            SettingValue::Choice {
                selected: 0,
                options: vec!["60 Hz".into(), "120 Hz".into(), "144 Hz".into()],
            },
        ));

        self.register_setting(SettingItem::new(
            "system.display.scaling",
            PanelCategory::System,
            "Display",
            "Display Scaling",
            "Adjust text and UI scaling factor for high-DPI displays",
            &["scale", "scaling", "dpi", "zoom", "100%", "125%", "150%", "200%"],
            SettingValue::Choice {
                selected: 0,
                options: vec!["100% (Standard)".into(), "125%".into(), "150%".into(), "200% (Retina/4K)".into()],
            },
        ));

        self.register_setting(SettingItem::new(
            "system.display.night_light",
            PanelCategory::System,
            "Display",
            "Night Light",
            "Reduce blue light emissions to reduce eye strain in the evening",
            &["night light", "blue light", "warmth", "color temperature", "eyes"],
            SettingValue::Bool(false),
        ));

        self.register_setting(SettingItem::new(
            "system.sound.output_device",
            PanelCategory::System,
            "Sound",
            "Audio Output Device",
            "Select primary sound reproduction hardware interface",
            &["sound", "audio", "speakers", "headphones", "output", "volume"],
            SettingValue::Choice {
                selected: 0,
                options: vec![
                    "Speakers (Intel HDA Analog)".into(),
                    "Headphones (Front Panel Jack)".into(),
                    "Digital Output (HDMI/DP Audio)".into(),
                ],
            },
        ));

        self.register_setting(SettingItem::new(
            "system.sound.master_volume",
            PanelCategory::System,
            "Sound",
            "Master Volume",
            "Adjust output volume level from 0 to 100 percent",
            &["volume", "sound", "loudness", "level", "audio"],
            SettingValue::Int(75),
        ));

        self.register_setting(SettingItem::new(
            "system.sound.mute",
            PanelCategory::System,
            "Sound",
            "Mute Audio",
            "Silences all audio output channels",
            &["mute", "silent", "sound", "quiet"],
            SettingValue::Bool(false),
        ));

        self.register_setting(SettingItem::new(
            "system.power.power_mode",
            PanelCategory::System,
            "Power & Battery",
            "Power Mode",
            "Balance energy efficiency against maximum computational performance",
            &["power", "battery", "performance", "energy", "sleep", "balanced"],
            SettingValue::Choice {
                selected: 0,
                options: vec![
                    "Balanced (Recommended)".into(),
                    "Best Performance".into(),
                    "Power Saver".into(),
                ],
            },
        ));

        self.register_setting(SettingItem::new(
            "system.power.screen_sleep_mins",
            PanelCategory::System,
            "Power & Battery",
            "Turn Off Screen After",
            "Minutes of inactivity before turning off display backlight",
            &["sleep", "timeout", "screen", "inactivity", "idle"],
            SettingValue::Choice {
                selected: 1,
                options: vec!["2 minutes".into(), "5 minutes".into(), "15 minutes".into(), "Never".into()],
            },
        ));

        // --- 2. NETWORK ---
        self.register_setting(SettingItem::new(
            "network.wifi.enabled",
            PanelCategory::Network,
            "Wi-Fi",
            "Wi-Fi Radio",
            "Turn wireless networking adapter on or off",
            &["wifi", "wireless", "wlan", "network", "internet", "radio"],
            SettingValue::Bool(true),
        ));

        self.register_setting(SettingItem::new(
            "network.wifi.current_ssid",
            PanelCategory::Network,
            "Wi-Fi",
            "Connected Wi-Fi SSID",
            "Currently connected 802.11 wireless network access point",
            &["ssid", "wifi", "network", "hotspot", "wlan"],
            SettingValue::Text("Nova_Corporate_5G".into()),
        ));

        self.register_setting(SettingItem::new(
            "network.ethernet.dhcp",
            PanelCategory::Network,
            "Ethernet",
            "IP Assignment Mode",
            "Configure IPv4 address allocation protocol via DHCP or Static",
            &["ethernet", "ip", "dhcp", "static", "address", "lan"],
            SettingValue::Choice {
                selected: 0,
                options: vec!["Automatic (DHCP)".into(), "Manual (Static IP)".into()],
            },
        ));

        self.register_setting(SettingItem::new(
            "network.ethernet.ip_address",
            PanelCategory::Network,
            "Ethernet",
            "IPv4 Address",
            "Static IP address assigned to physical adapter eth0",
            &["ip", "ipv4", "ethernet", "address", "subnet"],
            SettingValue::Text("192.168.1.150".into()),
        ));

        self.register_setting(SettingItem::new(
            "network.vpn.enabled",
            PanelCategory::Network,
            "VPN",
            "Virtual Private Network",
            "Encrypted WireGuard or OpenVPN network tunnel",
            &["vpn", "wireguard", "openvpn", "tunnel", "security", "privacy"],
            SettingValue::Bool(false),
        ));

        self.register_setting(SettingItem::new(
            "network.proxy.enabled",
            PanelCategory::Network,
            "Proxy",
            "Manual Proxy Server",
            "Route HTTP/HTTPS traffic through a secure intermediary proxy",
            &["proxy", "http", "socks", "pac", "gateway", "firewall"],
            SettingValue::Bool(false),
        ));

        // --- 3. PERSONALIZATION ---
        self.register_setting(SettingItem::new(
            "personalization.appearance.dark_mode",
            PanelCategory::Personalization,
            "Appearance",
            "Dark / Light Mode",
            "Select dark mica theme or light crisp theme for windows and menus",
            &["dark mode", "light mode", "theme", "appearance", "mica", "fluent"],
            SettingValue::Choice {
                selected: 0,
                options: vec!["Dark Theme".into(), "Light Theme".into(), "Auto (Scheduled)".into()],
            },
        ));

        self.register_setting(SettingItem::new(
            "personalization.appearance.accent_color",
            PanelCategory::Personalization,
            "Colors",
            "Accent Color",
            "Highlight color applied to toggles, focus rings, and dock badges",
            &["accent", "color", "blue", "indigo", "green", "coral", "palette"],
            SettingValue::Choice {
                selected: 0,
                options: vec![
                    "Fluent Blue (#0078D4)".into(),
                    "Electric Indigo (#5E5CE6)".into(),
                    "Emerald Green (#30D158)".into(),
                    "Coral Peach (#FF6B6B)".into(),
                    "Crimson Sunset (#FF375F)".into(),
                    "Amethyst Purple (#BF5AF2)".into(),
                    "Cyber Mint (#64D2FF)".into(),
                    "Amber Gold (#FF9F0A)".into(),
                ],
            },
        ));

        self.register_setting(SettingItem::new(
            "personalization.wallpaper.preset",
            PanelCategory::Personalization,
            "Wallpaper",
            "Desktop Wallpaper",
            "Select desktop background imagery",
            &["wallpaper", "background", "desktop", "image", "art"],
            SettingValue::Choice {
                selected: 0,
                options: vec![
                    "Nova Dusk (Default)".into(),
                    "Aurora Borealis".into(),
                    "Cosmic Wave".into(),
                    "Deep Space".into(),
                    "Abstract Flow".into(),
                ],
            },
        ));

        self.register_setting(SettingItem::new(
            "personalization.effects.transparency",
            PanelCategory::Personalization,
            "Effects",
            "Acrylic Glassmorphism Transparency",
            "Enable smooth Gaussian blur and mica backdrop material effects",
            &["transparency", "acrylic", "blur", "mica", "glassmorphism"],
            SettingValue::Bool(true),
        ));

        // --- 4. APPS ---
        self.register_setting(SettingItem::new(
            "apps.defaults.browser",
            PanelCategory::Apps,
            "Default Applications",
            "Default Web Browser",
            "Primary application handling HTTP, HTTPS, and HTML documents",
            &["browser", "default", "web", "html", "internet"],
            SettingValue::Choice {
                selected: 0,
                options: vec!["Nova Browser (nova-browse)".into(), "Chromium Engine".into()],
            },
        ));

        self.register_setting(SettingItem::new(
            "apps.defaults.terminal",
            PanelCategory::Apps,
            "Default Applications",
            "Default Terminal Emulator",
            "Command line terminal emulator spawned for scripts and shortcuts",
            &["terminal", "console", "pty", "shell", "nova-term"],
            SettingValue::Choice {
                selected: 0,
                options: vec!["Nova Terminal (nova-term)".into(), "Alacritty VT100".into()],
            },
        ));

        self.register_setting(SettingItem::new(
            "apps.defaults.editor",
            PanelCategory::Apps,
            "Default Applications",
            "Default Code / Text Editor",
            "Application opened for plain text and source code files",
            &["editor", "text", "code", "edit", "nova-edit", "vim"],
            SettingValue::Choice {
                selected: 0,
                options: vec!["Nova Edit (nova-edit)".into(), "Nano CLI".into(), "Vim".into()],
            },
        ));

        // --- 5. ACCOUNTS ---
        self.register_setting(SettingItem::new(
            "accounts.user.current",
            PanelCategory::Accounts,
            "User Info",
            "Current User Account",
            "Active logged-in user profile identity",
            &["user", "account", "profile", "admin", "login", "identity"],
            SettingValue::Text("admin (Nova Administrator)".into()),
        ));

        self.register_setting(SettingItem::new(
            "accounts.security.require_password_wake",
            PanelCategory::Accounts,
            "Security",
            "Require Password on Wake",
            "Prompt for account authentication when resuming from sleep or lock",
            &["password", "wake", "lock", "security", "auth"],
            SettingValue::Bool(true),
        ));

        self.register_setting(SettingItem::new(
            "accounts.privileges.sudo_enabled",
            PanelCategory::Accounts,
            "Privileges",
            "Wheel / Sudo Root Elevation",
            "Allow user to elevate privileges to execute system administration tasks",
            &["sudo", "wheel", "admin", "privileges", "root", "elevation"],
            SettingValue::Bool(true),
        ));

        // --- 6. UPDATES ---
        self.register_setting(SettingItem::new(
            "updates.status.active_slot",
            PanelCategory::Updates,
            "A/B Dual-Slot",
            "Active Boot Slot",
            "Physical partition slot currently booted and verified healthy",
            &["slot", "boot", "update", "recovery", "a/b", "kernel"],
            SettingValue::Text("Slot A (Active & Verified)".into()),
        ));

        self.register_setting(SettingItem::new(
            "updates.channel.release_channel",
            PanelCategory::Updates,
            "Update Channel",
            "Release Channel",
            "Select update stream for operating system packages and firmware",
            &["channel", "update", "stable", "beta", "canary"],
            SettingValue::Choice {
                selected: 0,
                options: vec!["Stable LTS".into(), "Beta Preview".into(), "Canary Daily".into()],
            },
        ));
    }

    pub fn register_setting(&mut self, item: SettingItem) {
        self.settings_store.insert(item.key.clone(), item);
    }

    pub fn get_setting(&self, key: &str) -> Option<&SettingItem> {
        self.settings_store.get(key)
    }

    pub fn get_setting_mut(&mut self, key: &str) -> Option<&mut SettingItem> {
        self.settings_store.get_mut(key)
    }

    /// Mutates a setting value, propagates to sysfs/IPC, and logs mutation.
    pub fn set_setting(&mut self, key: &str, new_val: SettingValue) -> Result<(), &'static str> {
        if let Some(item) = self.settings_store.get_mut(key) {
            item.value = new_val.clone();
            self.mutation_log.push(format!("MOD: {} -> {:?}", key, new_val));

            // Sync hardware-bound settings directly with sysfs
            match key {
                "system.display.resolution" => {
                    if let Some(s) = new_val.choice_str() {
                        let mode = if s.contains("1920x1080") {
                            "1920x1080@60"
                        } else if s.contains("2560x1440") {
                            "2560x1440@60"
                        } else if s.contains("3840x2160") {
                            "3840x2160@60"
                        } else {
                            "1024x768@60"
                        };
                        self.sysfs.write("/sys/class/drm/card0/mode", mode);
                    }
                }
                "system.display.scaling" => {
                    if let Some(s) = new_val.choice_str() {
                        let scale = if s.contains("200%") {
                            "200%"
                        } else if s.contains("150%") {
                            "150%"
                        } else if s.contains("125%") {
                            "125%"
                        } else {
                            "100%"
                        };
                        self.sysfs.write("/sys/class/drm/card0/scaling", scale);
                    }
                }
                "system.sound.master_volume" => {
                    if let Some(vol) = new_val.as_int() {
                        self.sysfs.write("/sys/class/sound/pcm0p/volume", &format!("{}", vol));
                    }
                }
                "system.sound.mute" => {
                    if let Some(m) = new_val.as_bool() {
                        self.sysfs.write("/sys/class/sound/pcm0p/mute", if m { "1" } else { "0" });
                    }
                }
                "system.power.power_mode" => {
                    if let Some(s) = new_val.choice_str() {
                        let prof = if s.contains("Performance") {
                            "performance"
                        } else if s.contains("Power Saver") {
                            "power-saver"
                        } else {
                            "balanced"
                        };
                        self.sysfs.write("/sys/class/power/profile", prof);
                    }
                }
                "network.wifi.enabled" => {
                    if let Some(en) = new_val.as_bool() {
                        self.sysfs.write("/sys/class/net/wlan0/operstate", if en { "up" } else { "down" });
                    }
                }
                _ => {}
            }

            Ok(())
        } else {
            Err("Setting key not found")
        }
    }

    /// Dispatches an IPC command to `novad`.
    pub fn dispatch_novad_ipc(&mut self, cmd: NovadCommand) -> NovadResponse {
        match cmd {
            NovadCommand::GetConfig { key } => {
                let val = self.settings_store.get(&key).map(|i| i.value.clone());
                NovadResponse::ConfigValue(val)
            }
            NovadCommand::SetConfig { key, value } => {
                if self.set_setting(&key, value).is_ok() {
                    NovadResponse::Ack
                } else {
                    NovadResponse::Error("Key not found".into())
                }
            }
            NovadCommand::ListUsers => NovadResponse::UserList(self.users.clone()),
            NovadCommand::AddUser { username, full_name, is_admin } => {
                let next_uid = self.users.iter().map(|u| u.uid).max().unwrap_or(1000) + 1;
                self.users.push(UserProfile {
                    uid: next_uid,
                    username,
                    full_name,
                    is_admin,
                    password_hash: "sha256$pbkdf2$default_salt_hash".into(),
                    groups: if is_admin { vec!["wheel".into()] } else { vec![] },
                });
                NovadResponse::Ack
            }
            NovadCommand::UpdatePassword { username, new_hash } => {
                if let Some(u) = self.users.iter_mut().find(|u| u.username == username) {
                    u.password_hash = new_hash;
                    NovadResponse::Ack
                } else {
                    NovadResponse::Error("User not found".into())
                }
            }
            NovadCommand::SetUserAdmin { username, is_admin } => {
                if let Some(u) = self.users.iter_mut().find(|u| u.username == username) {
                    u.is_admin = is_admin;
                    if is_admin && !u.groups.contains(&"wheel".to_string()) {
                        u.groups.push("wheel".to_string());
                    } else if !is_admin {
                        u.groups.retain(|g| g != "wheel");
                    }
                    NovadResponse::Ack
                } else {
                    NovadResponse::Error("User not found".into())
                }
            }
            NovadCommand::CheckUpdate => {
                NovadResponse::UpdateInfo(self.update_state.clone())
            }
            NovadCommand::ApplyUpdate => {
                self.update_state.download_progress_pct = 100;
                self.update_state.active_slot = BootSlot::SlotB;
                self.update_state.slot_health = SlotHealth::Testing;
                self.update_state.current_version = "2.1.0 LTS".to_string();
                self.update_state.available_version = None;
                self.set_setting(
                    "updates.status.active_slot",
                    SettingValue::Text("Slot B (Testing - Rebooting)".into()),
                ).ok();
                NovadResponse::UpdateInfo(self.update_state.clone())
            }
            NovadCommand::ConfirmSlotHealthy => {
                self.update_state.slot_health = SlotHealth::Healthy;
                self.set_setting(
                    "updates.status.active_slot",
                    SettingValue::Text("Slot B (Active & Verified Healthy)".into()),
                ).ok();
                NovadResponse::Ack
            }
            NovadCommand::RollbackSlot => {
                self.update_state.active_slot = BootSlot::SlotA;
                self.update_state.slot_health = SlotHealth::RolledBack;
                self.update_state.current_version = "2.0.0".to_string();
                self.set_setting(
                    "updates.status.active_slot",
                    SettingValue::Text("Slot A (Rolled Back - Safe)".into()),
                ).ok();
                NovadResponse::Ack
            }
            NovadCommand::ListStartupServices => {
                NovadResponse::ServiceList(self.startup_services.clone())
            }
            NovadCommand::ToggleStartupService { name, enabled } => {
                if let Some(svc) = self.startup_services.iter_mut().find(|s| s.name == name) {
                    svc.enabled = enabled;
                    NovadResponse::Ack
                } else {
                    NovadResponse::Error("Service not found".into())
                }
            }
        }
    }
}
