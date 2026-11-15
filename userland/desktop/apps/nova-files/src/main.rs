//! userland/desktop/apps/nova-files/src/main.rs
//!
//! Entry point for NovaOS Next-Gen File Explorer (nova-files) and selftest runner.

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--selftest" || arg == "-t") {
        println!("[*] Running nova-files selftest...");
        if nova_files::run_nova_files_selftest() {
            println!("[+] nova-files selftest: PASSED");
            process::exit(0);
        } else {
            eprintln!("[-] nova-files selftest: FAILED");
            process::exit(1);
        }
    }

    println!("NovaOS Next-Gen File Explorer (nova-files v2.0.0)");
    println!("Finder + File Explorer Synthesized architecture active.");
}
