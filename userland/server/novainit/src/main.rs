//! userland/server/novainit/src/main.rs
//!
//! CLI Entry Point for NovaOS Cloud-Init Automated Provisioning Engine.

use std::env;
use std::path::PathBuf;
use std::process;

use novainit::{
    run_cloud_init_selftest, CloudInitEngine, CloudInitStatus,
};

fn print_usage() {
    println!("NovaOS Cloud-Init Automated Provisioning Engine (novainit) v2.0.0");
    println!("Usage:");
    println!("  novainit init [--local] [--seed <dir>] [--force]  Execute early boot provisioning");
    println!("  novainit modules [--mode <config|final>]         Execute configuration modules");
    println!("  novainit status                                   Show cloud-init provisioning state");
    println!("  novainit query <key>                              Query parsed metadata or user-data");
    println!("  novainit --selftest                               Run autonomous qualification tests");
    println!("  novainit --help                                   Display this help text");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "--selftest" => {
            println!("[novainit] Running Cloud-Init Automated Provisioning Self-Test...");
            if run_cloud_init_selftest() {
                println!("[novainit] PASS: All 7 cloud-init self-test assertions passed 100%!");
                process::exit(0);
            } else {
                eprintln!("[novainit] FAIL: One or more self-test assertions failed.");
                process::exit(1);
            }
        }

        "--help" | "-h" => {
            print_usage();
            process::exit(0);
        }

        "--version" | "-v" => {
            println!("novainit v2.0.0 (NovaOS Cloud-Init Provisioning Engine)");
            process::exit(0);
        }

        "init" => {
            let mut seed_dir = None;
            let mut force = false;
            let mut is_local = false;

            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--seed" => {
                        if i + 1 < args.len() {
                            seed_dir = Some(args[i + 1].clone());
                            i += 1;
                        }
                    }
                    "--force" => force = true,
                    "--local" => is_local = true,
                    _ => {}
                }
                i += 1;
            }

            println!("[novainit] Starting cloud-init automated provisioning (local={})...", is_local);
            let mut engine = CloudInitEngine::new(PathBuf::from("/"), seed_dir);

            if is_local {
                match engine.load_and_parse() {
                    Ok(true) => {
                        if let Err(e) = engine.run_stage_local() {
                            eprintln!("[novainit] Local stage error: {}", e);
                            process::exit(1);
                        }
                        println!("[novainit] Local stage completed successfully.");
                    }
                    Ok(false) => {
                        println!("[novainit] No cloud datasource discovered; exiting gracefully.");
                    }
                    Err(e) => {
                        eprintln!("[novainit] Datasource parsing error: {}", e);
                        process::exit(1);
                    }
                }
            } else {
                match engine.run_all(force) {
                    Ok(CloudInitStatus::Done) => {
                        println!("[novainit] Cloud-init provisioning completed successfully.");
                    }
                    Ok(CloudInitStatus::Disabled) => {
                        println!("[novainit] Cloud-init disabled (no datasource found).");
                    }
                    Ok(other) => {
                        println!("[novainit] Cloud-init finished with state: {:?}", other);
                    }
                    Err(e) => {
                        eprintln!("[novainit] Provisioning failed: {}", e);
                        process::exit(1);
                    }
                }
            }
        }

        "modules" => {
            let mode = args.get(2).map(|s| s.as_str()).unwrap_or("config");
            println!("[novainit] Running cloud-init modules (mode: {})...", mode);

            let mut engine = CloudInitEngine::new(PathBuf::from("/"), None);
            if let Ok(true) = engine.load_and_parse() {
                if mode == "final" {
                    if let Err(e) = engine.run_stage_final() {
                        eprintln!("[novainit] Final stage error: {}", e);
                        process::exit(1);
                    }
                } else {
                    if let Err(e) = engine.run_stage_config() {
                        eprintln!("[novainit] Config stage error: {}", e);
                        process::exit(1);
                    }
                }
                println!("[novainit] Modules stage '{}' completed.", mode);
            } else {
                println!("[novainit] No active datasource for modules.");
            }
        }

        "status" => {
            let status_file = PathBuf::from("/run/cloud-init/status.json");
            if status_file.is_file() {
                let content = std::fs::read_to_string(&status_file).unwrap_or_default();
                println!("{}", content.trim());
            } else {
                let inst_id_file = PathBuf::from("/var/lib/cloud/instance/instance-id");
                if inst_id_file.is_file() {
                    let id = std::fs::read_to_string(&inst_id_file).unwrap_or_default();
                    println!("status: done\ninstance_id: {}", id.trim());
                } else {
                    println!("status: not-started");
                }
            }
        }

        "query" => {
            if args.len() < 3 {
                eprintln!("Error: 'query' requires a key (e.g. 'novainit query instance-id')");
                process::exit(1);
            }
            let key = &args[2];
            let mut engine = CloudInitEngine::new(PathBuf::from("/"), None);
            if let Ok(true) = engine.load_and_parse() {
                match key.as_str() {
                    "instance-id" | "instance_id" => println!("{}", engine.config.instance_id),
                    "hostname" | "local-hostname" => println!("{}", engine.config.hostname),
                    "users" => {
                        for u in &engine.config.users {
                            println!("{}", u.name);
                        }
                    }
                    _ => {
                        eprintln!("Unknown query key: {}", key);
                        process::exit(1);
                    }
                }
            } else {
                eprintln!("No cloud configuration active to query.");
                process::exit(1);
            }
        }

        other => {
            eprintln!("Unknown command: '{}'", other);
            print_usage();
            process::exit(1);
        }
    }
}
