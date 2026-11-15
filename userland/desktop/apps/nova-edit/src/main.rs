//! userland/desktop/apps/nova-edit/src/main.rs
//!
//! Entry point for NovaOS Code & Text Editor (nova-edit).

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--selftest" || arg == "-t") {
        println!("[*] Running nova-edit selftest...");
        if nova_edit::run_nova_edit_selftest() {
            println!("[+] nova-edit selftest: PASSED");
            process::exit(0);
        } else {
            eprintln!("[-] nova-edit selftest: FAILED");
            process::exit(1);
        }
    }

    let mut app = nova_edit::NovaEditApp::new();

    // Check if a file was passed as argument
    if let Some(path) = args.iter().skip(1).find(|arg| !arg.starts_with('-')) {
        println!("[*] Opening file: {}", path);
        app.open_file(path, "// Empty file\n");
    }

    println!("NovaOS Code & Text Editor (nova-edit v2.0.0)");
    println!("Piece-Table Buffer & Multi-Language Syntax Engine Active.");
    println!("Active document: {}", app.tabs.active().path);
}
