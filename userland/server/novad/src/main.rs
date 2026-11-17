//! userland/server/novad/src/main.rs
//!
//! Pure Rust Systemd-Style Service & Daemon Orchestrator CLI (novad).

use std::env;
use std::path::Path;
use std::process;

use novad::{run_novad_selftest, version, ServiceManager, UnitState};

fn print_usage() {
    println!(
        "NovaOS Service & Daemon Orchestrator (novad v2.0.0)\n\n\
         USAGE:\n    \
             novad <COMMAND> [OPTIONS]\n\n\
         COMMANDS:\n    \
             daemon              Run supervisor daemon / PID 1 init orchestrator\n    \
             start <UNIT>        Activate specified service or target unit\n    \
             stop <UNIT>         Deactivate specified unit\n    \
             restart <UNIT>      Restart specified unit\n    \
             reload <UNIT>       Reload configuration for specified unit\n    \
             status [UNIT]       Show runtime status and logs for unit\n    \
             list-units          List all known units and operational states\n    \
             boot [TARGET]       Execute deterministic parallel boot to target\n    \
             shutdown            Graceful shutdown in reverse topological waves\n    \
             notify <UNIT> <MSG> Send sd_notify signal (e.g. READY=1, WATCHDOG=1)\n    \
             selftest            Execute comprehensive orchestrator verification\n\n\
         OPTIONS:\n    \
             -u, --units-dir <PATH> Load additional units from directory\n    \
             -h, --help            Print help information\n    \
             -v, --version         Print version information\n    \
             --selftest            Execute self-test suite\n"
    );
}

fn format_status(mgr: &ServiceManager, unit_name: &str) {
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
            eprintln!("novad: error: {}", e);
            process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    let mut units_dir: Option<String> = None;
    let mut command: Option<String> = None;
    let mut cmd_args: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "-h" || arg == "--help" {
            print_usage();
            return;
        } else if arg == "-v" || arg == "--version" {
            println!("{}", version());
            return;
        } else if arg == "--selftest" || arg == "selftest" {
            if run_novad_selftest() {
                process::exit(0);
            } else {
                eprintln!("[novad] Self-test failed!");
                process::exit(1);
            }
        } else if arg == "-u" || arg == "--units-dir" {
            if i + 1 < args.len() {
                units_dir = Some(args[i + 1].clone());
                i += 1;
            }
        } else if command.is_none() {
            command = Some(arg.clone());
        } else {
            cmd_args.push(arg.clone());
        }
        i += 1;
    }

    let cmd = match command {
        Some(c) => c,
        None => {
            print_usage();
            return;
        }
    };

    let mut mgr = ServiceManager::new();
    if let Some(dir) = units_dir {
        let p = Path::new(&dir);
        let _ = mgr.load_units_from_dir(p);
    }
    // Check standard /etc/novad/system if present
    let _ = mgr.load_units_from_dir(Path::new("/etc/novad/system"));

    match cmd.as_str() {
        "start" => {
            if cmd_args.is_empty() {
                eprintln!("novad: start requires <UNIT>");
                process::exit(1);
            }
            let unit = &cmd_args[0];
            match mgr.start_unit(unit) {
                Ok(_) => {
                    println!("[novad] Successfully started unit '{}'", unit);
                    format_status(&mgr, unit);
                }
                Err(e) => {
                    eprintln!("novad: failed to start unit '{}': {}", unit, e);
                    process::exit(1);
                }
            }
        }
        "stop" => {
            if cmd_args.is_empty() {
                eprintln!("novad: stop requires <UNIT>");
                process::exit(1);
            }
            let unit = &cmd_args[0];
            match mgr.stop_unit(unit) {
                Ok(_) => println!("[novad] Successfully stopped unit '{}'", unit),
                Err(e) => {
                    eprintln!("novad: failed to stop unit '{}': {}", unit, e);
                    process::exit(1);
                }
            }
        }
        "restart" => {
            if cmd_args.is_empty() {
                eprintln!("novad: restart requires <UNIT>");
                process::exit(1);
            }
            let unit = &cmd_args[0];
            match mgr.restart_unit(unit) {
                Ok(_) => {
                    println!("[novad] Successfully restarted unit '{}'", unit);
                    format_status(&mgr, unit);
                }
                Err(e) => {
                    eprintln!("novad: failed to restart unit '{}': {}", unit, e);
                    process::exit(1);
                }
            }
        }
        "reload" => {
            if cmd_args.is_empty() {
                eprintln!("novad: reload requires <UNIT>");
                process::exit(1);
            }
            let unit = &cmd_args[0];
            match mgr.reload_unit(unit) {
                Ok(_) => println!("[novad] Successfully reloaded unit '{}'", unit),
                Err(e) => {
                    eprintln!("novad: failed to reload unit '{}': {}", unit, e);
                    process::exit(1);
                }
            }
        }
        "status" => {
            if cmd_args.is_empty() {
                let units = mgr.list_units();
                println!("{:<24} {:<12} {:<16} {:<8} {}", "UNIT", "TYPE", "STATE", "PID", "DESCRIPTION");
                println!("{:-<24} {:-<12} {:-<16} {:-<8} {:-<30}", "", "", "", "", "");
                for u in units {
                    let pid_str = u.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string());
                    println!("{:<24} {:<12} {:<16} {:<8} {}", u.name, u.unit_type, u.state, pid_str, u.description);
                }
            } else {
                format_status(&mgr, &cmd_args[0]);
            }
        }
        "list-units" | "list" => {
            let units = mgr.list_units();
            println!("{:<24} {:<12} {:<16} {:<8} {}", "UNIT", "TYPE", "STATE", "PID", "DESCRIPTION");
            println!("{:-<24} {:-<12} {:-<16} {:-<8} {:-<30}", "", "", "", "", "");
            for u in units {
                let pid_str = u.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string());
                println!("{:<24} {:<12} {:<16} {:<8} {}", u.name, u.unit_type, u.state, pid_str, u.description);
            }
        }
        "boot" => {
            let target = cmd_args.first().cloned().unwrap_or_else(|| "default.target".to_string());
            println!("[novad] Initiating boot sequence for target '{}'...", target);
            match mgr.boot_target(&target) {
                Ok(waves) => {
                    println!("[novad] Boot complete. Executed {} parallel waves:", waves.len());
                    for (idx, wave) in waves.iter().enumerate() {
                        println!("  Wave {}: {}", idx, wave.join(", "));
                    }
                }
                Err(e) => {
                    eprintln!("novad: boot failed: {}", e);
                    process::exit(1);
                }
            }
        }
        "shutdown" => {
            println!("[novad] Initiating graceful shutdown sequence...");
            let waves = mgr.shutdown();
            println!("[novad] Shutdown complete. Executed {} reverse waves:", waves.len());
            for (idx, wave) in waves.iter().enumerate() {
                println!("  Wave {}: {}", idx, wave.join(", "));
            }
        }
        "notify" => {
            if cmd_args.len() < 2 {
                eprintln!("novad: notify requires <UNIT_OR_PID> <MESSAGE>");
                process::exit(1);
            }
            let ident = &cmd_args[0];
            let msg = &cmd_args[1];
            match mgr.sd_notify(ident, msg) {
                Ok(_) => println!("[novad] Delivered notify message to '{}'", ident),
                Err(e) => {
                    eprintln!("novad: notify error: {}", e);
                    process::exit(1);
                }
            }
        }
        "daemon" => {
            println!("[novad] Starting NovaOS Service Orchestrator (PID 1 Init Daemon)...");
            let target = "default.target";
            match mgr.boot_target(target) {
                Ok(waves) => {
                    println!("[novad] System online. Parallel waves: {}", waves.len());
                    // In simulation / test environment, perform a step of supervision
                    mgr.supervise_step(1);
                    println!("[novad] Daemon supervisor active. Managing {} units.", mgr.units.len());
                }
                Err(e) => {
                    eprintln!("novad: daemon startup failed: {}", e);
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("novad: unknown command '{}'", cmd);
            print_usage();
            process::exit(1);
        }
    }
}
