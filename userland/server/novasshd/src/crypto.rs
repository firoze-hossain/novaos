//! userland/server/novasshd/src/crypto.rs
//!
//! Pure Rust Cryptographic Engine for OpenSSH-Compatible SSH-2 Server:
//! - SHA-256 and SHA-512 cryptographic hashing (RFC 6234)
//! - HMAC-SHA256 message authentication (RFC 2104)
//! - Curve25519 / X25519 ECDH key exchange (RFC 7748 / RFC 8731)
//! - Ed25519 digital signatures (RFC 8032 / RFC 8709)
//! - ChaCha20-Poly1305 authenticated encryption (chacha20-poly1305@openssh.com)
//! - AES-256-GCM authenticated encryption (aes256-gcm@openssh.com)
//! - RFC 4253 SSH key derivation function (KDF)
//! - SSH wire-format serialization (strings, mpint, name-lists)

// ============================================================================
// 1. SHA-256 Hashing
// ============================================================================

pub struct Sha256 {
    state: [u32; 8],
    count: u64,
    buffer: [u8; 64],
}

impl Sha256 {
    pub const fn new() -> Self {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            count: 0,
            buffer: [0u8; 64],
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        let idx = (self.count as usize) & 0x3F;
        self.count += data.len() as u64;

        let mut data_idx = 0;
        let mut len = data.len();

        if idx > 0 {
            let left = 64 - idx;
            if len >= left {
                self.buffer[idx..64].copy_from_slice(&data[..left]);
                Self::transform(&mut self.state, &self.buffer);
                data_idx += left;
                len -= left;
            } else {
                self.buffer[idx..idx + len].copy_from_slice(&data[..len]);
                return;
            }
        }

        while len >= 64 {
            Self::transform(&mut self.state, &data[data_idx..data_idx + 64]);
            data_idx += 64;
            len -= 64;
        }

        if len > 0 {
            self.buffer[..len].copy_from_slice(&data[data_idx..]);
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_count = self.count * 8;
        let idx = (self.count as usize) & 0x3F;
        let pad_len = if idx < 56 { 56 - idx } else { 120 - idx };

        let mut padding = [0u8; 128];
        padding[0] = 0x80;
        self.update(&padding[..pad_len]);
        self.update(&bit_count.to_be_bytes());

        let mut output = [0u8; 32];
        for (i, &val) in self.state.iter().enumerate() {
            output[i * 4..(i + 1) * 4].copy_from_slice(&val.to_be_bytes());
        }
        output
    }

    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finalize()
    }

    fn transform(state: &mut [u32; 8], block: &[u8]) {
        let k: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(k[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
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
}

// ============================================================================
// 2. SHA-512 Hashing
// ============================================================================

pub struct Sha512 {
    state: [u64; 8],
    count: u128,
    buffer: [u8; 128],
}

impl Sha512 {
    pub const fn new() -> Self {
        Sha512 {
            state: [
                0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
                0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
            ],
            count: 0,
            buffer: [0u8; 128],
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        let idx = (self.count as usize) & 0x7F;
        self.count += data.len() as u128;

        let mut data_idx = 0;
        let mut len = data.len();

        if idx > 0 {
            let left = 128 - idx;
            if len >= left {
                self.buffer[idx..128].copy_from_slice(&data[..left]);
                Self::transform(&mut self.state, &self.buffer);
                data_idx += left;
                len -= left;
            } else {
                self.buffer[idx..idx + len].copy_from_slice(&data[..len]);
                return;
            }
        }

        while len >= 128 {
            Self::transform(&mut self.state, &data[data_idx..data_idx + 128]);
            data_idx += 128;
            len -= 128;
        }

        if len > 0 {
            self.buffer[..len].copy_from_slice(&data[data_idx..]);
        }
    }

    pub fn finalize(mut self) -> [u8; 64] {
        let bit_count = self.count * 8;
        let idx = (self.count as usize) & 0x7F;
        let pad_len = if idx < 112 { 112 - idx } else { 240 - idx };

        let mut padding = [0u8; 256];
        padding[0] = 0x80;
        self.update(&padding[..pad_len]);
        self.update(&bit_count.to_be_bytes());

        let mut output = [0u8; 64];
        for (i, &val) in self.state.iter().enumerate() {
            output[i * 8..(i + 1) * 8].copy_from_slice(&val.to_be_bytes());
        }
        output
    }

    pub fn digest(data: &[u8]) -> [u8; 64] {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finalize()
    }

    fn transform(state: &mut [u64; 8], block: &[u8]) {
        let k: [u64; 80] = [
            0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
            0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
            0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
            0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
            0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
            0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
            0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
            0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
            0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
            0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
            0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
            0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
            0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
            0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
            0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
            0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
            0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
            0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
            0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
            0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
        ];

        let mut w = [0u64; 80];
        for i in 0..16 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&block[i * 8..(i + 1) * 8]);
            w[i] = u64::from_be_bytes(b);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(k[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
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
}

// ============================================================================
// 3. HMAC-SHA256
// ============================================================================

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let hash = Sha256::digest(key);
        k[..32].copy_from_slice(&hash);
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
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finalize()
}

// ============================================================================
// 4. Curve25519 / X25519 ECDH
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fe(pub u64, pub u64, pub u64, pub u64); // 4 limbs of 64 bits

impl Fe {
    const ZERO: Fe = Fe(0, 0, 0, 0);
    const ONE: Fe = Fe(1, 0, 0, 0);

    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let l0 = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let l1 = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let l2 = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let mut l3 = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        l3 &= 0x7FFFFFFFFFFFFFFF; // Clear bit 255
        Fe(l0, l1, l2, l3)
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        let mut reduced = *self;
        reduced.reduce_fully();
        let mut out = [0u8; 32];
        out[0..8].copy_from_slice(&reduced.0.to_le_bytes());
        out[8..16].copy_from_slice(&reduced.1.to_le_bytes());
        out[16..24].copy_from_slice(&reduced.2.to_le_bytes());
        out[24..32].copy_from_slice(&reduced.3.to_le_bytes());
        out
    }

    pub const P0: u64 = 0xFFFFFFFFFFFFFFED;
    pub const P1: u64 = 0xFFFFFFFFFFFFFFFF;
    pub const P2: u64 = 0xFFFFFFFFFFFFFFFF;
    pub const P3: u64 = 0x7FFFFFFFFFFFFFFF;

    pub fn add(&self, rhs: &Fe) -> Fe {
        let (r0, c0) = self.0.overflowing_add(rhs.0);
        let (r1, c1) = self.1.overflowing_add(rhs.1);
        let (r1, c1_2) = r1.overflowing_add(c0 as u64);
        let (r2, c2) = self.2.overflowing_add(rhs.2);
        let (r2, c2_2) = r2.overflowing_add((c1 || c1_2) as u64);
        let (r3, c3) = self.3.overflowing_add(rhs.3);
        let (r3, c3_2) = r3.overflowing_add((c2 || c2_2) as u64);
        let carry = (c3 || c3_2) as u64;

        let ge_p = carry != 0
            || r3 > Self::P3
            || (r3 == Self::P3
                && (r2 > Self::P2
                    || (r2 == Self::P2
                        && (r1 > Self::P1 || (r1 == Self::P1 && r0 >= Self::P0)))));

        if ge_p {
            let (d0, b0) = r0.overflowing_sub(Self::P0);
            let (d1, b1) = r1.overflowing_sub(Self::P1);
            let (d1, b1_2) = d1.overflowing_sub(b0 as u64);
            let (d2, b2) = r2.overflowing_sub(Self::P2);
            let (d2, b2_2) = d2.overflowing_sub((b1 || b1_2) as u64);
            let (d3, _b3) = r3.overflowing_sub(Self::P3);
            let (d3, _b3_2) = d3.overflowing_sub((b2 || b2_2) as u64);
            Fe(d0, d1, d2, d3 & 0x7FFFFFFFFFFFFFFF)
        } else {
            Fe(r0, r1, r2, r3)
        }
    }

    pub fn sub(&self, rhs: &Fe) -> Fe {
        let (r0, b0) = self.0.overflowing_sub(rhs.0);
        let (r1, b1) = self.1.overflowing_sub(rhs.1);
        let (r1, b1_2) = r1.overflowing_sub(b0 as u64);
        let (r2, b2) = self.2.overflowing_sub(rhs.2);
        let (r2, b2_2) = r2.overflowing_sub((b1 || b1_2) as u64);
        let (r3, b3) = self.3.overflowing_sub(rhs.3);
        let (r3, b3_2) = r3.overflowing_sub((b2 || b2_2) as u64);

        if b3 || b3_2 {
            let (s0, c0) = r0.overflowing_add(Self::P0);
            let (s1, c1) = r1.overflowing_add(Self::P1);
            let (s1, c1_2) = s1.overflowing_add(c0 as u64);
            let (s2, c2) = r2.overflowing_add(Self::P2);
            let (s2, c2_2) = s2.overflowing_add((c1 || c1_2) as u64);
            let (s3, _c3) = r3.overflowing_add(Self::P3);
            let (s3, _c3_2) = s3.overflowing_add((c2 || c2_2) as u64);
            Fe(s0, s1, s2, s3 & 0x7FFFFFFFFFFFFFFF)
        } else {
            Fe(r0, r1, r2, r3)
        }
    }

    pub fn mul(&self, rhs: &Fe) -> Fe {
        let a = [
            (self.0 & 0xFFFFFFFF) as u128,
            ((self.0 >> 32) & 0xFFFFFFFF) as u128,
            (self.1 & 0xFFFFFFFF) as u128,
            ((self.1 >> 32) & 0xFFFFFFFF) as u128,
            (self.2 & 0xFFFFFFFF) as u128,
            ((self.2 >> 32) & 0xFFFFFFFF) as u128,
            (self.3 & 0xFFFFFFFF) as u128,
            ((self.3 >> 32) & 0xFFFFFFFF) as u128,
        ];
        let b = [
            (rhs.0 & 0xFFFFFFFF) as u128,
            ((rhs.0 >> 32) & 0xFFFFFFFF) as u128,
            (rhs.1 & 0xFFFFFFFF) as u128,
            ((rhs.1 >> 32) & 0xFFFFFFFF) as u128,
            (rhs.2 & 0xFFFFFFFF) as u128,
            ((rhs.2 >> 32) & 0xFFFFFFFF) as u128,
            (rhs.3 & 0xFFFFFFFF) as u128,
            ((rhs.3 >> 32) & 0xFFFFFFFF) as u128,
        ];

        let mut prod = [0u128; 16];
        for i in 0..8 {
            for j in 0..8 {
                prod[i + j] += a[i] * b[j];
            }
        }

        let mut carry = 0u128;
        for i in 0..15 {
            prod[i] += carry;
            carry = prod[i] >> 32;
            prod[i] &= 0xFFFFFFFF;
        }
        prod[15] += carry;

        let mut r = [
            prod[0], prod[1], prod[2], prod[3],
            prod[4], prod[5], prod[6], prod[7],
        ];

        let mut c = 0u128;
        for i in 0..8 {
            let term = prod[8 + i] * 38 + c;
            r[i] += term & 0xFFFFFFFF;
            c = term >> 32;
            if r[i] > 0xFFFFFFFF {
                c += r[i] >> 32;
                r[i] &= 0xFFFFFFFF;
            }
        }

        let mut final_carry = c * 38;
        r[0] += final_carry;
        c = r[0] >> 32;
        r[0] &= 0xFFFFFFFF;
        for i in 1..8 {
            r[i] += c;
            c = r[i] >> 32;
            r[i] &= 0xFFFFFFFF;
        }
        if c > 0 {
            final_carry = c * 38;
            r[0] += final_carry;
            c = r[0] >> 32;
            r[0] &= 0xFFFFFFFF;
            for i in 1..8 {
                r[i] += c;
                c = r[i] >> 32;
                r[i] &= 0xFFFFFFFF;
            }
        }

        let l0 = (r[0] as u64) | ((r[1] as u64) << 32);
        let l1 = (r[2] as u64) | ((r[3] as u64) << 32);
        let l2 = (r[4] as u64) | ((r[5] as u64) << 32);
        let l3 = (r[6] as u64) | ((r[7] as u64) << 32);

        let mut fe = Fe(l0, l1, l2, l3);
        fe.reduce_fully();
        fe
    }

    pub fn square(&self) -> Fe {
        self.mul(self)
    }

    fn mul_scalar(&self, s: u64) -> Fe {
        self.mul(&Fe(s, 0, 0, 0))
    }

    pub fn invert(&self) -> Fe {
        let mut res = Fe::ONE;
        let mut base = *self;
        for bit in 0..255 {
            if bit != 2 && bit != 4 {
                res = res.mul(&base);
            }
            base = base.square();
        }
        res
    }

    fn reduce_fully(&mut self) {
        let (r0, b0) = self.0.overflowing_sub(0xFFFFFFFFFFFFFFED);
        let (r1, b1) = self.1.overflowing_sub(0xFFFFFFFFFFFFFFFF);
        let (r1, b1_2) = r1.overflowing_sub(b0 as u64);
        let (r2, b2) = self.2.overflowing_sub(0xFFFFFFFFFFFFFFFF);
        let (r2, b2_2) = r2.overflowing_sub((b1 || b1_2) as u64);
        let (r3, b3) = self.3.overflowing_sub(0x7FFFFFFFFFFFFFFF);
        let (r3, b3_2) = r3.overflowing_sub((b2 || b2_2) as u64);

        if !(b3 || b3_2) {
            self.0 = r0;
            self.1 = r1;
            self.2 = r2;
            self.3 = r3;
        }
    }

    fn cswap(a: &mut Fe, b: &mut Fe, swap: bool) {
        let mask = if swap { 0xFFFFFFFFFFFFFFFF } else { 0 };
        let d0 = (a.0 ^ b.0) & mask;
        let d1 = (a.1 ^ b.1) & mask;
        let d2 = (a.2 ^ b.2) & mask;
        let d3 = (a.3 ^ b.3) & mask;
        a.0 ^= d0; a.1 ^= d1; a.2 ^= d2; a.3 ^= d3;
        b.0 ^= d0; b.1 ^= d1; b.2 ^= d2; b.3 ^= d3;
    }

    pub fn neg(&self) -> Fe {
        Fe::ZERO.sub(self)
    }

    pub fn is_odd(&self) -> bool {
        let mut reduced = *self;
        reduced.reduce_fully();
        (reduced.0 & 1) != 0
    }

    pub fn pow_p58(&self) -> Fe {
        let mut res = Fe::ONE;
        let mut base = *self;
        for i in 0..252 {
            if i != 1 {
                res = res.mul(&base);
            }
            base = base.square();
        }
        res
    }

    pub const D: Fe = Fe(
        0x75eb4dca135978a3,
        0x00700a4d4141d8ab,
        0x8cc740797779e898,
        0x52036cee2b6ffe73,
    );

    pub const D2: Fe = Fe(
        0xebd69b9426b2f159,
        0x00e0149a8283b156,
        0x198e80f2eef3d130,
        0x2406d9dc56dffce7,
    );

    pub const I: Fe = Fe(
        0xc4ee1b274a0ea0b0,
        0x2f431806ad2fe478,
        0x2b4d00993dfbd7a7,
        0x2b8324804fc1df0b,
    );

    pub const B_X: Fe = Fe(
        0xc9562d608f25d51a,
        0x692cc7609525a7b2,
        0xc0a4e231fdd6dc5c,
        0x216936d3cd6e53fe,
    );

    pub const B_Y: Fe = Fe(
        0x6666666666666658,
        0x6666666666666666,
        0x6666666666666666,
        0x6666666666666666,
    );
}

pub fn x25519(scalar: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    let u = Fe::from_bytes(point);
    let x_1 = u;
    let mut x_2 = Fe::ONE;
    let mut z_2 = Fe::ZERO;
    let mut x_3 = u;
    let mut z_3 = Fe::ONE;
    let mut swap = false;

    for t in (0..255).rev() {
        let byte_idx = t / 8;
        let bit_idx = t % 8;
        let k_t = ((k[byte_idx] >> bit_idx) & 1) != 0;

        swap ^= k_t;
        Fe::cswap(&mut x_2, &mut x_3, swap);
        Fe::cswap(&mut z_2, &mut z_3, swap);
        swap = k_t;

        let a = x_2.add(&z_2);
        let aa = a.square();
        let b = x_2.sub(&z_2);
        let bb = b.square();
        let e = aa.sub(&bb);
        let c = x_3.add(&z_3);
        let d = x_3.sub(&z_3);
        let da = d.mul(&a);
        let cb = c.mul(&b);

        x_3 = da.add(&cb).square();
        z_3 = x_1.mul(&da.sub(&cb).square());
        x_2 = aa.mul(&bb);
        z_2 = e.mul(&aa.add(&e.mul_scalar(121666)));
    }

    Fe::cswap(&mut x_2, &mut x_3, swap);
    Fe::cswap(&mut z_2, &mut z_3, swap);

    let res = x_2.mul(&z_2.invert());
    res.to_bytes()
}

pub fn x25519_base_point() -> [u8; 32] {
    let mut p = [0u8; 32];
    p[0] = 9;
    p
}

// ============================================================================
// 5. ChaCha20 Stream Cipher & Poly1305 MAC (chacha20-poly1305@openssh.com)
// ============================================================================

pub struct ChaCha20 {
    state: [u32; 16],
}

impl ChaCha20 {
    pub fn new(key: &[u8; 32], nonce: &[u8; 12], counter: u32) -> Self {
        let mut state = [0u32; 16];
        state[0] = 0x61707865;
        state[1] = 0x3320646e;
        state[2] = 0x79622d32;
        state[3] = 0x6b206574;

        for i in 0..8 {
            state[4 + i] = u32::from_le_bytes(key[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        state[12] = counter;
        for i in 0..3 {
            state[13 + i] = u32::from_le_bytes(nonce[i * 4..(i + 1) * 4].try_into().unwrap());
        }

        ChaCha20 { state }
    }

    pub fn apply_keystream(&mut self, data: &mut [u8]) {
        let mut offset = 0;
        while offset < data.len() {
            let mut block = self.state;
            for _ in 0..10 {
                // Quarter rounds
                Self::quarter_round(&mut block, 0, 4, 8, 12);
                Self::quarter_round(&mut block, 1, 5, 9, 13);
                Self::quarter_round(&mut block, 2, 6, 10, 14);
                Self::quarter_round(&mut block, 3, 7, 11, 15);
                Self::quarter_round(&mut block, 0, 5, 10, 15);
                Self::quarter_round(&mut block, 1, 6, 11, 12);
                Self::quarter_round(&mut block, 2, 7, 8, 13);
                Self::quarter_round(&mut block, 3, 4, 9, 14);
            }

            let mut key_bytes = [0u8; 64];
            for i in 0..16 {
                let val = block[i].wrapping_add(self.state[i]);
                key_bytes[i * 4..(i + 1) * 4].copy_from_slice(&val.to_le_bytes());
            }

            let chunk_len = (data.len() - offset).min(64);
            for i in 0..chunk_len {
                data[offset + i] ^= key_bytes[i];
            }
            offset += chunk_len;
            self.state[12] = self.state[12].wrapping_add(1);
        }
    }

    #[inline(always)]
    fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        s[a] = s[a].wrapping_add(s[b]); s[d] = (s[d] ^ s[a]).rotate_left(16);
        s[c] = s[c].wrapping_add(s[d]); s[b] = (s[b] ^ s[c]).rotate_left(12);
        s[a] = s[a].wrapping_add(s[b]); s[d] = (s[d] ^ s[a]).rotate_left(8);
        s[c] = s[c].wrapping_add(s[d]); s[b] = (s[b] ^ s[c]).rotate_left(7);
    }
}

pub struct Poly1305 {
    r: [u32; 5],
    h: [u32; 5],
    pad: [u32; 4],
    buffer: [u8; 16],
    buf_len: usize,
}

impl Poly1305 {
    pub fn new(key: &[u8; 32]) -> Self {
        let t0 = u32::from_le_bytes(key[0..4].try_into().unwrap());
        let t1 = u32::from_le_bytes(key[4..8].try_into().unwrap());
        let t2 = u32::from_le_bytes(key[8..12].try_into().unwrap());
        let t3 = u32::from_le_bytes(key[12..16].try_into().unwrap());

        let r0 = t0 & 0x3FFFFFF;
        let r1 = ((t0 >> 26) | (t1 << 6)) & 0x3FFFF03;
        let r2 = ((t1 >> 20) | (t2 << 12)) & 0x3FFC0FF;
        let r3 = ((t2 >> 14) | (t3 << 18)) & 0x3F03FFF;
        let r4 = (t3 >> 8) & 0x00FFFFF;

        let pad0 = u32::from_le_bytes(key[16..20].try_into().unwrap());
        let pad1 = u32::from_le_bytes(key[20..24].try_into().unwrap());
        let pad2 = u32::from_le_bytes(key[24..28].try_into().unwrap());
        let pad3 = u32::from_le_bytes(key[28..32].try_into().unwrap());

        Poly1305 {
            r: [r0, r1, r2, r3, r4],
            h: [0; 5],
            pad: [pad0, pad1, pad2, pad3],
            buffer: [0u8; 16],
            buf_len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        if self.buf_len > 0 {
            let left = 16 - self.buf_len;
            if data.len() >= left {
                self.buffer[self.buf_len..16].copy_from_slice(&data[..left]);
                let block = self.buffer;
                self.process_block(&block, false);
                data = &data[left..];
                self.buf_len = 0;
            } else {
                self.buffer[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
        }

        while data.len() >= 16 {
            let mut block = [0u8; 16];
            block.copy_from_slice(&data[..16]);
            self.process_block(&block, false);
            data = &data[16..];
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    pub fn finalize(mut self) -> [u8; 16] {
        if self.buf_len > 0 {
            let mut block = [0u8; 16];
            block[..self.buf_len].copy_from_slice(&self.buffer[..self.buf_len]);
            block[self.buf_len] = 1; // Append high bit
            self.process_block(&block, true);
        }

        let mut h = self.h;
        let mut c = h[0] >> 26; h[0] &= 0x3FFFFFF;
        h[1] += c; c = h[1] >> 26; h[1] &= 0x3FFFFFF;
        h[2] += c; c = h[2] >> 26; h[2] &= 0x3FFFFFF;
        h[3] += c; c = h[3] >> 26; h[3] &= 0x3FFFFFF;
        h[4] += c; c = h[4] >> 26; h[4] &= 0x3FFFFFF;
        h[0] += c * 5;

        let g0 = h[0].wrapping_add(5); c = g0 >> 26;
        let g1 = h[1].wrapping_add(c); c = g1 >> 26;
        let g2 = h[2].wrapping_add(c); c = g2 >> 26;
        let g3 = h[3].wrapping_add(c); c = g3 >> 26;
        let g4 = h[4].wrapping_add(c).wrapping_sub(1 << 26);

        let mask = (g4 >> 31).wrapping_sub(1);
        let gmask = !mask;
        h[0] = (h[0] & mask) | ((g0 & 0x3FFFFFF) & gmask);
        h[1] = (h[1] & mask) | ((g1 & 0x3FFFFFF) & gmask);
        h[2] = (h[2] & mask) | ((g2 & 0x3FFFFFF) & gmask);
        h[3] = (h[3] & mask) | ((g3 & 0x3FFFFFF) & gmask);
        h[4] = (h[4] & mask) | ((g4 & 0x3FFFFFF) & gmask);

        let f0 = ((h[0]) | (h[1] << 26)) as u64 + self.pad[0] as u64;
        let f1 = ((h[1] >> 6) | (h[2] << 20)) as u64 + self.pad[1] as u64 + (f0 >> 32);
        let f2 = ((h[2] >> 12) | (h[3] << 14)) as u64 + self.pad[2] as u64 + (f1 >> 32);
        let f3 = ((h[3] >> 18) | (h[4] << 8)) as u64 + self.pad[3] as u64 + (f2 >> 32);

        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&(f0 as u32).to_le_bytes());
        out[4..8].copy_from_slice(&(f1 as u32).to_le_bytes());
        out[8..12].copy_from_slice(&(f2 as u32).to_le_bytes());
        out[12..16].copy_from_slice(&(f3 as u32).to_le_bytes());
        out
    }

    fn process_block(&mut self, block: &[u8; 16], is_final: bool) {
        let t0 = u32::from_le_bytes(block[0..4].try_into().unwrap());
        let t1 = u32::from_le_bytes(block[4..8].try_into().unwrap());
        let t2 = u32::from_le_bytes(block[8..12].try_into().unwrap());
        let t3 = u32::from_le_bytes(block[12..16].try_into().unwrap());

        self.h[0] += t0 & 0x3FFFFFF;
        self.h[1] += ((t0 >> 26) | (t1 << 6)) & 0x3FFFFFF;
        self.h[2] += ((t1 >> 20) | (t2 << 12)) & 0x3FFFFFF;
        self.h[3] += ((t2 >> 14) | (t3 << 18)) & 0x3FFFFFF;
        self.h[4] += (t3 >> 8) | (if is_final { 0 } else { 1 << 24 });

        let h0 = self.h[0] as u64;
        let h1 = self.h[1] as u64;
        let h2 = self.h[2] as u64;
        let h3 = self.h[3] as u64;
        let h4 = self.h[4] as u64;

        let r0 = self.r[0] as u64;
        let r1 = self.r[1] as u64;
        let r2 = self.r[2] as u64;
        let r3 = self.r[3] as u64;
        let r4 = self.r[4] as u64;

        let s1 = r1 * 5;
        let s2 = r2 * 5;
        let s3 = r3 * 5;
        let s4 = r4 * 5;

        let d0 = h0 * r0 + h1 * s4 + h2 * s3 + h3 * s2 + h4 * s1;
        let d1 = h0 * r1 + h1 * r0 + h2 * s4 + h3 * s3 + h4 * s2;
        let d2 = h0 * r2 + h1 * r1 + h2 * r0 + h3 * s4 + h4 * s3;
        let d3 = h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * s4;
        let d4 = h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0;

        let mut c: u64;
        self.h[0] = (d0 & 0x3FFFFFF) as u32; c = d0 >> 26;
        let t = d1 + c; self.h[1] = (t & 0x3FFFFFF) as u32; c = t >> 26;
        let t = d2 + c; self.h[2] = (t & 0x3FFFFFF) as u32; c = t >> 26;
        let t = d3 + c; self.h[3] = (t & 0x3FFFFFF) as u32; c = t >> 26;
        let t = d4 + c; self.h[4] = (t & 0x3FFFFFF) as u32; c = t >> 26;
        self.h[0] += (c * 5) as u32;
    }
}

/// OpenSSH ChaCha20-Poly1305 AEAD packet encryptor.
pub fn chacha20_poly1305_openssh_encrypt(
    k_main: &[u8; 32],
    k_len: &[u8; 32],
    seq: u32,
    packet_len: u32,
    payload_and_padding: &mut [u8],
) -> (Vec<u8>, [u8; 16]) {
    let mut nonce = [0u8; 12];
    nonce[8..12].copy_from_slice(&seq.to_be_bytes());

    // 1. Encrypt 4-byte packet length with k_len
    let mut enc_len = packet_len.to_be_bytes().to_vec();
    let mut cipher_len = ChaCha20::new(k_len, &nonce, 0);
    cipher_len.apply_keystream(&mut enc_len);

    // 2. Derive Poly1305 one-time key with k_main counter 0
    let mut poly_key_block = [0u8; 64];
    let mut cipher_poly = ChaCha20::new(k_main, &nonce, 0);
    cipher_poly.apply_keystream(&mut poly_key_block);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&poly_key_block[..32]);

    // 3. Encrypt payload and padding with k_main counter 1
    let mut cipher_payload = ChaCha20::new(k_main, &nonce, 1);
    cipher_payload.apply_keystream(payload_and_padding);

    // 4. Compute Poly1305 MAC over (encrypted length || encrypted payload)
    let mut poly = Poly1305::new(&poly_key);
    poly.update(&enc_len);
    poly.update(payload_and_padding);
    let tag = poly.finalize();

    (enc_len, tag)
}

/// OpenSSH ChaCha20-Poly1305 AEAD packet decryptor.
pub fn chacha20_poly1305_openssh_decrypt(
    k_main: &[u8; 32],
    k_len: &[u8; 32],
    seq: u32,
    enc_len: &[u8; 4],
    enc_payload: &mut [u8],
    expected_tag: &[u8; 16],
) -> Result<u32, &'static str> {
    let mut nonce = [0u8; 12];
    nonce[8..12].copy_from_slice(&seq.to_be_bytes());

    // 1. Derive Poly1305 key
    let mut poly_key_block = [0u8; 64];
    let mut cipher_poly = ChaCha20::new(k_main, &nonce, 0);
    cipher_poly.apply_keystream(&mut poly_key_block);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&poly_key_block[..32]);

    // 2. Verify MAC tag
    let mut poly = Poly1305::new(&poly_key);
    poly.update(enc_len);
    poly.update(enc_payload);
    let tag = poly.finalize();
    if &tag != expected_tag {
        return Err("Poly1305 authentication failed");
    }

    // 3. Decrypt packet length
    let mut len_bytes = *enc_len;
    let mut cipher_len = ChaCha20::new(k_len, &nonce, 0);
    cipher_len.apply_keystream(&mut len_bytes);
    let packet_len = u32::from_be_bytes(len_bytes);

    // 4. Decrypt payload
    let mut cipher_payload = ChaCha20::new(k_main, &nonce, 1);
    cipher_payload.apply_keystream(enc_payload);

    Ok(packet_len)
}

// ============================================================================
// 6. RFC 4253 SSH Key Derivation Function (KDF)
// ============================================================================

pub struct SshKeys {
    pub iv_c2s: Vec<u8>,
    pub iv_s2c: Vec<u8>,
    pub enc_c2s: Vec<u8>,
    pub enc_s2c: Vec<u8>,
    pub mac_c2s: Vec<u8>,
    pub mac_s2c: Vec<u8>,
}

pub fn derive_ssh_keys(k: &[u8], h: &[u8], session_id: &[u8], enc_len: usize, mac_len: usize, iv_len: usize) -> SshKeys {
    let derive = |ch: u8, target_len: usize| -> Vec<u8> {
        let mut key = Vec::new();
        let mut hasher = Sha256::new();
        hasher.update(k);
        hasher.update(h);
        hasher.update(&[ch]);
        hasher.update(session_id);
        let mut k1 = hasher.finalize();
        key.extend_from_slice(&k1);

        while key.len() < target_len {
            let mut hasher = Sha256::new();
            hasher.update(k);
            hasher.update(h);
            hasher.update(&k1);
            k1 = hasher.finalize();
            key.extend_from_slice(&k1);
        }
        key.truncate(target_len);
        key
    };

    SshKeys {
        iv_c2s: derive(b'A', iv_len),
        iv_s2c: derive(b'B', iv_len),
        enc_c2s: derive(b'C', enc_len),
        enc_s2c: derive(b'D', enc_len),
        mac_c2s: derive(b'E', mac_len),
        mac_s2c: derive(b'F', mac_len),
    }
}

// ============================================================================
// 7. SSH Wire Serialization & Deserialization
// ============================================================================

pub fn put_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_be_bytes());
}

pub fn put_string(buf: &mut Vec<u8>, data: &[u8]) {
    put_u32(buf, data.len() as u32);
    buf.extend_from_slice(data);
}

pub fn put_str(buf: &mut Vec<u8>, s: &str) {
    put_string(buf, s.as_bytes());
}

pub fn put_mpint(buf: &mut Vec<u8>, val: &[u8]) {
    // Strip leading zeros
    let mut start = 0;
    while start < val.len() && val[start] == 0 {
        start += 1;
    }
    if start == val.len() {
        put_u32(buf, 0);
        return;
    }
    let slice = &val[start..];
    if (slice[0] & 0x80) != 0 {
        put_u32(buf, (slice.len() + 1) as u32);
        buf.push(0);
    } else {
        put_u32(buf, slice.len() as u32);
    }
    buf.extend_from_slice(slice);
}

pub fn get_u32(buf: &[u8], offset: &mut usize) -> Result<u32, &'static str> {
    if *offset + 4 > buf.len() {
        return Err("Buffer underrun reading u32");
    }
    let val = u32::from_be_bytes(buf[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    Ok(val)
}

pub fn get_string<'a>(buf: &'a [u8], offset: &mut usize) -> Result<&'a [u8], &'static str> {
    let len = get_u32(buf, offset)? as usize;
    if *offset + len > buf.len() {
        return Err("Buffer underrun reading string");
    }
    let slice = &buf[*offset..*offset + len];
    *offset += len;
    Ok(slice)
}

pub fn get_str<'a>(buf: &'a [u8], offset: &mut usize) -> Result<&'a str, &'static str> {
    let slice = get_string(buf, offset)?;
    core::str::from_utf8(slice).map_err(|_| "Invalid UTF-8 string")
}

// ============================================================================
// 8. Edwards25519 Point Arithmetic & Ed25519 Digital Signatures (RFC 8032)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdwardsPoint {
    pub x: Fe,
    pub y: Fe,
    pub z: Fe,
    pub t: Fe,
}

impl EdwardsPoint {
    pub const IDENTITY: EdwardsPoint = EdwardsPoint {
        x: Fe::ZERO,
        y: Fe::ONE,
        z: Fe::ONE,
        t: Fe::ZERO,
    };

    pub fn base_point() -> Self {
        EdwardsPoint {
            x: Fe::B_X,
            y: Fe::B_Y,
            z: Fe::ONE,
            t: Fe::B_X.mul(&Fe::B_Y),
        }
    }

    pub fn double(&self) -> Self {
        self.add(self)
    }

    pub fn add(&self, rhs: &Self) -> Self {
        let a = self.x.mul(&rhs.x);
        let b = self.y.mul(&rhs.y);
        let c = Fe::D.mul(&self.t.mul(&rhs.t));
        let d = self.z.mul(&rhs.z);
        let e = (self.x.add(&self.y)).mul(&(rhs.x.add(&rhs.y))).sub(&a).sub(&b);
        let f = d.sub(&c);
        let g = d.add(&c);
        let h = b.add(&a);

        EdwardsPoint {
            x: e.mul(&f),
            y: g.mul(&h),
            z: f.mul(&g),
            t: e.mul(&h),
        }
    }

    pub fn scalar_mul(&self, scalar: &[u8; 32]) -> Self {
        let mut res = Self::IDENTITY;
        for bit_idx in (0..256).rev() {
            res = res.double();
            let byte = bit_idx / 8;
            let bit = bit_idx % 8;
            if ((scalar[byte] >> bit) & 1) != 0 {
                res = res.add(self);
            }
        }
        res
    }

    pub fn compress(&self) -> [u8; 32] {
        let z_inv = self.z.invert();
        let x = self.x.mul(&z_inv);
        let y = self.y.mul(&z_inv);

        let mut out = y.to_bytes();
        if x.is_odd() {
            out[31] |= 0x80;
        }
        out
    }

    pub fn decompress(bytes: &[u8; 32]) -> Option<Self> {
        let mut y_bytes = *bytes;
        let sign_x = (y_bytes[31] >> 7) != 0;
        y_bytes[31] &= 0x7F;

        let y = Fe::from_bytes(&y_bytes);
        let y2 = y.square();
        let u = y2.sub(&Fe::ONE);
        let v = Fe::D.mul(&y2).add(&Fe::ONE);

        let v3 = v.square().mul(&v);
        let v7 = v3.square().mul(&v);
        let uv7 = u.mul(&v7);
        let uv3 = u.mul(&v3);

        let candidate = uv3.mul(&uv7.pow_p58());
        let cand2_v = candidate.square().mul(&v);

        let x = if cand2_v.to_bytes() == u.to_bytes() {
            candidate
        } else if cand2_v.to_bytes() == u.neg().to_bytes() {
            candidate.mul(&Fe::I)
        } else {
            return None;
        };

        let mut x_final = x;
        if x_final.is_odd() != sign_x {
            x_final = x_final.neg();
        }

        Some(EdwardsPoint {
            x: x_final,
            y,
            z: Fe::ONE,
            t: x_final.mul(&y),
        })
    }
}

pub fn mod_l_512(val: &[u8; 64]) -> [u8; 32] {
    let mut num = [0u64; 8];
    for i in 0..8 {
        num[i] = u64::from_le_bytes(val[i * 8..(i + 1) * 8].try_into().unwrap());
    }

    let l = [
        0x5812631a5cf5d3edu64,
        0x14def9dea2f79cd6u64,
        0x0000000000000000u64,
        0x1000000000000000u64,
    ];

    for k in (0..=259).rev() {
        let word_shift = k / 64;
        let bit_shift = k % 64;
        let mut shifted = [0u64; 8];

        for i in 0..4 {
            let low = l[i] << bit_shift;
            let high = if bit_shift > 0 { l[i] >> (64 - bit_shift) } else { 0 };
            shifted[i + word_shift] |= low;
            if i + word_shift + 1 < 8 {
                shifted[i + word_shift + 1] |= high;
            }
        }

        let mut ge = true;
        for i in (0..8).rev() {
            if num[i] > shifted[i] {
                break;
            } else if num[i] < shifted[i] {
                ge = false;
                break;
            }
        }

        if ge {
            let mut borrow = 0u64;
            for i in 0..8 {
                let (diff1, b1) = num[i].overflowing_sub(shifted[i]);
                let (diff2, b2) = diff1.overflowing_sub(borrow);
                num[i] = diff2;
                borrow = (b1 || b2) as u64;
            }
        }
    }

    let mut out = [0u8; 32];
    for i in 0..4 {
        out[i * 8..(i + 1) * 8].copy_from_slice(&num[i].to_le_bytes());
    }
    out
}

pub fn sc_mul_add(k_bytes: &[u8; 32], s_bytes: &[u8; 32], r_bytes: &[u8; 32]) -> [u8; 32] {
    let mut k_l = [0u64; 8];
    let mut s_l = [0u64; 8];
    for i in 0..8 {
        k_l[i] = u32::from_le_bytes(k_bytes[i * 4..(i + 1) * 4].try_into().unwrap()) as u64;
        s_l[i] = u32::from_le_bytes(s_bytes[i * 4..(i + 1) * 4].try_into().unwrap()) as u64;
    }

    let mut prod = [0u128; 16];
    for i in 0..8 {
        for j in 0..8 {
            prod[i + j] += (k_l[i] as u128) * (s_l[j] as u128);
        }
    }

    for i in 0..8 {
        let r_val = u32::from_le_bytes(r_bytes[i * 4..(i + 1) * 4].try_into().unwrap()) as u128;
        prod[i] += r_val;
    }

    let mut carry = 0u128;
    for i in 0..15 {
        prod[i] += carry;
        carry = prod[i] >> 32;
        prod[i] &= 0xFFFFFFFF;
    }
    prod[15] += carry;

    let mut prod_bytes = [0u8; 64];
    for i in 0..16 {
        let chunk = (prod[i] as u32).to_le_bytes();
        prod_bytes[i * 4..(i + 1) * 4].copy_from_slice(&chunk);
    }

    mod_l_512(&prod_bytes)
}

#[derive(Clone, Debug)]
pub struct Ed25519Keypair {
    pub seed: [u8; 32],
    pub public_key: [u8; 32],
}

impl Ed25519Keypair {
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = f.read_exact(&mut seed);
        } else {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let hash = Sha256::digest(&t.to_le_bytes());
            seed.copy_from_slice(&hash);
        }
        Self::from_seed(&seed)
    }

    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let h = Sha512::digest(seed);
        let mut a = [0u8; 32];
        a.copy_from_slice(&h[0..32]);
        a[0] &= 248;
        a[31] &= 127;
        a[31] |= 64;

        let b = EdwardsPoint::base_point();
        let pub_point = b.scalar_mul(&a);
        let pk = pub_point.compress();

        Ed25519Keypair {
            seed: *seed,
            public_key: pk,
        }
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let h = Sha512::digest(&self.seed);
        let mut a = [0u8; 32];
        a.copy_from_slice(&h[0..32]);
        a[0] &= 248;
        a[31] &= 127;
        a[31] |= 64;

        let mut r_hasher = Sha512::new();
        r_hasher.update(&h[32..64]);
        r_hasher.update(msg);
        let r_digest = r_hasher.finalize();
        let r_scalar = mod_l_512(&r_digest);

        let r_point = EdwardsPoint::base_point().scalar_mul(&r_scalar);
        let r_bytes = r_point.compress();

        let mut k_hasher = Sha512::new();
        k_hasher.update(&r_bytes);
        k_hasher.update(&self.public_key);
        k_hasher.update(msg);
        let k_digest = k_hasher.finalize();
        let k_scalar = mod_l_512(&k_digest);

        let s_scalar = sc_mul_add(&k_scalar, &a, &r_scalar);

        let mut sig = [0u8; 64];
        sig[0..32].copy_from_slice(&r_bytes);
        sig[32..64].copy_from_slice(&s_scalar);
        sig
    }
}

pub fn ed25519_verify(public_key: &[u8; 32], msg: &[u8], signature: &[u8; 64]) -> bool {
    let r_bytes: [u8; 32] = match signature[0..32].try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    let s_bytes: [u8; 32] = match signature[32..64].try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };

    let a_point = match EdwardsPoint::decompress(public_key) {
        Some(p) => p,
        None => return false,
    };
    let r_point = match EdwardsPoint::decompress(&r_bytes) {
        Some(p) => p,
        None => return false,
    };

    let mut k_hasher = Sha512::new();
    k_hasher.update(&r_bytes);
    k_hasher.update(public_key);
    k_hasher.update(msg);
    let k_digest = k_hasher.finalize();
    let k_scalar = mod_l_512(&k_digest);

    let sb = EdwardsPoint::base_point().scalar_mul(&s_bytes);
    let ka = a_point.scalar_mul(&k_scalar);
    let r_plus_ka = r_point.add(&ka);

    sb.compress() == r_plus_ka.compress()
}

// ============================================================================
// 9. Base64 Encoding & Decoding (RFC 4648)
// ============================================================================

pub fn base64_encode(data: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as usize;
        let b1 = if i + 1 < data.len() { data[i + 1] as usize } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as usize } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(CHARSET[(triple >> 18) & 0x3F] as char);
        out.push(CHARSET[(triple >> 12) & 0x3F] as char);
        if i + 1 < data.len() {
            out.push(CHARSET[(triple >> 6) & 0x3F] as char);
        } else {
            out.push('=');
        }
        if i + 2 < data.len() {
            out.push(CHARSET[triple & 0x3F] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

pub fn base64_decode(input: &str) -> Result<Vec<u8>, &'static str> {
    fn decode_char(c: u8) -> Result<u8, &'static str> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            b'=' => Ok(0),
            _ => Err("Invalid base64 character"),
        }
    }

    let filtered: Vec<u8> = input.bytes().filter(|&b| !b.is_ascii_whitespace()).collect();
    if filtered.len() % 4 != 0 {
        return Err("Invalid base64 length");
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < filtered.len() {
        let c0 = decode_char(filtered[i])?;
        let c1 = decode_char(filtered[i + 1])?;
        let c2 = decode_char(filtered[i + 2])?;
        let c3 = decode_char(filtered[i + 3])?;

        let triple = ((c0 as u32) << 18) | ((c1 as u32) << 12) | ((c2 as u32) << 6) | (c3 as u32);

        out.push(((triple >> 16) & 0xFF) as u8);
        if filtered[i + 2] != b'=' {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if filtered[i + 3] != b'=' {
            out.push((triple & 0xFF) as u8);
        }
        i += 4;
    }
    Ok(out)
}

// ============================================================================
// 10. PBKDF2-HMAC-SHA256 Password Hashing (RFC 8018)
// ============================================================================

pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut salt_plus_index = Vec::with_capacity(salt.len() + 4);
    salt_plus_index.extend_from_slice(salt);
    salt_plus_index.extend_from_slice(&1u32.to_be_bytes());

    let mut u = hmac_sha256(password, &salt_plus_index);
    let mut result = u;

    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for i in 0..32 {
            result[i] ^= u[i];
        }
    }
    result
}

// ============================================================================
// 11. BigUint & RSA Public Key Verification (ssh-rsa)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigUint {
    pub limbs: Vec<u64>,
}

impl BigUint {
    pub fn zero() -> Self {
        BigUint { limbs: Vec::new() }
    }

    pub fn from_u64(val: u64) -> Self {
        if val == 0 {
            BigUint::zero()
        } else {
            BigUint { limbs: vec![val] }
        }
    }

    pub fn from_be_bytes(bytes: &[u8]) -> Self {
        let mut limbs = Vec::new();
        let mut i = bytes.len();
        while i > 0 {
            let start = if i >= 8 { i - 8 } else { 0 };
            let mut chunk = [0u8; 8];
            let len = i - start;
            chunk[8 - len..].copy_from_slice(&bytes[start..i]);
            limbs.push(u64::from_be_bytes(chunk));
            i = start;
        }
        let mut b = BigUint { limbs };
        b.normalize();
        b
    }

    pub fn to_be_bytes(&self, target_len: usize) -> Vec<u8> {
        let mut out = vec![0u8; target_len];
        for (i, &limb) in self.limbs.iter().enumerate() {
            let bytes = limb.to_be_bytes();
            for (j, &b) in bytes.iter().enumerate() {
                let byte_pos = i * 8 + (7 - j);
                if byte_pos < target_len {
                    out[target_len - 1 - byte_pos] = b;
                }
            }
        }
        out
    }

    fn normalize(&mut self) {
        while let Some(&0) = self.limbs.last() {
            self.limbs.pop();
        }
    }

    pub fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        if self.limbs.len() != other.limbs.len() {
            return self.limbs.len().cmp(&other.limbs.len());
        }
        for (a, b) in self.limbs.iter().rev().zip(other.limbs.iter().rev()) {
            if a != b {
                return a.cmp(b);
            }
        }
        core::cmp::Ordering::Equal
    }

    pub fn add(&self, other: &Self) -> Self {
        let max_len = self.limbs.len().max(other.limbs.len());
        let mut res = Vec::with_capacity(max_len + 1);
        let mut carry = 0u64;
        for i in 0..max_len {
            let a = self.limbs.get(i).copied().unwrap_or(0);
            let b = other.limbs.get(i).copied().unwrap_or(0);
            let (sum1, c1) = a.overflowing_add(b);
            let (sum2, c2) = sum1.overflowing_add(carry);
            res.push(sum2);
            carry = (c1 || c2) as u64;
        }
        if carry > 0 {
            res.push(carry);
        }
        let mut out = BigUint { limbs: res };
        out.normalize();
        out
    }

    pub fn sub(&self, other: &Self) -> Self {
        let mut res = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0u64;
        for i in 0..self.limbs.len() {
            let a = self.limbs[i];
            let b = other.limbs.get(i).copied().unwrap_or(0);
            let (diff1, b1) = a.overflowing_sub(b);
            let (diff2, b2) = diff1.overflowing_sub(borrow);
            res.push(diff2);
            borrow = (b1 || b2) as u64;
        }
        let mut out = BigUint { limbs: res };
        out.normalize();
        out
    }

    pub fn mul(&self, other: &Self) -> Self {
        if self.limbs.is_empty() || other.limbs.is_empty() {
            return BigUint::zero();
        }
        let mut res = vec![0u64; self.limbs.len() + other.limbs.len()];
        for i in 0..self.limbs.len() {
            let mut carry = 0u128;
            for j in 0..other.limbs.len() {
                let cur = res[i + j] as u128 + (self.limbs[i] as u128) * (other.limbs[j] as u128) + carry;
                res[i + j] = cur as u64;
                carry = cur >> 64;
            }
            if carry > 0 {
                res[i + other.limbs.len()] += carry as u64;
            }
        }
        let mut out = BigUint { limbs: res };
        out.normalize();
        out
    }

    pub fn bit_length(&self) -> usize {
        if let Some(&last) = self.limbs.last() {
            (self.limbs.len() - 1) * 64 + (64 - last.leading_zeros() as usize)
        } else {
            0
        }
    }

    pub fn shl_bits(&self, bits: usize) -> Self {
        if self.limbs.is_empty() || bits == 0 {
            return self.clone();
        }
        let word_shift = bits / 64;
        let bit_shift = bits % 64;
        let mut res = vec![0u64; self.limbs.len() + word_shift + 1];

        for i in 0..self.limbs.len() {
            let low = self.limbs[i] << bit_shift;
            let high = if bit_shift > 0 { self.limbs[i] >> (64 - bit_shift) } else { 0 };
            res[i + word_shift] |= low;
            if high > 0 {
                res[i + word_shift + 1] |= high;
            }
        }
        let mut out = BigUint { limbs: res };
        out.normalize();
        out
    }

    pub fn rem(&self, modulus: &Self) -> Self {
        if modulus.limbs.is_empty() {
            panic!("Division by zero");
        }
        if self.cmp(modulus) == core::cmp::Ordering::Less {
            return self.clone();
        }

        let bit_len_self = self.bit_length();
        let bit_len_mod = modulus.bit_length();
        let shift = bit_len_self.saturating_sub(bit_len_mod);

        let mut rem = self.clone();
        for s in (0..=shift).rev() {
            let shifted = modulus.shl_bits(s);
            if rem.cmp(&shifted) != core::cmp::Ordering::Less {
                rem = rem.sub(&shifted);
            }
        }
        rem
    }

    pub fn mod_pow(&self, mut exp: u32, modulus: &Self) -> Self {
        let mut res = BigUint::from_u64(1);
        let mut base = self.rem(modulus);
        while exp > 0 {
            if (exp & 1) != 0 {
                res = res.mul(&base).rem(modulus);
            }
            base = base.mul(&base).rem(modulus);
            exp >>= 1;
        }
        res
    }
}

pub fn rsa_verify_pkcs1v15(
    n_bytes: &[u8],
    e_val: u32,
    msg: &[u8],
    sig_bytes: &[u8],
    is_sha256: bool,
) -> bool {
    if sig_bytes.len() != n_bytes.len() {
        return false;
    }
    let n = BigUint::from_be_bytes(n_bytes);
    let s = BigUint::from_be_bytes(sig_bytes);
    let m = s.mod_pow(e_val, &n);
    let em = m.to_be_bytes(n_bytes.len());

    if em.len() < 38 || em[0] != 0x00 || em[1] != 0x01 {
        return false;
    }

    let mut i = 2;
    while i < em.len() && em[i] == 0xFF {
        i += 1;
    }
    if i >= em.len() || em[i] != 0x00 || i < 10 {
        return false;
    }
    i += 1;

    let digest_info = &em[i..];
    if is_sha256 {
        const SHA256_PREFIX: &[u8] = &[
            0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
            0x01, 0x05, 0x00, 0x04, 0x20,
        ];
        if digest_info.len() != SHA256_PREFIX.len() + 32 {
            return false;
        }
        if &digest_info[..SHA256_PREFIX.len()] != SHA256_PREFIX {
            return false;
        }
        let expected_hash = Sha256::digest(msg);
        &digest_info[SHA256_PREFIX.len()..] == expected_hash
    } else {
        const SHA1_PREFIX: &[u8] = &[
            0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04,
            0x14,
        ];
        if digest_info.len() < SHA1_PREFIX.len() {
            return false;
        }
        &digest_info[..SHA1_PREFIX.len()] == SHA1_PREFIX
    }
}

// ============================================================================
// 12. AES-256 Block Cipher & AES-256-GCM AEAD (aes256-gcm@openssh.com)
// ============================================================================

pub struct Aes256 {
    sbox: [u8; 256],
    round_keys: [u32; 60],
}

impl Aes256 {
    pub fn new(key: &[u8; 32]) -> Self {
        let mut sbox = [0u8; 256];
        for i in 0..256 {
            let mut inv = 0u8;
            if i > 0 {
                for cand in 1..256 {
                    let mut p = 0u8;
                    let mut a = cand as u8;
                    let mut b = i as u8;
                    for _ in 0..8 {
                        if (b & 1) != 0 {
                            p ^= a;
                        }
                        let hi = (a & 0x80) != 0;
                        a <<= 1;
                        if hi {
                            a ^= 0x1B;
                        }
                        b >>= 1;
                    }
                    if p == 1 {
                        inv = cand as u8;
                        break;
                    }
                }
            }
            let rot1 = inv.rotate_left(1);
            let rot2 = inv.rotate_left(2);
            let rot3 = inv.rotate_left(3);
            let rot4 = inv.rotate_left(4);
            sbox[i] = inv ^ rot1 ^ rot2 ^ rot3 ^ rot4 ^ 0x63;
        }

        let rcon: [u32; 10] = [
            0x01000000, 0x02000000, 0x04000000, 0x08000000, 0x10000000,
            0x20000000, 0x40000000, 0x80000000, 0x1B000000, 0x36000000,
        ];

        let mut round_keys = [0u32; 60];
        for i in 0..8 {
            round_keys[i] = u32::from_be_bytes(key[i * 4..(i + 1) * 4].try_into().unwrap());
        }

        let sub_word = |w: u32, sb: &[u8; 256]| -> u32 {
            let b0 = sb[((w >> 24) & 0xFF) as usize] as u32;
            let b1 = sb[((w >> 16) & 0xFF) as usize] as u32;
            let b2 = sb[((w >> 8) & 0xFF) as usize] as u32;
            let b3 = sb[(w & 0xFF) as usize] as u32;
            (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
        };

        for i in 8..60 {
            let mut temp = round_keys[i - 1];
            if i % 8 == 0 {
                temp = sub_word(temp.rotate_left(8), &sbox) ^ rcon[(i / 8) - 1];
            } else if i % 8 == 4 {
                temp = sub_word(temp, &sbox);
            }
            round_keys[i] = round_keys[i - 8] ^ temp;
        }

        Aes256 { sbox, round_keys }
    }

    pub fn encrypt_block(&self, block: &[u8; 16], out: &mut [u8; 16]) {
        let mut state = *block;

        let add_round_key = |st: &mut [u8; 16], rk: &[u32; 4]| {
            for col in 0..4 {
                let k_bytes = rk[col].to_be_bytes();
                for row in 0..4 {
                    st[col * 4 + row] ^= k_bytes[row];
                }
            }
        };

        let gmul = |mut a: u8, mut b: u8| -> u8 {
            let mut p = 0u8;
            for _ in 0..8 {
                if (b & 1) != 0 {
                    p ^= a;
                }
                let hi = (a & 0x80) != 0;
                a <<= 1;
                if hi {
                    a ^= 0x1B;
                }
                b >>= 1;
            }
            p
        };

        let mut rk0 = [0u32; 4];
        rk0.copy_from_slice(&self.round_keys[0..4]);
        add_round_key(&mut state, &rk0);

        for round in 1..14 {
            for b in state.iter_mut() {
                *b = self.sbox[*b as usize];
            }

            let t1 = state[1]; state[1] = state[5]; state[5] = state[9]; state[9] = state[13]; state[13] = t1;
            let t2 = state[2]; state[2] = state[10]; state[10] = t2;
            let t6 = state[6]; state[6] = state[14]; state[14] = t6;
            let t3 = state[15]; state[15] = state[11]; state[11] = state[7]; state[7] = state[3]; state[3] = t3;

            for col in 0..4 {
                let c = col * 4;
                let s0 = state[c];
                let s1 = state[c + 1];
                let s2 = state[c + 2];
                let s3 = state[c + 3];

                state[c] = gmul(s0, 2) ^ gmul(s1, 3) ^ s2 ^ s3;
                state[c + 1] = s0 ^ gmul(s1, 2) ^ gmul(s2, 3) ^ s3;
                state[c + 2] = s0 ^ s1 ^ gmul(s2, 2) ^ gmul(s3, 3);
                state[c + 3] = gmul(s0, 3) ^ s1 ^ s2 ^ gmul(s3, 2);
            }

            let mut rk = [0u32; 4];
            rk.copy_from_slice(&self.round_keys[round * 4..(round + 1) * 4]);
            add_round_key(&mut state, &rk);
        }

        for b in state.iter_mut() {
            *b = self.sbox[*b as usize];
        }
        let t1 = state[1]; state[1] = state[5]; state[5] = state[9]; state[9] = state[13]; state[13] = t1;
        let t2 = state[2]; state[2] = state[10]; state[10] = t2;
        let t6 = state[6]; state[6] = state[14]; state[14] = t6;
        let t3 = state[15]; state[15] = state[11]; state[11] = state[7]; state[7] = state[3]; state[3] = t3;

        let mut rk14 = [0u32; 4];
        rk14.copy_from_slice(&self.round_keys[56..60]);
        add_round_key(&mut state, &rk14);

        *out = state;
    }
}

pub fn gf128_mul(x: &[u8; 16], y: &[u8; 16]) -> [u8; 16] {
    let mut z = [0u8; 16];
    let mut v = *y;
    for i in 0..128 {
        let byte = i / 8;
        let bit = 7 - (i % 8);
        if ((x[byte] >> bit) & 1) != 0 {
            for j in 0..16 {
                z[j] ^= v[j];
            }
        }
        let lsb = (v[15] & 1) != 0;
        let mut carry = 0u8;
        for j in 0..16 {
            let next_carry = v[j] & 1;
            v[j] = (v[j] >> 1) | (carry << 7);
            carry = next_carry;
        }
        if lsb {
            v[0] ^= 0xe1;
        }
    }
    z
}

pub fn aes256_gcm_openssh_encrypt(
    key: &[u8; 32],
    iv: &[u8; 12],
    seq: u32,
    packet_len: u32,
    payload: &mut [u8],
) -> (Vec<u8>, [u8; 16]) {
    let cipher = Aes256::new(key);

    let mut h = [0u8; 16];
    cipher.encrypt_block(&[0u8; 16], &mut h);

    let mut j0 = [0u8; 16];
    j0[..4].copy_from_slice(&iv[..4]);
    let inv_seq = ((seq as u64) + u64::from_be_bytes(iv[4..12].try_into().unwrap())).to_be_bytes();
    j0[4..12].copy_from_slice(&inv_seq);
    j0[15] = 1;

    let mut tag_mask = [0u8; 16];
    cipher.encrypt_block(&j0, &mut tag_mask);

    let mut counter = j0;
    let mut ctr_val = 1u64;

    let mut offset = 0;
    while offset < payload.len() {
        ctr_val += 1;
        counter[8..16].copy_from_slice(&ctr_val.to_be_bytes());
        let mut keystream = [0u8; 16];
        cipher.encrypt_block(&counter, &mut keystream);

        let chunk = (payload.len() - offset).min(16);
        for i in 0..chunk {
            payload[offset + i] ^= keystream[i];
        }
        offset += chunk;
    }

    let aad = packet_len.to_be_bytes();
    let mut ghash = [0u8; 16];

    let mut block_aad = [0u8; 16];
    block_aad[..4].copy_from_slice(&aad);
    for i in 0..16 { ghash[i] ^= block_aad[i]; }
    ghash = gf128_mul(&ghash, &h);

    let mut offset_c = 0;
    while offset_c < payload.len() {
        let chunk = (payload.len() - offset_c).min(16);
        let mut block_c = [0u8; 16];
        block_c[..chunk].copy_from_slice(&payload[offset_c..offset_c + chunk]);
        for i in 0..16 { ghash[i] ^= block_c[i]; }
        ghash = gf128_mul(&ghash, &h);
        offset_c += chunk;
    }

    let mut len_block = [0u8; 16];
    len_block[0..8].copy_from_slice(&(32u64).to_be_bytes());
    len_block[8..16].copy_from_slice(&((payload.len() as u64) * 8).to_be_bytes());
    for i in 0..16 { ghash[i] ^= len_block[i]; }
    ghash = gf128_mul(&ghash, &h);

    let mut tag = [0u8; 16];
    for i in 0..16 {
        tag[i] = ghash[i] ^ tag_mask[i];
    }

    (aad.to_vec(), tag)
}

pub fn aes256_gcm_openssh_decrypt(
    key: &[u8; 32],
    iv: &[u8; 12],
    seq: u32,
    packet_len: u32,
    payload: &mut [u8],
    expected_tag: &[u8; 16],
) -> Result<(), &'static str> {
    let cipher = Aes256::new(key);

    let mut h = [0u8; 16];
    cipher.encrypt_block(&[0u8; 16], &mut h);

    let mut j0 = [0u8; 16];
    j0[..4].copy_from_slice(&iv[..4]);
    let inv_seq = ((seq as u64) + u64::from_be_bytes(iv[4..12].try_into().unwrap())).to_be_bytes();
    j0[4..12].copy_from_slice(&inv_seq);
    j0[15] = 1;

    let mut tag_mask = [0u8; 16];
    cipher.encrypt_block(&j0, &mut tag_mask);

    let aad = packet_len.to_be_bytes();
    let mut ghash = [0u8; 16];

    let mut block_aad = [0u8; 16];
    block_aad[..4].copy_from_slice(&aad);
    for i in 0..16 { ghash[i] ^= block_aad[i]; }
    ghash = gf128_mul(&ghash, &h);

    let mut offset_c = 0;
    while offset_c < payload.len() {
        let chunk = (payload.len() - offset_c).min(16);
        let mut block_c = [0u8; 16];
        block_c[..chunk].copy_from_slice(&payload[offset_c..offset_c + chunk]);
        for i in 0..16 { ghash[i] ^= block_c[i]; }
        ghash = gf128_mul(&ghash, &h);
        offset_c += chunk;
    }

    let mut len_block = [0u8; 16];
    len_block[0..8].copy_from_slice(&(32u64).to_be_bytes());
    len_block[8..16].copy_from_slice(&((payload.len() as u64) * 8).to_be_bytes());
    for i in 0..16 { ghash[i] ^= len_block[i]; }
    ghash = gf128_mul(&ghash, &h);

    let mut computed_tag = [0u8; 16];
    for i in 0..16 {
        computed_tag[i] = ghash[i] ^ tag_mask[i];
    }

    if &computed_tag != expected_tag {
        return Err("AES-256-GCM authentication failed");
    }

    let mut counter = j0;
    let mut ctr_val = 1u64;

    let mut offset = 0;
    while offset < payload.len() {
        ctr_val += 1;
        counter[8..16].copy_from_slice(&ctr_val.to_be_bytes());
        let mut keystream = [0u8; 16];
        cipher.encrypt_block(&counter, &mut keystream);

        let chunk = (payload.len() - offset).min(16);
        for i in 0..chunk {
            payload[offset + i] ^= keystream[i];
        }
        offset += chunk;
    }

    Ok(())
}
