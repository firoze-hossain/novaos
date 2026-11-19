//! userland/desktop/apps/nova-installer/src/crypto.rs
//!
//! Cryptographic primitives for NovaOS Graphical Installer:
//! - SHA-256 (RFC 6234)
//! - HMAC-SHA256 (RFC 2104)
//! - PBKDF2-HMAC-SHA256 (RFC 8018) for user password hashing
//! - Constant-time comparison
//! - IEEE 802.3 CRC32 calculation for GPT partition headers

pub const SALT_LEN: usize = 16;
pub const HASH_LEN: usize = 32;
pub const DEFAULT_PBKDF2_ITERATIONS: u32 = 4096;

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

#[derive(Clone, Copy)]
pub struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: H0,
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.total_len += data.len() as u64;
        let mut offset = 0;
        if self.buf_len > 0 {
            let to_copy = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + to_copy]
                .copy_from_slice(&data[..to_copy]);
            self.buf_len += to_copy;
            offset += to_copy;
            if self.buf_len == 64 {
                let block = self.buf;
                process_block(&mut self.state, &block);
                self.buf_len = 0;
            }
        }
        while offset + 64 <= data.len() {
            process_block(&mut self.state, &data[offset..offset + 64]);
            offset += 64;
        }
        if offset < data.len() {
            let rem = data.len() - offset;
            self.buf[..rem].copy_from_slice(&data[offset..]);
            self.buf_len = rem;
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len * 8;
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;

        if self.buf_len > 56 {
            for i in self.buf_len..64 {
                self.buf[i] = 0;
            }
            let block = self.buf;
            process_block(&mut self.state, &block);
            self.buf = [0u8; 64];
            self.buf_len = 0;
        }

        for i in self.buf_len..56 {
            self.buf[i] = 0;
        }
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.buf;
        process_block(&mut self.state, &block);

        let mut out = [0u8; 32];
        for i in 0..8 {
            out[i * 4..i * 4 + 4].copy_from_slice(&self.state[i].to_be_bytes());
        }
        out
    }

    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut h = Self::new();
        h.update(data);
        h.finalize()
    }
}

pub struct HmacSha256 {
    inner: Sha256,
    opad_key: [u8; 64],
}

impl HmacSha256 {
    pub fn new(key: &[u8]) -> Self {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            let hashed = Sha256::digest(key);
            k[..32].copy_from_slice(&hashed);
        } else {
            k[..key.len()].copy_from_slice(key);
        }

        let mut ipad = [0u8; 64];
        let mut opad = [0u8; 64];
        for i in 0..64 {
            ipad[i] = k[i] ^ 0x36;
            opad[i] = k[i] ^ 0x5c;
        }

        let mut inner = Sha256::new();
        inner.update(&ipad);

        Self {
            inner,
            opad_key: opad,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; 32] {
        let inner_hash = self.inner.finalize();
        let mut outer = Sha256::new();
        outer.update(&self.opad_key);
        outer.update(&inner_hash);
        outer.finalize()
    }

    pub fn mac(key: &[u8], data: &[u8]) -> [u8; 32] {
        let mut hmac = Self::new(key);
        hmac.update(data);
        hmac.finalize()
    }
}

pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8; 32]) {
    let mut salt_and_block = [0u8; 128];
    let slen = salt.len().min(124);
    salt_and_block[..slen].copy_from_slice(&salt[..slen]);
    salt_and_block[slen..slen + 4].copy_from_slice(&1u32.to_be_bytes());

    let mut u = HmacSha256::mac(password, &salt_and_block[..slen + 4]);
    let mut t = u;

    for _ in 1..iterations {
        u = HmacSha256::mac(password, &u);
        for j in 0..32 {
            t[j] ^= u[j];
        }
    }

    out.copy_from_slice(&t);
}

/// Constant-time comparison for cryptographic hashes
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Computes IEEE 802.3 CRC-32 (polynomial 0xEDB88320)
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_empty_and_known_vectors() {
        // SHA-256("")
        let empty_hash = Sha256::digest(b"");
        assert_eq!(
            empty_hash,
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14,
                0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
                0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c,
                0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
            ]
        );

        // SHA-256("abc")
        let abc_hash = Sha256::digest(b"abc");
        assert_eq!(
            abc_hash,
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
                0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
                0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
                0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn test_pbkdf2_derivation_and_verification() {
        let password = b"NovaOS@2026!";
        let salt = b"salt_for_novaos_";
        let mut hash1 = [0u8; 32];
        pbkdf2_hmac_sha256(password, salt, 100, &mut hash1);

        let mut hash2 = [0u8; 32];
        pbkdf2_hmac_sha256(password, salt, 100, &mut hash2);

        assert!(ct_eq(&hash1, &hash2));

        let mut wrong_hash = [0u8; 32];
        pbkdf2_hmac_sha256(b"WrongPassword", salt, 100, &mut wrong_hash);
        assert!(!ct_eq(&hash1, &wrong_hash));
    }

    #[test]
    fn test_crc32_check() {
        let data = b"123456789";
        // Standard check value for CRC-32 on "123456789" is 0xCBF43926
        assert_eq!(crc32(data), 0xCBF43926);
    }
}
