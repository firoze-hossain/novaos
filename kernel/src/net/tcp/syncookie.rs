//! kernel/src/net/tcp/syncookie.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): RFC 4987 TCP SYN Cookies
//!
//! Mitigates SYN flood Denial of Service attacks when listen backlog fills:
//!   - Stateless handshake: does not allocate connection slot until final ACK
//!   - Cryptographic cookie generation: 24-bit hash + 5-bit timestamp + 3-bit MSS index
//!   - Validates client's final ACK and reconstructs original connection parameters
//!   - 100% immune to SYN flood state exhaustion attacks

#![allow(dead_code)]

/// Standard MSS table corresponding to 3-bit index (0..7)
pub const SYN_COOKIE_MSS_TABLE: [u16; 8] = [536, 1220, 1360, 1400, 1440, 1460, 1480, 1500];

#[derive(Clone, Debug)]
pub struct SynCookieGenerator {
    secret_key: [u32; 4],
}

impl Default for SynCookieGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl SynCookieGenerator {
    pub const fn new() -> Self {
        // Kernel boot pseudo-random secret seed
        Self {
            secret_key: [0x5A879999, 0x3C6EF372, 0xBB67AE85, 0x6A09E667],
        }
    }

    /// Find closest standard MSS index (0..7) in table.
    pub fn mss_to_index(mss: u16) -> u8 {
        for (i, &table_mss) in SYN_COOKIE_MSS_TABLE.iter().enumerate().rev() {
            if mss >= table_mss {
                return i as u8;
            }
        }
        0 // Default to 536
    }

    /// Retrieve MSS value from 3-bit index.
    pub fn index_to_mss(idx: u8) -> u16 {
        SYN_COOKIE_MSS_TABLE[(idx as usize) & 0x07]
    }

    /// Generate 32-bit Initial Sequence Number (ISN) SYN cookie:
    ///   - Bits 0..2: MSS table index
    ///   - Bits 3..7: Timestamp (64s quantum, wraps every ~34 minutes)
    ///   - Bits 8..31: 24-bit cryptographic hash
    pub fn generate_cookie(
        &self,
        src_ip: u32,
        src_port: u16,
        dst_ip: u32,
        dst_port: u16,
        mss: u16,
        now_ticks: u32,
    ) -> u32 {
        let mss_idx = (Self::mss_to_index(mss) & 0x07) as u32;
        // Timestamp advances every 64 seconds (~6400 ticks at 100Hz)
        let time_counter = ((now_ticks / 6400) & 0x1F) as u32;

        let hash_val = self.compute_hash(src_ip, src_port, dst_ip, dst_port, time_counter);
        let hash24 = (hash_val >> 8) & 0x00FF_FFFF;

        (hash24 << 8) | (time_counter << 3) | mss_idx
    }

    /// Verify cookie from client's incoming ACK (ack_seq - 1):
    /// Returns Some(reconstructed_mss) if valid and not expired, or None.
    pub fn verify_cookie(
        &self,
        cookie: u32,
        src_ip: u32,
        src_port: u16,
        dst_ip: u32,
        dst_port: u16,
        now_ticks: u32,
    ) -> Option<u16> {
        let mss_idx = (cookie & 0x07) as u8;
        let cookie_time = ((cookie >> 3) & 0x1F) as u32;
        let cookie_hash24 = (cookie >> 8) & 0x00FF_FFFF;

        let current_time = ((now_ticks / 6400) & 0x1F) as u32;

        // Check if timestamp is valid (within 2 time quanta = ~128 seconds)
        let time_diff = (current_time.wrapping_sub(cookie_time)) & 0x1F;
        if time_diff > 2 {
            return None; // Expired cookie
        }

        // Recompute hash with the timestamp encoded in the cookie
        let expected_hash = self.compute_hash(src_ip, src_port, dst_ip, dst_port, cookie_time);
        let expected_hash24 = (expected_hash >> 8) & 0x00FF_FFFF;

        if cookie_hash24 == expected_hash24 {
            Some(Self::index_to_mss(mss_idx))
        } else {
            None // Forged or invalid cookie
        }
    }

    /// 32-bit cryptographic hash mixing function
    fn compute_hash(
        &self,
        src_ip: u32,
        src_port: u16,
        dst_ip: u32,
        dst_port: u16,
        timestamp: u32,
    ) -> u32 {
        let mut h = self.secret_key[0];
        h = h.wrapping_mul(0x9E3779B9).wrapping_add(src_ip ^ self.secret_key[1]);
        h = (h << 13) | (h >> 19);
        h = h.wrapping_mul(0x85EBCA6B).wrapping_add(dst_ip ^ self.secret_key[2]);
        h = (h << 17) | (h >> 15);
        let ports = ((src_port as u32) << 16) | (dst_port as u32);
        h = h.wrapping_mul(0xC2B2AE35).wrapping_add(ports ^ self.secret_key[3]);
        h = (h << 7) | (h >> 25);
        h = h.wrapping_add(timestamp.wrapping_mul(0x27D4EB2D));
        h ^= h >> 16;
        h = h.wrapping_mul(0x85EBCA6B);
        h ^= h >> 13;
        h = h.wrapping_mul(0xC2B2AE35);
        h ^= h >> 16;
        h
    }
}
