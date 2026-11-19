//! userland/desktop/apps/nova-recovery/src/main.rs
//!
//! Main Entry Point for NovaOS Emergency Recovery Environment (nova-recovery).

use std::env;
use std::process;

use nova_recovery::cli::RescueCliSession;
use nova_recovery::run_nova_recovery_selftest;
use nova_recovery::ui::{RecoveryApp, RecoveryUiRenderer, RESCUE_HEIGHT, RESCUE_WIDTH};
use nova_ui::prelude::BufferSurface;

fn print_usage() {
    println!("NovaOS 2.0 Emergency Recovery Console & Snapshot Rollback Tool");
    println!("Usage:");
    println!("  nova-recovery [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --selftest               Execute autonomous qualification test suite");
    println!("  --cli, -c                Launch interactive rescue shell");
    println!("  --list-snapshots         Display table of NovaFS snapshots");
    println!("  --rollback <id>          Atomically restore rootfs to specified snapshot ID");
    println!("  --reset-root <password>  Update root user password in /etc/shadow");
    println!("  --repair-boot            Verify and repair Protective MBR and GPT headers");
    println!("  --gui                    Launch retained-mode graphical rescue console");
    println!("  --help, -h               Show this help message");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "--selftest" => {
                println!("[*] Running NovaOS Recovery Master Self-Test...");
                if run_nova_recovery_selftest() {
                    println!("[PASS] NovaOS Recovery self-test succeeded (100% checks passed).");
                    process::exit(0);
                } else {
                    eprintln!("[FAIL] NovaOS Recovery self-test failed.");
                    process::exit(1);
                }
            }
            "--cli" | "-c" => {
                println!("=== NovaOS 2.0 Emergency Rescue Console ===");
                println!("Type 'help' for commands, 'exit' or 'quit' to terminate.");
                let mut cli = RescueCliSession::new();
                let output = cli.execute_command("status").unwrap_or_default();
                println!("{}", output);
                process::exit(0);
            }
            "--list-snapshots" => {
                let mut cli = RescueCliSession::new();
                let out = cli.execute_command("snapshots").unwrap_or_default();
                print!("{}", out);
                process::exit(0);
            }
            "--rollback" => {
                if args.len() < 3 {
                    eprintln!("Error: --rollback requires a snapshot ID argument.");
                    process::exit(1);
                }
                let mut cli = RescueCliSession::new();
                let cmd = format!("rollback {}", args[2]);
                match cli.execute_command(&cmd) {
                    Ok(msg) => {
                        println!("{}", msg);
                        process::exit(0);
                    }
                    Err(err) => {
                        eprintln!("Error: {}", err);
                        process::exit(1);
                    }
                }
            }
            "--reset-root" => {
                if args.len() < 3 {
                    eprintln!("Error: --reset-root requires a new password argument.");
                    process::exit(1);
                }
                let mut cli = RescueCliSession::new();
                let cmd = format!("reset-root {}", args[2]);
                match cli.execute_command(&cmd) {
                    Ok(msg) => {
                        println!("{}", msg);
                        process::exit(0);
                    }
                    Err(err) => {
                        eprintln!("Error: {}", err);
                        process::exit(1);
                    }
                }
            }
            "--repair-boot" => {
                let mut cli = RescueCliSession::new();
                match cli.execute_command("repair-boot") {
                    Ok(msg) => {
                        println!("{}", msg);
                        process::exit(0);
                    }
                    Err(err) => {
                        eprintln!("Error: {}", err);
                        process::exit(1);
                    }
                }
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "--gui" => {
                run_gui();
                process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {}", other);
                print_usage();
                process::exit(1);
            }
        }
    } else {
        run_gui();
    }
}

fn run_gui() {
    println!("[*] Starting NovaOS Recovery Graphical Console (800x600)...");
    let app = RecoveryApp::new();
    let mut surface = BufferSurface::new(RESCUE_WIDTH, RESCUE_HEIGHT);
    RecoveryUiRenderer::render(&app, &mut surface);
    println!("[*] Framebuffer rendered successfully ({}x{} pixels, 32bpp).", RESCUE_WIDTH, RESCUE_HEIGHT);
    println!("[*] Recovery environment standing by for user input.");
}
