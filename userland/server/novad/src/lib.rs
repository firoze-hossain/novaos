//! userland/server/novad/src/lib.rs
//!
//! Pure Rust Systemd-Style Service & Daemon Orchestrator (novad):
//! - Declarative .service, .socket, and .target configuration units
//! - Dependency graph resolution with deterministic parallel startup waves (Requires=, Wants=, After=, Before=)
//! - On-demand socket activation engine
//! - Health check watchdogs (sd_notify WATCHDOG=1)
//! - Automatic crash restart with exponential backoff & burst limit protection
//! - Graceful reverse topological shutdown sequencing
//! - 100% Pure Safe Rust (#![forbid(unsafe_code)]) with zero external dependencies

#![forbid(unsafe_code)]

pub mod graph;
pub mod manager;
pub mod socket;
pub mod unit;

pub use graph::DependencyGraph;
pub use manager::{ServiceManager, UnitStatusInfo};
pub use socket::{SocketActivationManager, SocketEndpoint, SocketState};
pub use unit::{
    InstallSection, RestartPolicy, ServiceSection, ServiceType, SocketSection, UnitFile,
    UnitInstance, UnitSection, UnitState, UnitType,
};

pub fn version() -> &'static str {
    "novad 2.0.0 (x86_64-nova)"
}

/// Comprehensive internal self-test validating all features of the novad service orchestrator.
pub fn run_novad_selftest() -> bool {
    println!("============================================================");
    println!("     NovaOS Service Orchestrator (novad) Self-Test          ");
    println!("============================================================");

    // 1. Declarative Unit Parser Verification
    print!("[TEST 1/8] Declarative INI Unit Parser (.service, .socket, .target)... ");
    let sample_service = r#"
[Unit]
Description=Test Web Service
Documentation=https://novaos.org/docs/test
Requires=network.target db.service
After=network.target db.service
Conflicts=maintenance.service

[Service]
Type=simple
ExecStart=/bin/webserver --port 8080
Restart=on-failure
RestartSec=2s
RestartMaxSec=32s
RestartBackoffMultiplier=2.0
StartLimitBurst=4
StartLimitIntervalSec=10s
WatchdogSec=5s
Environment=PORT=8080
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
"#;

    let parsed = match UnitFile::parse(sample_service) {
        Ok(u) => u,
        Err(e) => {
            println!("FAILED: {}", e);
            return false;
        }
    };

    if parsed.unit.description != "Test Web Service"
        || parsed.unit.requires != vec!["network.target", "db.service"]
        || parsed.unit.conflicts != vec!["maintenance.service"]
    {
        println!("FAILED: Unit section parsed incorrectly");
        return false;
    }

    let svc = match &parsed.service {
        Some(s) => s,
        None => {
            println!("FAILED: Missing service section");
            return false;
        }
    };

    if svc.exec_start != "/bin/webserver --port 8080"
        || svc.restart != RestartPolicy::OnFailure
        || svc.restart_sec != 2
        || svc.restart_max_sec != 32
        || svc.watchdog_sec != 5
        || svc.environment.len() != 2
    {
        println!("FAILED: Service section parsed incorrectly");
        return false;
    }
    println!("PASSED");

    // 2. Dependency Graph Resolution & Parallel Startup Waves
    print!("[TEST 2/8] Dependency Graph DAG & Parallel Startup Waves... ");
    let mut graph = DependencyGraph::new();
    let u1 = UnitFile::parse("[Unit]\nDescription=U1\n").unwrap();
    let u2 = UnitFile::parse("[Unit]\nDescription=U2\nAfter=u1.service\nRequires=u1.service\n").unwrap();
    let u3 = UnitFile::parse("[Unit]\nDescription=U3\nAfter=u1.service\n").unwrap();
    let u4 = UnitFile::parse("[Unit]\nDescription=U4\nAfter=u2.service u3.service\n").unwrap();

    graph.add_unit("u1.service", u1);
    graph.add_unit("u2.service", u2);
    graph.add_unit("u3.service", u3);
    graph.add_unit("u4.service", u4);
    graph.compile();

    let mut all_set = std::collections::HashSet::new();
    all_set.insert("u1.service".to_string());
    all_set.insert("u2.service".to_string());
    all_set.insert("u3.service".to_string());
    all_set.insert("u4.service".to_string());

    let waves = graph.compute_parallel_startup_waves(&all_set);
    // Wave 0 should be u1.service
    // Wave 1 should be u2.service and u3.service (parallel!)
    // Wave 2 should be u4.service
    if waves.len() != 3 {
        println!("FAILED: Expected 3 waves, got {}", waves.len());
        return false;
    }
    if waves[0] != vec!["u1.service"] {
        println!("FAILED: Wave 0 should contain only u1.service, got {:?}", waves[0]);
        return false;
    }
    if waves[1] != vec!["u2.service", "u3.service"] {
        println!("FAILED: Wave 1 should contain u2 and u3 in parallel, got {:?}", waves[1]);
        return false;
    }
    if waves[2] != vec!["u4.service"] {
        println!("FAILED: Wave 2 should contain u4.service, got {:?}", waves[2]);
        return false;
    }
    println!("PASSED (Wave 0: 1 unit, Wave 1: 2 units in parallel, Wave 2: 1 unit)");

    // 3. Cycle Detection and Breaking
    print!("[TEST 3/8] Dependency Cycle Detection & Breaking... ");
    let mut cycle_graph = DependencyGraph::new();
    let ca = UnitFile::parse("[Unit]\nDescription=CA\nAfter=cb.service\n").unwrap();
    let cb = UnitFile::parse("[Unit]\nDescription=CB\nAfter=cc.service\n").unwrap();
    let cc = UnitFile::parse("[Unit]\nDescription=CC\nAfter=ca.service\n").unwrap();
    cycle_graph.add_unit("ca.service", ca);
    cycle_graph.add_unit("cb.service", cb);
    cycle_graph.add_unit("cc.service", cc);
    cycle_graph.compile(); // Should break cycle without panic

    let mut cycle_set = std::collections::HashSet::new();
    cycle_set.insert("ca.service".to_string());
    cycle_set.insert("cb.service".to_string());
    cycle_set.insert("cc.service".to_string());
    let cycle_waves = cycle_graph.compute_parallel_startup_waves(&cycle_set);
    let total_scheduled: usize = cycle_waves.iter().map(|w| w.len()).sum();
    if total_scheduled != 3 {
        println!("FAILED: Expected 3 scheduled units after cycle break, got {}", total_scheduled);
        return false;
    }
    println!("PASSED (Cycle broken, all 3 units scheduled)");

    // 4. On-Demand Socket Activation
    print!("[TEST 4/8] On-Demand Socket Activation Engine... ");
    let mut mgr = ServiceManager::default();
    let sock_def = r#"
[Unit]
Description=Echo Socket
Before=echo.service

[Socket]
ListenStream=127.0.0.1:9099
Service=echo.service
"#;
    let echo_def = r#"
[Unit]
Description=Echo Service
Requires=echo.socket
After=echo.socket

[Service]
Type=simple
ExecStart=/bin/echo-daemon
Restart=always
"#;

    if mgr.load_unit_from_str("echo.socket", sock_def).is_err()
        || mgr.load_unit_from_str("echo.service", echo_def).is_err()
    {
        println!("FAILED: Could not load socket/service pair");
        return false;
    }

    // Start only the socket unit initially
    if mgr.start_unit("echo.socket").is_err() {
        println!("FAILED: Could not start echo.socket");
        return false;
    }

    let echo_status = mgr.status_unit("echo.service").unwrap();
    if echo_status.state != UnitState::Inactive {
        println!("FAILED: echo.service should initially be inactive");
        return false;
    }

    // Trigger simulated socket activity
    let activated = mgr.socket_mgr.trigger_socket("echo.socket", mgr.now_sec);
    if activated != Some("echo.service".to_string()) {
        println!("FAILED: Socket trigger did not return echo.service");
        return false;
    }

    // Advance supervisor step: it should automatically spin up echo.service!
    mgr.supervise_step(1);
    let echo_status_after = mgr.status_unit("echo.service").unwrap();
    if echo_status_after.state != UnitState::Active {
        println!("FAILED: echo.service was not activated by socket event: {:?}", echo_status_after.state);
        return false;
    }
    println!("PASSED (echo.service dormant -> connection arrived -> activated with PID {:?})", echo_status_after.pid);

    // 5. Health Check Watchdog & Crash Recovery
    print!("[TEST 5/8] Health Check Watchdog (sd_notify WATCHDOG=1)... ");
    let wd_service = r#"
[Unit]
Description=Watchdog Monitored Daemon

[Service]
Type=simple
ExecStart=/bin/worker
Restart=always
RestartSec=1s
WatchdogSec=4s
"#;
    mgr.load_unit_from_str("worker.service", wd_service).unwrap();
    mgr.start_unit("worker.service").unwrap();

    let pid = mgr.status_unit("worker.service").unwrap().pid.unwrap();
    // Send watchdog heartbeat
    mgr.sd_notify(&pid.to_string(), "WATCHDOG=1").unwrap();

    // Advance clock by 2s (within 4s timeout)
    mgr.supervise_step(2);
    let st1 = mgr.status_unit("worker.service").unwrap();
    if st1.state != UnitState::Active {
        println!("FAILED: Unit should still be active within watchdog interval");
        return false;
    }

    // Advance clock by 5s (exceeding 4s watchdog timeout without heartbeat)
    mgr.supervise_step(5);
    let st2 = mgr.status_unit("worker.service").unwrap();
    if st2.state != UnitState::Activating || !st2.substate.contains("auto-restart") {
        println!("FAILED: Expected auto-restart after watchdog timeout, got state {:?}, substate {}", st2.state, st2.substate);
        return false;
    }
    println!("PASSED (Expired watchdog triggered auto-restart kill)");

    // 6. Exponential Backoff & Start Limit Burst Protection
    print!("[TEST 6/8] Exponential Backoff & Start Limit Burst Protection... ");
    let backoff_service = r#"
[Unit]
Description=Crashy Daemon

[Service]
Type=simple
ExecStart=/bin/crashy
Restart=on-failure
RestartSec=1s
RestartMaxSec=16s
RestartBackoffMultiplier=2.0
StartLimitBurst=3
StartLimitIntervalSec=10s
"#;
    mgr.load_unit_from_str("crashy.service", backoff_service).unwrap();
    mgr.start_unit("crashy.service").unwrap();

    // Crash 1
    let inst = mgr.units.get_mut("crashy.service").unwrap();
    inst.record_exit(1, mgr.now_sec, false);
    if inst.current_backoff_sec != 1 {
        println!("FAILED: Crash 1 backoff expected 1, got {}", inst.current_backoff_sec);
        return false;
    }

    // Crash 2
    inst.record_exit(1, mgr.now_sec, false);
    if inst.current_backoff_sec != 2 {
        println!("FAILED: Crash 2 backoff expected 2, got {}", inst.current_backoff_sec);
        return false;
    }

    // Crash 3
    inst.record_exit(1, mgr.now_sec, false);
    if inst.current_backoff_sec != 4 {
        println!("FAILED: Crash 3 backoff expected 4, got {}", inst.current_backoff_sec);
        return false;
    }

    // Now attempt rapid start attempts beyond StartLimitBurst=3 within interval
    let mut burst_blocked = false;
    for _ in 0..5 {
        if inst.record_start(mgr.now_sec).is_err() {
            burst_blocked = true;
            break;
        }
    }

    if !burst_blocked || inst.state != UnitState::Failed {
        println!("FAILED: StartLimitBurst protection did not trigger");
        return false;
    }
    println!("PASSED (1s -> 2s -> 4s backoff; start-limit-hit quarantined to Failed)");

    // 7. Graceful Reverse Topological Shutdown Sequencing
    print!("[TEST 7/8] Graceful Shutdown in Reverse Topological Waves... ");
    let mut boot_mgr = ServiceManager::default();
    boot_mgr.load_default_system_units();
    let startup_waves = boot_mgr.boot_target("default.target").unwrap();
    if startup_waves.is_empty() {
        println!("FAILED: Startup waves empty");
        return false;
    }

    let shutdown_waves = boot_mgr.shutdown();
    if shutdown_waves.is_empty() {
        println!("FAILED: Shutdown waves empty");
        return false;
    }

    // The first wave of shutdown should correspond to the last wave of startup
    let last_start = startup_waves.last().unwrap();
    let first_stop = shutdown_waves.first().unwrap();
    if last_start != first_stop {
        println!("FAILED: Shutdown waves should be exact reverse of startup waves. Start last: {:?}, Stop first: {:?}", last_start, first_stop);
        return false;
    }

    let all_stopped = boot_mgr
        .units
        .values()
        .all(|u| u.name == "shutdown.target" || u.state == UnitState::Inactive);
    if !all_stopped {
        println!("FAILED: Not all units inactive after shutdown");
        return false;
    }
    println!("PASSED (Reversed {} waves into {} shutdown waves)", startup_waves.len(), shutdown_waves.len());

    // 8. Default System Units Verification
    print!("[TEST 8/8] System Units (novasshd, novafw, novactl, targets)... ");
    let mut sys_mgr = ServiceManager::new();
    let list = sys_mgr.list_units();
    let names: Vec<String> = list.iter().map(|u| u.name.clone()).collect();
    let required_units = [
        "novasshd.service",
        "novafw.service",
        "novactl.service",
        "network.target",
        "multi-user.target",
        "default.target",
        "shutdown.target",
    ];

    for req in &required_units {
        if !names.contains(&req.to_string()) {
            println!("FAILED: Missing system unit '{}'", req);
            return false;
        }
    }

    // Test booting default.target
    let boot_res = sys_mgr.boot_target("default.target");
    if boot_res.is_err() {
        println!("FAILED: boot_target(default.target) failed: {:?}", boot_res);
        return false;
    }

    let sshd_status = sys_mgr.status_unit("novasshd.service").unwrap();
    let fw_status = sys_mgr.status_unit("novafw.service").unwrap();
    let ctl_status = sys_mgr.status_unit("novactl.service").unwrap();

    if sshd_status.state != UnitState::Active || fw_status.state != UnitState::Active || ctl_status.state != UnitState::Active {
        println!("FAILED: System units not all active after boot: sshd={:?}, fw={:?}, ctl={:?}", sshd_status.state, fw_status.state, ctl_status.state);
        return false;
    }

    println!("PASSED (All default services active under multi-user.target)");
    println!("============================================================");
    println!("  ALL 8/8 NOVAD ORCHESTRATOR CHECKS PASSED SUCCESSFULLY     ");
    println!("============================================================");

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_selftest() {
        assert!(run_novad_selftest(), "All novad self-test checks must pass");
    }

    #[test]
    fn test_unit_parser_and_validation() {
        let text = r#"
[Unit]
Description=Test Suite Unit
After=syslog.target

[Service]
ExecStart=/bin/true
Restart=always
RestartSec=500ms
"#;
        let u = UnitFile::parse(text).expect("parse failed");
        assert_eq!(u.unit.description, "Test Suite Unit");
        let svc = u.service.expect("missing service");
        assert_eq!(svc.restart_sec, 1); // 500ms rounded up to 1s
    }

    #[test]
    fn test_dependency_ordering() {
        let mut graph = DependencyGraph::new();
        graph.add_unit("a.service", UnitFile::parse("[Unit]\nDescription=A\n").unwrap());
        graph.add_unit("b.service", UnitFile::parse("[Unit]\nDescription=B\nAfter=a.service\n").unwrap());
        graph.compile();

        let mut set = std::collections::HashSet::new();
        set.insert("a.service".to_string());
        set.insert("b.service".to_string());

        let waves = graph.compute_parallel_startup_waves(&set);
        assert_eq!(waves, vec![vec!["a.service"], vec!["b.service"]]);
    }
}
