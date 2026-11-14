//! userland/desktop/nova-search/src/main.rs
//!
//! Entry point for NovaOS Spotlight Search daemon and selftest runner.

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--selftest" || arg == "-t") {
        println!("[*] Running nova-search selftest...");
        if nova_search::run_nova_search_selftest() {
            println!("[+] nova-search selftest: PASSED");
            process::exit(0);
        } else {
            eprintln!("[-] nova-search selftest: FAILED");
            process::exit(1);
        }
    }

    println!("NovaOS Spotlight Instant Search daemon v2.0.0");
    println!("Shortcut: Super+Space / Ctrl+Space active.");
}
