//! kernel/rust/users.rs - Phase 47: a user database - UID/GID
//! accounts and password authentication, the genuine first
//! prerequisite for real users/permissions on this kernel.
//!
//! Gap this fills: before this phase, this kernel had no concept of
//! "who owns this process" at all - the first-run wizard's own
//! "username" was purely cosmetic (a string shown in the shell
//! prompt), not tied to any credential check or process identity.
//! This is the item this project's own release-readiness roadmap
//! names directly: "Linux/Unix's UID/GID + permission-bits model...
//! Start here, not with something more elaborate."
//!
//! Scope, stated plainly, and bounded by a real, external constraint
//! discovered before writing any code, not assumed: this kernel's
//! only writable filesystem (FAT32) has no on-disk field for file
//! ownership or permission bits at all - its directory entry format
//! (kernel/fs/fat32.c's own `fat_dirent_t`) is exactly the real,
//! standard 32-byte FAT32 entry (name, attributes, timestamps,
//! cluster, size), with nothing resembling a uid/gid/mode field.
//! Adding one would mean a non-standard extension, breaking
//! compatibility with every other FAT32 implementation - not
//! attempted. This phase therefore builds real, working PROCESS-level
//! identity (every process has a genuine UID/GID, checkable via a
//! real syscall, set only through real password authentication) -
//! the actual foundation "everything else (sudo, ACLs, containers)
//! builds on," per this project's own framing - without claiming
//! file-level ownership/chmod, which stays a real, separate, honestly
//! documented gap (see PROGRESS.md's own Phase 47 "Known
//! limitations").
//!
//! Password hashing note, updated by Phase 50: this now uses real
//! PBKDF2-HMAC-SHA256 (kernel/rust/pbkdf2.rs, built on kernel/rust/
//! hmac_sha256.rs and kernel/rust/sha256.rs, all three implemented
//! from scratch in this same phase - no crates.io, no external
//! dependencies exist in this freestanding kernel) with a real,
//! per-account salt and 4096 iterations - deliberately slow, measured
//! directly on this kernel's own hardware/emulation at roughly 50-60ms
//! per attempt (see PROGRESS.md's own Phase 50 entry for the exact
//! measured timing), not just assumed reasonable. Superseding this
//! phase's own predecessor, FNV-1a (Phase 47), which was always
//! explicitly documented as not a secure password hash at all - this
//! is the real one that documentation promised as follow-up work.
//!
//! Salt derivation is honestly, not silently, bounded: this kernel
//! has no real entropy source (confirmed directly before writing any
//! code, not assumed - grepped the whole kernel tree; the only
//! existing precedent, kernel/net/tcp.c's own initial sequence number,
//! is itself explicitly documented as "not cryptographically random").
//! The salt is derived from `timer_get_ticks()` (passed in by the C
//! caller, since reading a hardware timer is not this Rust module's
//! own concern) mixed with the username through SHA-256 - varying
//! per-account and per-boot-moment, genuinely defeating precomputed
//! rainbow-table attacks (a salt's actual job), but not
//! unpredictable in the cryptographic sense a hardware RNG would
//! provide. A real hardware entropy source (RDRAND, if available; a
//! /dev/random-equivalent otherwise) is real, separate follow-up work.

use crate::pbkdf2::pbkdf2_hmac_sha256;
use crate::sha256::sha256;

const MAX_USERS: usize = 8;
const USERNAME_MAX: usize = 32;
const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32; // PBKDF2-HMAC-SHA256's own fixed output
                            // size - see kernel/rust/pbkdf2.rs's own
                            // header comment on why this
                            // implementation only supports exactly
                            // this length

/// 4096 - measured directly on this kernel's own hardware/emulation
/// (kernel/init/main.c's own timing probe, bracketing a real call
/// with real timer reads), not chosen from a table and assumed
/// reasonable: roughly 50-60ms per attempt at this kernel's default
/// 100Hz timer resolution (see PROGRESS.md's own Phase 50 entry for
/// the exact measured tick count). Far below modern, general-purpose
/// guidance for a networked, multi-user, high-value system (which
/// commonly recommends 100k+ iterations) - a deliberate, honest
/// trade-off for this kernel's own single-machine, hobby-OS context:
/// slow enough to meaningfully matter against a fast automated
/// guesser (thousands of times slower than the FNV-1a hash this
/// replaces), while staying imperceptible to a real person typing
/// their own password at the login prompt this computes it for
/// (Phase 49). A real, general-purpose OS serving untrusted, remote,
/// or high-value accounts would reasonably choose a much higher
/// value; revisiting this if that ever becomes this kernel's own
/// actual threat model is real, separate, future work.
const PBKDF2_ITERATIONS: u32 = 4096;

#[derive(Clone, Copy)]
struct UserRecord {
    in_use: bool,
    username: [u8; USERNAME_MAX],
    username_len: u8,
    uid: u32,
    gid: u32,
    salt: [u8; SALT_LEN],
    password_hash: [u8; HASH_LEN],
    /// Phase 49: consecutive failed authentication attempts since the
    /// last success - the genuine "session concept" half of a real
    /// login screen, not just the credential check itself. Reset to 0
    /// on any successful authentication; once it reaches
    /// LOCKOUT_THRESHOLD, further attempts are rejected outright, even
    /// with the correct password, until either a successful login
    /// clears it or the system reboots.
    ///
    /// Deliberately NOT part of the on-disk USERS.CFG format (see
    /// rust_users_save()/load() below, both untouched by this field) -
    /// a real, honest simplification: this counter resets on every
    /// reboot, which a determined attacker could exploit by rebooting
    /// between attempts. A persistent lockout counter is real,
    /// separate follow-up work; this is still a genuine improvement
    /// over no rate-limiting at all within a single boot session, and
    /// changing the on-disk format at all would risk this project's
    /// own already-verified tools/fixtures/USERS.CFG fixture for a
    /// property (persistent lockout) not actually being asked for.
    failed_attempts: u32,
}

/// After this many consecutive failed attempts against one account
/// (within a single boot - see failed_attempts' own doc comment),
/// authentication is refused outright, even with the correct
/// password. Chosen high enough that no existing self-test's own
/// intentional wrong-password checks (kernel/rust/users.rs's own
/// rust_users_selftest, kernel/task/sandbox_demo.c's real ring-3
/// test) could ever accidentally trigger it - each makes exactly one
/// wrong-password attempt against any given account, nowhere near
/// this threshold.
const LOCKOUT_THRESHOLD: u32 = 5;

impl UserRecord {
    const fn empty() -> Self {
        UserRecord {
            in_use: false,
            username: [0u8; USERNAME_MAX],
            username_len: 0,
            uid: 0,
            gid: 0,
            salt: [0u8; SALT_LEN],
            password_hash: [0u8; HASH_LEN],
            failed_attempts: 0,
        }
    }
}

const EMPTY_USER: UserRecord = UserRecord::empty();
static mut USERS: [UserRecord; MAX_USERS] = [EMPTY_USER; MAX_USERS];
static mut USER_COUNT: usize = 0;

/// Derives this account's salt from `seed` (the C caller's own
/// `timer_get_ticks()` reading - see this module's own header comment
/// on why that specific source, and its honest limitation) mixed with
/// the username through SHA-256, so two accounts created in the same
/// tick still get different salts.
fn derive_salt(seed: u32, username: &[u8]) -> [u8; SALT_LEN] {
    let mut input = [0u8; 4 + USERNAME_MAX];
    input[..4].copy_from_slice(&seed.to_le_bytes());
    input[4..4 + username.len()].copy_from_slice(username);
    let hash = sha256(&input[..4 + username.len()]);
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&hash[..SALT_LEN]);
    salt
}

/// Compares two hashes without short-circuiting on the first
/// mismatched byte - a real, if modest, improvement over `==` for
/// comparing secret-derived values: a naive comparison that returns
/// as soon as it finds a difference leaks, via how long the
/// comparison itself takes, roughly how many leading bytes an
/// attacker's guess got right, which a truly constant-time compare
/// (this one) does not.
fn constant_time_eq(a: &[u8; HASH_LEN], b: &[u8; HASH_LEN]) -> bool {
    let mut diff: u8 = 0;
    for i in 0..HASH_LEN {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn username_matches(record: &UserRecord, name: &[u8]) -> bool {
    record.in_use
        && record.username_len as usize == name.len()
        && &record.username[..name.len()] == name
}

/// # Safety
/// Only ever called from single-threaded, interrupt-disabled syscall/
/// boot context, exactly like this kernel's other `static mut` kernel
/// state - see kernel/rust/pipe.rs's own concurrency note for the
/// same reasoning applied here.
unsafe fn users() -> &'static mut [UserRecord; MAX_USERS] {
    &mut *core::ptr::addr_of_mut!(USERS)
}

/// Adds a new user account. Returns `false` if the username already
/// exists, the username is longer than `USERNAME_MAX`, or the table
/// is full (`MAX_USERS`) - this kernel's usual fixed-capacity-
/// exhaustion failure mode, matching every other fixed-size table in
/// this project.
///
/// `salt_seed`: the C caller's own `timer_get_ticks()` reading, used
/// to derive this account's salt (see `derive_salt`'s own doc comment)
/// - reading a hardware timer is a C-side concern, not this Rust
/// module's own, so the value is passed in rather than read here.
///
/// # Safety
/// `username_ptr`/`password_ptr` must be valid for reads of
/// `username_len`/`password_len` bytes respectively.
#[no_mangle]
pub unsafe extern "C" fn rust_users_add(
    username_ptr: *const u8,
    username_len: u32,
    uid: u32,
    gid: u32,
    password_ptr: *const u8,
    password_len: u32,
    salt_seed: u32,
) -> bool {
    if username_len as usize > USERNAME_MAX {
        return false;
    }
    let name = core::slice::from_raw_parts(username_ptr, username_len as usize);
    let password =
        core::slice::from_raw_parts(password_ptr, password_len as usize);

    let table = users();
    for existing in table.iter() {
        if username_matches(existing, name) {
            return false; // username already taken
        }
    }

    let salt = derive_salt(salt_seed, name);
    let hash = pbkdf2_hmac_sha256(password, &salt, PBKDF2_ITERATIONS);

    for slot in table.iter_mut() {
        if !slot.in_use {
            slot.in_use = true;
            slot.username = [0u8; USERNAME_MAX];
            slot.username[..name.len()].copy_from_slice(name);
            slot.username_len = name.len() as u8;
            slot.uid = uid;
            slot.gid = gid;
            slot.salt = salt;
            slot.password_hash = hash;
            USER_COUNT += 1;
            return true;
        }
    }
    false // table full
}

/// Checks a username/password pair against the user database. On a
/// match, writes the account's uid/gid to `out_uid`/`out_gid` and
/// returns `true`. Returns `false` on any mismatch (unknown username
/// *or* wrong password - deliberately not distinguished in the return
/// value, the same reasoning real login prompts use: revealing
/// "username exists but password is wrong" vs. "no such username" at
/// all makes username enumeration trivial for an attacker) - and also
/// `false`, even for the *correct* password, once an account has
/// LOCKOUT_THRESHOLD consecutive failures (see UserRecord's own
/// failed_attempts field doc comment) - deliberately not distinguished
/// from a plain wrong-password result either, the same reasoning:
/// revealing "this account is locked" is itself information a
/// generic failure message shouldn't leak.
///
/// # Safety
/// `username_ptr`/`password_ptr` must be valid for reads of
/// `username_len`/`password_len` bytes. `out_uid`/`out_gid` must be
/// valid, writable `u32`s.
#[no_mangle]
pub unsafe extern "C" fn rust_users_authenticate(
    username_ptr: *const u8,
    username_len: u32,
    password_ptr: *const u8,
    password_len: u32,
    out_uid: *mut u32,
    out_gid: *mut u32,
) -> bool {
    let name = core::slice::from_raw_parts(username_ptr, username_len as usize);
    let password =
        core::slice::from_raw_parts(password_ptr, password_len as usize);

    for record in users().iter_mut() {
        if username_matches(record, name) {
            if record.failed_attempts >= LOCKOUT_THRESHOLD {
                return false; // locked out - not even the correct
                              // password is accepted until a reboot
                              // (see failed_attempts' own doc comment)
            }
            let attempt_hash =
                pbkdf2_hmac_sha256(password, &record.salt, PBKDF2_ITERATIONS);
            if constant_time_eq(&record.password_hash, &attempt_hash) {
                record.failed_attempts = 0; // a success clears any
                                             // prior failures
                core::ptr::write(out_uid, record.uid);
                core::ptr::write(out_gid, record.gid);
                return true;
            }
            record.failed_attempts += 1;
            return false; // right username, wrong password
        }
    }
    false // no such username
}

/// Phase 51: the actual sudo gate - re-authenticates the account
/// identified by `uid` (the *calling process's own* current uid,
/// looked up by numeric ID rather than by re-typing a username, since
/// a process genuinely only knows its own uid, not which username it
/// corresponds to - real sudo works the same way, mapping the real
/// calling uid back to an account) against `password`, and returns
/// `true` only if *both* the password is correct *and* that account
/// is a member of what this kernel calls the "admin group" - `gid ==
/// 0`, the same numeric value as root's own gid, a deliberately
/// simple convention rather than a general group-membership system
/// this kernel has no other use for yet. A correct password for an
/// account that is not in the admin group returns `false`, same as a
/// wrong password - "authenticated but not authorized" and "not even
/// authenticated" are deliberately not distinguished in the return
/// value, the same anti-information-leak reasoning
/// `rust_users_authenticate`'s own doc comment already established
/// for wrong-password vs. unknown-username.
///
/// # Safety
/// `password_ptr` must be valid for reads of `password_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_users_sudo_check(
    uid: u32,
    password_ptr: *const u8,
    password_len: u32,
) -> bool {
    let password =
        core::slice::from_raw_parts(password_ptr, password_len as usize);

    for record in users().iter_mut() {
        if record.in_use && record.uid == uid {
            if record.failed_attempts >= LOCKOUT_THRESHOLD {
                return false;
            }
            let attempt_hash =
                pbkdf2_hmac_sha256(password, &record.salt, PBKDF2_ITERATIONS);
            if constant_time_eq(&record.password_hash, &attempt_hash) {
                record.failed_attempts = 0;
                return record.gid == 0; // authenticated - but only
                                         // authorized if also in the
                                         // admin group
            }
            record.failed_attempts += 1;
            return false;
        }
    }
    false // no account with this uid - shouldn't happen for a real,
          // already-logged-in caller, but never trusted regardless
}

#[no_mangle]
pub extern "C" fn rust_users_count() -> u32 {
    unsafe { USER_COUNT as u32 }
}

/// On-disk record layout (for USERS.CFG, via kernel/fs/vfs.c's own
/// read/write, the same "kernel reads/writes a plain file, Rust
/// (de)serializes it" pattern SYSTEM.CFG already established):
/// username_len(1) + username(32, zero-padded) + uid(4) + gid(4) +
/// salt(16) + password_hash(32) = 89 bytes per record (grown from
/// Phase 47's original 45 - the salt and the full 32-byte PBKDF2
/// output this phase adds, replacing the old 4-byte FNV-1a value),
/// MAX_USERS records back to back, no header/checksum - deliberately
/// as simple as SYSTEM.CFG's own fixed-layout format, not a new, more
/// elaborate scheme.
const RECORD_SIZE: usize = 1 + USERNAME_MAX + 4 + 4 + SALT_LEN + HASH_LEN;

#[no_mangle]
pub extern "C" fn rust_users_serialized_size() -> u32 {
    (RECORD_SIZE * MAX_USERS) as u32
}

/// Serializes every in-use record into `out`. Returns the number of
/// bytes written, or `-1` if `out_len` is smaller than
/// `rust_users_serialized_size()`.
///
/// # Safety
/// `out` must be valid for writes of at least `out_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_users_save(out: *mut u8, out_len: u32) -> i32 {
    let needed = rust_users_serialized_size();
    if out_len < needed {
        return -1;
    }
    let buf = core::slice::from_raw_parts_mut(out, needed as usize);
    for (i, record) in users().iter().enumerate() {
        let base = i * RECORD_SIZE;
        buf[base] = if record.in_use { record.username_len } else { 0 };
        buf[base + 1..base + 1 + USERNAME_MAX].copy_from_slice(&record.username);
        let mut off = base + 1 + USERNAME_MAX;
        buf[off..off + 4].copy_from_slice(&record.uid.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&record.gid.to_le_bytes());
        off += 4;
        buf[off..off + SALT_LEN].copy_from_slice(&record.salt);
        off += SALT_LEN;
        buf[off..off + HASH_LEN].copy_from_slice(&record.password_hash);
    }
    needed as i32
}

/// Loads records from a previously-`rust_users_save`'d buffer,
/// replacing whatever is currently in memory - used once, at boot, if
/// USERS.CFG already exists (see kernel/init/main.c's own call site).
/// Returns `false` (leaving the in-memory database unchanged) if
/// `data_len` doesn't exactly match `rust_users_serialized_size()` -
/// an old-format or corrupted file is treated as absent, not
/// partially trusted, the same "exact-size match or treat as invalid"
/// discipline kernel/config/sysconfig.c already established for
/// SYSTEM.CFG.
///
/// # Safety
/// `data` must be valid for reads of `data_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_users_load(data: *const u8, data_len: u32) -> bool {
    let needed = rust_users_serialized_size();
    if data_len != needed {
        return false;
    }
    let buf = core::slice::from_raw_parts(data, data_len as usize);

    let mut loaded = [EMPTY_USER; MAX_USERS];
    let mut count = 0usize;
    for i in 0..MAX_USERS {
        let base = i * RECORD_SIZE;
        let username_len = buf[base];
        if username_len == 0 {
            continue; // an empty slot - in_use stays false
        }
        if username_len as usize > USERNAME_MAX {
            return false; // corrupt record - refuse the whole file
                          // rather than load a partially-valid database
        }
        let mut record = UserRecord::empty();
        record.in_use = true;
        record.username_len = username_len;
        record.username
            .copy_from_slice(&buf[base + 1..base + 1 + USERNAME_MAX]);
        let mut off = base + 1 + USERNAME_MAX;
        record.uid = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        off += 4;
        record.gid = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        off += 4;
        record.salt.copy_from_slice(&buf[off..off + SALT_LEN]);
        off += SALT_LEN;
        record.password_hash.copy_from_slice(&buf[off..off + HASH_LEN]);
        loaded[i] = record;
        count += 1;
    }

    *users() = loaded;
    USER_COUNT = count;
    true
}

/// Ring-0 self-test, called directly from kernel_main() - proves the
/// add/authenticate/serialize/load round trip works correctly before
/// anything else in this kernel depends on it: adds a user, confirms
/// the right password succeeds and returns the right uid/gid, confirms
/// a wrong password fails, confirms an unknown username fails,
/// serializes the database, wipes it, reloads from the serialized
/// bytes, and confirms authentication still works identically after
/// that round trip - proving USERS.CFG persistence itself is correct,
/// not just the in-memory add/authenticate logic. Also confirms
/// Phase 49's own lockout behavior: after LOCKOUT_THRESHOLD consecutive
/// wrong-password attempts against a (separate) account, even that
/// account's genuinely correct password is rejected. Phase 50 adds one
/// more: two different accounts given the exact same password but
/// different salt seeds must end up with different stored hashes -
/// the actual, observable point of salting at all, not just that
/// authentication still works (which would be true even if salting
/// were silently broken and every account shared one salt). Phase 51
/// adds direct verification of `rust_users_sudo_check` itself: a
/// wrong password rejected, an admin-group account's correct password
/// accepted, and - the actual point of the function - a genuinely
/// correct password for a *non*-admin-group account still rejected.
#[no_mangle]
pub extern "C" fn rust_users_selftest() -> i32 {
    let mut code = 0;

    let username = b"selftest";
    let password = b"correct-password";
    let wrong_password = b"wrong-password";

    let added = unsafe {
        rust_users_add(
            username.as_ptr(),
            username.len() as u32,
            42,
            43,
            password.as_ptr(),
            password.len() as u32,
            0x1111_1111,
        )
    };
    if !added {
        code |= 1;
    }

    let mut uid = 0u32;
    let mut gid = 0u32;
    let right_password_ok = unsafe {
        rust_users_authenticate(
            username.as_ptr(),
            username.len() as u32,
            password.as_ptr(),
            password.len() as u32,
            &mut uid,
            &mut gid,
        )
    };
    if !right_password_ok || uid != 42 || gid != 43 {
        code |= 2;
    }

    let wrong_password_ok = unsafe {
        rust_users_authenticate(
            username.as_ptr(),
            username.len() as u32,
            wrong_password.as_ptr(),
            wrong_password.len() as u32,
            &mut uid,
            &mut gid,
        )
    };
    if wrong_password_ok {
        code |= 4; // must NOT succeed with the wrong password
    }

    let unknown_user = b"nobody";
    let unknown_ok = unsafe {
        rust_users_authenticate(
            unknown_user.as_ptr(),
            unknown_user.len() as u32,
            password.as_ptr(),
            password.len() as u32,
            &mut uid,
            &mut gid,
        )
    };
    if unknown_ok {
        code |= 8; // must NOT succeed for a username that was never added
    }

    // Persistence round trip: serialize, wipe the in-memory database,
    // reload from the serialized bytes, confirm authentication still
    // works identically - proving USERS.CFG save/load itself is
    // correct, not just the in-memory logic above.
    let mut buf = [0u8; RECORD_SIZE * MAX_USERS];
    let written = unsafe { rust_users_save(buf.as_mut_ptr(), buf.len() as u32) };
    if written != buf.len() as i32 {
        code |= 16;
    }

    unsafe {
        *users() = [EMPTY_USER; MAX_USERS];
        USER_COUNT = 0;
    }

    let loaded = unsafe { rust_users_load(buf.as_ptr(), buf.len() as u32) };
    let mut uid2 = 0u32;
    let mut gid2 = 0u32;
    let after_reload_ok = loaded
        && unsafe {
            rust_users_authenticate(
                username.as_ptr(),
                username.len() as u32,
                password.as_ptr(),
                password.len() as u32,
                &mut uid2,
                &mut gid2,
            )
        };
    if !after_reload_ok || uid2 != 42 || gid2 != 43 {
        code |= 32;
    }

    // Phase 49: lockout verification - a separate account from
    // "selftest" above, specifically so this segment's own repeated
    // wrong-password attempts can't interact with (or be confused
    // with) the earlier, single wrong-password check. Exactly
    // LOCKOUT_THRESHOLD consecutive wrong attempts, then confirms the
    // *correct* password is still rejected - proving this isn't just
    // "wrong passwords keep failing" (which would be true regardless
    // of any lockout logic) but that the account is genuinely locked.
    let lock_username = b"locktest";
    let lock_password = b"lock-correct-password";
    let lock_wrong = b"lock-wrong-password";
    unsafe {
        rust_users_add(
            lock_username.as_ptr(),
            lock_username.len() as u32,
            99,
            99,
            lock_password.as_ptr(),
            lock_password.len() as u32,
            0x2222_2222,
        );
    }

    let mut lock_uid = 0u32;
    let mut lock_gid = 0u32;
    for _ in 0..LOCKOUT_THRESHOLD {
        unsafe {
            rust_users_authenticate(
                lock_username.as_ptr(),
                lock_username.len() as u32,
                lock_wrong.as_ptr(),
                lock_wrong.len() as u32,
                &mut lock_uid,
                &mut lock_gid,
            );
        }
    }
    let correct_after_lockout = unsafe {
        rust_users_authenticate(
            lock_username.as_ptr(),
            lock_username.len() as u32,
            lock_password.as_ptr(),
            lock_password.len() as u32,
            &mut lock_uid,
            &mut lock_gid,
        )
    };
    if correct_after_lockout {
        code |= 64; // the correct password must still be rejected -
                    // the account is locked, not just "recently wrong"
    }

    // Phase 50: the actual, observable point of salting - two
    // different accounts given the exact same password but different
    // salt seeds must end up with different stored hashes. Without
    // this check, a self-test could pass in full (every add/
    // authenticate call behaving correctly) even if salting were
    // silently a no-op and every account secretly shared one fixed
    // salt - authentication would still work identically either way,
    // so only directly inspecting the stored hashes themselves proves
    // salting is actually happening.
    let salt_username_a = b"salttest_a";
    let salt_username_b = b"salttest_b";
    let shared_password = b"same-password-both-accounts";
    unsafe {
        rust_users_add(
            salt_username_a.as_ptr(),
            salt_username_a.len() as u32,
            1,
            1,
            shared_password.as_ptr(),
            shared_password.len() as u32,
            0x3333_3333,
        );
        rust_users_add(
            salt_username_b.as_ptr(),
            salt_username_b.len() as u32,
            2,
            2,
            shared_password.as_ptr(),
            shared_password.len() as u32,
            0x4444_4444,
        );
    }
    let (hash_a, hash_b, salt_a, salt_b) = unsafe {
        let table = users();
        let record_a = table
            .iter()
            .find(|r| username_matches(r, salt_username_a))
            .unwrap();
        let hash_a = record_a.password_hash;
        let salt_a = record_a.salt;
        let record_b = table
            .iter()
            .find(|r| username_matches(r, salt_username_b))
            .unwrap();
        (hash_a, record_b.password_hash, salt_a, record_b.salt)
    };
    if salt_a == salt_b || hash_a == hash_b {
        code |= 128; // same password, different salts, MUST produce
                     // different salts and different stored hashes
    }

    // Phase 51: sudo-check verification - directly exercises
    // rust_users_sudo_check() itself, not just inferred from a later
    // syscall/shell test. Two accounts: one in the admin group
    // (gid == 0) and one genuinely not, both with real, different
    // passwords - proving three things separately, not just "it
    // returns something": a wrong password for the admin account is
    // rejected; the admin account's own correct password is accepted;
    // and - the check this function actually exists to make, not
    // just password verification alone - a *correct* password for a
    // non-admin account is still rejected, because authentication
    // alone is not authorization.
    let sudo_admin_username = b"sudoadmintest";
    let sudo_admin_password = b"sudo-admin-correct-password";
    let sudo_regular_username = b"sudoregulartest";
    let sudo_regular_password = b"sudo-regular-correct-password";
    unsafe {
        rust_users_add(
            sudo_admin_username.as_ptr(),
            sudo_admin_username.len() as u32,
            801,
            0, // admin group
            sudo_admin_password.as_ptr(),
            sudo_admin_password.len() as u32,
            0x5555_5555,
        );
        rust_users_add(
            sudo_regular_username.as_ptr(),
            sudo_regular_username.len() as u32,
            802,
            802, // NOT the admin group
            sudo_regular_password.as_ptr(),
            sudo_regular_password.len() as u32,
            0x6666_6666,
        );
    }

    let admin_wrong_password_ok = unsafe {
        rust_users_sudo_check(801, b"totally-wrong".as_ptr(), 13)
    };
    if admin_wrong_password_ok {
        code |= 256; // must NOT succeed with the wrong password, even
                     // for an admin-group account
    }

    let admin_correct_password_ok = unsafe {
        rust_users_sudo_check(
            801,
            sudo_admin_password.as_ptr(),
            sudo_admin_password.len() as u32,
        )
    };
    if !admin_correct_password_ok {
        code |= 512; // the admin-group account's own correct
                     // password MUST succeed
    }

    let regular_correct_password_ok = unsafe {
        rust_users_sudo_check(
            802,
            sudo_regular_password.as_ptr(),
            sudo_regular_password.len() as u32,
        )
    };
    if regular_correct_password_ok {
        code |= 1024; // the actual point of this function: a
                      // genuinely correct password for a non-admin
                      // account must still be rejected - authenticated
                      // is not the same as authorized
    }

    // Leave the database empty for whatever real code runs after this
    // self-test - a leftover "selftest" account with a known password
    // would itself be a real, if minor, security smell to ship.
    unsafe {
        *users() = [EMPTY_USER; MAX_USERS];
        USER_COUNT = 0;
    }

    code
}
