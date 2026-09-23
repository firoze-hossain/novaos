//! kernel/rust/pkgsign_core.rs - Phase 69: the one, single definition
//! of the package-signing key and signature computation, `include!`'d
//! identically by both kernel/rust/pkgsign.rs (which verifies, inside
//! the kernel) and tools/pkgsign/src/main.rs (which signs, on the
//! host, at package-build time) - see pkgsign.rs's own module doc
//! comment for the full scheme and its honest, symmetric-HMAC trust-
//! model limitations.
//!
//! Deliberately its own, separate file rather than living directly in
//! pkgsign.rs: everything else in pkgsign.rs (the `#[no_mangle]
//! extern "C"` FFI surface, the self-test) is kernel-specific and has
//! no reason to exist in a plain host binary, but the key and the
//! exact hashing/MAC sequence *must* be byte-for-byte identical on
//! both sides, or a package this tool signs would never verify inside
//! the kernel. `include!`ing this one real file into both, rather
//! than hand-copying the same few lines into two places, is what
//! actually guarantees that - not a comment asking a future editor to
//! remember to keep two copies in sync.

/// The shared HMAC key both this kernel (verifying) and
/// tools/pkgsign (signing, on the host, at build time) use - see
/// pkgsign.rs's own module doc comment for the real, honest trust-
/// model limitation this implies. A real, fixed 32-byte key, not a
/// placeholder - generated once for this project and baked into both
/// sides identically; changing it here without re-signing every
/// existing package with the same new key would break every one of
/// them, by design (that's what "the key changed" is supposed to
/// do).
const PKG_VERIFY_KEY: [u8; 32] = [
    0x4e, 0x6f, 0x76, 0x61, 0x4f, 0x53, 0x2d, 0x70, 0x6b, 0x67, 0x2d, 0x73,
    0x69, 0x67, 0x6e, 0x2d, 0x76, 0x31, 0x2d, 0xa7, 0x3c, 0x91, 0x5d, 0x02,
    0xe8, 0x4f, 0x1b, 0x6a, 0xc3, 0x77, 0x0d, 0x29,
];

/// The signature field's own size, in bytes.
pub const PKG_SIGNATURE_LEN: usize = 32;

/// Computes the signature for `header` (with its own signature field
/// already zeroed by the caller) followed by `payload` - see
/// pkgsign.rs's own module doc comment for why hashing (not directly
/// HMAC-ing) the payload first is the right construction here, and
/// why zeroing the signature field rather than excluding it by
/// slicing is the one shared shape both signing and verifying use.
pub fn compute_signature(header: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut hasher = crate::sha256::Sha256Streaming::new();
    hasher.update(header);
    hasher.update(payload);
    let digest = hasher.finalize();
    crate::hmac_sha256::hmac_sha256(&PKG_VERIFY_KEY, &digest)
}
