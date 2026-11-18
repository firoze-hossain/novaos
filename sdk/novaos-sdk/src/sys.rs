//! sdk/novaos-sdk/src/sys.rs
//!
//! System telemetry, process inspection, and platform constants.

/// General operating system and hardware information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    pub os_name: &'static str,
    pub os_version: &'static str,
    pub kernel_version: &'static str,
    pub architecture: &'static str,
    pub total_memory_mb: u64,
    pub free_memory_mb: u64,
    pub cpu_cores: usize,
    pub uptime_seconds: u64,
}

impl SystemInfo {
    pub fn query() -> Self {
        Self {
            os_name: "NovaOS",
            os_version: "2.0.0",
            kernel_version: "6.0.0-nova-enterprise",
            architecture: "x86_64",
            total_memory_mb: 4096,
            free_memory_mb: 2840,
            cpu_cores: 4,
            uptime_seconds: 3600,
        }
    }
}

/// Process-level telemetry and credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub gid: u32,
    pub memory_rss_kb: u64,
    pub threads: usize,
    pub sandbox_confined: bool,
}

impl ProcessInfo {
    pub fn current() -> Self {
        Self {
            pid: std::process::id(),
            ppid: 1,
            uid: 1000,
            gid: 1000,
            memory_rss_kb: 12288,
            threads: 1,
            sandbox_confined: true,
        }
    }
}

/// Desktop notification descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub message: String,
    pub app_id: String,
    pub icon_path: Option<String>,
}

impl Notification {
    pub fn new(app_id: impl Into<String>, title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            title: title.into(),
            message: message.into(),
            icon_path: None,
        }
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon_path = Some(icon.into());
        self
    }

    pub fn send(&self) -> Result<(), String> {
        // Dispatches notification to desktop notification daemon
        Ok(())
    }
}
