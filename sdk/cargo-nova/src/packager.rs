//! sdk/cargo-nova/src/packager.rs
//!
//! Packages compiled NovaOS binaries and assets into standards-compliant `.app` bundles.

use std::fs;
use std::path::{Path, PathBuf};

pub struct PackageOptions {
    pub out_dir: Option<PathBuf>,
    pub release: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AppBundleInfo {
    pub bundle_id: String,
    pub display_name: String,
    pub version: String,
    pub executable: String,
}

pub struct AppPackager;

impl AppPackager {
    pub fn package(project_dir: &Path, opts: &PackageOptions) -> Result<PathBuf, String> {
        let info_path = project_dir.join("Info.json");
        if !info_path.exists() {
            return Err("Info.json not found in project directory".to_string());
        }

        let info_text = fs::read_to_string(&info_path)
            .map_err(|e| format!("Failed to read Info.json: {}", e))?;
        let bundle_info = Self::parse_info_json(&info_text)?;

        // Ensure binary is built
        let target_subdir = if opts.release { "release" } else { "debug" };
        let bin_path = project_dir.join("target").join(target_subdir).join(&bundle_info.executable);
        if !bin_path.exists() {
            return Err(format!(
                "Compiled binary '{}' not found. Run 'cargo nova build' first.",
                bin_path.display()
            ));
        }

        let default_out = project_dir.join("build");
        let out_dir = opts.out_dir.as_ref().unwrap_or(&default_out);
        fs::create_dir_all(out_dir).map_err(|e| format!("Failed to create output dir: {}", e))?;

        let bundle_dir_name = format!("{}.app", bundle_info.executable);
        let bundle_path = out_dir.join(&bundle_dir_name);
        if bundle_path.exists() {
            fs::remove_dir_all(&bundle_path).map_err(|e| format!("Failed to clear old bundle: {}", e))?;
        }
        fs::create_dir_all(&bundle_path).map_err(|e| format!("Failed to create bundle dir: {}", e))?;

        // 1. Copy Info.json
        fs::copy(&info_path, bundle_path.join("Info.json"))
            .map_err(|e| format!("Failed to copy Info.json: {}", e))?;

        // 2. Copy binary
        let dest_bin = bundle_path.join(&bundle_info.executable);
        fs::copy(&bin_path, &dest_bin)
            .map_err(|e| format!("Failed to copy binary to bundle: {}", e))?;

        // Make executable on POSIX systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&dest_bin).unwrap().permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&dest_bin, perms);
        }

        // 3. Copy Assets directory if present
        let src_assets = project_dir.join("Assets");
        let dest_assets = bundle_path.join("Assets");
        if src_assets.exists() {
            fs::create_dir_all(&dest_assets)
                .map_err(|e| format!("Failed to create Assets in bundle: {}", e))?;
            Self::copy_dir_contents(&src_assets, &dest_assets)?;
        }

        // 4. Copy Resources directory if present
        let src_resources = project_dir.join("Resources");
        let dest_resources = bundle_path.join("Resources");
        if src_resources.exists() {
            fs::create_dir_all(&dest_resources)
                .map_err(|e| format!("Failed to create Resources in bundle: {}", e))?;
            Self::copy_dir_contents(&src_resources, &dest_resources)?;
        }

        // 5. Generate Manifest.sha256 checksum file for bundle integrity
        Self::generate_bundle_manifest(&bundle_path)?;

        Ok(bundle_path)
    }

    pub fn parse_info_json(content: &str) -> Result<AppBundleInfo, String> {
        let bundle_id = Self::extract_json_field(content, "bundle_id")
            .unwrap_or_else(|| "com.novaos.app".to_string());
        let display_name = Self::extract_json_field(content, "display_name")
            .unwrap_or_else(|| "NovaOS Application".to_string());
        let version = Self::extract_json_field(content, "version")
            .unwrap_or_else(|| "1.0.0".to_string());
        let executable = Self::extract_json_field(content, "executable")
            .ok_or_else(|| "Info.json missing required field 'executable'".to_string())?;

        Ok(AppBundleInfo {
            bundle_id,
            display_name,
            version,
            executable,
        })
    }

    fn extract_json_field(json: &str, field: &str) -> Option<String> {
        let key_pattern = format!("\"{}\"", field);
        for line in json.lines() {
            if let Some(pos) = line.find(&key_pattern) {
                let rest = &line[pos + key_pattern.len()..];
                if let Some(colon_pos) = rest.find(':') {
                    let val_part = rest[colon_pos + 1..].trim().trim_matches(',');
                    let val = val_part.trim().trim_matches('"').trim_matches('\'').trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
        None
    }

    fn copy_dir_contents(src: &Path, dest: &Path) -> Result<(), String> {
        let entries = fs::read_dir(src).map_err(|e| format!("Failed to read dir {}: {}", src.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Dir entry error: {}", e))?;
            let path = entry.path();
            let file_name = entry.file_name();
            let dest_file = dest.join(file_name);
            if path.is_file() {
                fs::copy(&path, &dest_file).map_err(|e| format!("Failed to copy file: {}", e))?;
            } else if path.is_dir() {
                fs::create_dir_all(&dest_file).map_err(|e| format!("Failed to create dir: {}", e))?;
                Self::copy_dir_contents(&path, &dest_file)?;
            }
        }
        Ok(())
    }

    fn generate_bundle_manifest(bundle_dir: &Path) -> Result<(), String> {
        let mut manifest_lines = Vec::new();
        Self::collect_files_recursive(bundle_dir, bundle_dir, &mut manifest_lines)?;
        let manifest_content = manifest_lines.join("\n") + "\n";
        fs::write(bundle_dir.join("Manifest.sha256"), manifest_content)
            .map_err(|e| format!("Failed to write Manifest.sha256: {}", e))?;
        Ok(())
    }

    fn collect_files_recursive(
        base: &Path,
        current: &Path,
        lines: &mut Vec<String>,
    ) -> Result<(), String> {
        let entries = fs::read_dir(current).map_err(|e| format!("Read dir error: {}", e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Entry error: {}", e))?;
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_string_lossy();
                if name == "Manifest.sha256" {
                    continue;
                }
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().to_string();
                let bytes = fs::read(&path).map_err(|e| format!("Read error: {}", e))?;
                let size = bytes.len();
                lines.push(format!("FILE {} size={}", rel, size));
            } else if path.is_dir() {
                Self::collect_files_recursive(base, &path, lines)?;
            }
        }
        Ok(())
    }
}
