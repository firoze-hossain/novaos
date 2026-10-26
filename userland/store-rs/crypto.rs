//! userland/store-rs/crypto.rs - Cryptographic primitives for NovaOS Package Signing and Verification.
//! Implements SHA-256 (RFC 6234), HMAC-SHA256 (RFC 2104), and constant-time equality in pure #![no_std].

#![allow(dead_code)]

/// The shared HMAC key used across NovaOS kernel and host tools for package signing.
pub const PKG_VERIFY_KEY: [u8; 32] = [
    0x4e, 0x6f, 0x76, 0x61, 0x4f, 0x53, 0x2d, 0x70, 0x6b, 0x67, 0x2d, 0x73,
    0x69, 0x67, 0x6e, 0x2d, 0x76, 0x31, 0x2d, 0xa7, 0x3c, 0x91, 0x5d, 0x02,
    0xe8, 0x4f, 0x1b, 0x6a, 0xc3, 0x77, 0x0d, 0x29,
];

pub const PKG_HEADER_LEN: usize = 128;
pub const PKG_SIGNATURE_OFFSET: usize = 96;
pub const PKG_SIGNATURE_LEN: usize = 32;

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
        let big_s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        let ch = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(big_s1)
            .wrapping_add(ch)
            .wrapping_add(K[t])
            .wrapping_add(w[t]);
        let big_s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = big_s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
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

/// Streaming SHA-256 for chunked inputs of arbitrary length.
pub struct Sha256Streaming {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    total_len: u64,
}

impl Sha256Streaming {
    pub fn new() -> Self {
        Sha256Streaming {
            state: H0,
            buffer: [0u8; 64],
            buffer_len: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);

        if self.buffer_len > 0 {
            let need = 64 - self.buffer_len;
            let take = need.min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take]
                .copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];
            if self.buffer_len == 64 {
                let block = self.buffer;
                process_block(&mut self.state, &block);
                self.buffer_len = 0;
            }
        }

        while data.len() >= 64 {
            process_block(&mut self.state, &data[..64]);
            data = &data[64..];
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        let mut pad = [0u8; 128];
        pad[..self.buffer_len].copy_from_slice(&self.buffer[..self.buffer_len]);
        pad[self.buffer_len] = 0x80;
        let total_pad_len = ((self.buffer_len + 1 + 8 + 63) / 64) * 64;
        pad[total_pad_len - 8..total_pad_len].copy_from_slice(&bit_len.to_be_bytes());

        let mut offset = 0;
        while offset < total_pad_len {
            process_block(&mut self.state, &pad[offset..offset + 64]);
            offset += 64;
        }

        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256Streaming::new();
    hasher.update(data);
    hasher.finalize()
}

pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner = Sha256Streaming::new();
    let mut ipad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = key_block[i] ^ 0x36;
    }
    inner.update(&ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256Streaming::new();
    let mut opad = [0u8; 64];
    for i in 0..64 {
        opad[i] = key_block[i] ^ 0x5c;
    }
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finalize()
}

/// Constant-time comparison between two byte slices of equal length to prevent side-channel timing attacks.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Computes the HMAC-SHA256 signature for a package header and payload.
/// Sets the signature field in header to zero before computing digest.
pub fn compute_package_signature(header_bytes: &[u8], payload: &[u8]) -> [u8; 32] {
    if header_bytes.len() < PKG_HEADER_LEN {
        return [0u8; 32];
    }
    let mut hdr_copy = [0u8; PKG_HEADER_LEN];
    hdr_copy.copy_from_slice(&header_bytes[..PKG_HEADER_LEN]);
    hdr_copy[PKG_SIGNATURE_OFFSET..PKG_SIGNATURE_OFFSET + PKG_SIGNATURE_LEN].fill(0);

    let mut hasher = Sha256Streaming::new();
    hasher.update(&hdr_copy);
    hasher.update(payload);
    let digest = hasher.finalize();

    hmac_sha256(&PKG_VERIFY_KEY, &digest)
}

/// Verifies whether the signature embedded in `header_bytes` matches the recomputed HMAC-SHA256 signature.
pub fn verify_package_signature(header_bytes: &[u8], payload: &[u8]) -> bool {
    if header_bytes.len() < PKG_HEADER_LEN {
        return false;
    }
    let expected_sig = compute_package_signature(header_bytes, payload);
    let embedded_sig = &header_bytes[PKG_SIGNATURE_OFFSET..PKG_SIGNATURE_OFFSET + PKG_SIGNATURE_LEN];
    constant_time_eq(&expected_sig, embedded_sig)
}
