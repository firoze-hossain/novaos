//! userland/server/novactl/src/service.rs
//!
//! Service Supervision CLI Integration for novactl:
//! Provides the `novactl service` command suite interfacing with the novad orchestrator:
//!   - novactl service start <unit>
//!   - novactl service stop <unit>
//!   - novactl service restart <unit>
//!   - novactl service status [unit]
//!   - novactl service list
//!   - novactl service reload <unit>
//!   - novactl service enable <unit>
//!   - novactl service disable <unit>
//!   - novactl service boot [target]
//!   - novactl service shutdown

use std::path::Path;
use novad::{ServiceManager, UnitState};

pub fn print_service_usage() {
    println!(
        "novactl service - Manage systemd-style services & daemons (novad orchestrator)\n\n\
         USAGE:\n    \
             novactl service <SUBCOMMAND> [OPTIONS]\n\n\
         SUBCOMMANDS:\n    \
             start <UNIT>        Start a service or target unit\n    \
             stop <UNIT>         Stop a running service unit\n    \
             restart <UNIT>      Restart a service unit\n    \
             status [UNIT]       Show detailed status and recent logs for a unit\n    \
             list                List all system units and states\n    \
             reload <UNIT>       Reload configuration for a service unit\n    \
             enable <UNIT>       Enable a service unit\n    \
             disable <UNIT>      Disable a service unit\n    \
             boot [TARGET]       Execute deterministic parallel boot sequence\n    \
             shutdown            Graceful shutdown in reverse topological waves\n"
    );
}

pub fn handle_service_cmd(args: &[String]) {
    if args.is_empty() {
        print_service_usage();
        return;
    }

    let subcmd = args[0].as_str();
    let subargs = &args[1..];

    let mut mgr = ServiceManager::new();
    let _ = mgr.load_units_from_dir(Path::new("/etc/novad/system"));

    match subcmd {
        "-h" | "--help" | "help" => {
            print_service_usage();
        }
        "start" => {
            if subargs.is_empty() {
                eprintln!("novactl: service start requires <UNIT>");
                std::process::exit(1);
            }
            let unit = &subargs[0];
            match mgr.start_unit(unit) {
                Ok(_) => {
                    println!("[novactl] Successfully started service '{}'", unit);
                    display_unit_status(&mgr, unit);
                }
                Err(e) => {
                    eprintln!("novactl: error starting service '{}': {}", unit, e);
                    std::process::exit(1);
                }
            }
        }
        "stop" => {
            if subargs.is_empty() {
                eprintln!("novactl: service stop requires <UNIT>");
                std::process::exit(1);
            }
            let unit = &subargs[0];
            match mgr.stop_unit(unit) {
                Ok(_) => println!("[novactl] Successfully stopped service '{}'", unit),
                Err(e) => {
                    eprintln!("novactl: error stopping service '{}': {}", unit, e);
                    std::process::exit(1);
                }
            }
        }
        "restart" => {
            if subargs.is_empty() {
                eprintln!("novactl: service restart requires <UNIT>");
                std::process::exit(1);
            }
            let unit = &subargs[0];
            match mgr.restart_unit(unit) {
                Ok(_) => {
                    println!("[novactl] Successfully restarted service '{}'", unit);
                    display_unit_status(&mgr, unit);
                }
                Err(e) => {
                    eprintln!("novactl: error restarting service '{}': {}", unit, e);
                    std::process::exit(1);
                }
            }
        }
        "reload" => {
            if subargs.is_empty() {
                eprintln!("novactl: service reload requires <UNIT>");
                std::process::exit(1);
            }
            let unit = &subargs[0];
            match mgr.reload_unit(unit) {
                Ok(_) => println!("[novactl] Successfully reloaded service '{}'", unit),
                Err(e) => {
                    eprintln!("novactl: error reloading service '{}': {}", unit, e);
                    std::process::exit(1);
                }
            }
        }
        "status" => {
            if subargs.is_empty() {
                display_unit_list(&mgr);
            } else {
                display_unit_status(&mgr, &subargs[0]);
            }
        }
        "list" | "list-units" => {
            display_unit_list(&mgr);
        }
        "enable" => {
            if subargs.is_empty() {
                eprintln!("novactl: service enable requires <UNIT>");
                std::process::exit(1);
            }
            let unit = &subargs[0];
            match mgr.enable_unit(unit) {
                Ok(_) => println!("[novactl] Enabled service '{}'", unit),
                Err(e) => {
                    eprintln!("novactl: error enabling service '{}': {}", unit, e);
                    std::process::exit(1);
                }
            }
        }
        "disable" => {
            if subargs.is_empty() {
                eprintln!("novactl: service disable requires <UNIT>");
                std::process::exit(1);
            }
            let unit = &subargs[0];
            match mgr.disable_unit(unit) {
                Ok(_) => println!("[novactl] Disabled service '{}'", unit),
                Err(e) => {
                    eprintln!("novactl: error disabling service '{}': {}", unit, e);
                    std::process::exit(1);
                }
            }
        }
        "boot" => {
            let target = subargs.first().cloned().unwrap_or_else(|| "default.target".to_string());
            println!("[novactl] Booting to target '{}'...", target);
            match mgr.boot_target(&target) {
                Ok(waves) => {
                    println!("[novactl] Boot complete. Executed {} parallel waves:", waves.len());
                    for (i, wave) in waves.iter().enumerate() {
                        println!("  Wave {}: {}", i, wave.join(", "));
                    }
                }
                Err(e) => {
                    eprintln!("novactl: boot failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "shutdown" => {
            println!("[novactl] Initiating graceful reverse shutdown sequence...");
            let waves = mgr.shutdown();
            println!("[novactl] Shutdown complete. Executed {} reverse waves:", waves.len());
            for (i, wave) in waves.iter().enumerate() {
                println!("  Wave {}: {}", i, wave.join(", "));
            }
        }
        _ => {
            eprintln!("novactl: unknown service command '{}'", subcmd);
            print_service_usage();
            std::process::exit(1);
        }
    }
}

fn display_unit_status(mgr: &ServiceManager, unit_name: &str) {
    match mgr.status_unit(unit_name) {
        Ok(info) => {
            let state_color = match info.state {
                UnitState::Active => "\x1b[1;32mactive (running)\x1b[0m",
                UnitState::Activating => "\x1b[1;33mactivating\x1b[0m",
                UnitState::Failed => "\x1b[1;31mfailed\x1b[0m",
                UnitState::Deactivating => "\x1b[1;33mdeactivating\x1b[0m",
                UnitState::WaitingSocket => "\x1b[1;36mwaiting-socket\x1b[0m",
                _ => "inactive (dead)",
            };

            println!("● {}", info.name);
            println!("     Loaded: loaded (/etc/novad/system/{}; enabled)", info.name);
            println!("     Active: {} since boot; substate: {}", state_color, info.substate);
            if let Some(pid) = info.pid {
                println!("   Main PID: {} (active)", pid);
            }
            if info.restart_count > 0 {
                println!("   Restarts: {} (current backoff: {}s)", info.restart_count, info.backoff_sec);
            }
            if let Some(uptime) = info.uptime_sec {
                println!("     Uptime: {}s", uptime);
            }
            if let Some(code) = info.last_exit_code {
                println!("  Last Exit: {}", code);
            }
            if !info.recent_logs.is_empty() {
                println!("\nRecent Journal Log Output:");
                for line in &info.recent_logs {
                    println!("    {}", line);
                }
            }
        }
        Err(e) => {
            eprintln!("novactl service: error: {}", e);
            std::process::exit(1);
        }
    }
}

fn display_unit_list(mgr: &ServiceManager) {
    let units = mgr.list_units();
    println!("{:<24} {:<12} {:<16} {:<8} {}", "UNIT", "TYPE", "STATE", "PID", "DESCRIPTION");
    println!("{:-<24} {:-<12} {:-<16} {:-<8} {:-<30}", "", "", "", "", "");
    for u in units {
        let pid_str = u.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string());
        println!("{:<24} {:<12} {:<16} {:<8} {}", u.name, u.unit_type, u.state, pid_str, u.description);
    }
}
