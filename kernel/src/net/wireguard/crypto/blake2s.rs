//! kernel/src/net/wireguard/crypto/blake2s.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Native In-Kernel WireGuard VPN
//!
//! BLAKE2s-256 Hash Function, HMAC-BLAKE2s, and WireGuard KDF1/KDF2/KDF3 (RFC 7693).
//! Pure `#![no_std]` Rust implementation with zero allocations.

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];

const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

pub struct Blake2s {
    h: [u32; 8],
    t: [u32; 2],
    buf: [u8; 64],
    buf_len: usize,
    out_len: usize,
}

impl Blake2s {
    pub fn new(out_len: usize) -> Self {
        Self::with_key(&[], out_len)
    }

    pub fn with_key(key: &[u8], out_len: usize) -> Self {
        let mut h = IV;
        let param0 = (out_len as u32)
            | ((key.len() as u32) << 8)
            | (1 << 16)
            | (1 << 24);
        h[0] ^= param0;

        let mut ctx = Blake2s {
            h,
            t: [0, 0],
            buf: [0; 64],
            buf_len: 0,
            out_len,
        };

        if !key.is_empty() {
            let mut key_block = [0u8; 64];
            key_block[..key.len()].copy_from_slice(key);
            ctx.update(&key_block);
        }

        ctx
    }

    fn compress(&mut self, last: bool) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                self.buf[i * 4],
                self.buf[i * 4 + 1],
                self.buf[i * 4 + 2],
                self.buf[i * 4 + 3],
            ]);
        }

        let mut v = [0u32; 16];
        v[0..8].copy_from_slice(&self.h);
        v[8..16].copy_from_slice(&IV);
        v[12] ^= self.t[0];
        v[13] ^= self.t[1];
        if last {
            v[14] ^= 0xFFFFFFFF;
        }

        #[inline(always)]
        fn g(v: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, x: u32, y: u32) {
            v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
            v[d] = (v[d] ^ v[a]).rotate_right(16);
            v[c] = v[c].wrapping_add(v[d]);
            v[b] = (v[b] ^ v[c]).rotate_right(12);
            v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
            v[d] = (v[d] ^ v[a]).rotate_right(8);
            v[c] = v[c].wrapping_add(v[d]);
            v[b] = (v[b] ^ v[c]).rotate_right(7);
        }

        for r in 0..10 {
            let s = &SIGMA[r];
            g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
            g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
            g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
            g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
            g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
            g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
            g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
            g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
        }

        for i in 0..8 {
            self.h[i] ^= v[i] ^ v[i + 8];
        }
    }

    fn increment_counter(&mut self, inc: u32) {
        let (t0, c) = self.t[0].overflowing_add(inc);
        self.t[0] = t0;
        if c {
            self.t[1] = self.t[1].wrapping_add(1);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            if self.buf_len == 64 {
                self.increment_counter(64);
                self.compress(false);
                self.buf_len = 0;
            }
            let want = 64 - self.buf_len;
            let take = core::cmp::min(want, data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
        }
    }

    pub fn finalize(mut self, out: &mut [u8]) {
        let rem = self.buf_len as u32;
        self.increment_counter(rem);
        for i in self.buf_len..64 {
            self.buf[i] = 0;
        }
        self.compress(true);

        let mut digest = [0u8; 32];
        for i in 0..8 {
            digest[i * 4..(i + 1) * 4].copy_from_slice(&self.h[i].to_le_bytes());
        }
        out[..self.out_len].copy_from_slice(&digest[..self.out_len]);
    }
}

pub fn blake2s_256(data: &[u8]) -> [u8; 32] {
    let mut ctx = Blake2s::new(32);
    ctx.update(data);
    let mut out = [0u8; 32];
    ctx.finalize(&mut out);
    out
}

pub fn blake2s_mac16(key: &[u8], data: &[u8]) -> [u8; 16] {
    let mut ctx = Blake2s::with_key(key, 16);
    ctx.update(data);
    let mut out = [0u8; 16];
    ctx.finalize(&mut out);
    out
}

pub fn hmac_blake2s(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let hashed_key = blake2s_256(key);
        k[..32].copy_from_slice(&hashed_key);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Blake2s::new(32);
    inner.update(&ipad);
    inner.update(data);
    let mut inner_hash = [0u8; 32];
    inner.finalize(&mut inner_hash);

    let mut outer = Blake2s::new(32);
    outer.update(&opad);
    outer.update(&inner_hash);
    let mut out = [0u8; 32];
    outer.finalize(&mut out);
    out
}

/// WireGuard KDF1:
/// tau0 = HMAC(key, input)
/// tau1 = HMAC(tau0, 0x01)
pub fn kdf1(key: &[u8; 32], input: &[u8]) -> [u8; 32] {
    let tau0 = hmac_blake2s(key, input);
    hmac_blake2s(&tau0, &[0x01])
}

/// WireGuard KDF2:
/// tau0 = HMAC(key, input)
/// tau1 = HMAC(tau0, 0x01)
/// tau2 = HMAC(tau0, tau1 || 0x02)
pub fn kdf2(key: &[u8; 32], input: &[u8]) -> ([u8; 32], [u8; 32]) {
    let tau0 = hmac_blake2s(key, input);
    let tau1 = hmac_blake2s(&tau0, &[0x01]);
    let mut buf = [0u8; 33];
    buf[..32].copy_from_slice(&tau1);
    buf[32] = 0x02;
    let tau2 = hmac_blake2s(&tau0, &buf);
    (tau1, tau2)
}

/// WireGuard KDF3:
/// tau0 = HMAC(key, input)
/// tau1 = HMAC(tau0, 0x01)
/// tau2 = HMAC(tau0, tau1 || 0x02)
/// tau3 = HMAC(tau0, tau2 || 0x03)
pub fn kdf3(key: &[u8; 32], input: &[u8]) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let tau0 = hmac_blake2s(key, input);
    let tau1 = hmac_blake2s(&tau0, &[0x01]);
    let mut buf = [0u8; 33];
    buf[..32].copy_from_slice(&tau1);
    buf[32] = 0x02;
    let tau2 = hmac_blake2s(&tau0, &buf);
    buf[..32].copy_from_slice(&tau2);
    buf[32] = 0x03;
    let tau3 = hmac_blake2s(&tau0, &buf);
    (tau1, tau2, tau3)
}
