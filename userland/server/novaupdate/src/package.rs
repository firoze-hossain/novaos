//! userland/server/novaupdate/src/package.rs
//!
//! Cryptographically Signed OS Delta Package Engine
//!
//! Handles creating, parsing, and cryptographically verifying OS delta packages (.pkg).
//! Guarantees authenticity and integrity with HMAC-SHA256 cryptographic signatures
//! and SHA-256 digest validation before staging into the inactive boot slot.

use std::fs;
use std::path::Path;

use crate::ab_boot::{simple_sha256, Sha256};

pub const PACKAGE_MAGIC: &[u8; 8] = b"NOVAUPKG";
pub const PACKAGE_VERSION: u32 = 1;
pub const DEFAULT_SIGNING_KEY: &[u8] = b"NovaOS-Enterprise-Release-Root-Key-2026";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    pub min_version: String,
    pub release_timestamp: u64,
    pub component: String,
    pub payload_size: usize,
    pub payload_sha256: [u8; 32],
    pub signature: [u8; 32],
}

impl PackageManifest {
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        out.push_str("magic=NOVAUPKG\n");
        out.push_str(&format!("version={}\n", PACKAGE_VERSION));
        out.push_str(&format!("name={}\n", self.name));
        out.push_str(&format!("target_version={}\n", self.version));
        out.push_str(&format!("min_version={}\n", self.min_version));
        out.push_str(&format!("release_timestamp={}\n", self.release_timestamp));
        out.push_str(&format!("component={}\n", self.component));
        out.push_str(&format!("payload_size={}\n", self.payload_size));
        out.push_str(&format!("payload_sha256={}\n", hex_encode(&self.payload_sha256)));
        out.push_str(&format!("signature={}\n", hex_encode(&self.signature)));
        out
    }

    pub fn deserialize(content: &str) -> Result<Self, String> {
        let mut name = String::new();
        let mut version = String::new();
        let mut min_version = "2.0.0".to_string();
        let mut release_timestamp = 0u64;
        let mut component = "kernel".to_string();
        let mut payload_size = 0usize;
        let mut payload_sha256 = [0u8; 32];
        let mut signature = [0u8; 32];

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "name" => name = v.to_string(),
                    "target_version" | "version" if !v.chars().all(|c| c.is_ascii_digit()) => {
                        version = v.to_string();
                    }
                    "min_version" => min_version = v.to_string(),
                    "release_timestamp" => release_timestamp = v.parse().unwrap_or(0),
                    "component" => component = v.to_string(),
                    "payload_size" => payload_size = v.parse().unwrap_or(0),
                    "payload_sha256" => hex_decode(v, &mut payload_sha256),
                    "signature" => hex_decode(v, &mut signature),
                    _ => {}
                }
            }
        }

        if name.is_empty() || version.is_empty() {
            return Err("Incomplete package manifest".to_string());
        }

        Ok(Self {
            name,
            version,
            min_version,
            release_timestamp,
            component,
            payload_size,
            payload_sha256,
            signature,
        })
    }
}

#[derive(Clone, Debug)]
pub struct DeltaPackage {
    pub manifest: PackageManifest,
    pub payload: Vec<u8>,
}

impl DeltaPackage {
    /// Creates and signs a new update package with the provided secret key.
    pub fn build(
        name: &str,
        version: &str,
        min_version: &str,
        component: &str,
        payload: Vec<u8>,
        signing_key: &[u8],
    ) -> Self {
        let payload_sha256 = simple_sha256(&payload);
        let payload_size = payload.len();
        let release_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Sign manifest parameters + payload hash
        let mut sig_input = Vec::new();
        sig_input.extend_from_slice(name.as_bytes());
        sig_input.extend_from_slice(version.as_bytes());
        sig_input.extend_from_slice(component.as_bytes());
        sig_input.extend_from_slice(&payload_sha256);

        let signature = hmac_sha256(signing_key, &sig_input);

        let manifest = PackageManifest {
            name: name.to_string(),
            version: version.to_string(),
            min_version: min_version.to_string(),
            release_timestamp,
            component: component.to_string(),
            payload_size,
            payload_sha256,
            signature,
        };

        Self { manifest, payload }
    }

    /// Serializes the complete signed package into binary bytes for on-disk or HTTPS transport.
    pub fn encode(&self) -> Vec<u8> {
        let manifest_str = self.manifest.serialize();
        let manifest_bytes = manifest_str.as_bytes();
        let manifest_len = manifest_bytes.len() as u32;

        let mut out = Vec::with_capacity(12 + manifest_bytes.len() + self.payload.len());
        out.extend_from_slice(PACKAGE_MAGIC);
        out.extend_from_slice(&manifest_len.to_le_bytes());
        out.extend_from_slice(manifest_bytes);
        out.extend_from_slice(&self.payload);
        out
    }

    /// Parses and validates a binary update package.
    pub fn decode(bytes: &[u8], trusted_key: &[u8]) -> Result<Self, String> {
        if bytes.len() < 12 {
            return Err("Package file too short".to_string());
        }

        if &bytes[0..8] != PACKAGE_MAGIC {
            return Err("Invalid package magic. Not a NovaOS delta package".to_string());
        }

        let manifest_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        if bytes.len() < 12 + manifest_len {
            return Err("Package corrupted: truncated manifest".to_string());
        }

        let manifest_str = std::str::from_utf8(&bytes[12..12 + manifest_len])
            .map_err(|e| format!("Invalid UTF-8 manifest: {}", e))?;
        let manifest = PackageManifest::deserialize(manifest_str)?;

        let payload = bytes[12 + manifest_len..].to_vec();
        if payload.len() != manifest.payload_size {
            return Err(format!(
                "Payload size mismatch: expected {} bytes, got {}",
                manifest.payload_size,
                payload.len()
            ));
        }

        // 1. Verify SHA-256 payload integrity
        let computed_sha256 = simple_sha256(&payload);
        if computed_sha256 != manifest.payload_sha256 {
            return Err("Payload SHA-256 checksum mismatch! Package has been tampered with or corrupted".to_string());
        }

        // 2. Verify HMAC-SHA256 signature
        let mut sig_input = Vec::new();
        sig_input.extend_from_slice(manifest.name.as_bytes());
        sig_input.extend_from_slice(manifest.version.as_bytes());
        sig_input.extend_from_slice(manifest.component.as_bytes());
        sig_input.extend_from_slice(&manifest.payload_sha256);

        let expected_signature = hmac_sha256(trusted_key, &sig_input);
        if !constant_time_eq(&manifest.signature, &expected_signature) {
            return Err("Cryptographic signature verification failed! Untrusted package rejected".to_string());
        }

        Ok(Self { manifest, payload })
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), String> {
        let bytes = self.encode();
        fs::write(path, bytes).map_err(|e| format!("Failed to write package to {:?}: {}", path, e))
    }

    pub fn load_from_file(path: &Path, trusted_key: &[u8]) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|e| format!("Failed to read package from {:?}: {}", path, e))?;
        Self::decode(&bytes, trusted_key)
    }
}

// =============================================================================
// Cryptographic Helpers
// =============================================================================

pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let key_hash = simple_sha256(key);
        k[..32].copy_from_slice(&key_hash);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5c;
    }

    // Inner hash: H(K ^ ipad || message)
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    // Outer hash: H(K ^ opad || inner_hash)
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finalize()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn hex_decode(hex: &str, out: &mut [u8]) {
    let hex = hex.trim();
    let bytes = hex.as_bytes();
    for i in 0..out.len() {
        if i * 2 + 1 < bytes.len() {
            let h = match bytes[i * 2] {
                b'0'..=b'9' => bytes[i * 2] - b'0',
                b'a'..=b'f' => bytes[i * 2] - b'a' + 10,
                b'A'..=b'F' => bytes[i * 2] - b'A' + 10,
                _ => 0,
            };
            let l = match bytes[i * 2 + 1] {
                b'0'..=b'9' => bytes[i * 2 + 1] - b'0',
                b'a'..=b'f' => bytes[i * 2 + 1] - b'a' + 10,
                b'A'..=b'F' => bytes[i * 2 + 1] - b'A' + 10,
                _ => 0,
            };
            out[i] = (h << 4) | l;
        }
    }
}
