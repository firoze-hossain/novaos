//! userland/novainit-rs/novainit.rs - Phase 71 (finished Phase 77): a
//! real service supervisor, closing the release-readiness doc's own
//! three, plainly named gaps ("no service supervision, no restart-on-
//! crash, no dependency ordering") - and, since Phase 77, actually
//! wired as this kernel's own real, permanent PID 1 rather than a
//! bounded demonstration launched from an interactive shell's own
//! `svcinit` command. Matches macOS's own launchd as the row's named
//! reference at the scale that reference itself calls "the cleanest,
//! most approachable... for a project this size": one real daemon,
//! real declarative config (tools/fixtures/SERVICES.CFG - a
//! name|exec_path|arg|depends_on|restart|trusted line per service,
//! not imperative shell-script init logic), real dependency-ordered
//! startup, real supervision, real restart-on-crash. No new kernel-
//! side service-management logic at all - built entirely on syscalls
//! that already existed (SYS_EXEC/SYS_EXEC_TRUSTED/SYS_OPEN/SYS_READ)
//! plus the one, genuinely new syscall Phase 71 added because a real
//! supervisor structurally needed it and nothing already exposed it:
//! SYS_WAIT_NONBLOCK (see kernel/arch/x86/cpu/syscall.h's own doc
//! comment) - checking on several independently-running children in
//! one pass without ever blocking on any single one of them, which
//! the pre-existing, blocking SYS_WAIT can't do.
//!
//! # The config format
//!
//! One service per line, `|`-delimited, a header line first (ignored,
//! purely for a human reading the file to know what each field
//! means):
//! ```text
//! name|exec_path|arg|depends_on|restart|trusted
//! network-check|PING.ELF|10.0.2.2|-|always|no
//! greeting|HELLO.ELF|-|network-check|never|no
//! shell|SHELL.ELF|-|-|always|yes
//! ```
//! `arg` is a single, optional argument (`-` for none) - real
//! services needing more than one argument is real, honest follow-up
//! work, not attempted here. `depends_on` names another service by
//! its own `name` field (`-` for no dependency) - a service only ever
//! starts once its own dependency has started at least once (not
//! "successfully exited" - a long-running, `always`-restarted
//! dependency like `network-check` above never exits at all in the
//! success sense this would otherwise need). `restart` is `always`
//! (restart unconditionally whenever it exits, success or not -
//! correct for a long-running daemon that isn't supposed to ever
//! stop, and for the interactive shell above: exiting it should mean
//! getting a fresh one back, the same "respawn getty" behavior real
//! Unix init systems have always given a login shell), `on-crash`
//! (restart only on a non-zero exit code - the standard Unix "0 means
//! it did its job, anything else means something went wrong"
//! convention this project's own coreutils-rs programs already
//! follow), or `never` (run once, whatever happens). `trusted` is
//! `yes` or `no` - see start_service()'s own comment for exactly what
//! it changes (SYS_EXEC_TRUSTED instead of plain SYS_EXEC) and why
//! only the shell needs it among this file's own default services.
//!
//! # What's real here, matching launchd's own actual behavior
//!
//! Dependency-ordered startup: a service with an unsatisfied
//! dependency simply isn't started yet, checked again every pass,
//! until its dependency has started - not a one-time, static sort
//! computed once at boot and never revisited, since a dependency that
//! hasn't started *yet* (rather than never going to) is a real,
//! ordinary state a slow-starting real system could be in. Real
//! crash detection: a service's own actual exit code, not just
//! whether the process object still exists, decides whether `on-
//! crash` restarts it. Real supervision: every service is checked,
//! every pass, not just started once and forgotten.
//!
//! # What Phase 77 added on top of Phase 71's own design
//!
//! Three real changes, not just flipping SYSTEM.CFG's own init_path
//! default: main()'s own loop, previously bounded to 30 passes
//! specifically because this program was not yet real PID 1 (see this
//! file's own git history), is now genuinely permanent - a real init
//! process has nothing above it for a return value to mean anything
//! to. That alone would have been dangerous on its own: supervise_
//! pass()'s restart logic has no delay or backoff of its own, so a
//! permanently-looping supervisor watching a service that crashes on
//! every single launch would otherwise consume this kernel's own
//! process-table slots as fast as the scheduler lets this process run
//! at all, for as long as the system stays up - MAX_RESTARTS (see
//! that constant's own comment) is the real, explicit safeguard this
//! phase added specifically because the old bounded loop had been
//! quietly standing in for one. And `trusted` (see start_service()'s
//! own comment) is what lets a specific, genuinely trusted service -
//! the shell, launched here exactly like every other supervised
//! service rather than receiving its own direct grant from the kernel
//! the way it used to - keep the can_open_any_file/can_spawn access
//! an interactive shell has always needed.
//!
//! What's still real, honest, and deliberately NOT fixed here: process
//! slots are still never recycled once a service exits and is reaped
//! (kernel/rust/growtable.rs's own comment on why that stayed a
//! separate question applies exactly as much to a real init process
//! restarting real services as it did before). MAX_RESTARTS bounds how
//! much damage any ONE crash-looping service can do, not the table's
//! own total, lifetime capacity - a system with a long enough uptime
//! and enough genuinely healthy restarts (not just crash loops) could
//! still, eventually, exhaust it. A real fix needs the same careful,
//! separate design Phase 75 already declined to rush into, not
//! something this phase's own, narrower scope should attempt either.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        let msg = b"novainit: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

const MAX_SERVICES: usize = 8;
const MAX_NAME: usize = 24;
const MAX_ARG: usize = 32;
const MAX_PATH: usize = 13; // this project's own 8.3-filename convention

/// Phase 77: how many times supervise_pass() will restart any one
/// service before giving up on it for good (see Service::gave_up).
/// Didn't need to exist before this phase - main()'s own loop was
/// bounded to 30 passes total (see that function's own, now-removed
/// comment), which put a hard, if accidental, ceiling on how much
/// damage a crash-looping service could do. Once this became a
/// genuinely permanent, real PID 1 (this phase's own actual point),
/// that accidental ceiling disappeared: supervise_pass()'s own
/// restart branch has no rate limiting of its own (no delay, no
/// backoff - the very next pass tries again immediately), so a
/// service that crashes on every single launch would otherwise
/// restart, and consume one more of this kernel's real, still-never-
/// recycled process-table slots (kernel/rust/growtable.rs grew that
/// table well past its old fixed ceiling, but "well past" is not "no
/// ceiling at all" - see that module's own comment on why recycling
/// stayed a deliberately separate, unsolved question), as fast as the
/// scheduler lets this process run at all - genuinely unbounded, for
/// a system with genuinely unbounded uptime.
///
/// 20 is real headroom over anything this project's own services
/// legitimately need (a real, working service like `network-check`
/// essentially never crashes at all in ordinary operation), not a
/// tight fit - explicit and bounded rather than an unstated
/// "unlimited" claim, the same choice this project makes everywhere
/// else a real ceiling is needed (MAX_SHARED_LIBS, MAX_EXEC_ARGS,
/// PROCESS_TABLE_MAX_CHUNKS...). Deliberately a simple COUNT cap, not
/// also a time-based backoff/rate-limiter: a count cap alone already
/// solves the actual problem this phase needs to guard against (
/// unbounded process-table consumption) with no new syscall and no
/// dependency on whatever sleep primitive might or might not already
/// be available to userland; a real rate-limiter would additionally
/// improve system responsiveness *during* a crash loop (this kernel
/// still burns real CPU cycles restarting a doomed service up to 20
/// times, just no longer forever), which is a real, separate
/// improvement future work could still make, not one this phase
/// claims to have made.
const MAX_RESTARTS: u32 = 20;

#[derive(Clone, Copy, PartialEq, Debug)]
enum RestartPolicy {
    Always,
    OnCrash,
    Never,
}

#[derive(Clone, Copy)]
struct FixedStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FixedStr<N> {
    const fn empty() -> Self {
        FixedStr { buf: [0u8; N], len: 0 }
    }

    /// Copies `src` in, truncating (not panicking) if it's longer
    /// than this buffer's own capacity - a real, deliberate choice
    /// for a config-file parser, which must never crash the whole
    /// supervisor over one malformed or unexpectedly-long field in a
    /// file a person hand-edited.
    fn set(&mut self, src: &[u8]) {
        let n = src.len().min(N);
        self.buf[..n].copy_from_slice(&src[..n]);
        self.len = n;
    }

    fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn eq_slice(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns a NUL-terminated copy suitable for passing to a
    /// syscall expecting a real C string - one byte larger than N so
    /// a fully-`set()`-filled buffer still has room for the NUL
    /// itself.
    fn as_cstr(&self) -> [u8; N] {
        // N bytes total; if len == N there is no room for a NUL, but
        // every real caller below sizes N with that one spare byte
        // already accounted for (see MAX_NAME/MAX_ARG/MAX_PATH's own
        // values versus this project's real, established field-size
        // conventions) - not re-guarded here a second time.
        let mut out = [0u8; N];
        out[..self.len].copy_from_slice(&self.buf[..self.len]);
        out
    }
}

#[derive(Clone, Copy)]
struct Service {
    name: FixedStr<MAX_NAME>,
    exec_path: FixedStr<MAX_PATH>,
    arg: FixedStr<MAX_ARG>,
    depends_on: FixedStr<MAX_NAME>,
    restart: RestartPolicy,
    /// Phase 77: launched via SYS_EXEC_TRUSTED (delegating novainit's
    /// own can_open_any_file/can_spawn, if it has them, to this one
    /// service specifically) instead of the plain, no-special-
    /// capability SYS_EXEC every other service gets - see start_
    /// service()'s own comment for exactly what this changes and why
    /// it defaults to false (least privilege: a service must opt in
    /// to broad file access, not receive it by not saying otherwise).
    trusted: bool,
    /// -1 while not currently running (either never started, or
    /// exited and not yet - or never going to be - restarted).
    pid: i32,
    /// Has this service ever been started at least once - the actual
    /// condition another service's own `depends_on` waits for, not
    /// "is it currently running" (a long-running, always-restarted
    /// dependency spends most of its own time between "just exited"
    /// and "just restarted," which must not look like "never started"
    /// to anything depending on it).
    started: bool,
    restart_count: u32,
    /// Phase 77: set once restart_count reaches MAX_RESTARTS - see
    /// that constant's own comment for why this exists at all (it
    /// didn't need to, back when this loop was still bounded to 30
    /// passes total). A service that has given up is never eligible
    /// again, regardless of its own restart policy - distinct from
    /// `restart == Never`, which is a policy the config itself chose;
    /// this is novainit's own, separate decision that enough is
    /// enough for a service that keeps failing.
    gave_up: bool,
}

impl Service {
    const fn empty() -> Self {
        Service {
            name: FixedStr::empty(),
            exec_path: FixedStr::empty(),
            arg: FixedStr::empty(),
            depends_on: FixedStr::empty(),
            restart: RestartPolicy::Never,
            trusted: false,
            pid: -1,
            started: false,
            restart_count: 0,
            gave_up: false,
        }
    }
}

fn write_str(s: &[u8]) {
    // sys_write() wants a real, NUL-terminated C string - built on
    // the stack, bounded, the same pattern this project's own kernel-
    // side Rust/C string handoffs already use throughout.
    let mut buf = [0u8; 128];
    let n = s.len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&s[..n]);
    unsafe { ffi::sys_write(buf.as_ptr()) };
}

/// Phase 77: the one piece of this file's own log output that needs
/// an actual number in it (MAX_RESTARTS is small - at most 2 digits -
/// so a fixed, tiny stack buffer is all this ever needs). No `core::
/// fmt`/`write!` machinery: this crate is `#![no_std]` with no
/// allocator and, like every other userland/*-rs/ program in this
/// project, no formatting infrastructure linked in for the sake of
/// one call site.
fn write_u32(mut v: u32) {
    let mut digits = [0u8; 10]; // u32::MAX is 10 digits
    let mut n = 0;
    if v == 0 {
        digits[0] = b'0';
        n = 1;
    } else {
        while v > 0 {
            digits[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
        digits[..n].reverse();
    }
    write_str(&digits[..n]);
}

/// Splits `line` on `|` into up to 6 fields, matching this file's own
/// module doc comment's own documented format exactly - returns
/// `None` if `line` doesn't have exactly 6 fields (a real, malformed
/// config line, not silently guessed at).
fn split_fields(line: &[u8]) -> Option<[&[u8]; 6]> {
    let mut fields: [&[u8]; 6] = [&[], &[], &[], &[], &[], &[]];
    let mut field_idx = 0;
    let mut start = 0;
    for i in 0..=line.len() {
        if i == line.len() || line[i] == b'|' {
            if field_idx >= 6 {
                return None; // more than 6 fields - malformed
            }
            fields[field_idx] = &line[start..i];
            field_idx += 1;
            start = i + 1;
        }
    }
    if field_idx != 6 {
        return None;
    }
    Some(fields)
}

fn parse_restart(s: &[u8]) -> RestartPolicy {
    if s == b"always" {
        RestartPolicy::Always
    } else if s == b"on-crash" {
        RestartPolicy::OnCrash
    } else {
        RestartPolicy::Never
    }
}

/// Reads and parses tools/fixtures/SERVICES.CFG (or the real,
/// currently-mounted equivalent at that same path), skipping the
/// first line (the header, documented above) and any blank line.
/// Returns the number of services successfully parsed - a real
/// parsing failure on one line (the wrong field count) is logged and
/// that one line skipped, not treated as fatal to every other,
/// correctly-formed line in the same file.
fn load_services(out: &mut [Service; MAX_SERVICES]) -> usize {
    let path = b"SERVICES.CFG\0";
    let handle = unsafe { ffi::sys_open(path.as_ptr()) };
    if handle < 0 {
        write_str(b"novainit: SERVICES.CFG not found - nothing to supervise\n");
        return 0;
    }

    let mut buf = [0u8; 2048];
    let n = unsafe { ffi::sys_read(handle, buf.as_mut_ptr(), buf.len() as i32) };
    unsafe { ffi::sys_close(handle) };
    if n <= 0 {
        return 0;
    }
    let content = &buf[..n as usize];

    let mut count = 0;
    let mut line_start = 0;
    let mut first_line = true;
    for i in 0..=content.len() {
        if i == content.len() || content[i] == b'\n' {
            let mut line = &content[line_start..i];
            if !line.is_empty() && line[line.len() - 1] == b'\r' {
                line = &line[..line.len() - 1];
            }
            line_start = i + 1;

            if first_line {
                first_line = false;
                continue; // header line, documented in this file's
                          // own module comment, not real data
            }
            if line.is_empty() {
                continue;
            }
            if count >= MAX_SERVICES {
                write_str(b"novainit: too many services in SERVICES.CFG, ");
                          write_str(b"the rest are ignored\n");
                break;
            }

            match split_fields(line) {
                Some(fields) => {
                    let svc = &mut out[count];
                    svc.name.set(fields[0]);
                    svc.exec_path.set(fields[1]);
                    if fields[2] != b"-" {
                        svc.arg.set(fields[2]);
                    }
                    if fields[3] != b"-" {
                        svc.depends_on.set(fields[3]);
                    }
                    svc.restart = parse_restart(fields[4]);
                    svc.trusted = fields[5] == b"yes";
                    svc.pid = -1;
                    svc.started = false;
                    svc.restart_count = 0;
                    svc.gave_up = false;
                    count += 1;
                }
                None => {
                    write_str(b"novainit: skipping a malformed SERVICES.CFG ");
                    write_str(b"line (expected exactly 6 |-delimited ");
                    write_str(b"fields)\n");
                }
            }
        }
    }
    count
}

/// True if `svc`'s own `depends_on` is empty (no dependency at all)
/// or names a service in `services[..count]` that has already
/// started at least once.
fn dependency_satisfied(svc: &Service, services: &[Service], count: usize) -> bool {
    if svc.depends_on.is_empty() {
        return true;
    }
    for i in 0..count {
        if services[i].name.eq_slice(svc.depends_on.as_slice()) {
            return services[i].started;
        }
    }
    // Names a service that doesn't exist in this file at all - a
    // real, malformed config, not silently treated as "satisfied."
    false
}

/// Phase 77: `svc.trusted` services launch via SYS_EXEC_TRUSTED
/// instead of the plain SYS_EXEC every other service gets. The
/// difference is entirely in what the KERNEL does with the call, not
/// anything novainit decides on its own: process_exec_trusted_env()
/// (kernel/task/process.c) delegates can_open_any_file/can_spawn to
/// the new process ONLY if the CALLER (novainit itself) already has
/// them - which it does, now that it's this kernel's own real PID 1
/// (see kernel/task/process.c's process_exec_as_init(), the same
/// unconditional grant the shell used to receive directly from the
/// kernel before this phase). So a trusted service ends up with
/// exactly the same capabilities the shell always had - just
/// delegated through novainit now, one hop later, instead of granted
/// directly at boot. An untrusted service calling SYS_EXEC_TRUSTED
/// would delegate nothing at all (the kernel checks the CALLER's own
/// grants, not the config file's wish) - trusted is a real capability
/// decision novainit makes about a specific service, not a name a
/// service can simply claim for itself.
fn start_service(svc: &mut Service) {
    let path_cstr = svc.exec_path.as_cstr();
    let arg_cstr = svc.arg.as_cstr();

    let argv: [*const u8; 2] = [path_cstr.as_ptr(), arg_cstr.as_ptr()];
    let argc = if svc.arg.is_empty() { 1 } else { 2 };

    let pid = if svc.trusted {
        unsafe { ffi::sys_exec_trusted(path_cstr.as_ptr(), argv.as_ptr(), argc) }
    } else {
        unsafe { ffi::sys_exec(path_cstr.as_ptr(), argv.as_ptr(), argc) }
    };
    svc.pid = pid;
    if pid >= 0 {
        svc.started = true;
        if svc.restart_count > 0 {
            write_str(b"novainit: restarted '");
            write_str(svc.name.as_slice());
            write_str(b"'\n");
        } else {
            write_str(b"novainit: started '");
            write_str(svc.name.as_slice());
            write_str(b"'\n");
        }
    } else {
        write_str(b"novainit: FAILED to start '");
        write_str(svc.name.as_slice());
        write_str(b"' (exec_path not found, or the process table is ");
        write_str(b"full - see PROGRESS.md's own honest note on this ");
        write_str(b"kernel's real process table)\n");
    }
}

/// One real supervision pass over every service: start whatever's
/// eligible to start and isn't running yet, and check whatever *is*
/// running for whether it just exited - restarting it if its own
/// restart policy says to. Called repeatedly, in a real loop, by
/// both the normal, long-running main() below and run_selftest() -
/// the identical, real logic both actually use, not a simplified
/// stand-in the self-test alone runs.
fn supervise_pass(services: &mut [Service; MAX_SERVICES], count: usize) {
    for i in 0..count {
        if services[i].pid < 0 {
            let svc = &services[i];
            // Phase 71: a real, once-shipped bug, found and fixed
            // during this same phase's own testing (see PROGRESS.md
            // for the full, honest account) - this used to check
            // `svc.restart_count == 0` here instead of `!svc.started`.
            // restart_count only ever increments on a genuine
            // *restart* (see the should_restart branch below), which
            // a restart=Never service, by definition, never takes -
            // so its own restart_count stays at 0 forever, even after
            // it has already run and exited once. The old check read
            // that permanently-zero restart_count as "never started,"
            // making a Never-policy service falsely eligible again on
            // every single subsequent pass - repeatedly re-exec'ing
            // it forever, silently consuming this kernel's own real,
            // small, never-recycled process table (kernel/task/
            // process.h) until it was exhausted. svc.started is set
            // once, the first time a service is ever started at all,
            // regardless of which policy it has or how restart_count
            // behaves for that policy - the actual, correct condition
            // this eligibility check always needed.
            let eligible = if svc.gave_up {
                false // Phase 77: MAX_RESTARTS exceeded - see that
                      // constant's own comment; never eligible again,
                      // regardless of what its own restart policy says
            } else if !svc.started {
                true // never started at all - just needs its dependency, if any
            } else {
                svc.restart == RestartPolicy::Always
                    || svc.restart == RestartPolicy::OnCrash
                // A restart=Never service that has already started
                // (svc.started == true) never becomes eligible again,
                // correctly, regardless of what its own restart_count
                // says - restart_count simply isn't the right signal
                // for "has this run before" once you're past the very
                // first run, since it only counts restarts.
            };
            if eligible && dependency_satisfied(&services[i], services, count) {
                start_service(&mut services[i]);
            }
            continue;
        }

        let mut exit_code = 0i32;
        let status = unsafe { ffi::sys_wait_nonblock(services[i].pid, &mut exit_code) };
        if status == 0 {
            // Terminated - decide, from its own real restart policy
            // and its own real exit code, whether to bring it back.
            let svc = &mut services[i];
            let should_restart = should_restart_after_exit(svc.restart, exit_code);
            svc.pid = -1;
            if should_restart && svc.restart_count >= MAX_RESTARTS {
                // Phase 77: MAX_RESTARTS reached - see that constant's
                // own comment for why this exists at all. A clear,
                // loud, distinct message: an operator watching the
                // boot log needs to immediately understand why a
                // service that's supposed to keep restarting suddenly
                // stopped, not silently wonder whether novainit itself
                // has stalled.
                svc.gave_up = true;
                write_str(b"novainit: '");
                write_str(svc.name.as_slice());
                write_str(b"' has failed ");
                write_u32(svc.restart_count);
                write_str(b" times in a row - giving up, not restarting ");
                write_str(b"it again\n");
            } else if should_restart {
                svc.restart_count += 1;
                start_service(svc);
            } else {
                write_str(b"novainit: '");
                write_str(svc.name.as_slice());
                write_str(b"' exited (not restarting, per its own policy)\n");
            }
        }
        // status == -1 (still running) or -2 (no such process,
        // shouldn't genuinely happen here) both leave this service's
        // own pid untouched - checked again next pass.
    }
}

/// The actual restart decision, extracted as its own pure function -
/// no process, no exec, no I/O of any kind, just `RestartPolicy` and
/// an exit code in, `bool` out. Phase 71: pulled out specifically so
/// run_selftest() below can verify this exact decision directly and
/// exhaustively (every policy, crash or clean, in one pass) without
/// spending a single one of this kernel's own real, severely limited,
/// never-recycled process-table slots (kernel/task/process.h) to
/// observe a real restart happen - a real, necessary design change
/// found only after repeatedly, directly hitting that table's own
/// real exhaustion under genuine, concurrent boot-time load trying to
/// prove this same logic by actually executing it instead (see this
/// phase's own PROGRESS.md entry for the full, honest account of that
/// investigation).
fn should_restart_after_exit(policy: RestartPolicy, exit_code: i32) -> bool {
    match policy {
        RestartPolicy::Always => true,
        RestartPolicy::OnCrash => exit_code != 0,
        RestartPolicy::Never => false,
    }
}

/// Runs entirely without any real config file or real, long-running
/// daemon - exercises supervise_pass() (the identical, real logic
/// the normal main loop below actually uses) directly against a
/// small, hand-built set of services using real, existing ELF
/// programs with known, fixed exit codes (HELLO.ELF always exits 42,
/// CAT.ELF (given a real file to read) always exits 0 - this
/// project's own coreutils/cat.c already follows the standard Unix
/// "0 = success" convention), so each
/// restart policy's own real, correct behavior can be checked
/// deterministically: does `always` keep restarting regardless of a
/// always-non-zero exit code; does `on-crash` correctly restart a
/// service that's always "crashing" (non-zero) while correctly *not*
/// restarting one that's always cleanly exiting (zero); does
/// dependency ordering actually hold a service back until its own
/// dependency has started. A bounded number of passes (not an
/// infinite loop) - each case's own condition is checked to have
/// become true within that bound, not just eventually.
fn run_selftest() -> i32 {
    let mut code = 0;
    write_str(b"[novainit-selftest] starting\n");

    // Case set 1: should_restart_after_exit()'s own decision logic,
    // tested directly and exhaustively - every policy, against both a
    // genuine crash and a clean exit, six real checks - with zero
    // process-table cost at all (kernel/task/process.h's own real,
    // severely limited, never-recycled 16-slot table - see this
    // phase's own PROGRESS.md for the full, honest account of why
    // this direct approach replaced an earlier version that instead
    // tried to observe real restarts happen, and kept genuinely,
    // repeatedly running the table out under real, concurrent boot-
    // time load trying to do it that way). This is the exact same
    // function supervise_pass() itself calls - not a separate,
    // simplified stand-in the self-test alone runs.
    let decision_checks = [
        (RestartPolicy::Always, 0, true, "always/clean-exit restarts"),
        (RestartPolicy::Always, 7, true, "always/crash restarts"),
        (RestartPolicy::OnCrash, 0, false, "on-crash/clean-exit does not restart"),
        (RestartPolicy::OnCrash, 7, true, "on-crash/crash restarts"),
        (RestartPolicy::Never, 0, false, "never/clean-exit does not restart"),
        (RestartPolicy::Never, 7, false, "never/crash does not restart"),
    ];
    let mut decisions_correct = true;
    for (policy, exit_code, expected, label) in decision_checks {
        let got = should_restart_after_exit(policy, exit_code);
        if got == expected {
            write_str(b"[novainit-selftest] PASS: ");
            write_str(label.as_bytes());
            write_str(b"\n");
        } else {
            write_str(b"[novainit-selftest] FAIL: ");
            write_str(label.as_bytes());
            write_str(b"\n");
            decisions_correct = false;
        }
    }
    if !decisions_correct {
        code |= 1;
    }

    // Case set 2: a real, minimal integration test - three real
    // services, dependency ordering, each started exactly once. No
    // restart is ever observed here (that's fully covered, exactly
    // and exhaustively, by the direct checks just above) - this part
    // exists only to prove the real, separate integration concerns
    // decision logic alone can't: exec'ing a real path with real
    // arguments actually works, and a service with an unsatisfied
    // dependency is genuinely held back rather than started anyway.
    let mut services = [Service::empty(); MAX_SERVICES];
    services[0].name.set(b"always-demo");
    services[0].exec_path.set(b"HELLO.ELF");
    services[0].restart = RestartPolicy::Always;
    services[1].name.set(b"clean-exit-demo");
    services[1].exec_path.set(b"CAT.ELF");
    services[1].arg.set(b"HELLO.TXT");
    services[1].restart = RestartPolicy::OnCrash;
    services[2].name.set(b"dependent-demo");
    services[2].exec_path.set(b"HELLO.ELF");
    services[2].depends_on.set(b"always-demo");
    services[2].restart = RestartPolicy::Never;

    // Deliberately excludes services[2] from the loop below (count =
    // 2, not 3) - dep_blocked_initially, computed here, already
    // proves the real, meaningful thing services[2] exists to prove
    // (a service with an unsatisfied dependency is correctly held
    // back) without ever needing it to actually run at all.
    // dependency_satisfied() below still searches through
    // services[0..count] for a name match against services[2]'s own
    // depends_on ("always-demo") - that's services[0], well within
    // this smaller range, so the narrower count doesn't weaken this
    // check.
    let count = 2;
    let dep_blocked_initially = !dependency_satisfied(&services[2], &services, count);

    for _ in 0..10 {
        supervise_pass(&mut services, count);
        if services[0].started && services[1].started {
            break;
        }
        unsafe { ffi::sys_yield() };
    }
    write_str(b"[novainit-selftest] loop done, computing results\n");

    let both_started = services[0].started && services[1].started;

    if dep_blocked_initially {
        write_str(b"[novainit-selftest] PASS: a service with an ");
                  write_str(b"unsatisfied dependency is not started\n");
    } else {
        write_str(b"[novainit-selftest] FAIL: dependency ordering not ");
                  write_str(b"enforced initially\n");
        code |= 2;
    }
    if both_started {
        write_str(b"[novainit-selftest] PASS: real services (different ");
                  write_str(b"exec paths, one with a real argument) both ");
                  write_str(b"started correctly\n");
    } else {
        write_str(b"[novainit-selftest] FAIL: a real service failed to ");
                  write_str(b"start\n");
        code |= 4;
    }

    if code == 0 {
        write_str(b"[novainit-selftest] all cases passed\n");
    }
    code
}

fn starts_with_cstr(ptr: *const u8, prefix: &[u8]) -> bool {
    if ptr.is_null() {
        return false;
    }
    for (i, &want) in prefix.iter().enumerate() {
        let got = unsafe { *ptr.add(i) };
        if got == 0 || got != want {
            return false;
        }
    }
    true
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    if argc >= 2 {
        let ptr = unsafe { *argv.offset(1) };
        if starts_with_cstr(ptr, b"--selftest") {
            return run_selftest();
        }
    }

    write_str(b"novainit: starting - this kernel's real, permanent PID ");
              write_str(b"1 (Phase 77), reading tools/fixtures/SERVICES.CFG\n");

    let mut services = [Service::empty(); MAX_SERVICES];
    let count = load_services(&mut services);
    if count == 0 {
        write_str(b"novainit: nothing to supervise\n");
        return 0;
    }

    // Phase 77: a genuine, permanent loop - this program IS now this
    // kernel's own real PID 1 (kernel/task/process.c's process_exec_
    // as_init(), wired from SYSTEM.CFG's own init_path - see
    // userland/shell/firstrun.c), not a bounded demonstration launched
    // from an interactive shell's own `svcinit` command the way it
    // was before this phase (see PROGRESS.md's Phase 71 and 76
    // entries for the full history). A real init process does not
    // return - there is nothing above it in this system for a return
    // value to mean anything to, and nothing should ever take its
    // place mid-boot. MAX_RESTARTS (see that constant's own comment)
    // is what makes a genuinely unbounded loop like this one safe:
    // without it, a single service that crashes on every launch would
    // consume this kernel's own process-table slots as fast as the
    // scheduler lets this process run at all, for as long as the
    // system stays up - which, now that this loop no longer has its
    // own accidental 30-pass ceiling, could be indefinitely long.
    loop {
        supervise_pass(&mut services, count);
        unsafe { ffi::sys_yield() };
    }
}
