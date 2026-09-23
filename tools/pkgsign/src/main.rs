//! tools/pkgsign/src/main.rs - Phase 69: the host-side half of
//! package signing. Signs (or verifies, for local testing without
//! booting the kernel) a NovaOS `.PKG` file in place, using the exact
//! same key and hash-then-MAC construction kernel/rust/pkgsign.rs
//! verifies with - see that file's own module doc comment for the
//! full scheme and its honest, symmetric-HMAC trust-model
//! limitations, and kernel/rust/pkgsign_core.rs's own doc comment for
//! why the actual signing logic lives there, `include!`'d by both
//! sides, rather than being duplicated here by hand.
//!
//! Usage:
//!   pkgsign sign <path-to.PKG>     - signs the file in place
//!   pkgsign verify <path-to.PKG>   - checks an existing signature,
//!                                    without modifying the file
//!
//! # The on-disk header layout this tool assumes
//!
//! userland/pkg/pkgmgr.h's own `pkg_header_t` (`__attribute__((
//! packed))`, so no compiler-inserted padding to account for):
//!   offset  0, 4 bytes  - magic ("NVPK")
//!   offset  4, 16 bytes - name
//!   offset 20, 8 bytes  - version
//!   offset 28, 64 bytes - description
//!   offset 92, 4 bytes  - payload_size (u32, little-endian - x86)
//!   offset 96, 32 bytes - signature
//!   (128 bytes total, payload immediately follows)
//! This tool has no way to `#include` that C header directly (it's a
//! separate, host-side Rust binary), so these offsets are hand-kept
//! in sync with pkgmgr.h's own struct - the same real constraint
//! PKG_SIGNATURE_OFFSET's own comment in pkgmgr.c names for the C
//! side. HEADER_LEN/SIGNATURE_OFFSET below are the one place this
//! tool would need updating if that struct's own layout ever changes.

#[path = "../../../kernel/rust/sha256.rs"]
mod sha256;
#[path = "../../../kernel/rust/hmac_sha256.rs"]
mod hmac_sha256;
#[path = "../../../kernel/rust/pkgsign_core.rs"]
mod pkgsign_core;

use std::env;
use std::fs;
use std::process::ExitCode;

const HEADER_LEN: usize = 128;
const SIGNATURE_OFFSET: usize = 96;
const SIGNATURE_LEN: usize = pkgsign_core::PKG_SIGNATURE_LEN;
const PAYLOAD_SIZE_OFFSET: usize = 92;

/// The header's own size *before* Phase 69 added the signature field
/// - magic(4) + name(16) + version(8) + description(64) +
/// payload_size(4) = 96 bytes, payload immediately following. Every
/// package fixture this project shipped before this phase is still in
/// this exact, real, on-disk shape - not a corrupted or malformed
/// file, just built before this field existed.
const OLD_HEADER_LEN: usize = 96;

fn read_payload_size(header: &[u8]) -> u32 {
    u32::from_le_bytes([
        header[PAYLOAD_SIZE_OFFSET],
        header[PAYLOAD_SIZE_OFFSET + 1],
        header[PAYLOAD_SIZE_OFFSET + 2],
        header[PAYLOAD_SIZE_OFFSET + 3],
    ])
}

/// Migrates an old-format (96-byte header, no signature field) `.PKG`
/// file's own bytes to the new, 128-byte-header format, by inserting
/// 32 zero bytes right where the signature field now belongs (offset
/// 96) - the payload itself, and every field before payload_size, is
/// completely unchanged; only the file's own total length and
/// everything after the insertion point shift. Detected, not assumed:
/// checks the file's own total length against what an old-format
/// header's own payload_size field would actually predict, since a
/// new-format file (already migrated and possibly already signed)
/// must be left exactly as it is rather than have a second, spurious
/// 32 bytes inserted into it.
fn migrate_if_old_format(data: Vec<u8>, path: &str) -> Result<Vec<u8>, String> {
    if data.len() < OLD_HEADER_LEN {
        return Err(format!(
            "'{path}' is only {} bytes, shorter than even the old, pre-\
             signature {OLD_HEADER_LEN}-byte package header - not a real \
             .PKG file",
            data.len()
        ));
    }
    let old_format_payload_size = read_payload_size(&data[..OLD_HEADER_LEN.min(data.len())]).max(0);
    // read_payload_size() itself reads from PAYLOAD_SIZE_OFFSET (92),
    // which is identical in both the old and new header layout (the
    // signature field Phase 69 added comes *after* payload_size, not
    // before it) - so this same read is valid evidence for telling
    // the two formats apart, not something that needs its own,
    // separate old-header-specific offset.
    let new_format_payload_size = if data.len() >= HEADER_LEN {
        read_payload_size(&data[..HEADER_LEN])
    } else {
        u32::MAX // can't possibly be the new format if it's not even
                 // long enough to have a full new-format header
    };

    if data.len() == OLD_HEADER_LEN + old_format_payload_size as usize {
        println!(
            "'{path}': old, pre-signature format detected (96-byte header) \
             - migrating to the new 128-byte-header format before signing"
        );
        let mut migrated = Vec::with_capacity(data.len() + (HEADER_LEN - OLD_HEADER_LEN));
        migrated.extend_from_slice(&data[..OLD_HEADER_LEN]);
        migrated.extend(std::iter::repeat(0u8).take(HEADER_LEN - OLD_HEADER_LEN));
        migrated.extend_from_slice(&data[OLD_HEADER_LEN..]);
        Ok(migrated)
    } else if data.len() == HEADER_LEN + new_format_payload_size as usize {
        Ok(data) // already the new format - nothing to migrate
    } else {
        Err(format!(
            "'{path}': {} total bytes matches neither the old format \
             ({OLD_HEADER_LEN}-byte header + payload_size={old_format_payload_size}) \
             nor the new one ({HEADER_LEN}-byte header + payload_size=\
             {new_format_payload_size}) - the file's own header disagrees \
             with its own actual length either way",
            data.len()
        ))
    }
}

/// Computes the correct signature for `data` (a full .PKG file's own
/// bytes: header immediately followed by payload) - zeroing the
/// header's own signature field in a local copy first, the same
/// "zero, don't slice around it" shape kernel/rust/pkgsign.rs's own
/// verifier uses, so both sides hash the identical byte layout.
fn compute_file_signature(data: &[u8]) -> [u8; 32] {
    let mut header = [0u8; HEADER_LEN];
    header.copy_from_slice(&data[..HEADER_LEN]);
    for b in &mut header[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN] {
        *b = 0;
    }
    let payload = &data[HEADER_LEN..];
    pkgsign_core::compute_signature(&header, payload)
}

fn cmd_sign(path: &str) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| format!("reading '{path}': {e}"))?;
    if data.len() < OLD_HEADER_LEN || &data[0..4] != b"NVPK" {
        return Err(format!("'{path}' does not start with the \"NVPK\" magic"));
    }

    let mut data = migrate_if_old_format(data, path)?;

    let payload_size = read_payload_size(&data[..HEADER_LEN]) as usize;
    let actual_payload_len = data.len() - HEADER_LEN;
    if payload_size != actual_payload_len {
        return Err(format!(
            "'{path}': header claims payload_size={payload_size}, but the \
             file actually has {actual_payload_len} bytes of payload after \
             the header - refusing to sign a package whose own header \
             already disagrees with its own contents"
        ));
    }

    let sig = compute_file_signature(&data);
    data[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN].copy_from_slice(&sig);

    fs::write(path, &data).map_err(|e| format!("writing '{path}': {e}"))?;
    println!("signed '{path}' ({payload_size} bytes of payload)");
    Ok(())
}

fn cmd_verify(path: &str) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| format!("reading '{path}': {e}"))?;
    if data.len() < HEADER_LEN {
        return Err(format!("'{path}' is too short to be a real .PKG file"));
    }
    if &data[0..4] != b"NVPK" {
        return Err(format!("'{path}' does not start with the \"NVPK\" magic"));
    }

    let expected = compute_file_signature(&data);
    let actual = &data[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN];

    // A plain, non-constant-time comparison is fine here - unlike
    // kernel/rust/pkgsign.rs's own verifier (which runs where a real
    // remote or untrusted caller could time it), this is a local CLI
    // tool a developer runs against their own file; there's no
    // network-observable timing channel to defend against.
    if expected.as_slice() == actual {
        println!("'{path}': signature is valid");
        Ok(())
    } else {
        Err(format!("'{path}': signature does NOT match - either it was \
                      never signed, or it (or its payload) has been \
                      modified since signing"))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 || (args[1] != "sign" && args[1] != "verify") {
        eprintln!("usage: {} sign|verify <path-to.PKG>", args.get(0).map(String::as_str).unwrap_or("pkgsign"));
        return ExitCode::FAILURE;
    }

    let result = if args[1] == "sign" {
        cmd_sign(&args[2])
    } else {
        cmd_verify(&args[2])
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
