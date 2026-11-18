//! sdk/cargo-nova/src/selftest.rs
//!
//! Automated end-to-end self-test suite for cargo-nova CLI tool.

use crate::builder::{AppBuilder, BuildOptions};
use crate::packager::{AppPackager, PackageOptions};
use crate::template::{ProjectGenerator, TemplateKind};
use std::fs;

/// Executes the full cargo-nova CLI self-test suite.
/// Returns true if and only if all lifecycle stages pass.
pub fn run_cargo_nova_selftest() -> bool {
    let tmp_base = std::env::temp_dir().join("cargo_nova_selftest");
    if tmp_base.exists() {
        let _ = fs::remove_dir_all(&tmp_base);
    }
    if fs::create_dir_all(&tmp_base).is_err() {
        return false;
    }

    println!("[selftest] 1. Generating sample GUI application project...");
    let gui_dir = tmp_base.join("demo_gui");
    // Find absolute or relative path to novaos-sdk
    let sdk_path = match std::env::current_dir() {
        Ok(cwd) => {
            let candidate1 = cwd.join("sdk/novaos-sdk");
            let candidate2 = cwd.join("../novaos-sdk");
            if candidate1.exists() {
                candidate1.to_string_lossy().to_string()
            } else if candidate2.exists() {
                candidate2.to_string_lossy().to_string()
            } else {
                "../novaos-sdk".to_string()
            }
        }
        Err(_) => "../novaos-sdk".to_string(),
    };

    if ProjectGenerator::generate(&gui_dir, "demo_gui", TemplateKind::Gui, Some(&sdk_path)).is_err() {
        eprintln!("[selftest] Failed to generate GUI project template");
        return false;
    }

    // Verify template structure
    if !gui_dir.join("Cargo.toml").exists()
        || !gui_dir.join("Info.json").exists()
        || !gui_dir.join("src/main.rs").exists()
        || !gui_dir.join("Assets/icon.png").exists()
    {
        eprintln!("[selftest] GUI project template missing required files");
        return false;
    }

    println!("[selftest] 2. Generating sample CLI application project...");
    let cli_dir = tmp_base.join("demo_cli");
    if ProjectGenerator::generate(&cli_dir, "demo_cli", TemplateKind::Cli, Some(&sdk_path)).is_err() {
        eprintln!("[selftest] Failed to generate CLI project template");
        return false;
    }

    if !cli_dir.join("Cargo.toml").exists()
        || !cli_dir.join("Info.json").exists()
        || !cli_dir.join("src/main.rs").exists()
    {
        eprintln!("[selftest] CLI project template missing required files");
        return false;
    }

    println!("[selftest] 3. Building GUI application via AppBuilder...");
    let b_opts = BuildOptions {
        release: false,
        quiet: true,
        ..Default::default()
    };
    let bin_result = AppBuilder::build(&gui_dir, &b_opts);
    if let Err(e) = bin_result {
        eprintln!("[selftest] Build failed: {}", e);
        return false;
    }

    println!("[selftest] 4. Packaging GUI application into .app bundle...");
    let p_opts = PackageOptions {
        out_dir: None,
        release: false,
    };
    let bundle_result = AppPackager::package(&gui_dir, &p_opts);
    let bundle_path = match bundle_result {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[selftest] Packaging failed: {}", e);
            return false;
        }
    };

    // Verify .app bundle contents
    if !bundle_path.exists()
        || !bundle_path.join("Info.json").exists()
        || !bundle_path.join("demo_gui").exists()
        || !bundle_path.join("Assets/icon.png").exists()
        || !bundle_path.join("Manifest.sha256").exists()
    {
        eprintln!("[selftest] .app bundle missing expected structure: {}", bundle_path.display());
        return false;
    }

    println!("[selftest] 5. Running packaged application in host simulation mode...");
    let run_opts = crate::simulator::RunOptions {
        emulator: false,
        release: false,
        args: vec![],
    };
    if let Err(e) = crate::simulator::AppRunner::run(&gui_dir, &run_opts) {
        eprintln!("[selftest] Simulator run failed: {}", e);
        return false;
    }

    // Cleanup
    let _ = fs::remove_dir_all(&tmp_base);

    println!("[selftest] All cargo-nova self-test checks PASSED successfully!");
    true
}
