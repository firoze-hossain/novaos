//! kernel/src/net/wireguard/crypto/chacha20poly1305.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Native In-Kernel WireGuard VPN
//!
//! ChaCha20 Stream Cipher & Poly1305 Authenticator (RFC 7539 / RFC 8439).
//! Implements authenticated encryption with associated data (AEAD) in pure `#![no_std]` Rust.

pub struct ChaCha20 {
    state: [u32; 16],
}

impl ChaCha20 {
    pub fn new(key: &[u8; 32], nonce: &[u8; 12], counter: u32) -> Self {
        let load32 = |b: &[u8]| -> u32 {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        };
        let mut state = [0u32; 16];
        state[0] = 0x61707865;
        state[1] = 0x3320646e;
        state[2] = 0x79622d32;
        state[3] = 0x6b206574;

        for i in 0..8 {
            state[4 + i] = load32(&key[i * 4..(i + 1) * 4]);
        }
        state[12] = counter;
        state[13] = load32(&nonce[0..4]);
        state[14] = load32(&nonce[4..8]);
        state[15] = load32(&nonce[8..12]);

        ChaCha20 { state }
    }

    #[inline(always)]
    fn quarter_round(x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        x[a] = x[a].wrapping_add(x[b]); x[d] ^= x[a]; x[d] = x[d].rotate_left(16);
        x[c] = x[c].wrapping_add(x[d]); x[b] ^= x[c]; x[b] = x[b].rotate_left(12);
        x[a] = x[a].wrapping_add(x[b]); x[d] ^= x[a]; x[d] = x[d].rotate_left(8);
        x[c] = x[c].wrapping_add(x[d]); x[b] ^= x[c]; x[b] = x[b].rotate_left(7);
    }

    pub fn block(&self, counter: u32, out: &mut [u8; 64]) {
        let mut x = self.state;
        x[12] = counter;

        for _ in 0..10 {
            // Column round
            Self::quarter_round(&mut x, 0, 4, 8, 12);
            Self::quarter_round(&mut x, 1, 5, 9, 13);
            Self::quarter_round(&mut x, 2, 6, 10, 14);
            Self::quarter_round(&mut x, 3, 7, 11, 15);
            // Diagonal round
            Self::quarter_round(&mut x, 0, 5, 10, 15);
            Self::quarter_round(&mut x, 1, 6, 11, 12);
            Self::quarter_round(&mut x, 2, 7, 8, 13);
            Self::quarter_round(&mut x, 3, 4, 9, 14);
        }

        let mut orig = self.state;
        orig[12] = counter;

        for i in 0..16 {
            let val = x[i].wrapping_add(orig[i]);
            out[i * 4..(i + 1) * 4].copy_from_slice(&val.to_le_bytes());
        }
    }

    pub fn apply_keystream(&self, counter_start: u32, data: &mut [u8]) {
        let mut block_buf = [0u8; 64];
        let mut ctr = counter_start;
        let mut offset = 0;
        while offset < data.len() {
            self.block(ctr, &mut block_buf);
            let chunk = core::cmp::min(64, data.len() - offset);
            for i in 0..chunk {
                data[offset + i] ^= block_buf[i];
            }
            offset += chunk;
            ctr = ctr.wrapping_add(1);
        }
    }
}

pub struct Poly1305 {
    h: [u64; 5],
    r: [u64; 5],
    pad: [u32; 4],
    buf: [u8; 16],
    buf_len: usize,
}

impl Poly1305 {
    pub fn new(key: &[u8; 32]) -> Self {
        let load32 = |b: &[u8]| -> u32 {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        };

        // Clamp r
        let mut r_bytes = [0u8; 16];
        r_bytes.copy_from_slice(&key[0..16]);
        r_bytes[3] &= 15;
        r_bytes[7] &= 15;
        r_bytes[11] &= 15;
        r_bytes[15] &= 15;
        r_bytes[4] &= 252;
        r_bytes[8] &= 252;
        r_bytes[12] &= 252;

        let r0 = load32(&r_bytes[0..4]) as u64;
        let r1 = load32(&r_bytes[4..8]) as u64;
        let r2 = load32(&r_bytes[8..12]) as u64;
        let r3 = load32(&r_bytes[12..16]) as u64;

        let r = [
            r0 & 0x3FFFFFF,
            ((r0 >> 26) | (r1 << 6)) & 0x3FFFFFF,
            ((r1 >> 20) | (r2 << 12)) & 0x3FFFFFF,
            ((r2 >> 14) | (r3 << 18)) & 0x3FFFFFF,
            (r3 >> 8) & 0x3FFFFFF,
        ];

        let pad = [
            load32(&key[16..20]),
            load32(&key[20..24]),
            load32(&key[24..28]),
            load32(&key[28..32]),
        ];

        Poly1305 {
            h: [0; 5],
            r,
            pad,
            buf: [0; 16],
            buf_len: 0,
        }
    }

    fn update_block(&mut self, chunk: &[u8]) {
        let mut block = [0u8; 17];
        block[..chunk.len()].copy_from_slice(chunk);
        block[chunk.len()] = 1;

        let load32 = |b: &[u8]| -> u32 {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        };

        let b0 = load32(&block[0..4]) as u64;
        let b1 = load32(&block[4..8]) as u64;
        let b2 = load32(&block[8..12]) as u64;
        let b3 = load32(&block[12..16]) as u64;
        let b4 = block[16] as u64;

        self.h[0] += b0 & 0x3FFFFFF;
        self.h[1] += ((b0 >> 26) | (b1 << 6)) & 0x3FFFFFF;
        self.h[2] += ((b1 >> 20) | (b2 << 12)) & 0x3FFFFFF;
        self.h[3] += ((b2 >> 14) | (b3 << 18)) & 0x3FFFFFF;
        self.h[4] += (b3 >> 8) | (b4 << 24);

        let h = self.h;
        let r = self.r;

        let r1_5 = r[1] * 5;
        let r2_5 = r[2] * 5;
        let r3_5 = r[3] * 5;
        let r4_5 = r[4] * 5;

        let d0 = (h[0] as u128) * (r[0] as u128)
            + (h[1] as u128) * (r4_5 as u128)
            + (h[2] as u128) * (r3_5 as u128)
            + (h[3] as u128) * (r2_5 as u128)
            + (h[4] as u128) * (r1_5 as u128);

        let d1 = (h[0] as u128) * (r[1] as u128)
            + (h[1] as u128) * (r[0] as u128)
            + (h[2] as u128) * (r4_5 as u128)
            + (h[3] as u128) * (r3_5 as u128)
            + (h[4] as u128) * (r2_5 as u128);

        let d2 = (h[0] as u128) * (r[2] as u128)
            + (h[1] as u128) * (r[1] as u128)
            + (h[2] as u128) * (r[0] as u128)
            + (h[3] as u128) * (r4_5 as u128)
            + (h[4] as u128) * (r3_5 as u128);

        let d3 = (h[0] as u128) * (r[3] as u128)
            + (h[1] as u128) * (r[2] as u128)
            + (h[2] as u128) * (r[1] as u128)
            + (h[3] as u128) * (r[0] as u128)
            + (h[4] as u128) * (r4_5 as u128);

        let d4 = (h[0] as u128) * (r[4] as u128)
            + (h[1] as u128) * (r[3] as u128)
            + (h[2] as u128) * (r[2] as u128)
            + (h[3] as u128) * (r[1] as u128)
            + (h[4] as u128) * (r[0] as u128);

        let mut c = d0 >> 26;
        self.h[0] = (d0 & 0x3FFFFFF) as u64;

        let d1 = d1 + c;
        c = d1 >> 26;
        self.h[1] = (d1 & 0x3FFFFFF) as u64;

        let d2 = d2 + c;
        c = d2 >> 26;
        self.h[2] = (d2 & 0x3FFFFFF) as u64;

        let d3 = d3 + c;
        c = d3 >> 26;
        self.h[3] = (d3 & 0x3FFFFFF) as u64;

        let d4 = d4 + c;
        c = d4 >> 26;
        self.h[4] = (d4 & 0x3FFFFFF) as u64;

        self.h[0] += (c as u64) * 5;
        c = (self.h[0] >> 26) as u128;
        self.h[0] &= 0x3FFFFFF;
        self.h[1] += c as u64;
    }

    pub fn update(&mut self, mut data: &[u8]) {
        if self.buf_len > 0 {
            let want = 16 - self.buf_len;
            let take = core::cmp::min(want, data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 16 {
                let block = self.buf;
                self.update_block(&block);
                self.buf_len = 0;
            }
        }

        while data.len() >= 16 {
            self.update_block(&data[..16]);
            data = &data[16..];
        }

        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    pub fn finish(mut self) -> [u8; 16] {
        if self.buf_len > 0 {
            let len = self.buf_len;
            let block = self.buf;
            self.update_block(&block[..len]);
        }

        let mut c = self.h[1] >> 26; self.h[1] &= 0x3FFFFFF;
        self.h[2] += c; c = self.h[2] >> 26; self.h[2] &= 0x3FFFFFF;
        self.h[3] += c; c = self.h[3] >> 26; self.h[3] &= 0x3FFFFFF;
        self.h[4] += c; c = self.h[4] >> 26; self.h[4] &= 0x3FFFFFF;
        self.h[0] += c * 5; c = self.h[0] >> 26; self.h[0] &= 0x3FFFFFF;
        self.h[1] += c;

        let mut g = self.h;
        g[0] += 5; c = g[0] >> 26; g[0] &= 0x3FFFFFF;
        g[1] += c; c = g[1] >> 26; g[1] &= 0x3FFFFFF;
        g[2] += c; c = g[2] >> 26; g[2] &= 0x3FFFFFF;
        g[3] += c; c = g[3] >> 26; g[3] &= 0x3FFFFFF;
        g[4] += c;

        let mask = if (g[4] >> 26) != 0 { u64::MAX } else { 0 };
        g[4] &= 0x3FFFFFF;

        let h0 = (self.h[0] & !mask) | (g[0] & mask);
        let h1 = (self.h[1] & !mask) | (g[1] & mask);
        let h2 = (self.h[2] & !mask) | (g[2] & mask);
        let h3 = (self.h[3] & !mask) | (g[3] & mask);
        let h4 = (self.h[4] & !mask) | (g[4] & mask);

        let val = (h0 as u128)
            | ((h1 as u128) << 26)
            | ((h2 as u128) << 52)
            | ((h3 as u128) << 78)
            | ((h4 as u128) << 104);

        let p0 = self.pad[0] as u128 | ((self.pad[1] as u128) << 32);
        let p1 = self.pad[2] as u128 | ((self.pad[3] as u128) << 32);
        let pad_128 = p0 | (p1 << 64);

        let final_val = val.wrapping_add(pad_128);
        final_val.to_le_bytes()
    }
}

/// ChaCha20-Poly1305 authenticated encryption (RFC 7539 / RFC 8439).
/// Output buffer must have length = `plaintext.len() + 16`.
pub fn chacha20poly1305_encrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
    out: &mut [u8],
) {
    let cipher = ChaCha20::new(key, nonce, 0);

    // 1. One-time Poly1305 key from block 0
    let mut poly_block = [0u8; 64];
    cipher.block(0, &mut poly_block);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&poly_block[0..32]);

    // 2. Encrypt plaintext
    let pt_len = plaintext.len();
    out[..pt_len].copy_from_slice(plaintext);
    cipher.apply_keystream(1, &mut out[..pt_len]);

    // 3. Poly1305 MAC
    let mut poly = Poly1305::new(&poly_key);
    poly.update(aad);
    if aad.len() % 16 != 0 {
        let pad_len = 16 - (aad.len() % 16);
        poly.update(&[0u8; 16][..pad_len]);
    }

    poly.update(&out[..pt_len]);
    if pt_len % 16 != 0 {
        let pad_len = 16 - (pt_len % 16);
        poly.update(&[0u8; 16][..pad_len]);
    }

    let aad_len_bytes = (aad.len() as u64).to_le_bytes();
    let pt_len_bytes = (pt_len as u64).to_le_bytes();
    poly.update(&aad_len_bytes);
    poly.update(&pt_len_bytes);

    let tag = poly.finish();
    out[pt_len..pt_len + 16].copy_from_slice(&tag);
}

/// ChaCha20-Poly1305 authenticated decryption (RFC 7539 / RFC 8439).
/// Verifies authentication tag in constant time.
/// Returns number of decrypted bytes on success.
pub fn chacha20poly1305_decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
    out: &mut [u8],
) -> Result<usize, ()> {
    if ciphertext_and_tag.len() < 16 {
        return Err(());
    }
    let pt_len = ciphertext_and_tag.len() - 16;
    let ciphertext = &ciphertext_and_tag[..pt_len];
    let tag = &ciphertext_and_tag[pt_len..];

    let cipher = ChaCha20::new(key, nonce, 0);

    let mut poly_block = [0u8; 64];
    cipher.block(0, &mut poly_block);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&poly_block[0..32]);

    let mut poly = Poly1305::new(&poly_key);
    poly.update(aad);
    if aad.len() % 16 != 0 {
        let pad_len = 16 - (aad.len() % 16);
        poly.update(&[0u8; 16][..pad_len]);
    }

    poly.update(ciphertext);
    if pt_len % 16 != 0 {
        let pad_len = 16 - (pt_len % 16);
        poly.update(&[0u8; 16][..pad_len]);
    }

    let aad_len_bytes = (aad.len() as u64).to_le_bytes();
    let pt_len_bytes = (pt_len as u64).to_le_bytes();
    poly.update(&aad_len_bytes);
    poly.update(&pt_len_bytes);

    let expected_tag = poly.finish();

    // Constant-time compare tag
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= expected_tag[i] ^ tag[i];
    }
    if diff != 0 {
        return Err(());
    }

    out[..pt_len].copy_from_slice(ciphertext);
    cipher.apply_keystream(1, &mut out[..pt_len]);
    Ok(pt_len)
}
