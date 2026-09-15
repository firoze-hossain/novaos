//! kernel/rust/hmac_sha256.rs - Phase 50: HMAC-SHA256 (RFC 2104/6234),
//! built directly on kernel/rust/sha256.rs's own from-scratch
//! implementation - the second layer of "kernel/rust/pbkdf2.rs (this
//! same phase) needs a keyed pseudorandom function, and this is the
//! standard one," not a new design.

use crate::sha256::sha256;

const BLOCK_SIZE: usize = 64; // SHA-256's own block size
const MAX_KEY_LEN: usize = 128; // generous bound above every real
                                 // caller's actual key size in this
                                 // kernel (passwords, bounded by
                                 // shell.c's own 64-byte input buffer,
                                 // and PBKDF2's own derived keys,
                                 // always exactly 32 bytes)
const MAX_MESSAGE_LEN: usize = 256; // generous bound above every real
                                     // caller's actual message size
                                     // (PBKDF2's own per-block input is
                                     // salt (16 bytes) + a 4-byte
                                     // counter + (after the first
                                     // iteration) a 32-byte digest -
                                     // well under this)

/// HMAC-SHA256(key, message). Panics if `key`/`message` exceed
/// `MAX_KEY_LEN`/`MAX_MESSAGE_LEN` - the same "a real caller is always
/// well within this; hitting it means a real bug, not a condition to
/// degrade from" reasoning kernel/rust/sha256.rs's own bound uses.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    assert!(key.len() <= MAX_KEY_LEN, "hmac_sha256: key exceeds MAX_KEY_LEN");
    assert!(
        message.len() <= MAX_MESSAGE_LEN,
        "hmac_sha256: message exceeds MAX_MESSAGE_LEN"
    );

    // Step 1: normalize the key to exactly BLOCK_SIZE bytes - hash it
    // down if it's longer than a block, zero-pad if shorter (RFC
    // 2104's own K' construction).
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = sha256(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    // Step 2: inner hash = SHA256((K' xor ipad) || message)
    let mut inner_input = [0u8; BLOCK_SIZE + MAX_MESSAGE_LEN];
    for i in 0..BLOCK_SIZE {
        inner_input[i] = key_block[i] ^ 0x36;
    }
    inner_input[BLOCK_SIZE..BLOCK_SIZE + message.len()].copy_from_slice(message);
    let inner_hash = sha256(&inner_input[..BLOCK_SIZE + message.len()]);

    // Step 3: outer hash = SHA256((K' xor opad) || inner_hash)
    let mut outer_input = [0u8; BLOCK_SIZE + 32];
    for i in 0..BLOCK_SIZE {
        outer_input[i] = key_block[i] ^ 0x5c;
    }
    outer_input[BLOCK_SIZE..BLOCK_SIZE + 32].copy_from_slice(&inner_hash);
    sha256(&outer_input)
}

/// Ring-0 self-test - verifies against RFC 4231's own Test Case 1
/// (the standard first HMAC-SHA256 test vector: a 20-byte key of
/// 0x0b bytes, the message "Hi There"), independently cross-checked
/// (Python's hmac/hashlib modules) before being hardcoded here, the
/// same discipline kernel/rust/sha256.rs's own test vectors used.
#[no_mangle]
pub extern "C" fn rust_hmac_sha256_selftest() -> i32 {
    let mut code = 0;

    let key = [0x0bu8; 20];
    let message = b"Hi There";
    let expected: [u8; 32] = [
        0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf,
        0xce, 0xaf, 0x0b, 0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83,
        0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c, 0x2e, 0x32, 0xcf, 0xf7,
    ];
    if hmac_sha256(&key, message) != expected {
        code |= 1;
    }

    code
}
