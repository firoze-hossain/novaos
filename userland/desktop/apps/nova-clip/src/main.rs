//! userland/desktop/apps/nova-clip/src/main.rs
//!
//! # NovaOS Clipboard CLI & Self-Test Qualification Runner
//!
//! Provides the command-line interface, telemetry status printer,
//! and standalone self-test runner for `nova-clip`.

use std::env;
use std::process::exit;

use nova_clip::model::{create_native_clipboard, MnemosHub};
use nova_clip::run_nova_clip_selftest;

fn print_usage() {
    println!("NovaOS Sovereign Clipboard (nova-clip / Mnemos) v1.0.0");
    println!("Synthesizing Apple macOS Maccy with Sovereign NovaOS Ring-3 IPC");
    println!();
    println!("Usage:");
    println!("  nova-clip                        Display active clipboard history & telemetry");
    println!("  nova-clip --status               Print detailed status of clipboard subsystems");
    println!("  nova-clip --selftest             Execute autonomous qualification test suite");
    println!("  nova-clip --search <QUERY>       Search history with fuzzy ranking");
    println!("  nova-clip --copy <TEXT>          Copy string into system clipboard");
    println!("  nova-clip --paste                Print current clipboard contents");
    println!("  nova-clip --help                 Display this reference manual");
    println!();
    println!("Subsystems:");
    println!("  1. Translucent Acrylic Card: 360px floating window with rounded corners");
    println!("  2. Instant Fuzzy Search: Prefix, substring & acronym ranking engine");
    println!("  3. Inline Color Swatches: #284058 hex extraction with square preview");
    println!("  4. Flyout Preview Popover: Application source, timestamps & full text");
    println!("  5. Numeric Shortcuts: Direct pasting with keys 1..9");
    println!("  6. Privacy Filter: Excludes password managers and confidential tokens");
}

fn print_status() {
    let hub = MnemosHub::with_maccy_demo();
    let backend = create_native_clipboard();

    println!("=== NOVAOS CLIPBOARD (MNEMOS) TELEMETRY ===");
    println!("Active Backend:       {}", backend.backend_name());
    println!("History Count:        {} items", hub.history.items.len());
    let pinned = hub.history.items.iter().filter(|i| i.pinned).count();
    println!("Pinned Items:         {}", pinned);
    println!("Max Storage Capacity: {} items", hub.config.max_history_size);
    println!("Show Flyout Preview:  {}", hub.config.show_preview_panel);
    println!("Paste Automatically:  {}", hub.config.paste_on_select);
    println!("Ignored Applications: {}", hub.history.ignored_apps.join(", "));
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_usage();
                exit(0);
            }
            "--selftest" => {
                println!("[*] Launching NovaOS Clipboard Autonomous Qualification Suite...");
                let ok = run_nova_clip_selftest();
                if ok {
                    println!("[PASS] NovaOS Clipboard (nova-clip / Mnemos) passed all qualification checks.");
                    exit(0);
                } else {
                    eprintln!("[FAIL] One or more nova-clip tests failed.");
                    exit(1);
                }
            }
            "--status" => {
                print_status();
                exit(0);
            }
            "--search" => {
                if args.len() > 2 {
                    let mut hub = MnemosHub::with_maccy_demo();
                    hub.set_search(&args[2]);
                    let items = hub.filtered_items();
                    println!("Matches for '{}':", args[2]);
                    for (i, item) in items.iter().take(9).enumerate() {
                        println!("  [{}] {}: {}", i + 1, item.source_app, item.preview_line(40));
                    }
                    exit(0);
                }
            }
            "--copy" => {
                if args.len() > 2 {
                    let mut clip = create_native_clipboard();
                    clip.write_text(&args[2]);
                    println!("[nova-clip] Copied {} characters.", args[2].len());
                    exit(0);
                }
            }
            "--paste" => {
                let mut clip = create_native_clipboard();
                if let Some(text) = clip.read_text() {
                    print!("{}", text);
                }
                exit(0);
            }
            _ => {}
        }
    }

    print_status();
}
