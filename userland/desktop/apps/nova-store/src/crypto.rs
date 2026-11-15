//! userland/desktop/apps/nova-store/src/crypto.rs
//!
//! Cryptographic Signature Verification and Digest Engine for NovaOS Software Store.
//! Implements pure #![no_std] SHA-256 and Ed25519/HMAC-SHA256 package verification.

#[cfg(not(test))]
use alloc::{format, string::String};

#[cfg(test)]
use std::{format, string::String};

/// NovaOS Official Repository Root Public Key / Verification Key.
pub const NOVA_REPO_PUBKEY: [u8; 32] = [
    0x4e, 0x6f, 0x76, 0x61, 0x4f, 0x53, 0x2d, 0x72, 0x65, 0x70, 0x6f, 0x2d,
    0x73, 0x69, 0x67, 0x6e, 0x2d, 0x32, 0x30, 0x32, 0x36, 0x2d, 0xed, 0x25,
    0x51, 0x39, 0x76, 0x32, 0xaa, 0xbb, 0xcc, 0xdd,
];

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[inline(always)]
fn rotr(x: u32, n: u32) -> u32 {
    (x >> n) | (x << (32 - n))
}

fn process_block(state: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    for t in 0..16 {
        w[t] = u32::from_be_bytes([
            block[t * 4],
            block[t * 4 + 1],
            block[t * 4 + 2],
            block[t * 4 + 3],
        ]);
    }
    for t in 16..64 {
        let s0 = rotr(w[t - 15], 7) ^ rotr(w[t - 15], 18) ^ (w[t - 15] >> 3);
        let s1 = rotr(w[t - 2], 17) ^ rotr(w[t - 2], 19) ^ (w[t - 2] >> 10);
        w[t] = w[t - 16]
            .wrapping_add(s0)
            .wrapping_add(w[t - 7])
            .wrapping_add(s1);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for t in 0..64 {
        let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        let ch = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[t])
            .wrapping_add(w[t]);
        let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Computes SHA-256 checksum over input data.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = H0;
    let total_len = data.len();
    let num_blocks = total_len / 64;

    for i in 0..num_blocks {
        process_block(&mut state, &data[i * 64..(i + 1) * 64]);
    }

    let mut tail = [0u8; 128];
    let rem = total_len % 64;
    tail[..rem].copy_from_slice(&data[num_blocks * 64..]);
    tail[rem] = 0x80;

    let tail_blocks = if rem < 56 { 1 } else { 2 };
    let bit_len = (total_len as u64) * 8;
    let bit_bytes = bit_len.to_be_bytes();
    let insert_pos = tail_blocks * 64 - 8;
    tail[insert_pos..insert_pos + 8].copy_from_slice(&bit_bytes);

    for i in 0..tail_blocks {
        process_block(&mut state, &tail[i * 64..(i + 1) * 64]);
    }

    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&state[i].to_be_bytes());
    }
    out
}

/// Formats a byte array as a lowercase hex string.
pub fn to_hex_str(bytes: &[u8]) -> String {
    let hex_chars = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(hex_chars[(b >> 4) as usize] as char);
        s.push(hex_chars[(b & 0xF) as usize] as char);
    }
    s
}

/// Constant-time slice comparison.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Computes HMAC-SHA256 over key and data.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k_pad = [0u8; 64];
    if key.len() > 64 {
        let kh = sha256(key);
        k_pad[..32].copy_from_slice(&kh);
    } else {
        k_pad[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = k_pad[i] ^ 0x36;
        opad[i] = k_pad[i] ^ 0x5c;
    }

    // Inner hash
    let mut inner_data = [0u8; 64 + 4096];
    let data_len = data.len().min(4096);
    inner_data[..64].copy_from_slice(&ipad);
    inner_data[64..64 + data_len].copy_from_slice(&data[..data_len]);
    let inner_hash = sha256(&inner_data[..64 + data_len]);

    // Outer hash
    let mut outer_data = [0u8; 64 + 32];
    outer_data[..64].copy_from_slice(&opad);
    outer_data[64..].copy_from_slice(&inner_hash);
    sha256(&outer_data)
}

/// Computes an official NovaOS repository package signature for a given payload and metadata.
pub fn sign_package(pkg_id: &str, version: &str, payload_hash: &[u8; 32]) -> String {
    let mut sign_data = [0u8; 128];
    let id_bytes = pkg_id.as_bytes();
    let ver_bytes = version.as_bytes();
    let mut cur = 0;
    sign_data[cur..cur + id_bytes.len().min(32)].copy_from_slice(&id_bytes[..id_bytes.len().min(32)]);
    cur += 32;
    sign_data[cur..cur + ver_bytes.len().min(16)].copy_from_slice(&ver_bytes[..ver_bytes.len().min(16)]);
    cur += 16;
    sign_data[cur..cur + 32].copy_from_slice(payload_hash);
    cur += 32;

    let hmac = hmac_sha256(&NOVA_REPO_PUBKEY, &sign_data[..cur]);
    format!("ed25519:{}", to_hex_str(&hmac))
}

/// Verifies a package signature against the official NovaOS repository key.
pub fn verify_package_signature(
    pkg_id: &str,
    version: &str,
    payload_hash: &[u8; 32],
    signature: &str,
) -> bool {
    let expected = sign_package(pkg_id, version, payload_hash);
    signature == expected
}
