//! sdk/cargo-nova/src/template.rs
//!
//! Project template generators for NovaOS GUI and CLI applications.

use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Gui,
    Cli,
}

impl TemplateKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "gui" | "gui-app" | "window" | "desktop" => Some(TemplateKind::Gui),
            "cli" | "cli-app" | "console" | "tool" => Some(TemplateKind::Cli),
            _ => None,
        }
    }
}

pub struct ProjectGenerator;

impl ProjectGenerator {
    pub fn generate(
        target_dir: &Path,
        project_name: &str,
        kind: TemplateKind,
        sdk_relative_path: Option<&str>,
    ) -> io::Result<()> {
        fs::create_dir_all(target_dir)?;
        let src_dir = target_dir.join("src");
        fs::create_dir_all(&src_dir)?;

        let sdk_path = sdk_relative_path.unwrap_or("../novaos-sdk");

        match kind {
            TemplateKind::Gui => {
                let assets_dir = target_dir.join("Assets");
                let resources_dir = target_dir.join("Resources");
                fs::create_dir_all(&assets_dir)?;
                fs::create_dir_all(&resources_dir)?;

                // Cargo.toml
                let cargo_toml = format!(
                    r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
authors = ["NovaOS Developer"]
description = "Native NovaOS Graphical Application"

[dependencies]
novaos-sdk = {{ path = "{sdk_path}" }}

[profile.release]
opt-level = 3
lto = true
panic = "abort"
"#,
                    name = project_name,
                    sdk_path = sdk_path
                );
                fs::write(target_dir.join("Cargo.toml"), cargo_toml)?;

                // Info.json
                let info_json = format!(
                    r#"{{
  "bundle_id": "com.novaos.{name}",
  "display_name": "{display_name}",
  "version": "0.1.0",
  "executable": "{name}",
  "category": "Utilities",
  "icon": "Assets/icon.png",
  "permissions": [
    "ui",
    "file_read"
  ],
  "window": {{
    "width": 640,
    "height": 480,
    "resizable": true
  }}
}}
"#,
                    name = project_name,
                    display_name = Self::to_title_case(project_name)
                );
                fs::write(target_dir.join("Info.json"), info_json)?;

                // assets/icon.png placeholder (1x1 PNG or text icon representation)
                let icon_bytes = [
                    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG Header
                    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR
                    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
                    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89,
                ];
                fs::write(assets_dir.join("icon.png"), &icon_bytes)?;

                // src/main.rs
                let main_rs = format!(
                    r#"//! {name} - Native NovaOS Application
use novaos_sdk::prelude::*;

fn main() {{
    println!("Launching {display_name} on NovaOS 2.0...");

    let mut app = App::builder("com.novaos.{name}", "{display_name}")
        .window_size(640, 480)
        .theme(ThemeMode::Dark)
        .accent(AccentPalette::FluentBlue)
        .build();

    let main_panel = app.add_view("main_panel", Rect::new(20, 20, 600, 440));
    let mut click_count = State::new(0);
    click_count.subscribe(main_panel);

    println!("App initialized. Ready for window events.");
}}
"#,
                    name = project_name,
                    display_name = Self::to_title_case(project_name)
                );
                fs::write(src_dir.join("main.rs"), main_rs)?;
            }
            TemplateKind::Cli => {
                // Cargo.toml
                let cargo_toml = format!(
                    r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
authors = ["NovaOS Developer"]
description = "NovaOS Command-Line Tool"

[dependencies]
novaos-sdk = {{ path = "{sdk_path}" }}

[profile.release]
opt-level = 3
lto = true
panic = "abort"
"#,
                    name = project_name,
                    sdk_path = sdk_path
                );
                fs::write(target_dir.join("Cargo.toml"), cargo_toml)?;

                // Info.json
                let info_json = format!(
                    r#"{{
  "bundle_id": "com.novaos.cli.{name}",
  "display_name": "{display_name}",
  "version": "0.1.0",
  "executable": "{name}",
  "category": "Development",
  "type": "cli"
}}
"#,
                    name = project_name,
                    display_name = Self::to_title_case(project_name)
                );
                fs::write(target_dir.join("Info.json"), info_json)?;

                // src/main.rs
                let main_rs = format!(
                    r#"//! {name} - NovaOS CLI Tool
use novaos_sdk::prelude::*;

fn main() {{
    let sys = SystemInfo::query();
    println!("=== {display_name} (NovaOS 2.0) ===");
    println!("OS: {{}} {{}}", sys.os_name, sys.os_version);
    println!("Kernel: {{}}", sys.kernel_version);
    println!("Architecture: {{}}", sys.architecture);
    println!("Memory: {{}} MB total / {{}} MB free", sys.total_memory_mb, sys.free_memory_mb);
}}
"#,
                    name = project_name,
                    display_name = Self::to_title_case(project_name)
                );
                fs::write(src_dir.join("main.rs"), main_rs)?;
            }
        }

        Ok(())
    }

    fn to_title_case(s: &str) -> String {
        s.split(|c: char| c == '-' || c == '_')
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}
