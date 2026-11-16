//! kernel/src/net/wireguard/crypto/x25519.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Native In-Kernel WireGuard VPN
//!
//! Constant-time X25519 Montgomery Ladder implementation (RFC 7748).
//! Operates over the prime field GF(2^255 - 19) in pure `#![no_std]` Rust
//! without external crates or floating point operations.

const P: [u64; 4] = [
    0xFFFFFFFFFFFFFFED,
    0xFFFFFFFFFFFFFFFF,
    0xFFFFFFFFFFFFFFFF,
    0x7FFFFFFFFFFFFFFF,
];

/// Field element in GF(2^255 - 19) represented by 4 limbs of 64-bit unsigned integers (little-endian).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Fe(pub [u64; 4]);

impl Fe {
    pub const ZERO: Self = Fe([0; 4]);
    pub const ONE: Self = Fe([1, 0, 0, 0]);

    /// Unpack 32-byte little-endian array into field element, masking bit 255 per RFC 7748.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let mut limbs = [0u64; 4];
        for i in 0..4 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[i * 8..(i + 1) * 8]);
            limbs[i] = u64::from_le_bytes(b);
        }
        limbs[3] &= 0x7FFFFFFFFFFFFFFF;
        Self::reduce_256(limbs)
    }

    /// Pack field element into 32-byte canonical little-endian array.
    pub fn to_bytes(&self) -> [u8; 32] {
        let canonical = Self::reduce_256(self.0);
        let mut out = [0u8; 32];
        for i in 0..4 {
            out[i * 8..(i + 1) * 8].copy_from_slice(&canonical.0[i].to_le_bytes());
        }
        out
    }

    fn sub_borrow(a: [u64; 4], b: [u64; 4]) -> ([u64; 4], u64) {
        let mut res = [0u64; 4];
        let mut borrow = 0u128;
        for i in 0..4 {
            let diff = (a[i] as u128).wrapping_sub(b[i] as u128).wrapping_sub(borrow);
            res[i] = diff as u64;
            borrow = (diff >> 127) & 1;
        }
        (res, borrow as u64)
    }

    fn add_carry(a: [u64; 4], b: [u64; 4]) -> ([u64; 4], u64) {
        let mut res = [0u64; 4];
        let mut carry = 0u128;
        for i in 0..4 {
            let sum = (a[i] as u128) + (b[i] as u128) + carry;
            res[i] = sum as u64;
            carry = sum >> 64;
        }
        (res, carry as u64)
    }

    /// Fully reduce a 256-bit number modulo 2^255 - 19.
    pub fn reduce_256(mut x: [u64; 4]) -> Self {
        let top_bit = x[3] >> 63;
        x[3] &= 0x7FFFFFFFFFFFFFFF;
        if top_bit > 0 {
            let mut carry = (top_bit as u128) * 19;
            for i in 0..4 {
                let sum = (x[i] as u128) + carry;
                x[i] = sum as u64;
                carry = sum >> 64;
            }
        }

        let (diff, borrow) = Self::sub_borrow(x, P);
        let mask = if borrow == 0 { u64::MAX } else { 0 };
        let mut res = [0u64; 4];
        for i in 0..4 {
            res[i] = (diff[i] & mask) | (x[i] & !mask);
        }
        Fe(res)
    }

    pub fn add(&self, rhs: &Self) -> Self {
        let (sum, _) = Self::add_carry(self.0, rhs.0);
        let (diff, borrow) = Self::sub_borrow(sum, P);
        let mask = if borrow == 0 { u64::MAX } else { 0 };
        let mut res = [0u64; 4];
        for i in 0..4 {
            res[i] = (diff[i] & mask) | (sum[i] & !mask);
        }
        Fe(res)
    }

    pub fn sub(&self, rhs: &Self) -> Self {
        let (diff, borrow) = Self::sub_borrow(self.0, rhs.0);
        let (with_p, _) = Self::add_carry(diff, P);
        let mask = if borrow != 0 { u64::MAX } else { 0 };
        let mut res = [0u64; 4];
        for i in 0..4 {
            res[i] = (with_p[i] & mask) | (diff[i] & !mask);
        }
        Fe(res)
    }

    pub fn mul(&self, rhs: &Self) -> Self {
        let a = &self.0;
        let b = &rhs.0;

        let mut res = [0u64; 8];
        for i in 0..4 {
            let mut carry = 0u128;
            for j in 0..4 {
                let prod = (a[i] as u128) * (b[j] as u128) + (res[i + j] as u128) + carry;
                res[i + j] = prod as u64;
                carry = prod >> 64;
            }
            res[i + 4] += carry as u64;
        }

        // Modular reduction: 2^256 = 38 (mod 2^255 - 19)
        // Split 512-bit product into L = res[0..4] and H = res[4..8].
        let mut h_38 = [0u64; 5];
        let mut carry = 0u128;
        for i in 0..4 {
            let prod = (res[4 + i] as u128) * 38 + carry;
            h_38[i] = prod as u64;
            carry = prod >> 64;
        }
        h_38[4] = carry as u64;

        let mut val = [0u64; 5];
        let mut carry2 = 0u128;
        for i in 0..4 {
            let s = (res[i] as u128) + (h_38[i] as u128) + carry2;
            val[i] = s as u64;
            carry2 = s >> 64;
        }
        val[4] = (h_38[4] as u128 + carry2) as u64;

        let rem = (val[4] as u128) * 38;
        let mut val2 = [0u64; 4];
        let mut carry3 = rem;
        for i in 0..4 {
            let s = (val[i] as u128) + carry3;
            val2[i] = s as u64;
            carry3 = s >> 64;
        }
        if carry3 > 0 {
            let mut c = carry3 * 38;
            for i in 0..4 {
                let s = (val2[i] as u128) + c;
                val2[i] = s as u64;
                c = s >> 64;
            }
        }

        Self::reduce_256(val2)
    }

    pub fn square(&self) -> Self {
        self.mul(self)
    }

    /// Modular inverse using Fermat's Little Theorem and Bernstein's addition chain:
    /// a^(p - 2) = a^(2^255 - 21) (mod 2^255 - 19)
    pub fn invert(&self) -> Self {
        fn sq(mut val: Fe, n: usize) -> Fe {
            for _ in 0..n {
                val = val.square();
            }
            val
        }

        let z = *self;
        let z2 = sq(z, 1);
        let t0 = sq(z2, 2);
        let z9 = t0.mul(&z);
        let z11 = z9.mul(&z2);
        let t1 = sq(z11, 1);
        let z_2_5_minus_1 = t1.mul(&z9);

        let t2 = sq(z_2_5_minus_1, 5);
        let z_2_10_minus_1 = t2.mul(&z_2_5_minus_1);

        let t3 = sq(z_2_10_minus_1, 10);
        let z_2_20_minus_1 = t3.mul(&z_2_10_minus_1);

        let t4 = sq(z_2_20_minus_1, 20);
        let z_2_40_minus_1 = t4.mul(&z_2_20_minus_1);

        let t5 = sq(z_2_40_minus_1, 10);
        let z_2_50_minus_1 = t5.mul(&z_2_10_minus_1);

        let t6 = sq(z_2_50_minus_1, 50);
        let z_2_100_minus_1 = t6.mul(&z_2_50_minus_1);

        let t7 = sq(z_2_100_minus_1, 100);
        let z_2_200_minus_1 = t7.mul(&z_2_100_minus_1);

        let t8 = sq(z_2_200_minus_1, 50);
        let z_2_250_minus_1 = t8.mul(&z_2_50_minus_1);

        let t9 = sq(z_2_250_minus_1, 5);
        t9.mul(&z11)
    }

    /// Constant-time conditional swap.
    pub fn cswap(swap: u64, a: &mut Self, b: &mut Self) {
        let mask = if swap != 0 { u64::MAX } else { 0 };
        for i in 0..4 {
            let diff = mask & (a.0[i] ^ b.0[i]);
            a.0[i] ^= diff;
            b.0[i] ^= diff;
        }
    }
}

/// Curve25519 base point u = 9.
pub const BASE_POINT: [u8; 32] = {
    let mut b = [0u8; 32];
    b[0] = 9;
    b
};

/// Perform constant-time scalar multiplication on Curve25519 (RFC 7748).
pub fn x25519(scalar: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    let x1 = Fe::from_bytes(point);
    let mut x2 = Fe::ONE;
    let mut z2 = Fe::ZERO;
    let mut x3 = x1;
    let mut z3 = Fe::ONE;
    let mut swap: u64 = 0;

    let a24 = Fe([121665, 0, 0, 0]);

    for t in (0..=254).rev() {
        let byte_idx = t / 8;
        let bit_idx = t % 8;
        let kt = ((k[byte_idx] >> bit_idx) & 1) as u64;

        let s = kt ^ swap;
        Fe::cswap(s, &mut x2, &mut x3);
        Fe::cswap(s, &mut z2, &mut z3);
        swap = kt;

        let a = x2.add(&z2);
        let aa = a.square();
        let b = x2.sub(&z2);
        let bb = b.square();
        let e = aa.sub(&bb);
        let c = x3.add(&z3);
        let d = x3.sub(&z3);
        let da = d.mul(&a);
        let cb = c.mul(&b);

        let dacb_add = da.add(&cb);
        let dacb_sub = da.sub(&cb);

        x3 = dacb_add.square();
        z3 = x1.mul(&dacb_sub.square());
        x2 = aa.mul(&bb);
        z2 = e.mul(&aa.add(&e.mul(&a24)));
    }

    Fe::cswap(swap, &mut x2, &mut x3);
    Fe::cswap(swap, &mut z2, &mut z3);

    let res = x2.mul(&z2.invert());
    res.to_bytes()
}

/// Derive the 32-byte public key from a 32-byte private key.
pub fn public_key(private_key: &[u8; 32]) -> [u8; 32] {
    x25519(private_key, &BASE_POINT)
}
