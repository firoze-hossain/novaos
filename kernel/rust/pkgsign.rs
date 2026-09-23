//! kernel/rust/pkgsign.rs - Phase 69: package signature verification,
//! closing the real, explicitly-named gap this project's own release-
//! readiness doc tracked since Phase 64 ("Package signing /
//! verification... No code-signing of any kind exists yet - any ELF
//! that parses, runs... nothing stops a corrupted or malicious
//! package from running silently").
//!
//! # The trust model, stated plainly, not glossed over
//!
//! macOS's own Gatekeeper and Windows' own Authenticode - the two
//! reference points this feature was asked to match "the core idea
//! of, not the exact mechanism" - are both built on *asymmetric*
//! (public/private key) signatures: Apple and each software vendor
//! each hold a private key nobody else has, and the operating system
//! ships only the corresponding *public* key, so verifying a
//! signature can never let you forge one. A full, from-scratch,
//! correct, constant-time RSA or Ed25519 implementation is a
//! substantial, high-stakes undertaking on its own - real-world
//! asymmetric-crypto bugs are a genuinely common source of severe,
//! practical breaks, and a broken implementation shipped with false
//! confidence would be worse than no signing at all - and this
//! kernel has no bignum or elliptic-curve arithmetic of any kind to
//! build one on top of yet.
//!
//! This phase instead builds real, working, *symmetric* verification
//! on `kernel/rust/hmac_sha256.rs`'s own already-proven-correct HMAC-
//! SHA256 (verified against RFC 4231's own standard test vectors,
//! Phase 50) - a genuine, unbroken, correctly-implemented MAC
//! construction, not a toy. The real, honest limitation, stated
//! outright rather than hidden: HMAC is symmetric - the same key
//! signs and verifies, so `PKG_VERIFY_KEY` below is not safe to
//! publish the way a true public key would be. Anyone who extracts it
//! from a built kernel image could forge a signature that verifies
//! successfully. This is a genuinely different, weaker trust boundary
//! than Gatekeeper/Authenticode's own - a real trade-off, not a full
//! equivalent - made deliberately in exchange for something that is
//! actually finished, actually verified, and actually closes the
//! specific gap named ("any ELF that parses, runs" - now, an
//! unsigned or tampered package is refused before anything from it is
//! ever written to disk or executed). True asymmetric signing remains
//! real, valuable, explicitly-tracked follow-up work - not attempted
//! here.
//!
//! # What's actually checked
//!
//! `rust_pkg_verify_signature()` recomputes HMAC-SHA256(`PKG_VERIFY_
//! KEY`, SHA256(header-with-signature-field-zeroed || payload)) and
//! compares it, in constant time (see `constant_time_eq()` below - a
//! real, deliberate defense against a timing side-channel that could
//! otherwise leak the correct signature one byte at a time), against
//! the signature embedded in the package's own header. The signature
//! field itself is zeroed before hashing (not skipped/excluded by
//! slicing around it) so the exact same, simple "hash the whole
//! header struct" logic works identically on both the signing side
//! (tools/pkgsign, which starts from an all-zero signature field) and
//! the verifying side (this file, which zeroes whatever's actually
//! there before recomputing) - one shared shape, not two subtly
//! different ones that could quietly drift apart.
//!
//! Hashing (not HMAC-ing) the large payload directly is the standard
//! "hash-then-MAC" construction: `hmac_sha256()`'s own bounded, 256-
//! byte `MAX_MESSAGE_LEN` (Phase 50, a real and correct bound for
//! every caller *that phase* had) is far too small for a real package
//! payload, so this phase's own new `Sha256Streaming` (kernel/rust/
//! sha256.rs) digests the arbitrarily-large header+payload down to a
//! fixed 32 bytes first, and only that fixed-size digest is ever
//! passed to HMAC - security-equivalent to MAC-ing the full message
//! directly, given SHA-256's own collision resistance, and it keeps
//! `hmac_sha256.rs` itself completely unchanged.
//!
//! The actual key and the hash-then-MAC sequence itself live in
//! `kernel/rust/pkgsign_core.rs`, not this file - `include!`'d
//! identically here and by `tools/pkgsign/src/main.rs` (the host-side
//! signing tool), so the exact same code computes a signature on both
//! the signing and verifying side, not two hand-written copies that
//! could quietly drift apart. See that file's own doc comment.

use crate::pkgsign_core::{compute_signature, PKG_SIGNATURE_LEN};

/// Constant-time byte-slice comparison - deliberately not `==`
/// (`[u8; N]`'s own `PartialEq` is not documented or guaranteed to be
/// constant-time, and in practice commonly compiles to a short-
/// circuiting `memcmp`). A real, if narrow, timing side-channel
/// otherwise exists here: a verifier that returns as soon as it finds
/// the first mismatched byte lets an attacker who can measure
/// response time discover a valid signature one byte at a time,
/// rather than needing to guess all 32 at once - exactly the kind of
/// subtle mistake real, deployed signature-verification code has
/// shipped with before. This instead always inspects every byte of
/// both slices, accumulating any difference into one running value
/// that's only checked once, at the very end.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Verifies a package's own signature. `header_ptr`/`header_len` must
/// cover the *entire* fixed-size header struct, including its own
/// signature field (this function zeroes that field in its own local
/// copy before hashing - the caller does not need to, and should
/// pass the header exactly as read from disk, signature bytes and
/// all) - `signature_offset` names where within that header the 32-
/// byte signature field itself starts, so this function knows both
/// what to zero and what to compare against. Returns `true` only if
/// the recomputed signature matches exactly (`constant_time_eq()`),
/// meaning both the header and the entire payload are byte-identical
/// to what was signed.
///
/// # Safety
/// `header_ptr` must be valid for reads of `header_len` bytes,
/// `payload_ptr` for `payload_len` bytes. `signature_offset +
/// PKG_SIGNATURE_LEN` must be within `header_len`.
#[no_mangle]
pub unsafe extern "C" fn rust_pkg_verify_signature(
    header_ptr: *const u8,
    header_len: u32,
    signature_offset: u32,
    payload_ptr: *const u8,
    payload_len: u32,
) -> bool {
    let header_len = header_len as usize;
    let signature_offset = signature_offset as usize;
    if signature_offset + PKG_SIGNATURE_LEN > header_len || header_len > 256 {
        // header_len > 256 is not a real limit this function needs -
        // every real package header (pkg_header_t) is under 128
        // bytes - it's a sanity bound against a caller passing a
        // garbage length, so this can use a small, fixed local buffer
        // rather than needing its own heap allocation for something
        // that should never legitimately be large.
        return false;
    }

    let header = core::slice::from_raw_parts(header_ptr, header_len);
    let payload = core::slice::from_raw_parts(payload_ptr, payload_len as usize);

    let mut header_zeroed = [0u8; 256];
    header_zeroed[..header_len].copy_from_slice(header);
    for b in &mut header_zeroed[signature_offset..signature_offset + PKG_SIGNATURE_LEN] {
        *b = 0;
    }

    let expected = compute_signature(&header_zeroed[..header_len], payload);
    let actual = &header[signature_offset..signature_offset + PKG_SIGNATURE_LEN];
    constant_time_eq(&expected, actual)
}

/// Ring-0 self-test: a real signature (computed the same way
/// `tools/pkgsign` does) verifies successfully; a payload tampered
/// after signing is correctly rejected; a header field tampered after
/// signing is correctly rejected; a syntactically-valid but wrong
/// signature is correctly rejected. The three rejection cases are the
/// actual point of a signature scheme - "a good signature verifies"
/// alone would also be true of a scheme that verified everything.
#[no_mangle]
pub extern "C" fn rust_pkgsign_selftest() -> i32 {
    let mut code = 0;

    // A minimal, realistic stand-in header: 16 bytes of "manifest
    // fields" then a 32-byte signature field - the same overall shape
    // as pkg_header_t (magic+name+version+description+payload_size,
    // then signature), without needing this no_std module to know
    // that C struct's own exact layout.
    let mut header = [0u8; 48];
    header[0] = b'N';
    header[1] = b'V';
    header[2] = b'P';
    header[3] = b'K';
    let signature_offset = 16usize;
    let payload = b"a real package payload, more than 64 bytes long so this genuinely exercises the streaming hasher's own multi-block path, not just a single short block";

    let sig = compute_signature(&header, payload);
    header[signature_offset..signature_offset + 32].copy_from_slice(&sig);

    // Case 1: an untampered, correctly-signed package verifies.
    let ok = unsafe {
        rust_pkg_verify_signature(
            header.as_ptr(), header.len() as u32, signature_offset as u32,
            payload.as_ptr(), payload.len() as u32,
        )
    };
    if !ok {
        code |= 1;
    }

    // Case 2: tampering with the payload after signing must be
    // caught - the actual point of signing a package at all.
    let mut tampered_payload = *payload;
    tampered_payload[0] ^= 0xFF;
    let ok = unsafe {
        rust_pkg_verify_signature(
            header.as_ptr(), header.len() as u32, signature_offset as u32,
            tampered_payload.as_ptr(), tampered_payload.len() as u32,
        )
    };
    if ok {
        code |= 2;
    }

    // Case 3: tampering with a header field after signing (e.g. a
    // corrupted or attacker-modified manifest name/version) must also
    // be caught, not just payload tampering.
    let mut tampered_header = header;
    tampered_header[4] ^= 0xFF;
    let ok = unsafe {
        rust_pkg_verify_signature(
            tampered_header.as_ptr(), tampered_header.len() as u32,
            signature_offset as u32, payload.as_ptr(), payload.len() as u32,
        )
    };
    if ok {
        code |= 4;
    }

    // Case 4: a syntactically-valid (right length) but simply wrong
    // signature - not derived from tampering with case 1's own real
    // one - must also be rejected, not just detectably-corrupted
    // signatures.
    let mut wrong_sig_header = header;
    for b in &mut wrong_sig_header[signature_offset..signature_offset + 32] {
        *b = 0x42;
    }
    let ok = unsafe {
        rust_pkg_verify_signature(
            wrong_sig_header.as_ptr(), wrong_sig_header.len() as u32,
            signature_offset as u32, payload.as_ptr(), payload.len() as u32,
        )
    };
    if ok {
        code |= 8;
    }

    code
}
