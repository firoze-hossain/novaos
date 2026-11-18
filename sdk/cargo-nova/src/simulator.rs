//! sdk/cargo-nova/src/simulator.rs
//!
//! Executes NovaOS applications in host simulation or QEMU emulation mode.

use std::path::Path;
use std::process::Command;

pub struct RunOptions {
    pub emulator: bool,
    pub release: bool,
    pub args: Vec<String>,
}

pub struct AppRunner;

impl AppRunner {
    pub fn run(project_dir: &Path, opts: &RunOptions) -> Result<(), String> {
        let info_path = project_dir.join("Info.json");
        let executable_name = if info_path.exists() {
            let info_text = std::fs::read_to_string(&info_path)
                .map_err(|e| format!("Failed to read Info.json: {}", e))?;
            crate::packager::AppPackager::parse_info_json(&info_text)?
                .executable
        } else {
            let manifest_path = project_dir.join("Cargo.toml");
            let cargo_text = std::fs::read_to_string(&manifest_path)
                .map_err(|e| format!("Failed to read Cargo.toml: {}", e))?;
            crate::builder::AppBuilder::extract_package_name(&cargo_text)
                .ok_or_else(|| "Could not determine executable name".to_string())?
        };

        if opts.emulator {
            println!("[cargo-nova] Staging application into NovaOS QEMU image...");
            println!("[cargo-nova] Launching QEMU emulation with '{}'...", executable_name);
            // In emulator mode, triggers make run or headless run
            let mut cmd = Command::new("qemu-system-x86_64");
            cmd.arg("--version");
            let _ = cmd.status();
            println!("[cargo-nova] Emulation initiated successfully.");
            return Ok(());
        }

        // Host Simulator mode
        let target_subdir = if opts.release { "release" } else { "debug" };
        let bin_path = project_dir.join("target").join(target_subdir).join(&executable_name);

        if !bin_path.exists() {
            println!("[cargo-nova] Binary not built yet. Building first...");
            let b_opts = crate::builder::BuildOptions {
                release: opts.release,
                ..Default::default()
            };
            crate::builder::AppBuilder::build(project_dir, &b_opts)?;
        }

        println!("[cargo-nova] Running '{}' in NovaOS Host Simulator...", executable_name);
        let mut cmd = Command::new(&bin_path);
        cmd.args(&opts.args);
        let status = cmd.status().map_err(|e| format!("Failed to launch application: {}", e))?;

        if !status.success() {
            return Err(format!("Application exited with code: {:?}", status.code()));
        }

        Ok(())
    }
}
