//! userland/server/novad/src/unit.rs
//!
//! Declarative Systemd-Style Unit Parser & Representation for novad:
//! - Unit types: .service, .socket, .target
//! - INI-syntax parsing: [Unit], [Service], [Socket], [Install]
//! - Directives: Requires=, Wants=, BindsTo=, Conflicts=, Before=, After=
//! - Service types: simple, forking, oneshot, notify
//! - Restart policies: no, always, on-success, on-failure, on-abnormal, on-watchdog
//! - Health check watchdog configurations & backoff parameters

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitType {
    Service,
    Socket,
    Target,
}

impl UnitType {
    pub fn from_name(name: &str) -> Option<Self> {
        if name.ends_with(".service") {
            Some(UnitType::Service)
        } else if name.ends_with(".socket") {
            Some(UnitType::Socket)
        } else if name.ends_with(".target") {
            Some(UnitType::Target)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    Inactive,
    Activating,
    Active,
    Deactivating,
    Failed,
    Reloading,
    WaitingSocket,
}

impl fmt::Display for UnitState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitState::Inactive => write!(f, "inactive"),
            UnitState::Activating => write!(f, "activating"),
            UnitState::Active => write!(f, "active"),
            UnitState::Deactivating => write!(f, "deactivating"),
            UnitState::Failed => write!(f, "failed"),
            UnitState::Reloading => write!(f, "reloading"),
            UnitState::WaitingSocket => write!(f, "waiting-socket"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceType {
    Simple,
    Forking,
    Oneshot,
    Notify,
}

impl Default for ServiceType {
    fn default() -> Self {
        ServiceType::Simple
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    No,
    Always,
    OnSuccess,
    OnFailure,
    OnAbnormal,
    OnWatchdog,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        RestartPolicy::No
    }
}

impl RestartPolicy {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "always" => RestartPolicy::Always,
            "on-success" => RestartPolicy::OnSuccess,
            "on-failure" => RestartPolicy::OnFailure,
            "on-abnormal" => RestartPolicy::OnAbnormal,
            "on-watchdog" => RestartPolicy::OnWatchdog,
            _ => RestartPolicy::No,
        }
    }

    pub fn should_restart(&self, exit_code: i32, watchdog_timeout: bool) -> bool {
        if watchdog_timeout {
            return matches!(self, RestartPolicy::Always | RestartPolicy::OnFailure | RestartPolicy::OnAbnormal | RestartPolicy::OnWatchdog);
        }
        match self {
            RestartPolicy::No => false,
            RestartPolicy::Always => true,
            RestartPolicy::OnSuccess => exit_code == 0,
            RestartPolicy::OnFailure => exit_code != 0,
            RestartPolicy::OnAbnormal => exit_code != 0 && exit_code != 1,
            RestartPolicy::OnWatchdog => false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct UnitSection {
    pub description: String,
    pub documentation: String,
    pub requires: Vec<String>,
    pub wants: Vec<String>,
    pub binds_to: Vec<String>,
    pub part_of: Vec<String>,
    pub conflicts: Vec<String>,
    pub before: Vec<String>,
    pub after: Vec<String>,
    pub default_dependencies: bool,
}

#[derive(Clone, Debug)]
pub struct ServiceSection {
    pub service_type: ServiceType,
    pub exec_start: String,
    pub exec_stop: Option<String>,
    pub exec_reload: Option<String>,
    pub restart: RestartPolicy,
    pub restart_sec: u64,
    pub restart_max_sec: u64,
    pub restart_backoff_mult: f64,
    pub start_limit_burst: u32,
    pub start_limit_interval_sec: u64,
    pub timeout_start_sec: u64,
    pub timeout_stop_sec: u64,
    pub watchdog_sec: u64,
    pub standard_output: String,
    pub standard_error: String,
    pub environment: Vec<(String, String)>,
    pub working_directory: Option<String>,
    pub user: Option<String>,
    pub group: Option<String>,
}

impl Default for ServiceSection {
    fn default() -> Self {
        ServiceSection {
            service_type: ServiceType::Simple,
            exec_start: String::new(),
            exec_stop: None,
            exec_reload: None,
            restart: RestartPolicy::No,
            restart_sec: 1,
            restart_max_sec: 60,
            restart_backoff_mult: 2.0,
            start_limit_burst: 5,
            start_limit_interval_sec: 10,
            timeout_start_sec: 90,
            timeout_stop_sec: 90,
            watchdog_sec: 0,
            standard_output: "journal".to_string(),
            standard_error: "journal".to_string(),
            environment: Vec::new(),
            working_directory: None,
            user: None,
            group: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SocketSection {
    pub listen_stream: Vec<String>,
    pub listen_datagram: Vec<String>,
    pub accept: bool,
    pub service: Option<String>,
    pub socket_mode: u32,
}

#[derive(Clone, Debug, Default)]
pub struct InstallSection {
    pub wanted_by: Vec<String>,
    pub required_by: Vec<String>,
    pub alias: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct UnitFile {
    pub unit: UnitSection,
    pub service: Option<ServiceSection>,
    pub socket: Option<SocketSection>,
    pub install: InstallSection,
}

impl UnitFile {
    /// Parses a duration string like "100ms", "5s", "2min", "1h" or plain integer seconds.
    pub fn parse_duration(s: &str) -> u64 {
        let trimmed = s.trim().to_lowercase();
        if trimmed.ends_with("ms") {
            let val: u64 = trimmed.trim_end_matches("ms").trim().parse().unwrap_or(0);
            (val + 999) / 1000 // ceiling round to seconds
        } else if trimmed.ends_with('s') {
            trimmed.trim_end_matches('s').trim().parse().unwrap_or(0)
        } else if trimmed.ends_with("min") {
            let val: u64 = trimmed.trim_end_matches("min").trim().parse().unwrap_or(0);
            val * 60
        } else if trimmed.ends_with('h') {
            let val: u64 = trimmed.trim_end_matches('h').trim().parse().unwrap_or(0);
            val * 3600
        } else {
            trimmed.parse().unwrap_or(0)
        }
    }

    /// Parses a boolean string ("yes", "no", "true", "false", "1", "0").
    pub fn parse_bool(s: &str) -> bool {
        match s.trim().to_lowercase().as_str() {
            "yes" | "true" | "1" | "on" => true,
            _ => false,
        }
    }

    /// Parses space-separated unit names.
    pub fn parse_unit_list(s: &str) -> Vec<String> {
        s.split_whitespace()
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect()
    }

    /// Parses declarative INI unit content into a UnitFile.
    pub fn parse(content: &str) -> Result<Self, String> {
        let mut unit_file = UnitFile::default();
        let mut current_section: Option<String> = None;
        let mut raw_service = ServiceSection::default();
        let mut has_service = false;
        let mut raw_socket = SocketSection::default();
        let mut has_socket = false;

        for (line_num, raw_line) in content.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                let section = line[1..line.len() - 1].trim().to_string();
                if section == "Service" {
                    has_service = true;
                } else if section == "Socket" {
                    has_socket = true;
                }
                current_section = Some(section);
                continue;
            }

            if let Some(eq_pos) = line.find('=') {
                let key = line[..eq_pos].trim();
                let value = line[eq_pos + 1..].trim();

                match current_section.as_deref() {
                    Some("Unit") => match key {
                        "Description" => unit_file.unit.description = value.to_string(),
                        "Documentation" => unit_file.unit.documentation = value.to_string(),
                        "Requires" => unit_file.unit.requires.extend(Self::parse_unit_list(value)),
                        "Wants" => unit_file.unit.wants.extend(Self::parse_unit_list(value)),
                        "BindsTo" => unit_file.unit.binds_to.extend(Self::parse_unit_list(value)),
                        "PartOf" => unit_file.unit.part_of.extend(Self::parse_unit_list(value)),
                        "Conflicts" => unit_file.unit.conflicts.extend(Self::parse_unit_list(value)),
                        "Before" => unit_file.unit.before.extend(Self::parse_unit_list(value)),
                        "After" => unit_file.unit.after.extend(Self::parse_unit_list(value)),
                        "DefaultDependencies" => unit_file.unit.default_dependencies = Self::parse_bool(value),
                        _ => {}
                    },
                    Some("Service") => {
                        has_service = true;
                        match key {
                            "Type" => {
                                raw_service.service_type = match value.to_lowercase().as_str() {
                                    "forking" => ServiceType::Forking,
                                    "oneshot" => ServiceType::Oneshot,
                                    "notify" => ServiceType::Notify,
                                    _ => ServiceType::Simple,
                                };
                            }
                            "ExecStart" => raw_service.exec_start = value.to_string(),
                            "ExecStop" => raw_service.exec_stop = Some(value.to_string()),
                            "ExecReload" => raw_service.exec_reload = Some(value.to_string()),
                            "Restart" => raw_service.restart = RestartPolicy::parse(value),
                            "RestartSec" => raw_service.restart_sec = Self::parse_duration(value),
                            "RestartMaxSec" => raw_service.restart_max_sec = Self::parse_duration(value),
                            "RestartBackoffMultiplier" => raw_service.restart_backoff_mult = value.parse().unwrap_or(2.0),
                            "StartLimitBurst" => raw_service.start_limit_burst = value.parse().unwrap_or(5),
                            "StartLimitIntervalSec" => raw_service.start_limit_interval_sec = Self::parse_duration(value),
                            "TimeoutStartSec" => raw_service.timeout_start_sec = Self::parse_duration(value),
                            "TimeoutStopSec" => raw_service.timeout_stop_sec = Self::parse_duration(value),
                            "WatchdogSec" => raw_service.watchdog_sec = Self::parse_duration(value),
                            "StandardOutput" => raw_service.standard_output = value.to_string(),
                            "StandardError" => raw_service.standard_error = value.to_string(),
                            "Environment" => {
                                if let Some(sep) = value.find('=') {
                                    let env_k = value[..sep].trim().to_string();
                                    let env_v = value[sep + 1..].trim().to_string();
                                    raw_service.environment.push((env_k, env_v));
                                }
                            }
                            "WorkingDirectory" => raw_service.working_directory = Some(value.to_string()),
                            "User" => raw_service.user = Some(value.to_string()),
                            "Group" => raw_service.group = Some(value.to_string()),
                            _ => {}
                        }
                    }
                    Some("Socket") => {
                        has_socket = true;
                        match key {
                            "ListenStream" => raw_socket.listen_stream.push(value.to_string()),
                            "ListenDatagram" => raw_socket.listen_datagram.push(value.to_string()),
                            "Accept" => raw_socket.accept = Self::parse_bool(value),
                            "Service" => raw_socket.service = Some(value.to_string()),
                            "SocketMode" => raw_socket.socket_mode = u32::from_str_radix(value.trim_start_matches("0o"), 8).unwrap_or(0o660),
                            _ => {}
                        }
                    }
                    Some("Install") => match key {
                        "WantedBy" => unit_file.install.wanted_by.extend(Self::parse_unit_list(value)),
                        "RequiredBy" => unit_file.install.required_by.extend(Self::parse_unit_list(value)),
                        "Alias" => unit_file.install.alias.extend(Self::parse_unit_list(value)),
                        _ => {}
                    },
                    _ => {
                        return Err(format!("Line {}: key '{}' outside of section", line_num + 1, key));
                    }
                }
            }
        }

        if has_service {
            unit_file.service = Some(raw_service);
        }
        if has_socket {
            unit_file.socket = Some(raw_socket);
        }

        Ok(unit_file)
    }
}

/// Runtime instance representing a managed unit.
#[derive(Clone, Debug)]
pub struct UnitInstance {
    pub name: String,
    pub unit_type: UnitType,
    pub unit_file: UnitFile,
    pub state: UnitState,
    pub substate: String,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub restart_count: u32,
    pub current_backoff_sec: u64,
    pub last_start_ts: Option<u64>,
    pub last_exit_ts: Option<u64>,
    pub last_watchdog_ping: Option<u64>,
    pub enabled: bool,
    pub start_history: Vec<u64>,
    pub log_output: Vec<String>,
}

impl UnitInstance {
    pub fn new(name: &str, unit_file: UnitFile) -> Self {
        let unit_type = UnitType::from_name(name).unwrap_or(UnitType::Service);
        let backoff = unit_file.service.as_ref().map(|s| s.restart_sec).unwrap_or(1);
        UnitInstance {
            name: name.to_string(),
            unit_type,
            unit_file,
            state: UnitState::Inactive,
            substate: "dead".to_string(),
            pid: None,
            exit_code: None,
            restart_count: 0,
            current_backoff_sec: backoff,
            last_start_ts: None,
            last_exit_ts: None,
            last_watchdog_ping: None,
            enabled: true,
            start_history: Vec::new(),
            log_output: Vec::new(),
        }
    }

    /// Appends log line to ring buffer.
    pub fn append_log(&mut self, line: &str) {
        if self.log_output.len() >= 100 {
            self.log_output.remove(0);
        }
        self.log_output.push(line.to_string());
    }

    /// Records a process startup event and checks whether burst rate limit is breached.
    pub fn record_start(&mut self, now_sec: u64) -> Result<(), String> {
        let (burst, interval) = match &self.unit_file.service {
            Some(s) => (s.start_limit_burst, s.start_limit_interval_sec),
            None => (5, 10),
        };

        // Filter timestamps within [now - interval, now]
        self.start_history.retain(|&ts| now_sec.saturating_sub(ts) <= interval);
        if self.start_history.len() >= burst as usize {
            self.state = UnitState::Failed;
            self.substate = "start-limit-hit".to_string();
            return Err(format!(
                "Unit '{}' exceeded start limit ({} starts within {}s)",
                self.name, burst, interval
            ));
        }

        self.start_history.push(now_sec);
        self.last_start_ts = Some(now_sec);
        self.last_watchdog_ping = Some(now_sec);
        self.state = UnitState::Active;
        self.substate = "running".to_string();
        Ok(())
    }

    /// Handles process termination, calculating next exponential backoff.
    pub fn record_exit(&mut self, exit_code: i32, now_sec: u64, watchdog_timeout: bool) -> bool {
        self.last_exit_ts = Some(now_sec);
        self.exit_code = Some(exit_code);
        self.pid = None;

        let should_restart = if let Some(svc) = &self.unit_file.service {
            svc.restart.should_restart(exit_code, watchdog_timeout)
        } else {
            false
        };

        if should_restart {
            self.restart_count += 1;
            let svc = self.unit_file.service.as_ref().unwrap();
            let base_sec = svc.restart_sec.max(1);
            let mult = svc.restart_backoff_mult;
            let max_sec = svc.restart_max_sec;

            let computed = (base_sec as f64) * mult.powi(self.restart_count.saturating_sub(1) as i32);
            self.current_backoff_sec = (computed as u64).min(max_sec).max(base_sec);

            self.state = UnitState::Activating;
            self.substate = format!("auto-restart (backoff {}s)", self.current_backoff_sec);
            true
        } else {
            if exit_code == 0 {
                self.state = UnitState::Inactive;
                self.substate = "dead".to_string();
            } else {
                self.state = UnitState::Failed;
                self.substate = format!("failed (exit-code {})", exit_code);
            }
            false
        }
    }

    /// Receives watchdog heartbeat.
    pub fn ping_watchdog(&mut self, now_sec: u64) {
        self.last_watchdog_ping = Some(now_sec);
    }

    /// Checks if watchdog timer has expired.
    pub fn is_watchdog_expired(&self, now_sec: u64) -> bool {
        if self.state != UnitState::Active {
            return false;
        }
        if let Some(svc) = &self.unit_file.service {
            if svc.watchdog_sec > 0 {
                if let Some(last) = self.last_watchdog_ping {
                    return now_sec.saturating_sub(last) > svc.watchdog_sec;
                }
            }
        }
        false
    }
}
