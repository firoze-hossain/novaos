//! kernel/src/net/wireguard/crypto/tai64n.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Native In-Kernel WireGuard VPN
//!
//! TAI64N Monotonic Timestamp Generator and Replay Resistance Validator.
//! Format: 8-byte big-endian seconds (2^62 + seconds) + 4-byte big-endian nanoseconds.

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tai64n([u8; 12]);

impl Tai64n {
    pub const ZERO: Self = Tai64n([0; 12]);

    /// Construct a TAI64N timestamp from seconds since Unix epoch and nanoseconds.
    pub fn new(seconds: u64, nanos: u32) -> Self {
        let mut b = [0u8; 12];
        let tai_secs = (1u64 << 62).wrapping_add(seconds);
        b[0..8].copy_from_slice(&tai_secs.to_be_bytes());
        b[8..12].copy_from_slice(&nanos.to_be_bytes());
        Tai64n(b)
    }

    pub fn from_bytes(bytes: &[u8; 12]) -> Self {
        Tai64n(*bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 12] {
        &self.0
    }

    /// Check if this timestamp is strictly greater than another timestamp (monotonic check).
    pub fn is_after(&self, other: &Self) -> bool {
        self.0 > other.0
    }
}
