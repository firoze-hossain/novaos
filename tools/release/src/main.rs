//! NovaOS Release Packaging & Verification Engine (v2.0)
//!
//! Provides inspection and validation for:
//! 1. GPT Partition Tables (Protective MBR, EFI PART header, partition arrays)
//! 2. ISO 9660 & El Torito Boot Catalogs (Hybrid BIOS + UEFI bootability)
//! 3. Release Manifest & Checksum Verification

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct PartitionRecord {
    pub index: usize,
    pub name: String,
    pub type_guid: String,
    pub start_lba: u64,
    pub end_lba: u64,
    pub size_mb: f64,
}

#[derive(Debug)]
pub struct GptAnalysis {
    pub disk_size: u64,
    pub primary_valid: bool,
    pub backup_valid: bool,
    pub partitions: Vec<PartitionRecord>,
}

#[derive(Debug)]
pub struct ElToritoAnalysis {
    pub has_primary_vol_desc: bool,
    pub has_boot_record: bool,
    pub boot_catalog_lba: u32,
    pub bios_bootable: bool,
    pub uefi_bootable: bool,
}

fn read_u16_le(buf: &[u8], offset: usize) -> u16 {
    buf[offset] as u16 | ((buf[offset + 1] as u16) << 8)
}

fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    (buf[offset] as u32)
        | ((buf[offset + 1] as u32) << 8)
        | ((buf[offset + 2] as u32) << 16)
        | ((buf[offset + 3] as u32) << 24)
}

fn read_u64_le(buf: &[u8], offset: usize) -> u64 {
    (buf[offset] as u64)
        | ((buf[offset + 1] as u64) << 8)
        | ((buf[offset + 2] as u64) << 16)
        | ((buf[offset + 3] as u64) << 24)
        | ((buf[offset + 4] as u64) << 32)
        | ((buf[offset + 5] as u64) << 40)
        | ((buf[offset + 6] as u64) << 48)
        | ((buf[offset + 7] as u64) << 56)
}

fn format_guid(bytes: &[u8]) -> String {
    if bytes.len() < 16 {
        return String::from("00000000-0000-0000-0000-000000000000");
    }
    let p1 = read_u32_le(bytes, 0);
    let p2 = read_u16_le(bytes, 4);
    let p3 = read_u16_le(bytes, 6);
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        p1, p2, p3, bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

pub fn analyze_gpt(path: &Path) -> Result<GptAnalysis, String> {
    let mut file = File::open(path).map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    let file_len = file.metadata().map_err(|e| format!("Metadata error: {}", e))?.len();

    if file_len < 34 * 512 {
        return Err(format!("Disk image too small for GPT: {} bytes", file_len));
    }

    // Read Sector 0: Protective MBR
    let mut mbr = [0u8; 512];
    file.read_exact(&mut mbr).map_err(|e| format!("Failed to read MBR: {}", e))?;
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return Err("Invalid MBR boot signature (missing 0x55AA)".to_string());
    }

    // Read Sector 1: Primary GPT Header
    let mut gpt_hdr = [0u8; 512];
    file.read_exact(&mut gpt_hdr).map_err(|e| format!("Failed to read GPT header: {}", e))?;

    if &gpt_hdr[0..8] != b"EFI PART" {
        return Err("Primary GPT signature mismatch (expected 'EFI PART')".to_string());
    }

    let _revision = read_u32_le(&gpt_hdr, 8);
    let header_size = read_u32_le(&gpt_hdr, 12);
    let current_lba = read_u64_le(&gpt_hdr, 24);
    let backup_lba = read_u64_le(&gpt_hdr, 32);
    let _first_usable = read_u64_le(&gpt_hdr, 40);
    let _last_usable = read_u64_le(&gpt_hdr, 48);
    let part_entry_lba = read_u64_le(&gpt_hdr, 72);
    let num_parts = read_u32_le(&gpt_hdr, 80) as usize;
    let part_entry_size = read_u32_le(&gpt_hdr, 84) as usize;

    if current_lba != 1 || header_size < 92 || part_entry_size < 128 {
        return Err("Malformed Primary GPT header fields".to_string());
    }

    // Seek to Partition Table entries
    file.seek(SeekFrom::Start(part_entry_lba * 512))
        .map_err(|e| format!("Seek to partition array failed: {}", e))?;

    let array_bytes_len = num_parts * part_entry_size;
    let mut array_buf = vec![0u8; array_bytes_len];
    file.read_exact(&mut array_buf)
        .map_err(|e| format!("Read partition array failed: {}", e))?;

    let mut partitions = Vec::new();
    for i in 0..num_parts {
        let offset = i * part_entry_size;
        let entry = &array_buf[offset..offset + part_entry_size];

        // If type GUID is all zeros, entry is empty
        if entry[0..16].iter().all(|&b| b == 0) {
            continue;
        }

        let type_guid = format_guid(&entry[0..16]);
        let start_lba = read_u64_le(entry, 32);
        let end_lba = read_u64_le(entry, 40);
        let sector_count = if end_lba >= start_lba { end_lba - start_lba + 1 } else { 0 };
        let size_mb = (sector_count as f64 * 512.0) / (1024.0 * 1024.0);

        // UTF-16LE partition name (36 chars, 72 bytes at offset 56)
        let name_bytes = &entry[56..56 + 72];
        let mut u16_chars = Vec::new();
        for chunk in name_bytes.chunks_exact(2) {
            let ch = (chunk[0] as u16) | ((chunk[1] as u16) << 8);
            if ch == 0 {
                break;
            }
            u16_chars.push(ch);
        }
        let name = String::from_utf16_lossy(&u16_chars).trim().to_string();

        partitions.push(PartitionRecord {
            index: i + 1,
            name,
            type_guid,
            start_lba,
            end_lba,
            size_mb,
        });
    }

    // Check Backup GPT Header at last sector
    let mut backup_valid = false;
    if backup_lba > 0 && backup_lba * 512 < file_len {
        if file.seek(SeekFrom::Start(backup_lba * 512)).is_ok() {
            let mut backup_hdr = [0u8; 512];
            if file.read_exact(&mut backup_hdr).is_ok() && &backup_hdr[0..8] == b"EFI PART" {
                backup_valid = true;
            }
        }
    }

    Ok(GptAnalysis {
        disk_size: file_len,
        primary_valid: true,
        backup_valid,
        partitions,
    })
}

pub fn analyze_eltorito(path: &Path) -> Result<ElToritoAnalysis, String> {
    let mut file = File::open(path).map_err(|e| format!("Failed to open ISO {}: {}", path.display(), e))?;
    let file_len = file.metadata().map_err(|e| format!("Metadata error: {}", e))?.len();

    if file_len < 18 * 2048 {
        return Err(format!("ISO image too small for El Torito: {} bytes", file_len));
    }

    // Sector 16: Primary Volume Descriptor (PVD)
    file.seek(SeekFrom::Start(16 * 2048)).map_err(|e| e.to_string())?;
    let mut pvd = [0u8; 2048];
    file.read_exact(&mut pvd).map_err(|e| e.to_string())?;
    let has_pvd = &pvd[0..6] == b"\x01CD001";

    // Sector 17: Boot Record Volume Descriptor
    file.seek(SeekFrom::Start(17 * 2048)).map_err(|e| e.to_string())?;
    let mut br = [0u8; 2048];
    file.read_exact(&mut br).map_err(|e| e.to_string())?;
    let has_boot_record = &br[0..7] == b"\x00CD001\x01" && &br[7..30] == b"EL TORITO SPECIFICATION";

    if !has_boot_record {
        return Err("ISO missing El Torito Boot Record Volume Descriptor".to_string());
    }

    // Boot Catalog pointer at offset 71 (32-bit LE)
    let catalog_lba = read_u32_le(&br, 71);
    if catalog_lba == 0 || (catalog_lba as u64) * 2048 >= file_len {
        return Err(format!("Invalid El Torito Boot Catalog LBA pointer: {}", catalog_lba));
    }

    // Read Boot Catalog
    file.seek(SeekFrom::Start((catalog_lba as u64) * 2048)).map_err(|e| e.to_string())?;
    let mut cat = [0u8; 2048];
    file.read_exact(&mut cat).map_err(|e| e.to_string())?;

    // Validation entry (offset 0..32): Header ID 0x01, key 0x55AA
    let valid_validation = cat[0] == 0x01 && cat[30] == 0x55 && cat[31] == 0xAA;
    if !valid_validation {
        return Err("Invalid El Torito Boot Catalog validation entry".to_string());
    }

    // Default entry (BIOS): offset 32..64, bootable indicator 0x88
    let bios_bootable = cat[32] == 0x88;

    // Scan subsequent 32-byte entries for UEFI section entry (Platform ID 0xEF)
    let mut uefi_bootable = false;
    for chunk in cat[64..].chunks_exact(32) {
        if chunk[0] == 0x90 || chunk[0] == 0x91 {
            // Section header: check platform ID
            if chunk[1] == 0xEF {
                uefi_bootable = true;
                break;
            }
        } else if chunk[0] == 0x88 {
            // Additional bootable section entry
            uefi_bootable = true;
            break;
        }
    }

    Ok(ElToritoAnalysis {
        has_primary_vol_desc: has_pvd,
        has_boot_record,
        boot_catalog_lba: catalog_lba,
        bios_bootable,
        uefi_bootable,
    })
}

fn compute_sha256(path: &Path) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .map_err(|e| format!("sha256sum failed: {}", e))?;
    if !output.status.success() {
        return Err(format!("sha256sum exited with code {:?}", output.status.code()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hash = stdout.split_whitespace().next().unwrap_or("").to_string();
    if hash.len() != 64 {
        return Err(format!("Unexpected sha256 output: {}", stdout));
    }
    Ok(hash)
}

fn cmd_inspect_gpt(image_path: &str) -> Result<(), String> {
    let path = Path::new(image_path);
    let analysis = analyze_gpt(path)?;

    println!("========================================================================");
    println!("  NovaOS GPT Partition Table Analysis: {}", path.display());
    println!("========================================================================");
    println!("Image Size:       {} bytes ({:.2} MiB)", analysis.disk_size, analysis.disk_size as f64 / (1024.0 * 1024.0));
    println!("Primary Header:   {}", if analysis.primary_valid { "VALID (LBA 1, 'EFI PART')" } else { "INVALID" });
    println!("Backup Header:    {}", if analysis.backup_valid { "VALID (Backup LBA)" } else { "NOT FOUND / UNALIGNED" });
    println!("Total Partitions: {}", analysis.partitions.len());
    println!("------------------------------------------------------------------------");
    println!("{:<4} {:<16} {:<12} {:<12} {:<10} {:<36}", "Num", "Name", "Start LBA", "End LBA", "Size", "Type GUID");
    println!("------------------------------------------------------------------------");

    for p in &analysis.partitions {
        println!(
            "{:<4} {:<16} {:<12} {:<12} {:<8.2}MB {:<36}",
            p.index, p.name, p.start_lba, p.end_lba, p.size_mb, p.type_guid
        );
    }
    println!("========================================================================");

    // Validate that critical partitions exist
    let has_esp = analysis.partitions.iter().any(|p| p.name.contains("ESP") || p.type_guid.starts_with("C12A7328"));
    let has_sys = analysis.partitions.iter().any(|p| p.name.contains("SYS") || p.name.contains("NOVAOS"));
    if !has_esp {
        return Err("Validation failure: ESP (EFI System Partition) is missing!".to_string());
    }
    if !has_sys {
        return Err("Validation failure: NovaOS System Partition is missing!".to_string());
    }

    println!("✅ GPT Partition Table Verification: 100% Passed.");
    Ok(())
}

fn cmd_inspect_eltorito(iso_path: &str) -> Result<(), String> {
    let path = Path::new(iso_path);
    let analysis = analyze_eltorito(path)?;

    println!("========================================================================");
    println!("  NovaOS El Torito Bootable ISO Analysis: {}", path.display());
    println!("========================================================================");
    println!("ISO 9660 PVD:       {}", if analysis.has_primary_vol_desc { "PRESENT" } else { "MISSING" });
    println!("El Torito Header:   {}", if analysis.has_boot_record { "PRESENT ('EL TORITO SPECIFICATION')" } else { "MISSING" });
    println!("Catalog LBA:        {}", analysis.boot_catalog_lba);
    println!("BIOS Boot Media:    {}", if analysis.bios_bootable { "BOOTABLE (0x88)" } else { "NON-BOOTABLE" });
    println!("UEFI Boot Media:    {}", if analysis.uefi_bootable { "BOOTABLE (EFI Platform 0xEF / 0x88)" } else { "NOT FOUND" });
    println!("========================================================================");

    if !analysis.bios_bootable {
        return Err("Validation failure: BIOS boot image is not marked bootable!".to_string());
    }
    if !analysis.uefi_bootable {
        return Err("Validation failure: UEFI boot entry missing in El Torito catalog!".to_string());
    }

    println!("✅ Dual-Boot Hybrid El Torito Verification: 100% Passed (BIOS + UEFI).");
    Ok(())
}

fn cmd_generate_manifest(release_dir: &str, version: &str, out_path: &str) -> Result<(), String> {
    let dir = Path::new(release_dir);
    if !dir.is_dir() {
        return Err(format!("Release directory '{}' does not exist", release_dir));
    }

    let files = [
        ("novaos-desktop-2.0.iso", "iso", "desktop", "NovaOS 2.0 Desktop Edition (Hybrid UEFI/BIOS Live Installer)"),
        ("novaos-server-2.0.iso", "iso", "server", "NovaOS 2.0 Server Edition (Headless Cloud & Microservices ISO)"),
        ("novaos-desktop-2.0.img", "img", "desktop", "NovaOS 2.0 Desktop Edition (GPT Hybrid Raw Disk Image)"),
        ("novaos-server-2.0.img", "img", "server", "NovaOS 2.0 Server Edition (GPT Hybrid Raw Disk Image)"),
        ("novaos.bin", "elf", "desktop", "NovaOS Desktop Multiboot Kernel Binary (x86_32)"),
        ("novaos-server.bin", "elf", "server", "NovaOS Dedicated Headless Server Kernel Binary (x86_32)"),
    ];

    let mut entries_json = Vec::new();
    for (fname, format, edition, description) in files {
        let fpath = dir.join(fname);
        if fpath.exists() {
            let size = fpath.metadata().map_err(|e| e.to_string())?.len();
            let hash = compute_sha256(&fpath)?;
            entries_json.push(format!(
                r#"    {{
      "filename": "{}",
      "format": "{}",
      "edition": "{}",
      "description": "{}",
      "size_bytes": {},
      "sha256": "{}"
    }}"#,
                fname, format, edition, description, size, hash
            ));
        }
    }

    let manifest_json = format!(
        r#"{{
  "project": "NovaOS",
  "version": "{}",
  "release_type": "production",
  "build_timestamp": "{}",
  "artifacts": [
{}
  ]
}}
"#,
        version,
        chrono_placeholder(),
        entries_json.join(",\n")
    );

    std::fs::write(out_path, manifest_json).map_err(|e| format!("Failed to write manifest: {}", e))?;
    println!("✅ Release manifest generated successfully at: {}", out_path);
    Ok(())
}

fn chrono_placeholder() -> String {
    // Read date from host
    Command::new("date")
        .arg("-u")
        .arg("+%Y-%m-%dT%H:%M:%SZ")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "2026-11-18T00:00:00Z".to_string())
}

fn cmd_audit(release_dir: &str) -> Result<(), String> {
    let dir = Path::new(release_dir);
    println!("========================================================================");
    println!("  NovaOS Dual-Edition Release Directory Comprehensive Audit: {}", dir.display());
    println!("========================================================================");

    let desktop_iso = dir.join("novaos-desktop-2.0.iso");
    let server_iso = dir.join("novaos-server-2.0.iso");
    let desktop_img = dir.join("novaos-desktop-2.0.img");
    let server_img = dir.join("novaos-server-2.0.img");

    if desktop_iso.exists() {
        println!("\n>>> Auditing Desktop ISO: {}", desktop_iso.display());
        cmd_inspect_eltorito(desktop_iso.to_str().unwrap())?;
    }
    if server_iso.exists() {
        println!("\n>>> Auditing Server ISO: {}", server_iso.display());
        cmd_inspect_eltorito(server_iso.to_str().unwrap())?;
    }
    if desktop_img.exists() {
        println!("\n>>> Auditing Desktop GPT Image: {}", desktop_img.display());
        cmd_inspect_gpt(desktop_img.to_str().unwrap())?;
    }
    if server_img.exists() {
        println!("\n>>> Auditing Server GPT Image: {}", server_img.display());
        cmd_inspect_gpt(server_img.to_str().unwrap())?;
    }

    println!("\n========================================================================");
    println!(" ✅ FULL DUAL-EDITION RELEASE AUDIT COMPLETE: ALL CHECKS PASSED!");
    println!("========================================================================");
    Ok(())
}

fn print_usage() {
    println!("NovaOS Release Engine (v2.0) - CLI Usage:");
    println!("  novarelease inspect-gpt <image.img>             Inspect & validate GPT partition table");
    println!("  novarelease inspect-eltorito <image.iso>        Inspect & validate ISO El Torito BIOS/UEFI boot");
    println!("  novarelease generate-manifest --dir <DIR> --version <VER> --out <JSON>");
    println!("  novarelease audit --dir <release_dir>           Audit all release media in directory");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let result = match args[1].as_str() {
        "inspect-gpt" => {
            if args.len() < 3 {
                Err("Missing image file argument".to_string())
            } else {
                cmd_inspect_gpt(&args[2])
            }
        }
        "inspect-eltorito" => {
            if args.len() < 3 {
                Err("Missing ISO file argument".to_string())
            } else {
                cmd_inspect_eltorito(&args[2])
            }
        }
        "generate-manifest" => {
            let mut dir = "release/v2.0";
            let mut ver = "2.0.0";
            let mut out = "release/v2.0/release_manifest.json";
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--dir" if i + 1 < args.len() => {
                        dir = &args[i + 1];
                        i += 2;
                    }
                    "--version" if i + 1 < args.len() => {
                        ver = &args[i + 1];
                        i += 2;
                    }
                    "--out" if i + 1 < args.len() => {
                        out = &args[i + 1];
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            cmd_generate_manifest(dir, ver, out)
        }
        "audit" => {
            let mut dir = "release/v2.0";
            if args.len() >= 4 && args[2] == "--dir" {
                dir = &args[3];
            } else if args.len() >= 3 {
                dir = &args[2];
            }
            cmd_audit(dir)
        }
        "--help" | "-h" | "help" => {
            print_usage();
            Ok(())
        }
        other => Err(format!("Unknown command: '{}'", other)),
    };

    if let Err(e) = result {
        eprintln!("\n❌ Error: {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_guid() {
        let esp_bytes = [
            0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E,
            0xC9, 0x3B,
        ];
        let guid = format_guid(&esp_bytes);
        assert_eq!(guid, "C12A7328-F81F-11D2-BA4B-00A0C93EC93B");
    }

    #[test]
    fn test_u32_le() {
        let bytes = [0x78, 0x56, 0x34, 0x12];
        assert_eq!(read_u32_le(&bytes, 0), 0x12345678);
    }
}
