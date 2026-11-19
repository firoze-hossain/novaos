//! userland/desktop/apps/nova-installer/src/main.rs
//!
//! Entry point for NovaOS Production Graphical Installer (nova-installer)
//! and automated selftest dispatcher.

use std::env;
use std::process;

use nova_installer::{
    run_nova_installer_selftest, InstallerWindowRenderer, InstallerWizardApp,
};

fn print_usage() {
    println!("NovaOS Production Graphical Installer (nova-installer) v2.0.0");
    println!("Usage:");
    println!("  nova-installer [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --selftest, -t       Run comprehensive autonomous qualification self-test");
    println!("  --dry-run            Run installer wizard in simulation mode (no disk writes)");
    println!("  --help, -h           Show this help message");
    println!("  --version, -v        Display application version");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    for arg in &args[1..] {
        match arg.as_str() {
            "--selftest" | "-t" => {
                println!("[*] Running nova-installer autonomous qualification self-test...");
                if run_nova_installer_selftest() {
                    println!("[+] PASS: All 8 nova-installer qualification self-test checks passed 100%!");
                    process::exit(0);
                } else {
                    eprintln!("[-] FAIL: One or more installer self-test assertions failed.");
                    process::exit(1);
                }
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "--version" | "-v" => {
                println!("nova-installer v2.0.0 (NovaOS Production Graphical Installer)");
                process::exit(0);
            }
            "--dry-run" => {
                println!("[*] Initializing nova-installer in dry-run simulation mode...");
            }
            _ => {}
        }
    }

    println!("================================================================================");
    println!("        NovaOS 2.0 - Production Graphical Installer (nova-installer)           ");
    println!("================================================================================");
    println!("Starting modern Out-of-Box Experience (OOBE) setup wizard...");

    let app = InstallerWizardApp::new();
    let mut surface = nova_ui::prelude::BufferSurface::new(1024, 700);
    InstallerWindowRenderer::render(&app, &mut surface);

    println!("[+] Initialized installer window buffer ({}x{} pixels).", surface.width, surface.height);
    println!("[+] Selected Target Disk: {}", app.partition_wizard.selected_disk().map(|d| d.path.as_str()).unwrap_or("none"));
    println!("[+] Active Step: {:?}", app.current_step);
    println!("[+] Ready for user input.");
}
