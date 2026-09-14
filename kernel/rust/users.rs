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
//! Password hashing note, stated as directly as the FAT32 limitation
//! above: this uses FNV-1a, a fast, well-known, NON-cryptographic
//! hash - explicitly not a secure password hash (no salt, fast rather
//! than deliberately slow, no resistance to brute-forcing if the
//! stored hash were ever leaked). Chosen deliberately as an honest,
//! minimal placeholder that proves the authentication *flow* works
//! correctly, not as a production-grade credential store - a real
//! password hash (bcrypt/scrypt/argon2, salted) is real, separate
//! follow-up work, not attempted here.

const MAX_USERS: usize = 8;
const USERNAME_MAX: usize = 32;

#[derive(Clone, Copy)]
struct UserRecord {
    in_use: bool,
    username: [u8; USERNAME_MAX],
    username_len: u8,
    uid: u32,
    gid: u32,
    password_hash: u32,
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
            password_hash: 0,
            failed_attempts: 0,
        }
    }
}

const EMPTY_USER: UserRecord = UserRecord::empty();
static mut USERS: [UserRecord; MAX_USERS] = [EMPTY_USER; MAX_USERS];
static mut USER_COUNT: usize = 0;

/// FNV-1a, 32-bit - see this module's own header comment on why this,
/// specifically, is not a secure password hash and isn't meant to be
/// one.
fn fnv1a_hash(data: &[u8]) -> u32 {
    let mut hash: u32 = 0x811C9DC5;
    for &byte in data {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
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

    for slot in table.iter_mut() {
        if !slot.in_use {
            slot.in_use = true;
            slot.username = [0u8; USERNAME_MAX];
            slot.username[..name.len()].copy_from_slice(name);
            slot.username_len = name.len() as u8;
            slot.uid = uid;
            slot.gid = gid;
            slot.password_hash = fnv1a_hash(password);
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
    let attempt_hash = fnv1a_hash(password);

    for record in users().iter_mut() {
        if username_matches(record, name) {
            if record.failed_attempts >= LOCKOUT_THRESHOLD {
                return false; // locked out - not even the correct
                              // password is accepted until a reboot
                              // (see failed_attempts' own doc comment)
            }
            if record.password_hash == attempt_hash {
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

#[no_mangle]
pub extern "C" fn rust_users_count() -> u32 {
    unsafe { USER_COUNT as u32 }
}

/// On-disk record layout (for USERS.CFG, via kernel/fs/vfs.c's own
/// read/write, the same "kernel reads/writes a plain file, Rust
/// (de)serializes it" pattern SYSTEM.CFG already established):
/// username_len(1) + username(32, zero-padded) + uid(4) + gid(4) +
/// password_hash(4) = 45 bytes per record, MAX_USERS records back to
/// back, no header/checksum - deliberately as simple as SYSTEM.CFG's
/// own fixed-layout format, not a new, more elaborate scheme.
const RECORD_SIZE: usize = 1 + USERNAME_MAX + 4 + 4 + 4;

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
        buf[off..off + 4].copy_from_slice(&record.password_hash.to_le_bytes());
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
        record.password_hash =
            u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
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
/// account's genuinely correct password is rejected.
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

    // Leave the database empty for whatever real code runs after this
    // self-test - a leftover "selftest" account with a known password
    // would itself be a real, if minor, security smell to ship.
    unsafe {
        *users() = [EMPTY_USER; MAX_USERS];
        USER_COUNT = 0;
    }

    code
}
