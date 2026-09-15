//! kernel/rust/pbkdf2.rs - Phase 50: PBKDF2-HMAC-SHA256 (RFC 8018),
//! built on kernel/rust/hmac_sha256.rs - the third and final layer:
//! SHA-256 is a hash, HMAC-SHA256 is a keyed pseudorandom function
//! built on it, and PBKDF2 is what actually makes a *password* hash
//! - deliberately slow (iterated many times) and salted, the two
//! properties kernel/rust/users.rs's own original FNV-1a was always
//! documented as missing.
//!
//! Fixed at a 32-byte output (`dkLen = hLen = 32`, matching SHA-256's
//! own digest size exactly) - a deliberate simplification: PBKDF2's
//! general form supports deriving keys longer than one hash's output
//! by concatenating multiple blocks (`F(P,S,c,1) || F(P,S,c,2) ||
//! ...`), but every real caller in this kernel wants exactly a
//! 32-byte password-verification hash, never a longer derived key -
//! so only the single-block (`i=1`) case is implemented, not the
//! general multi-block one this kernel has no actual use for.

use crate::hmac_sha256::hmac_sha256;

const MAX_SALT_LEN: usize = 32; // generous bound above the 16-byte
                                 // salt kernel/rust/users.rs actually
                                 // uses

/// PBKDF2-HMAC-SHA256(password, salt, iterations), producing exactly
/// 32 bytes - see this module's own header comment for why only the
/// single-block case is implemented. Panics if `salt` exceeds
/// `MAX_SALT_LEN`, the same "real callers are always well within
/// this" reasoning every other bound in this phase's own three
/// modules (sha256.rs, hmac_sha256.rs) already uses.
pub fn pbkdf2_hmac_sha256(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
) -> [u8; 32] {
    assert!(salt.len() <= MAX_SALT_LEN, "pbkdf2: salt exceeds MAX_SALT_LEN");
    assert!(iterations >= 1, "pbkdf2: iterations must be at least 1");

    // U_1 = HMAC(password, salt || INT_32_BE(1)) - block index 1,
    // the only block this fixed-32-byte-output implementation ever
    // computes (see this module's own header comment).
    let mut salt_plus_index = [0u8; MAX_SALT_LEN + 4];
    salt_plus_index[..salt.len()].copy_from_slice(salt);
    salt_plus_index[salt.len()..salt.len() + 4]
        .copy_from_slice(&1u32.to_be_bytes());

    let mut u = hmac_sha256(password, &salt_plus_index[..salt.len() + 4]);
    let mut result = u;

    // U_2..U_c, each XORed into the running result - RFC 8018's own
    // F(P,S,c,1) = U_1 xor U_2 xor ... xor U_c.
    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for i in 0..32 {
            result[i] ^= u[i];
        }
    }

    result
}

/// Runs one real PBKDF2 computation at this phase's own actual
/// production iteration count (4096) against fixed, arbitrary input,
/// discarding the result - exists purely so kernel/init/main.c can
/// bracket a call to this with real timer reads and log how long
/// 4096 iterations actually takes on this kernel's own hardware/
/// emulation, rather than assuming a chosen iteration count is
/// reasonable without ever measuring it.
#[no_mangle]
pub extern "C" fn rust_pbkdf2_timing_probe() -> u8 {
    let result = pbkdf2_hmac_sha256(b"timing-probe-password", b"timing-probe-salt", 4096);
    result[0] // returned only so the computation can't be optimized
              // away as dead code; the caller ignores it
}

/// Ring-0 self-test - verifies against three independently-generated
/// test vectors (Python's own `hashlib.pbkdf2_hmac`, a trusted,
/// standard-library implementation, not this project's own code)
/// covering iterations=1 (no XOR-accumulation loop at all), 
/// iterations=2 (exactly one accumulation step - catches an
/// off-by-one in the loop bounds that iterations=1 alone couldn't),
/// and iterations=4096 (this phase's own actual, chosen production
/// value - see kernel/rust/users.rs's own PBKDF2_ITERATIONS constant
/// and its doc comment on why 4096, not a larger, "more standard"
/// modern value).
#[no_mangle]
pub extern "C" fn rust_pbkdf2_selftest() -> i32 {
    let mut code = 0;

    let password = b"password";
    let salt = b"salt";

    let expected_1: [u8; 32] = [
        0x12, 0x0f, 0xb6, 0xcf, 0xfc, 0xf8, 0xb3, 0x2c, 0x43, 0xe7, 0x22,
        0x52, 0x56, 0xc4, 0xf8, 0x37, 0xa8, 0x65, 0x48, 0xc9, 0x2c, 0xcc,
        0x35, 0x48, 0x08, 0x05, 0x98, 0x7c, 0xb7, 0x0b, 0xe1, 0x7b,
    ];
    if pbkdf2_hmac_sha256(password, salt, 1) != expected_1 {
        code |= 1;
    }

    let expected_2: [u8; 32] = [
        0xae, 0x4d, 0x0c, 0x95, 0xaf, 0x6b, 0x46, 0xd3, 0x2d, 0x0a, 0xdf,
        0xf9, 0x28, 0xf0, 0x6d, 0xd0, 0x2a, 0x30, 0x3f, 0x8e, 0xf3, 0xc2,
        0x51, 0xdf, 0xd6, 0xe2, 0xd8, 0x5a, 0x95, 0x47, 0x4c, 0x43,
    ];
    if pbkdf2_hmac_sha256(password, salt, 2) != expected_2 {
        code |= 2;
    }

    let expected_4096: [u8; 32] = [
        0xc5, 0xe4, 0x78, 0xd5, 0x92, 0x88, 0xc8, 0x41, 0xaa, 0x53, 0x0d,
        0xb6, 0x84, 0x5c, 0x4c, 0x8d, 0x96, 0x28, 0x93, 0xa0, 0x01, 0xce,
        0x4e, 0x11, 0xa4, 0x96, 0x38, 0x73, 0xaa, 0x98, 0x13, 0x4a,
    ];
    if pbkdf2_hmac_sha256(password, salt, 4096) != expected_4096 {
        code |= 4;
    }

    code
}
