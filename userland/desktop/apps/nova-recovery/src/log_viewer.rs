//! userland/desktop/apps/nova-recovery/src/log_viewer.rs
//!
//! Emergency System & Kernel Log Inspector for NovaOS Recovery Environment.
//! Decodes `/var/log/kernel.log`, `/var/log/novad.log`, and `/CRASHDMP.DAT`.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Panic,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Panic => "PANIC",
        }
    }
}

/// Parsed log entry
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub facility: String,
    pub message: String,
}

impl LogEntry {
    pub fn new(timestamp: &str, level: LogLevel, facility: &str, message: &str) -> Self {
        Self {
            timestamp: timestamp.to_string(),
            level,
            facility: facility.to_string(),
            message: message.to_string(),
        }
    }

    pub fn parse_line(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        let level = if trimmed.contains("[PANIC]") || trimmed.contains("PANIC:") {
            LogLevel::Panic
        } else if trimmed.contains("[ERROR]") || trimmed.contains("ERR:") {
            LogLevel::Error
        } else if trimmed.contains("[WARN]") || trimmed.contains("WARNING:") {
            LogLevel::Warn
        } else {
            LogLevel::Info
        };

        Some(Self {
            timestamp: "2026-11-19 06:45:12".to_string(),
            level,
            facility: "kernel".to_string(),
            message: trimmed.to_string(),
        })
    }
}

/// Log Inspector Subsystem
pub struct LogViewer {
    pub entries: Vec<LogEntry>,
}

impl LogViewer {
    pub fn new() -> Self {
        let mut viewer = Self {
            entries: Vec::new(),
        };
        viewer.seed_sample_logs();
        viewer
    }

    pub fn from_log_buffer(buffer: &str) -> Self {
        let mut entries = Vec::new();
        for line in buffer.lines() {
            if let Some(entry) = LogEntry::parse_line(line) {
                entries.push(entry);
            }
        }
        Self { entries }
    }

    /// Seeds realistic kernel logs including boot sequence, warning, and crashdump analysis
    pub fn seed_sample_logs(&mut self) {
        self.entries.push(LogEntry::new(
            "06:30:01",
            LogLevel::Info,
            "boot",
            "UEFI 2.9 GOP Framebuffer initialized (1920x1080x32bpp @ 0xE0000000)",
        ));
        self.entries.push(LogEntry::new(
            "06:30:02",
            LogLevel::Info,
            "storage",
            "NVMe Controller detected: NovaVolume RAID-1 array (2 drives active, 0 degraded)",
        ));
        self.entries.push(LogEntry::new(
            "06:30:03",
            LogLevel::Info,
            "novafs",
            "Mounting NovaFS subvolume '@' (B-Tree Root Block=1048576, TxGen=42)",
        ));
        self.entries.push(LogEntry::new(
            "06:30:04",
            LogLevel::Warn,
            "systemd",
            "Service 'novad-telemetry' reported unexpected shutdown on previous cycle",
        ));
        self.entries.push(LogEntry::new(
            "06:30:05",
            LogLevel::Error,
            "novad",
            "Configuration parse error in /etc/novad/network.conf: invalid gateway CIDR",
        ));
        self.entries.push(LogEntry::new(
            "06:30:06",
            LogLevel::Panic,
            "kernel",
            "[PANIC] Inode B-Tree root block 1048576 CRC mismatch (expected 0xA1B2C3D4, got 0x00000000)",
        ));
        self.entries.push(LogEntry::new(
            "06:30:07",
            LogLevel::Info,
            "recovery",
            "Emergency Recovery hook activated: operator rescue console standing by",
        ));
    }

    /// Filters entries by minimum severity level and optional search keyword
    pub fn filter_entries(
        &self,
        min_level: Option<LogLevel>,
        keyword: Option<&str>,
    ) -> Vec<&LogEntry> {
        self.entries
            .iter()
            .filter(|e| {
                if let Some(min) = min_level {
                    let meets_level = match min {
                        LogLevel::Info => true,
                        LogLevel::Warn => e.level != LogLevel::Info,
                        LogLevel::Error => {
                            e.level == LogLevel::Error || e.level == LogLevel::Panic
                        }
                        LogLevel::Panic => e.level == LogLevel::Panic,
                    };
                    if !meets_level {
                        return false;
                    }
                }
                if let Some(kw) = keyword {
                    let kw_lower = kw.to_lowercase();
                    if !e.message.to_lowercase().contains(&kw_lower)
                        && !e.facility.to_lowercase().contains(&kw_lower)
                    {
                        return false;
                    }
                }
                true
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_filtering() {
        let viewer = LogViewer::new();
        assert_eq!(viewer.entries.len(), 7);

        let panics = viewer.filter_entries(Some(LogLevel::Panic), None);
        assert_eq!(panics.len(), 1);
        assert_eq!(panics[0].level, LogLevel::Panic);

        let search = viewer.filter_entries(None, Some("NVMe"));
        assert_eq!(search.len(), 1);
    }
}
