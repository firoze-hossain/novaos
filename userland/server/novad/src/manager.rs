//! userland/server/novad/src/manager.rs
//!
//! Service Supervision & PID 1 Orchestration Engine for novad:
//! - Unit catalog loading & management
//! - Dependency graph compilation & parallel startup wave execution
//! - On-demand socket activation dispatch
//! - Health check watchdogs & crash recovery with exponential backoff
//! - Reverse topological graceful shutdown sequencing
//! - Standard system unit seeding (novasshd, novafw, novactl, targets)
//! - Client control API (start, stop, restart, reload, status, list, notify)

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::graph::DependencyGraph;
use crate::socket::SocketActivationManager;
use crate::unit::{UnitFile, UnitInstance, UnitState, UnitType};

#[derive(Clone, Debug)]
pub struct UnitStatusInfo {
    pub name: String,
    pub unit_type: String,
    pub state: UnitState,
    pub substate: String,
    pub description: String,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub backoff_sec: u64,
    pub uptime_sec: Option<u64>,
    pub last_exit_code: Option<i32>,
    pub enabled: bool,
    pub recent_logs: Vec<String>,
}

pub struct ServiceManager {
    pub units: HashMap<String, UnitInstance>,
    pub graph: DependencyGraph,
    pub socket_mgr: SocketActivationManager,
    pub active_target: String,
    pub is_running: bool,
    pub now_sec: u64,
    pub next_synthetic_pid: u32,
}

impl Default for ServiceManager {
    fn default() -> Self {
        ServiceManager {
            units: HashMap::new(),
            graph: DependencyGraph::new(),
            socket_mgr: SocketActivationManager::new(),
            active_target: "default.target".to_string(),
            is_running: false,
            now_sec: 1_700_000_000,
            next_synthetic_pid: 200,
        }
    }
}

impl ServiceManager {
    pub fn new() -> Self {
        let mut mgr = ServiceManager::default();
        mgr.load_default_system_units();
        mgr.load_state();
        mgr
    }

    pub fn state_file_path() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("NOVAD_STATE_FILE") {
            std::path::PathBuf::from(p)
        } else {
            std::path::PathBuf::from("/tmp/novad_runtime_state.json")
        }
    }

    /// Persists runtime unit states across CLI invocations.
    pub fn save_state(&self) {
        let path = Self::state_file_path();
        let mut out = String::new();
        out.push_str("{\n  \"units\": [\n");
        let mut first = true;
        for (name, u) in &self.units {
            if !first {
                out.push_str(",\n");
            }
            first = false;
            let pid_val = match u.pid {
                Some(p) => p.to_string(),
                None => "null".to_string(),
            };
            let code_val = match u.exit_code {
                Some(c) => c.to_string(),
                None => "null".to_string(),
            };
            let start_val = match u.last_start_ts {
                Some(s) => s.to_string(),
                None => "null".to_string(),
            };
            out.push_str(&format!(
                "    {{\n      \"name\": \"{}\",\n      \"state\": \"{:?}\",\n      \"substate\": \"{}\",\n      \"pid\": {},\n      \"exit_code\": {},\n      \"restart_count\": {},\n      \"current_backoff_sec\": {},\n      \"last_start_ts\": {},\n      \"enabled\": {}\n    }}",
                name, u.state, u.substate, pid_val, code_val, u.restart_count, u.current_backoff_sec, start_val, u.enabled
            ));
        }
        out.push_str("\n  ]\n}\n");
        let _ = fs::write(&path, out);
    }

    /// Loads persisted runtime states from storage.
    pub fn load_state(&mut self) {
        let path = Self::state_file_path();
        if let Ok(content) = fs::read_to_string(&path) {
            for chunk in content.split('{') {
                if !chunk.contains("\"name\"") {
                    continue;
                }
                let mut name = String::new();
                let mut state_str = String::new();
                let mut substate = String::new();
                let mut pid: Option<u32> = None;
                let mut exit_code: Option<i32> = None;
                let mut restart_count: u32 = 0;
                let mut backoff: u64 = 1;
                let mut last_start: Option<u64> = None;
                let mut enabled = true;

                for line in chunk.lines() {
                    let line = line.trim().trim_end_matches(',').trim_end_matches('}');
                    if let Some((k, v)) = line.split_once(':') {
                        let key = k.trim().trim_matches('"');
                        let val = v.trim().trim_matches('"');
                        match key {
                            "name" => name = val.to_string(),
                            "state" => state_str = val.to_string(),
                            "substate" => substate = val.to_string(),
                            "pid" => pid = val.parse().ok(),
                            "exit_code" => exit_code = val.parse().ok(),
                            "restart_count" => restart_count = val.parse().unwrap_or(0),
                            "current_backoff_sec" => backoff = val.parse().unwrap_or(1),
                            "last_start_ts" => last_start = val.parse().ok(),
                            "enabled" => enabled = val != "false",
                            _ => {}
                        }
                    }
                }

                if let Some(inst) = self.units.get_mut(&name) {
                    inst.state = match state_str.as_str() {
                        "Active" => UnitState::Active,
                        "Activating" => UnitState::Activating,
                        "Deactivating" => UnitState::Deactivating,
                        "Failed" => UnitState::Failed,
                        "Reloading" => UnitState::Reloading,
                        "WaitingSocket" => UnitState::WaitingSocket,
                        _ => UnitState::Inactive,
                    };
                    if !substate.is_empty() {
                        inst.substate = substate;
                    }
                    inst.pid = pid;
                    inst.exit_code = exit_code;
                    inst.restart_count = restart_count;
                    inst.current_backoff_sec = backoff;
                    inst.last_start_ts = last_start;
                    inst.enabled = enabled;
                }
            }
        }
    }

    /// Loads and registers a unit from its string definition.
    pub fn load_unit_from_str(&mut self, name: &str, content: &str) -> Result<(), String> {
        let unit_file = UnitFile::parse(content)?;
        let instance = UnitInstance::new(name, unit_file.clone());

        // Register in graph and units table
        self.graph.add_unit(name, unit_file.clone());
        self.units.insert(name.to_string(), instance);

        // If unit is a socket, register with socket activation manager
        if let Some(sock) = &unit_file.socket {
            let target_svc = sock.service.clone().unwrap_or_else(|| {
                if name.ends_with(".socket") {
                    format!("{}.service", &name[..name.len() - 7])
                } else {
                    format!("{}.service", name)
                }
            });
            self.socket_mgr.register_socket(name, &target_svc, &sock.listen_stream);
        }

        Ok(())
    }

    /// Loads all unit files from a directory (.service, .socket, .target).
    pub fn load_units_from_dir(&mut self, dir_path: &Path) -> Result<usize, String> {
        if !dir_path.exists() {
            return Ok(0);
        }

        let mut loaded = 0;
        let entries = fs::read_dir(dir_path)
            .map_err(|e| format!("Failed to read directory {}: {}", dir_path.display(), e))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let filename = path.file_name().unwrap_or_default().to_string_lossy();
                if filename.ends_with(".service") || filename.ends_with(".socket") || filename.ends_with(".target") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if self.load_unit_from_str(&filename, &content).is_ok() {
                            loaded += 1;
                        }
                    }
                }
            }
        }

        self.graph.compile();
        Ok(loaded)
    }

    /// Populates built-in enterprise system unit declarations.
    pub fn load_default_system_units(&mut self) {
        // 1. novafw.service
        let novafw_unit = r#"
[Unit]
Description=NovaOS Stateful Packet-Filter Firewall & NAT Subsystem
Before=network.target

[Service]
Type=oneshot
ExecStart=/bin/novafw load /etc/novafw.conf
Restart=on-failure
RestartSec=1s

[Install]
WantedBy=multi-user.target
"#;
        let _ = self.load_unit_from_str("novafw.service", novafw_unit);

        // 2. network.target
        let network_target = r#"
[Unit]
Description=Network Initialization & Filtering Target
After=novafw.service
Wants=novafw.service

[Install]
WantedBy=multi-user.target
"#;
        let _ = self.load_unit_from_str("network.target", network_target);

        // 3. novasshd.service
        let novasshd_unit = r#"
[Unit]
Description=NovaOS OpenSSH-Compatible SSH-2 Server Daemon
After=network.target novafw.service
Requires=network.target
Wants=novafw.service

[Service]
Type=simple
ExecStart=/bin/novasshd -p 22
Restart=always
RestartSec=2s
RestartMaxSec=30s
WatchdogSec=15s
StandardOutput=journal

[Install]
WantedBy=multi-user.target
"#;
        let _ = self.load_unit_from_str("novasshd.service", novasshd_unit);

        // 4. cgroup.target
        let cgroup_target = r#"
[Unit]
Description=Control Groups v2 Unified Hierarchy Target
"#;
        let _ = self.load_unit_from_str("cgroup.target", cgroup_target);

        // 5. novactl.service
        let novactl_unit = r#"
[Unit]
Description=NovaOS Container Runtime Engine Daemon (novactl)
After=network.target cgroup.target
Requires=cgroup.target
Wants=network.target

[Service]
Type=simple
ExecStart=/bin/novactl daemon
Restart=on-failure
RestartSec=3s
RestartMaxSec=60s
WatchdogSec=30s

[Install]
WantedBy=multi-user.target
"#;
        let _ = self.load_unit_from_str("novactl.service", novactl_unit);

        // 6. novamon.service
        let novamon_unit = r#"
[Unit]
Description=NovaOS Prometheus Telemetry Exporter & Health Metrics Agent (novamon)
After=network.target novafw.service
Requires=network.target
Wants=novafw.service

[Service]
Type=simple
ExecStart=/bin/novamon -p 9100
Restart=always
RestartSec=2s
RestartMaxSec=30s
WatchdogSec=15s
StandardOutput=journal

[Install]
WantedBy=multi-user.target
"#;
        let _ = self.load_unit_from_str("novamon.service", novamon_unit);

        // 7. syslog.socket & syslog.service (demonstrating socket activation)
        let syslog_socket = r#"
[Unit]
Description=System Logging Socket
Before=syslog.service

[Socket]
ListenStream=/run/systemd/journal/stdout
ListenStream=127.0.0.1:5140
Service=syslog.service

[Install]
WantedBy=sockets.target
"#;
        let _ = self.load_unit_from_str("syslog.socket", syslog_socket);

        let syslog_service = r#"
[Unit]
Description=System Logging Service
After=syslog.socket
Requires=syslog.socket

[Service]
Type=simple
ExecStart=/bin/klogd
Restart=always
RestartSec=1s
"#;
        let _ = self.load_unit_from_str("syslog.service", syslog_service);

        // 7. sockets.target
        let sockets_target = r#"
[Unit]
Description=Sockets Activation Target
Wants=syslog.socket
"#;
        let _ = self.load_unit_from_str("sockets.target", sockets_target);

        // 9. multi-user.target
        let multi_user_target = r#"
[Unit]
Description=NovaOS Multi-User Server Environment Target
Requires=network.target sockets.target
Wants=novasshd.service novactl.service novamon.service
"#;
        let _ = self.load_unit_from_str("multi-user.target", multi_user_target);

        // 9. default.target
        let default_target = r#"
[Unit]
Description=Default System Boot Target
Requires=multi-user.target
After=multi-user.target
"#;
        let _ = self.load_unit_from_str("default.target", default_target);

        // 10. shutdown.target
        let shutdown_target = r#"
[Unit]
Description=System Graceful Shutdown Target
Conflicts=multi-user.target default.target
"#;
        let _ = self.load_unit_from_str("shutdown.target", shutdown_target);

        self.graph.compile();
    }

    /// Boots the system to the specified target using dependency graph parallel waves.
    pub fn boot_target(&mut self, target_name: &str) -> Result<Vec<Vec<String>>, String> {
        self.graph.compile();
        if !self.units.contains_key(target_name) {
            return Err(format!("Target unit '{}' not found in catalog", target_name));
        }

        self.active_target = target_name.to_string();
        self.is_running = true;

        // Collect all transitive dependencies
        let active_set = self.graph.collect_transitive_deps(target_name);
        let waves = self.graph.compute_parallel_startup_waves(&active_set);

        for wave in &waves {
            for unit_name in wave {
                let _ = self.start_unit_internal(unit_name);
            }
        }

        // Activate the target unit itself
        if let Some(target_inst) = self.units.get_mut(target_name) {
            target_inst.state = UnitState::Active;
            target_inst.substate = "active".to_string();
        }

        self.save_state();
        Ok(waves)
    }

    /// Executes internal unit start logic.
    fn start_unit_internal(&mut self, name: &str) -> Result<(), String> {
        // Collect current unit states
        let states: HashMap<String, UnitState> = self
            .units
            .iter()
            .map(|(k, v)| (k.clone(), v.state))
            .collect();

        // Check dependencies & conflicts
        let to_stop = self.graph.evaluate_dependencies(name, &states)?;
        for conflicting_unit in to_stop {
            let _ = self.stop_unit_internal(&conflicting_unit);
        }

        let pid = self.next_synthetic_pid;
        self.next_synthetic_pid += 1;

        let now = self.now_sec;
        let (unit_type, is_socket) = {
            let inst = self.units.get_mut(name).ok_or_else(|| format!("Unit '{}' not found", name))?;
            (inst.unit_type, inst.unit_type == UnitType::Socket)
        };

        if is_socket {
            let inst = self.units.get_mut(name).unwrap();
            inst.state = UnitState::Active;
            inst.substate = "listening".to_string();
            inst.append_log("[novad] Socket endpoint bound and listening.");
            return Ok(());
        }

        if unit_type == UnitType::Target {
            let inst = self.units.get_mut(name).unwrap();
            inst.state = UnitState::Active;
            inst.substate = "active".to_string();
            return Ok(());
        }

        // Service unit
        let inst = self.units.get_mut(name).unwrap();
        inst.record_start(now)?;
        inst.pid = Some(pid);
        inst.append_log(&format!("[novad] Started unit with PID {}.", pid));

        // Acknowledge socket if any
        self.socket_mgr.acknowledge_service_active(name);

        Ok(())
    }

    /// Stops unit internally.
    fn stop_unit_internal(&mut self, name: &str) -> Result<(), String> {
        let inst = self.units.get_mut(name).ok_or_else(|| format!("Unit '{}' not found", name))?;
        inst.state = UnitState::Deactivating;
        inst.substate = "stopping".to_string();
        inst.pid = None;
        inst.state = UnitState::Inactive;
        inst.substate = "dead".to_string();
        inst.append_log("[novad] Unit stopped gracefully.");
        Ok(())
    }

    /// Graceful shutdown sequencing in exact reverse topological order.
    pub fn shutdown(&mut self) -> Vec<Vec<String>> {
        self.is_running = false;
        let active_units: HashSet<String> = self
            .units
            .iter()
            .filter(|(_, u)| u.state == UnitState::Active || u.state == UnitState::Activating)
            .map(|(k, _)| k.clone())
            .collect();

        let shutdown_waves = self.graph.compute_shutdown_waves(&active_units);

        for wave in &shutdown_waves {
            for unit_name in wave {
                let _ = self.stop_unit_internal(unit_name);
            }
        }

        // Close all sockets
        for ep in self.socket_mgr.sockets.values_mut() {
            ep.close();
        }

        if let Some(target) = self.units.get_mut("shutdown.target") {
            target.state = UnitState::Active;
            target.substate = "active".to_string();
        }

        self.save_state();
        shutdown_waves
    }

    /// Advance supervisor clock and execute tick actions:
    /// - Poll socket activations
    /// - Watchdog monitoring and timeout kills
    /// - Scheduled exponential backoff restarts
    pub fn supervise_step(&mut self, delta_sec: u64) {
        self.now_sec += delta_sec;
        let now = self.now_sec;

        // 1. Check socket activations
        let activated_services = self.socket_mgr.poll_pending_activations(now);
        for svc_name in activated_services {
            if let Some(svc) = self.units.get(&svc_name) {
                if svc.state == UnitState::Inactive || svc.state == UnitState::WaitingSocket {
                    println!("[novad] On-demand socket activation triggered for '{}'", svc_name);
                    let _ = self.start_unit_internal(&svc_name);
                }
            }
        }

        // 2. Health check watchdog inspection
        let mut watchdog_timed_out = Vec::new();
        for (name, inst) in &self.units {
            if inst.is_watchdog_expired(now) {
                watchdog_timed_out.push(name.clone());
            }
        }

        for name in watchdog_timed_out {
            if let Some(inst) = self.units.get_mut(&name) {
                inst.append_log("[novad] Watchdog timeout expired! Sending SIGABRT...");
                let _ = inst.record_exit(134, now, true); // 128 + 6 (SIGABRT)
            }
        }

        // 3. Process exponential backoff restarts
        let mut ready_to_restart = Vec::new();
        for (name, inst) in &self.units {
            if inst.state == UnitState::Activating && inst.substate.starts_with("auto-restart") {
                if let Some(last_exit) = inst.last_exit_ts {
                    if now >= last_exit + inst.current_backoff_sec {
                        ready_to_restart.push(name.clone());
                    }
                }
            }
        }

        for name in ready_to_restart {
            println!("[novad] Exponential backoff window elapsed. Restarting '{}'...", name);
            let _ = self.start_unit_internal(&name);
        }

        self.save_state();
    }

    /// Client API: Starts a unit manually.
    pub fn start_unit(&mut self, name: &str) -> Result<(), String> {
        self.start_unit_internal(name)?;
        self.save_state();
        Ok(())
    }

    /// Client API: Stops a unit manually.
    pub fn stop_unit(&mut self, name: &str) -> Result<(), String> {
        self.stop_unit_internal(name)?;
        self.save_state();
        Ok(())
    }

    /// Client API: Restarts a unit manually.
    pub fn restart_unit(&mut self, name: &str) -> Result<(), String> {
        self.stop_unit_internal(name)?;
        self.start_unit_internal(name)?;
        self.save_state();
        Ok(())
    }

    /// Client API: Reloads configuration for a unit.
    pub fn reload_unit(&mut self, name: &str) -> Result<(), String> {
        let inst = self.units.get_mut(name).ok_or_else(|| format!("Unit '{}' not found", name))?;
        inst.state = UnitState::Reloading;
        inst.substate = "reloading".to_string();
        inst.append_log("[novad] Unit configuration reloaded.");
        inst.state = UnitState::Active;
        inst.substate = "running".to_string();
        self.save_state();
        Ok(())
    }

    /// Client API: Inquires status of a unit.
    pub fn status_unit(&self, name: &str) -> Result<UnitStatusInfo, String> {
        let inst = self.units.get(name).ok_or_else(|| format!("Unit '{}' not found", name))?;
        let desc = inst.unit_file.unit.description.clone();
        let uptime = inst.last_start_ts.map(|start| self.now_sec.saturating_sub(start));

        Ok(UnitStatusInfo {
            name: inst.name.clone(),
            unit_type: format!("{:?}", inst.unit_type),
            state: inst.state,
            substate: inst.substate.clone(),
            description: desc,
            pid: inst.pid,
            restart_count: inst.restart_count,
            backoff_sec: inst.current_backoff_sec,
            uptime_sec: uptime,
            last_exit_code: inst.exit_code,
            enabled: inst.enabled,
            recent_logs: inst.log_output.clone(),
        })
    }

    /// Client API: Lists all registered units and their states.
    pub fn list_units(&self) -> Vec<UnitStatusInfo> {
        let mut list: Vec<UnitStatusInfo> = self
            .units
            .keys()
            .filter_map(|name| self.status_unit(name).ok())
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Client API: Enables a unit (installs symlink or flags enabled).
    pub fn enable_unit(&mut self, name: &str) -> Result<(), String> {
        let inst = self.units.get_mut(name).ok_or_else(|| format!("Unit '{}' not found", name))?;
        inst.enabled = true;
        inst.append_log("[novad] Unit enabled.");
        self.save_state();
        Ok(())
    }

    /// Client API: Disables a unit.
    pub fn disable_unit(&mut self, name: &str) -> Result<(), String> {
        let inst = self.units.get_mut(name).ok_or_else(|| format!("Unit '{}' not found", name))?;
        inst.enabled = false;
        inst.append_log("[novad] Unit disabled.");
        self.save_state();
        Ok(())
    }

    /// Handles systemd-style sd_notify notifications.
    pub fn sd_notify(&mut self, identifier: &str, message: &str) -> Result<(), String> {
        let now = self.now_sec;
        let mut target_unit: Option<String> = None;

        // Identifier could be unit name or PID
        if let Ok(pid) = identifier.parse::<u32>() {
            for (name, inst) in &self.units {
                if inst.pid == Some(pid) {
                    target_unit = Some(name.clone());
                    break;
                }
            }
        } else if self.units.contains_key(identifier) {
            target_unit = Some(identifier.to_string());
        }

        let unit_name = target_unit.ok_or_else(|| format!("No active unit matches identifier '{}'", identifier))?;
        let inst = self.units.get_mut(&unit_name).unwrap();

        for part in message.split('\n') {
            let part = part.trim();
            if part == "READY=1" {
                inst.state = UnitState::Active;
                inst.substate = "running".to_string();
                inst.append_log("[sd_notify] Service reported READY=1.");
            } else if part == "WATCHDOG=1" {
                inst.ping_watchdog(now);
                inst.append_log("[sd_notify] Watchdog heartbeat received.");
            } else if part.starts_with("STATUS=") {
                let status_text = &part["STATUS=".len()..];
                inst.substate = status_text.to_string();
                inst.append_log(&format!("[sd_notify] Status updated: {}", status_text));
            } else if part == "STOPPING=1" {
                inst.state = UnitState::Deactivating;
                inst.substate = "stopping".to_string();
                inst.append_log("[sd_notify] Service reported STOPPING=1.");
            }
        }

        Ok(())
    }
}
