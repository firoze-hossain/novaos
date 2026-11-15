//! userland/desktop/apps/nova-term/src/main.rs
//!
//! Entry point for NovaOS Pro Developer Terminal (nova-term) and selftest runner.

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--selftest" || arg == "-t") {
        println!("[*] Running nova-term selftest...");
        if nova_term::run_nova_term_selftest() {
            println!("[+] nova-term selftest: PASSED");
            process::exit(0);
        } else {
            eprintln!("[-] nova-term selftest: FAILED");
            process::exit(1);
        }
    }

    println!("NovaOS Pro Developer Terminal (nova-term v2.0.0)");
    println!("GPU TrueColor & VT100 / xterm engine active.");
}
