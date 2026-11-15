//! userland/desktop/apps/nova-store/src/main.rs
//!
//! Entry point for NovaOS Graphical Software Store & Package Manager (nova-store).

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--selftest" || arg == "-t") {
        println!("[*] Running nova-store selftest...");
        if nova_store::run_nova_store_selftest() {
            println!("[+] nova-store selftest: PASSED");
            process::exit(0);
        } else {
            eprintln!("[-] nova-store selftest: FAILED");
            process::exit(1);
        }
    }

    let mut app = nova_store::NovaStoreApp::new();

    // Check for --install
    if let Some(pos) = args.iter().position(|arg| arg == "--install" || arg == "-i") {
        if let Some(id) = args.get(pos + 1) {
            println!("[*] Installing package '{}'...", id);
            match app.install(id) {
                Ok(installed) => {
                    println!("[+] Successfully installed: {}", installed.join(", "));
                    process::exit(0);
                }
                Err(err) => {
                    eprintln!("[-] Installation failed: {}", err);
                    process::exit(1);
                }
            }
        }
    }

    // Check for --search
    if let Some(pos) = args.iter().position(|arg| arg == "--search" || arg == "-s") {
        if let Some(query) = args.get(pos + 1) {
            println!("[*] Searching packages for '{}'...", query);
            let results = app.repo.search(query);
            for pkg in results {
                println!("  - {} ({}) v{} [{}]", pkg.name, pkg.id, pkg.version, pkg.category.title());
                println!("    {}", pkg.summary);
            }
            process::exit(0);
        }
    }

    // Check for --list
    if args.iter().any(|arg| arg == "--list" || arg == "-l") {
        println!("NovaOS Software Store - Available Packages (Total: {})", app.repo.packages.len());
        for pkg in app.repo.packages.values() {
            let status_str = if pkg.is_installed() { "[INSTALLED]" } else { "" };
            println!("  * {:<24} v{:<8} {:<10} {}", pkg.name, pkg.version, status_str, pkg.summary);
        }
        process::exit(0);
    }

    println!("NovaOS Graphical Software Store & Package Manager (nova-store v2.0.0)");
    println!("Ubuntu Software Center + macOS App Store Synthesized Architecture Active.");
    println!("Repository: {}", app.repo.repo_url);
    println!("Curated packages: {}", app.repo.packages.len());
}
