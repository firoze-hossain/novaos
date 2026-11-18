//! sdk/cargo-nova/src/main.rs
//!
//! Official CLI tool & project orchestrator for NovaOS Developer SDK.
//! Usage: `cargo nova <subcommand>` or `cargo-nova <subcommand>`.

mod builder;
mod packager;
mod selftest;
mod simulator;
mod template;

use builder::{AppBuilder, BuildOptions};
use packager::{AppPackager, PackageOptions};
use simulator::{AppRunner, RunOptions};
use std::env;
use std::path::{Path, PathBuf};
use template::{ProjectGenerator, TemplateKind};

const VERSION: &str = "2.0.0";

fn print_help() {
    println!(
        r#"cargo-nova {} - Official NovaOS Developer SDK CLI Tool

USAGE:
    cargo nova <SUBCOMMAND> [OPTIONS]
    cargo-nova <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    new <NAME> [--template <gui|cli>]   Create a new NovaOS application project
    build [--release] [--target <T>]   Compile project for NovaOS
    package [--release] [--out <DIR>]   Package app into a self-contained .app bundle
    run [--emulator] [--release]        Run app in host simulator or QEMU emulator
    info                                Display SDK version, environment, and paths
    --selftest                          Run automated end-to-end SDK verification
    help                                Show this help message

EXAMPLES:
    cargo nova new my-calc --template gui
    cd my-calc && cargo nova build
    cargo nova package --release
    cargo nova run
"#,
        VERSION
    );
}

fn print_info() {
    println!("=== NovaOS Developer SDK Environment ===");
    println!("SDK Version:        {}", VERSION);
    println!("Framework:          novaos-sdk (pure Rust, retained-mode UI)");
    println!("Bundle Format:      .app (Info.json, Executable, Assets, Resources)");
    println!("Supported Targets:  i686-novaos, x86_64-novaos, host-simulator");
    println!("Default UI Theme:   Fluent Acrylic Dark Mode (WCAG AA compliant)");
    println!("Host Simulator:     Active (BufferSurface, SceneGraph, EventDispatcher)");
    println!("QEMU Emulator:      novaos.iso (Headless & Graphical)");
}

fn main() {
    let mut args: Vec<String> = env::args().collect();

    // Strip cargo prefix if invoked as `cargo nova ...`
    if args.len() > 1 && args[1] == "nova" {
        args.remove(1);
    }

    if args.len() < 2 {
        print_help();
        return;
    }

    let subcommand = args[1].as_str();

    match subcommand {
        "--help" | "-h" | "help" => {
            print_help();
        }
        "--version" | "-v" | "version" => {
            println!("cargo-nova {}", VERSION);
        }
        "info" => {
            print_info();
        }
        "--selftest" | "selftest" | "test" => {
            println!("Running cargo-nova end-to-end self-test...");
            if selftest::run_cargo_nova_selftest() {
                println!("[OK] cargo-nova self-test completed successfully (100% pass)");
                std::process::exit(0);
            } else {
                eprintln!("[FAIL] cargo-nova self-test encountered errors");
                std::process::exit(1);
            }
        }
        "new" => {
            if args.len() < 3 {
                eprintln!("Error: 'new' requires a project name. Example: cargo nova new myapp");
                std::process::exit(1);
            }
            let name = &args[2];
            let mut kind = TemplateKind::Gui;
            let mut out_path = PathBuf::from(name);

            let mut i = 3;
            while i < args.len() {
                if args[i] == "--template" || args[i] == "-t" {
                    if i + 1 < args.len() {
                        if let Some(k) = TemplateKind::parse(&args[i + 1]) {
                            kind = k;
                        } else {
                            eprintln!("Warning: Unknown template '{}', defaulting to 'gui'", args[i + 1]);
                        }
                        i += 2;
                        continue;
                    }
                } else if args[i] == "--path" || args[i] == "-p" {
                    if i + 1 < args.len() {
                        out_path = PathBuf::from(&args[i + 1]);
                        i += 2;
                        continue;
                    }
                }
                i += 1;
            }

            println!("Creating new NovaOS {:?} project '{}' in '{}'...", kind, name, out_path.display());
            if let Err(e) = ProjectGenerator::generate(&out_path, name, kind, None) {
                eprintln!("Error creating project: {}", e);
                std::process::exit(1);
            }
            println!("Project '{}' created successfully! To get started:", name);
            println!("  cd {}", out_path.display());
            println!("  cargo nova run");
        }
        "build" => {
            let mut opts = BuildOptions::default();
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--release" || args[i] == "-r" {
                    opts.release = true;
                } else if args[i] == "--target" && i + 1 < args.len() {
                    opts.target = Some(args[i + 1].clone());
                    i += 1;
                }
                i += 1;
            }

            let project_dir = Path::new(".");
            println!("Compiling NovaOS application (release={})...", opts.release);
            match AppBuilder::build(project_dir, &opts) {
                Ok(bin) => {
                    println!("[OK] Build successful: {}", bin.display());
                }
                Err(e) => {
                    eprintln!("Build error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "package" => {
            let mut opts = PackageOptions {
                out_dir: None,
                release: false,
            };
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--release" || args[i] == "-r" {
                    opts.release = true;
                } else if (args[i] == "--out" || args[i] == "--out-dir") && i + 1 < args.len() {
                    opts.out_dir = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
                i += 1;
            }

            let project_dir = Path::new(".");
            println!("Packaging NovaOS .app bundle...");
            match AppPackager::package(project_dir, &opts) {
                Ok(bundle) => {
                    println!("[OK] Application packaged into .app bundle: {}", bundle.display());
                }
                Err(e) => {
                    eprintln!("Packaging error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "run" => {
            let mut opts = RunOptions {
                emulator: false,
                release: false,
                args: vec![],
            };
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--emulator" || args[i] == "-e" {
                    opts.emulator = true;
                } else if args[i] == "--simulator" || args[i] == "-s" {
                    opts.emulator = false;
                } else if args[i] == "--release" || args[i] == "-r" {
                    opts.release = true;
                } else {
                    opts.args.push(args[i].clone());
                }
                i += 1;
            }

            let project_dir = Path::new(".");
            if let Err(e) = AppRunner::run(project_dir, &opts) {
                eprintln!("Run error: {}", e);
                std::process::exit(1);
            }
        }
        other => {
            eprintln!("Unknown subcommand: '{}'. Run 'cargo nova help' for usage.", other);
            std::process::exit(1);
        }
    }
}
