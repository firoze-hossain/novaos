//! userland/desktop/apps/nova-settings/src/main.rs
//!
//! Entry point for NovaOS Unified System Settings & Control Hub (nova-settings).

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--selftest" || arg == "-t") {
        println!("[*] Running nova-settings selftest...");
        if nova_settings::run_nova_settings_selftest() {
            println!("[+] nova-settings selftest: PASSED");
            process::exit(0);
        } else {
            eprintln!("[-] nova-settings selftest: FAILED");
            process::exit(1);
        }
    }

    let mut app = nova_settings::NovaSettingsApp::new();

    // Check for --query argument
    if let Some(pos) = args.iter().position(|arg| arg == "--query" || arg == "-q") {
        if let Some(query) = args.get(pos + 1) {
            println!("[*] Searching settings for '{}'...", query);
            app.set_search_query(query);
            for (i, res) in app.search_results.iter().enumerate() {
                println!("  [{}] {} ({}) - score: {}", i + 1, res.title, res.breadcrumb, res.score);
            }
            process::exit(0);
        }
    }

    // Check for --panel argument
    if let Some(pos) = args.iter().position(|arg| arg == "--panel" || arg == "-p") {
        if let Some(panel_name) = args.get(pos + 1) {
            match panel_name.to_lowercase().as_str() {
                "system" => app.select_category(nova_settings::types::PanelCategory::System),
                "network" => app.select_category(nova_settings::types::PanelCategory::Network),
                "personalization" => app.select_category(nova_settings::types::PanelCategory::Personalization),
                "apps" => app.select_category(nova_settings::types::PanelCategory::Apps),
                "accounts" => app.select_category(nova_settings::types::PanelCategory::Accounts),
                "updates" => app.select_category(nova_settings::types::PanelCategory::Updates),
                _ => eprintln!("Unknown panel: {}", panel_name),
            }
        }
    }

    println!("NovaOS Unified System Settings & Control Hub (nova-settings v2.0.0)");
    println!("Windows 11 + macOS Synthesized Settings Architecture Active.");
    println!("Active panel: {}", app.active_category.title());
}
