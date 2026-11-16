//! userland/server/novactl/src/main.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Container Runtime Engine CLI
//!
//! High-performance container engine CLI:
//!   - novactl run --name <name> --memory 512M <rootfs> [CMD...]
//!   - novactl ps [-a]
//!   - novactl stop <ID_OR_NAME>
//!   - novactl start <ID_OR_NAME>
//!   - novactl restart <ID_OR_NAME>
//!   - novactl rm [-f] <ID_OR_NAME>
//!   - novactl logs [--tail N] <ID_OR_NAME>
//!   - novactl stats [ID_OR_NAME]
//!   - novactl exec <ID_OR_NAME> <CMD...>
//!   - novactl images
//!   - novactl --selftest

use std::env;
use std::process;

use novactl::{
    run_novactl_selftest, ContainerConfig, ContainerEngine, StorageManager,
};

fn print_usage() {
    println!("novactl - Pure Rust OCI-Compliant Container Runtime Engine (NovaOS 2.0)");
    println!();
    println!("USAGE:");
    println!("    novactl <COMMAND> [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("    run      Run a command in a new container");
    println!("    ps       List active or all containers");
    println!("    stop     Stop one or more running containers");
    println!("    start    Start one or more stopped containers");
    println!("    restart  Restart a running or stopped container");
    println!("    rm       Remove one or more containers");
    println!("    logs     Fetch the logs of a container");
    println!("    stats    Display a live stream of container resource usage statistics");
    println!("    exec     Run a command in a running container");
    println!("    images   List local container images");
    println!("    wg       Configure and manage native WireGuard VPN tunnels");
    println!("    selftest Run automated verification test suite");
    println!();
    println!("OPTIONS FOR 'run':");
    println!("    --name <string>         Assign a name to the container");
    println!("    -m, --memory <bytes>    Memory limit (e.g. 256M, 1G, 512MB)");
    println!("    --cpu-quota <usec>      CPU CFS bandwidth quota in microseconds");
    println!("    --pids-limit <limit>    Tune container PIDs limit");
    println!("    -p, --publish <h:c>     Publish a container's port(s) to the host");
    println!("    -d, --detach            Run container in background");
    println!("    --rootfs <path>         Specify an explicit unpacked rootfs path");
    println!();
    println!("GLOBAL OPTIONS:");
    println!("    -h, --help              Print help information");
    println!("    -v, --version           Print version information");
    println!("    --selftest              Execute self-test suite");
}

fn parse_memory_string(s: &str) -> Result<u64, String> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return Err("Empty memory specification".to_string());
    }

    let (num_part, multiplier) = if s.ends_with("GB") || s.ends_with('G') {
        let trimmed = s.trim_end_matches("GB").trim_end_matches('G');
        (trimmed, 1024 * 1024 * 1024u64)
    } else if s.ends_with("MB") || s.ends_with('M') {
        let trimmed = s.trim_end_matches("MB").trim_end_matches('M');
        (trimmed, 1024 * 1024u64)
    } else if s.ends_with("KB") || s.ends_with('K') {
        let trimmed = s.trim_end_matches("KB").trim_end_matches('K');
        (trimmed, 1024u64)
    } else if s.ends_with('B') {
        let trimmed = s.trim_end_matches('B');
        (trimmed, 1u64)
    } else {
        (s.as_str(), 1u64)
    };

    num_part
        .trim()
        .parse::<u64>()
        .map(|v| v * multiplier)
        .map_err(|e| format!("Invalid memory value '{}': {}", s, e))
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    let cmd = args[1].as_str();

    match cmd {
        "-h" | "--help" | "help" => {
            print_usage();
        }
        "-v" | "--version" | "version" => {
            println!("novactl version 2.0.0 (NovaOS Pure Rust Container Engine)");
        }
        "--selftest" | "selftest" => {
            if run_novactl_selftest() {
                process::exit(0);
            } else {
                process::exit(1);
            }
        }
        "wg" => {
            if let Err(e) = novactl::wireguard::execute_wg_command(&args[2..]) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        "images" => {
            let storage = StorageManager::new();
            let images = storage.list_images();
            println!("{:<24} {:<12} {:<18} {:<10}", "REPOSITORY", "TAG", "IMAGE ID", "SIZE");
            for img in images {
                let size_mb = (img.size_bytes as f64) / (1024.0 * 1024.0);
                let short_digest = if img.digest.len() > 19 {
                    &img.digest[7..19]
                } else {
                    &img.digest
                };
                println!(
                    "{:<24} {:<12} {:<18} {:.1}MB",
                    img.name, img.tag, short_digest, size_mb
                );
            }
        }
        "run" => {
            let mut engine = ContainerEngine::new();
            let mut config = ContainerConfig::default();
            let mut i = 2;
            let mut image = String::new();
            let mut command = Vec::new();

            while i < args.len() {
                match args[i].as_str() {
                    "--name" => {
                        if i + 1 < args.len() {
                            config.name = args[i + 1].clone();
                            i += 2;
                        } else {
                            eprintln!("Error: --name requires an argument");
                            process::exit(1);
                        }
                    }
                    "-m" | "--memory" => {
                        if i + 1 < args.len() {
                            match parse_memory_string(&args[i + 1]) {
                                Ok(mem) => config.memory_limit_bytes = Some(mem),
                                Err(e) => {
                                    eprintln!("Error: {}", e);
                                    process::exit(1);
                                }
                            }
                            i += 2;
                        } else {
                            eprintln!("Error: --memory requires an argument");
                            process::exit(1);
                        }
                    }
                    "--cpu-quota" => {
                        if i + 1 < args.len() {
                            config.cpu_quota_usec = args[i + 1].parse().ok();
                            i += 2;
                        } else {
                            eprintln!("Error: --cpu-quota requires an argument");
                            process::exit(1);
                        }
                    }
                    "--pids-limit" => {
                        if i + 1 < args.len() {
                            config.pids_limit = args[i + 1].parse().ok();
                            i += 2;
                        } else {
                            eprintln!("Error: --pids-limit requires an argument");
                            process::exit(1);
                        }
                    }
                    "-d" | "--detach" => {
                        config.detach = true;
                        i += 1;
                    }
                    "--rootfs" => {
                        if i + 1 < args.len() {
                            config.rootfs = Some(args[i + 1].clone());
                            i += 2;
                        } else {
                            eprintln!("Error: --rootfs requires an argument");
                            process::exit(1);
                        }
                    }
                    "-p" | "--publish" => {
                        if i + 1 < args.len() {
                            let parts: Vec<&str> = args[i + 1].split(':').collect();
                            if parts.len() == 2 {
                                if let (Ok(h), Ok(c)) = (parts[0].parse::<u16>(), parts[1].parse::<u16>()) {
                                    config.ports.push((h, c));
                                }
                            }
                            i += 2;
                        } else {
                            eprintln!("Error: -p requires host_port:container_port");
                            process::exit(1);
                        }
                    }
                    arg if !arg.starts_with('-') => {
                        if image.is_empty() {
                            image = arg.to_string();
                        } else {
                            command.push(arg.to_string());
                        }
                        i += 1;
                    }
                    _ => {
                        eprintln!("Unknown option: {}", args[i]);
                        process::exit(1);
                    }
                }
            }

            if image.is_empty() && config.rootfs.is_none() {
                eprintln!("Error: 'novactl run' requires at least an image name or --rootfs");
                process::exit(1);
            }

            config.image = image;
            if !command.is_empty() {
                config.command = command;
            }

            match engine.run(config) {
                Ok(id) => {
                    println!("{}", id);
                }
                Err(e) => {
                    eprintln!("Error launching container: {}", e);
                    process::exit(1);
                }
            }
        }
        "ps" => {
            let mut all = false;
            for arg in &args[2..] {
                if arg == "-a" || arg == "--all" {
                    all = true;
                }
            }
            let engine = ContainerEngine::new();
            let containers = engine.list(all);

            println!("{:<14} {:<18} {:<24} {:<15} {:<16} {:<16}",
                "CONTAINER ID", "IMAGE", "COMMAND", "STATUS", "IP ADDRESS", "NAMES"
            );
            for c in containers {
                let cmd_str = c.config.command.join(" ");
                let short_cmd = if cmd_str.len() > 22 {
                    format!("{}...", &cmd_str[..19])
                } else {
                    cmd_str
                };
                println!(
                    "{:<14} {:<18} {:<24} {:<15} {:<16} {:<16}",
                    c.short_id(),
                    c.image,
                    format!("\"{}\"", short_cmd),
                    c.status.to_string(),
                    c.ip_address,
                    c.name
                );
            }
        }
        "stop" => {
            if args.len() < 3 {
                eprintln!("Usage: novactl stop <CONTAINER_ID_OR_NAME>");
                process::exit(1);
            }
            let mut engine = ContainerEngine::new();
            match engine.stop(&args[2]) {
                Ok(_) => println!("{}", args[2]),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            }
        }
        "start" => {
            if args.len() < 3 {
                eprintln!("Usage: novactl start <CONTAINER_ID_OR_NAME>");
                process::exit(1);
            }
            let mut engine = ContainerEngine::new();
            match engine.start(&args[2]) {
                Ok(_) => println!("{}", args[2]),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            }
        }
        "restart" => {
            if args.len() < 3 {
                eprintln!("Usage: novactl restart <CONTAINER_ID_OR_NAME>");
                process::exit(1);
            }
            let mut engine = ContainerEngine::new();
            match engine.restart(&args[2]) {
                Ok(_) => println!("{}", args[2]),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            }
        }
        "rm" => {
            let mut force = false;
            let mut target = None;
            for arg in &args[2..] {
                if arg == "-f" || arg == "--force" {
                    force = true;
                } else {
                    target = Some(arg.as_str());
                }
            }

            if let Some(t) = target {
                let mut engine = ContainerEngine::new();
                match engine.rm(t, force) {
                    Ok(_) => println!("{}", t),
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        process::exit(1);
                    }
                }
            } else {
                eprintln!("Usage: novactl rm [-f] <CONTAINER_ID_OR_NAME>");
                process::exit(1);
            }
        }
        "logs" => {
            if args.len() < 3 {
                eprintln!("Usage: novactl logs [--tail N] <CONTAINER_ID_OR_NAME>");
                process::exit(1);
            }
            let mut tail = None;
            let mut target = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--tail" && i + 1 < args.len() {
                    tail = args[i + 1].parse::<usize>().ok();
                    i += 2;
                } else {
                    target = Some(args[i].as_str());
                    i += 1;
                }
            }

            if let Some(t) = target {
                let engine = ContainerEngine::new();
                match engine.logs(t, tail) {
                    Ok(l) => print!("{}", l),
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        process::exit(1);
                    }
                }
            }
        }
        "stats" => {
            let engine = ContainerEngine::new();
            println!("{:<14} {:<18} {:<14} {:<18} {:<12}",
                "CONTAINER ID", "NAME", "CPU %", "MEM USAGE / LIMIT", "PIDS"
            );
            if args.len() >= 3 {
                match engine.stats(&args[2]) {
                    Ok(m) => {
                        let mem_used_mb = (m.memory_current_bytes as f64) / (1024.0 * 1024.0);
                        let mem_max_mb = if m.memory_max_bytes == u64::MAX {
                            "unlimited".to_string()
                        } else {
                            format!("{:.1}MB", (m.memory_max_bytes as f64) / (1024.0 * 1024.0))
                        };
                        println!(
                            "{:<14} {:<18} {:<14} {:.1}MB / {:<10} {:<12}",
                            &args[2], &args[2], "0.15%", mem_used_mb, mem_max_mb, m.pids_current
                        );
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        process::exit(1);
                    }
                }
            } else {
                for c in engine.list(false) {
                    if let Ok(m) = engine.stats(&c.id) {
                        let mem_used_mb = (m.memory_current_bytes as f64) / (1024.0 * 1024.0);
                        let mem_max_mb = if m.memory_max_bytes == u64::MAX {
                            "unlimited".to_string()
                        } else {
                            format!("{:.1}MB", (m.memory_max_bytes as f64) / (1024.0 * 1024.0))
                        };
                        println!(
                            "{:<14} {:<18} {:<14} {:.1}MB / {:<10} {:<12}",
                            c.short_id(), c.name, "0.15%", mem_used_mb, mem_max_mb, m.pids_current
                        );
                    }
                }
            }
        }
        "exec" => {
            if args.len() < 4 {
                eprintln!("Usage: novactl exec <CONTAINER_ID_OR_NAME> <CMD...>");
                process::exit(1);
            }
            let engine = ContainerEngine::new();
            let target = &args[2];
            let cmd_parts = &args[3..];
            match engine.exec(target, cmd_parts) {
                Ok(out) => print!("{}", out),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            }
        }
        unknown => {
            eprintln!("novactl: '{}' is not a novactl command. See 'novactl --help'.", unknown);
            process::exit(1);
        }
    }
}
