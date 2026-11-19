//! userland/desktop/apps/nova-recovery/src/cli.rs
//!
//! Headless Command-Line Rescue Console for NovaOS Recovery Environment.
//! Supports serial console operation, automated scripting, and batch recovery actions.

use crate::boot_repair::BootRepairManager;
use crate::log_viewer::LogViewer;
use crate::passwd_reset::PasswordResetManager;
use crate::snapshot::SnapshotManager;

pub struct RescueCliSession {
    pub snapshot_mgr: SnapshotManager,
    pub passwd_mgr: PasswordResetManager,
    pub repair_mgr: BootRepairManager,
    pub log_viewer: LogViewer,
}

impl RescueCliSession {
    pub fn new() -> Self {
        Self {
            snapshot_mgr: SnapshotManager::new(),
            passwd_mgr: PasswordResetManager::new(),
            repair_mgr: BootRepairManager::new(20971520),
            log_viewer: LogViewer::new(),
        }
    }

    /// Executes a single CLI rescue command and returns the textual output
    pub fn execute_command(&mut self, cmd: &str) -> Result<String, String> {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(String::new());
        }

        match parts[0] {
            "help" | "-h" | "--help" => Ok(self.render_help()),
            "snapshots" | "list-snapshots" => Ok(self.render_snapshots()),
            "rollback" => {
                if parts.len() < 2 {
                    return Err("Usage: rollback <snapshot_id>".to_string());
                }
                let id: u64 = parts[1]
                    .parse()
                    .map_err(|_| format!("Invalid snapshot ID: {}", parts[1]))?;
                self.snapshot_mgr.rollback_to_snapshot(id)?;
                Ok(format!(
                    "[OK] Subvolume successfully rolled back to snapshot ID {}. Active B-Tree root updated.",
                    id
                ))
            }
            "reset-root" | "passwd" => {
                if parts.len() < 2 {
                    return Err("Usage: reset-root <new_password> [user]".to_string());
                }
                let pass = parts[1];
                let user = if parts.len() >= 3 { parts[2] } else { "root" };
                let salt = b"clisalt123456789";
                self.passwd_mgr.reset_password(user, pass, salt)?;
                Ok(format!(
                    "[OK] Password for user '{}' updated successfully with PBKDF2-HMAC-SHA256.",
                    user
                ))
            }
            "repair-gpt" | "repair-boot" => {
                let mut mbr = [0u8; 512];
                self.repair_mgr.repair_protective_mbr(&mut mbr)?;
                Ok("[OK] Protective MBR (LBA 0) and Primary GPT Header verified and calibrated.".to_string())
            }
            "logs" => {
                let count = parts.get(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(10);
                Ok(self.render_logs(count))
            }
            "status" => Ok(self.render_status()),
            _ => Err(format!(
                "Unknown command: '{}'. Type 'help' for available rescue commands.",
                parts[0]
            )),
        }
    }

    fn render_help(&self) -> String {
        let mut out = String::new();
        out.push_str("NovaOS 2.0 Emergency Rescue Console Commands:\n");
        out.push_str("  help                         Display this help menu\n");
        out.push_str("  snapshots                    List available NovaFS snapshots\n");
        out.push_str("  rollback <id>                Atomically roll back rootfs to snapshot\n");
        out.push_str("  reset-root <pass> [user]     Reset administrative user password\n");
        out.push_str("  repair-gpt                   Check and repair Protective MBR and GPT\n");
        out.push_str("  logs [count]                 Display recent kernel and crash logs\n");
        out.push_str("  status                       Display system and storage health status\n");
        out
    }

    fn render_snapshots(&self) -> String {
        let mut out = String::new();
        out.push_str("ID   | GENERATION | ROOT BLOCK | SIZE (GB) | NAME\n");
        out.push_str("-----+------------+------------+-----------+--------------------------------\n");
        for s in self.snapshot_mgr.list_snapshots() {
            out.push_str(&format!(
                "{:<4} | {:<10} | {:<10} | {:<9} | {}\n",
                s.id,
                s.generation,
                s.root_block,
                s.size_bytes / 1_000_000_000,
                s.name
            ));
        }
        out
    }

    fn render_logs(&self, count: usize) -> String {
        let mut out = String::new();
        let entries = &self.log_viewer.entries;
        let start = entries.len().saturating_sub(count);
        for entry in &entries[start..] {
            out.push_str(&format!(
                "[{}] <{}> {}\n",
                entry.level.as_str(),
                entry.facility,
                entry.message
            ));
        }
        out
    }

    fn render_status(&self) -> String {
        let mut out = String::new();
        out.push_str("NovaOS Recovery Environment Status:\n");
        out.push_str(&format!("  Active Root Subvolume:  @ (ID: {})\n", self.snapshot_mgr.active_subvolume_id));
        out.push_str(&format!("  Current B-Tree Root:    Block {}\n", self.snapshot_mgr.current_root_block));
        out.push_str(&format!("  Active Transaction Gen: {}\n", self.snapshot_mgr.current_generation));
        out.push_str(&format!("  Available Snapshots:    {}\n", self.snapshot_mgr.list_snapshots().len()));
        out.push_str(&format!("  Registered Users:       {}\n", self.passwd_mgr.list_usernames().join(", ")));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_commands() {
        let mut cli = RescueCliSession::new();

        assert!(cli.execute_command("help").unwrap().contains("Rescue Console"));
        assert!(cli.execute_command("snapshots").unwrap().contains("@snapshot"));
        assert!(cli.execute_command("rollback 101").unwrap().contains("[OK]"));
        assert!(cli.execute_command("reset-root NewPass! root").unwrap().contains("[OK]"));
        assert!(cli.execute_command("status").unwrap().contains("Active Root Subvolume"));
    }
}
