//! userland/desktop/nova-shell/src/main.rs
//!
//! Entry point for NovaOS Desktop Shell standalone binary and selftest runner.

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--selftest" || arg == "-t") {
        println!("[*] Running nova-shell selftest...");
        if nova_shell::run_nova_shell_selftest() {
            println!("[+] nova-shell selftest: PASSED");
            process::exit(0);
        } else {
            eprintln!("[-] nova-shell selftest: FAILED");
            process::exit(1);
        }
    }

    println!("NovaOS Desktop Shell daemon v2.0.0");
    println!("Persistent unified macOS-style menu bar active.");
}
