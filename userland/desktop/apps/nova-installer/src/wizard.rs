//! userland/desktop/apps/nova-installer/src/wizard.rs
//!
//! Multi-Step Graphical Installer State Machine & Wizard Controller for NovaOS.
//!
//! 9 Intuitive Setup Steps:
//!   0. Welcome & Architecture Overview
//!   1. Pre-Flight Hardware Checks
//!   2. Drive Partitioning Wizard (Guided Auto vs Custom Manual GPT)
//!   3. Timezone & Regional Localization
//!   4. User Account & Security Configuration
//!   5. Desktop Personalization & Live Preview
//!   6. Summary Blueprint & Confirmation
//!   7. Live Installation Progress & Animated Slide Showcase
//!   8. Setup Complete & Reboot Prompt

use crate::account::{PasswordStrength, UserAccountConfig};
use crate::engine::{InstallPipelineEngine, InstallStage};
use crate::partition_wizard::PartitionWizard;
use crate::preview::DesktopThemeConfig;
use crate::showcase::SlideShowcase;
use crate::timezone::TimezoneDatabase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Welcome = 0,
    PreFlightChecks = 1,
    Partitioning = 2,
    Timezone = 3,
    UserAccount = 4,
    DesktopPreview = 5,
    SummaryConfirm = 6,
    Installing = 7,
    Complete = 8,
}

impl WizardStep {
    pub fn title(&self) -> &'static str {
        match self {
            WizardStep::Welcome => "Welcome to NovaOS 2.0",
            WizardStep::PreFlightChecks => "Hardware Qualification",
            WizardStep::Partitioning => "Storage & Partitioning",
            WizardStep::Timezone => "Timezone & Localization",
            WizardStep::UserAccount => "User Account Setup",
            WizardStep::DesktopPreview => "Desktop Personalization",
            WizardStep::SummaryConfirm => "Installation Blueprint",
            WizardStep::Installing => "Installing NovaOS...",
            WizardStep::Complete => "Installation Complete",
        }
    }

    pub fn step_number(&self) -> usize {
        *self as usize + 1
    }

    pub fn total_steps() -> usize {
        9
    }
}

pub struct PreFlightStatus {
    pub cpu_cores: usize,
    pub ram_mb: usize,
    pub is_uefi_64bit: bool,
    pub has_storage: bool,
    pub power_connected: bool,
}

impl PreFlightStatus {
    pub fn detect() -> Self {
        Self {
            cpu_cores: 4,
            ram_mb: 8192,
            is_uefi_64bit: true,
            has_storage: true,
            power_connected: true,
        }
    }

    pub fn all_passed(&self) -> bool {
        self.cpu_cores >= 1 && self.ram_mb >= 2048 && self.is_uefi_64bit && self.has_storage
    }
}

pub struct InstallerWizardApp {
    pub current_step: WizardStep,
    pub preflight: PreFlightStatus,
    pub partition_wizard: PartitionWizard,
    pub timezone_db: TimezoneDatabase,
    pub account: UserAccountConfig,
    pub theme: DesktopThemeConfig,
    pub showcase: SlideShowcase,
    pub engine: InstallPipelineEngine,
    pub user_password_input: String,
    pub user_password_confirm: String,
    pub confirm_format_authorized: bool,
    pub error_banner: Option<String>,
}

impl InstallerWizardApp {
    pub fn new() -> Self {
        Self {
            current_step: WizardStep::Welcome,
            preflight: PreFlightStatus::detect(),
            partition_wizard: PartitionWizard::new(),
            timezone_db: TimezoneDatabase::new(),
            account: UserAccountConfig::new(),
            theme: DesktopThemeConfig::new(),
            showcase: SlideShowcase::new(),
            engine: InstallPipelineEngine::new(None, false),
            user_password_input: String::new(),
            user_password_confirm: String::new(),
            confirm_format_authorized: false,
            error_banner: None,
        }
    }

    /// Checks if user can advance to the next step
    pub fn can_advance(&self) -> bool {
        match self.current_step {
            WizardStep::Welcome => true,
            WizardStep::PreFlightChecks => self.preflight.all_passed(),
            WizardStep::Partitioning => self.partition_wizard.validate_layout().is_ok(),
            WizardStep::Timezone => true,
            WizardStep::UserAccount => {
                let u_ok = UserAccountConfig::validate_username(&self.account.username).is_ok();
                let h_ok = UserAccountConfig::validate_hostname(&self.account.hostname).is_ok();
                let p_ok = !self.user_password_input.is_empty()
                    && self.user_password_input == self.user_password_confirm
                    && UserAccountConfig::evaluate_password_strength(&self.user_password_input) != PasswordStrength::VeryWeak;
                u_ok && h_ok && p_ok
            }
            WizardStep::DesktopPreview => true,
            WizardStep::SummaryConfirm => self.confirm_format_authorized,
            WizardStep::Installing => self.engine.current_stage == InstallStage::Complete,
            WizardStep::Complete => false,
        }
    }

    /// Advances to the next step if permitted
    pub fn next_step(&mut self) -> bool {
        if !self.can_advance() {
            return false;
        }

        self.error_banner = None;

        match self.current_step {
            WizardStep::Welcome => {
                self.current_step = WizardStep::PreFlightChecks;
            }
            WizardStep::PreFlightChecks => {
                self.current_step = WizardStep::Partitioning;
            }
            WizardStep::Partitioning => {
                self.current_step = WizardStep::Timezone;
            }
            WizardStep::Timezone => {
                self.current_step = WizardStep::UserAccount;
            }
            WizardStep::UserAccount => {
                // Finalize account password
                self.account.set_password(&self.user_password_input);
                self.current_step = WizardStep::DesktopPreview;
            }
            WizardStep::DesktopPreview => {
                self.current_step = WizardStep::SummaryConfirm;
            }
            WizardStep::SummaryConfirm => {
                self.current_step = WizardStep::Installing;
                self.start_installation_pipeline();
            }
            WizardStep::Installing => {
                if self.engine.current_stage == InstallStage::Complete {
                    self.current_step = WizardStep::Complete;
                }
            }
            WizardStep::Complete => {}
        }
        true
    }

    /// Moves back to the previous step
    pub fn prev_step(&mut self) -> bool {
        self.error_banner = None;
        match self.current_step {
            WizardStep::Welcome => false,
            WizardStep::PreFlightChecks => {
                self.current_step = WizardStep::Welcome;
                true
            }
            WizardStep::Partitioning => {
                self.current_step = WizardStep::PreFlightChecks;
                true
            }
            WizardStep::Timezone => {
                self.current_step = WizardStep::Partitioning;
                true
            }
            WizardStep::UserAccount => {
                self.current_step = WizardStep::Timezone;
                true
            }
            WizardStep::DesktopPreview => {
                self.current_step = WizardStep::UserAccount;
                true
            }
            WizardStep::SummaryConfirm => {
                self.current_step = WizardStep::DesktopPreview;
                true
            }
            WizardStep::Installing | WizardStep::Complete => false, // Cannot go back while installing
        }
    }

    /// Starts the end-to-end installation engine
    pub fn start_installation_pipeline(&mut self) {
        let disk = match self.partition_wizard.selected_disk() {
            Some(d) => d.clone(),
            None => {
                self.error_banner = Some("No target disk selected".to_string());
                return;
            }
        };

        let partitions = self.partition_wizard.partitions.clone();
        let user = self.account.clone();
        let tz = self.timezone_db.selected().clone();
        let theme = self.theme.clone();

        match self.engine.run_installation(&disk, &partitions, &user, &tz, &theme) {
            Ok(()) => {
                self.current_step = WizardStep::Complete;
            }
            Err(e) => {
                self.error_banner = Some(format!("Installation failed: {}", e));
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_state_transitions() {
        let mut app = InstallerWizardApp::new();
        assert_eq!(app.current_step, WizardStep::Welcome);

        // Step 0 -> Step 1
        assert!(app.can_advance());
        assert!(app.next_step());
        assert_eq!(app.current_step, WizardStep::PreFlightChecks);

        // Step 1 -> Step 2
        assert!(app.can_advance());
        assert!(app.next_step());
        assert_eq!(app.current_step, WizardStep::Partitioning);

        // Step 2 -> Step 3
        assert!(app.can_advance());
        assert!(app.next_step());
        assert_eq!(app.current_step, WizardStep::Timezone);

        // Step 3 -> Step 4
        assert!(app.can_advance());
        assert!(app.next_step());
        assert_eq!(app.current_step, WizardStep::UserAccount);

        // Step 4: requires matching passwords
        assert!(!app.can_advance());
        app.user_password_input = "AdminPassword@2026".to_string();
        app.user_password_confirm = "AdminPassword@2026".to_string();
        assert!(app.can_advance());
        assert!(app.next_step());
        assert_eq!(app.current_step, WizardStep::DesktopPreview);

        // Step 5 -> Step 6
        assert!(app.can_advance());
        assert!(app.next_step());
        assert_eq!(app.current_step, WizardStep::SummaryConfirm);

        // Step 6: requires authorization checkbox
        assert!(!app.can_advance());
        app.confirm_format_authorized = true;
        assert!(app.can_advance());
        assert!(app.next_step());

        // Step 7 -> Complete
        assert_eq!(app.current_step, WizardStep::Complete);
    }
}
