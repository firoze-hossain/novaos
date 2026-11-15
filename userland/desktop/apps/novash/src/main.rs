//! userland/desktop/apps/novash/src/main.rs
//!
//! Entry point for NovaShell (novash) - Modern Interactive Rust Shell for NovaOS.

use std::env;
use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    // 1. Standalone Selftest Flag
    if args.iter().any(|arg| arg == "--selftest" || arg == "-t") {
        println!("[*] Running novash selftest...");
        if novash::run_novash_selftest() {
            println!("[+] novash selftest: PASSED");
            process::exit(0);
        } else {
            eprintln!("[-] novash selftest: FAILED");
            process::exit(1);
        }
    }

    let mut shell = novash::NovaShell::new();

    // 2. Execute single command (-c "command")
    if let Some(pos) = args.iter().position(|a| a == "-c") {
        if pos + 1 < args.len() {
            let cmd = &args[pos + 1];
            let res = shell.execute_line(cmd);
            if !res.stdout.is_empty() {
                print!("{}", res.stdout);
            }
            if !res.stderr.is_empty() {
                eprint!("{}", res.stderr);
            }
            process::exit(res.exit_code);
        } else {
            eprintln!("novash: option requires an argument -- 'c'");
            process::exit(2);
        }
    }

    // 3. Execute script file
    if args.len() > 1 && !args[1].starts_with('-') {
        let script_path = &args[1];
        match fs::read_to_string(script_path) {
            Ok(content) => {
                let code = shell.run_script(&content);
                process::exit(code);
            }
            Err(e) => {
                eprintln!("novash: cannot open file '{}': {}", script_path, e);
                process::exit(1);
            }
        }
    }

    // 4. Interactive Greeting
    println!("NovaShell (novash v2.0.0) - Modern Interactive Rust Shell");
    println!("Fish/Zsh-grade syntax highlighting, autosuggestions & git status active.");
    println!("Type 'help', 'history', or press Tab to explore.\n");
}
