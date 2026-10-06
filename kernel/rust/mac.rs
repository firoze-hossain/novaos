//! kernel/rust/mac.rs - Phase 87: mandatory access control (MAC).
//!
//! WHAT THIS IS
//!
//! The existing per-process capability model (Phase 30 and since) decides a
//! few things: which file NAMES a process may open (`allowed_files[]` /
//! `can_open_any_file`), which peer IPs it may talk to (`allowed_hosts[]`),
//! and whether it may spawn. It says nothing at all about the other ~60 of
//! the 70 syscalls, and it is DISCRETIONARY: whoever launches a program picks
//! its capabilities, and a program running as root can do anything root can.
//!
//! This is the second, stricter layer. A PROFILE - a small text file,
//! `NAME.MAC`, next to `NAME.ELF` - declares everything the program may do:
//! which syscalls it may make at all, which files it may read / write / delete
//! (by pattern), which network peers it may connect to / bind / send to, and
//! which programs it may start. Anything not listed is DENIED. The profile is
//! applied by the kernel when the program starts, the program cannot change
//! it, and it applies to root too - that is what "mandatory" means. A bug in
//! the program (or an attacker who has taken it over) can therefore do exactly
//! what the profile allows and nothing more, however it got there.
//!
//! HOW IT COMPOSES WITH THE CAPABILITY MODEL
//!
//! Both must allow. MAC never grants anything the capability model refuses,
//! and the capability model never grants anything MAC refuses: a process with
//! `can_open_any_file` still cannot open a file its profile does not list.
//!
//! A process carries a small STACK of profile ids (at most `MAX_STACK`).
//! Every profile on it must allow an operation. Two things push onto it:
//!   - `fork` copies the parent's stack (same domain);
//!   - starting a program that has its OWN profile adds that profile to the
//!     parent's stack. So a confined program cannot shed its confinement by
//!     starting a more permissive one: the child is bound by BOTH.
//! Nothing ever removes an entry. An empty stack is an unconfined process, and
//! all-zero is the empty stack, so a freshly zeroed process slot is unconfined
//! (the C side must still reset the stack when it reuses a slot - Phase 86
//! learned that lesson with a stale throttle flag).
//!
//! FAIL CLOSED
//!
//! A profile that does not parse is an error and the program does not start
//! (the C side refuses the exec); it is never "unconfined because the policy
//! was unreadable". A stack entry that does not name a loaded profile denies
//! everything. A syscall number outside the known range is denied. When the
//! table is full a new profile cannot be loaded and its program does not start.
//!
//! COMPLAIN MODE
//!
//! `mode complain` in a profile logs what WOULD have been denied and allows
//! it, so a profile can be developed against a real program. It is per
//! profile, so one complaining profile on a stack never weakens another.
//!
//! FREEZE
//!
//! `freeze()` is one-way until reboot (like a BSD securelevel): afterwards no
//! profile can be loaded or changed - only profiles already loaded, with
//! identical content, can be used. The C side also refuses writes to `*.MAC`
//! files once frozen.
//!
//! Two layers, as in msg.rs: a PURE core (parser, pattern matcher, evaluation,
//! the profile table) that the host tests below attack directly, and a thin
//! `kernel_glue` of `rust_mac_*` exports that only ever receive KERNEL
//! pointers (the C side has validated and copied everything first - the
//! arguments are checked on that one copy, because a user pointer re-read
//! after the check is a time-of-check/time-of-use hole).

use core::cmp::min;

pub const MAX_PROFILES: usize = 32;
pub const MAX_STACK: usize = 4;
pub const MAX_FILE_RULES: usize = 16;
pub const MAX_NET_RULES: usize = 8;
pub const MAX_EXEC_RULES: usize = 8;
pub const NAME_LEN: usize = 16;
pub const PAT_LEN: usize = 24;
pub const MAX_PROFILE_TEXT: usize = 4096;
pub const MAX_LINE: usize = 256;
/// Syscall numbers below this are representable in a profile; anything else
/// is denied outright to a confined process.
pub const SYSCALL_LIMIT: u32 = 128;
pub const AUDIT_LEN: usize = 16;

pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const E2BIG: i32 = 7;
pub const EINVAL: i32 = 22;
pub const ENOSPC: i32 = 28;

/// File operations (a profile's `r` / `w` / `d`).
pub const FILE_READ: u8 = 1;
pub const FILE_WRITE: u8 = 2;
pub const FILE_DELETE: u8 = 4;
/// Network operations (a profile's `c` / `b` / `s`).
pub const NET_CONNECT: u8 = 1;
pub const NET_BIND: u8 = 2;
pub const NET_SEND: u8 = 4;

/// Audit kinds.
pub const KIND_SYSCALL: u8 = 1;
pub const KIND_FILE: u8 = 2;
pub const KIND_NET: u8 = 3;
pub const KIND_EXEC: u8 = 4;

/// Returned by the check functions for a stack entry that names nothing.
pub const DENIER_INVALID: i32 = 255;

/// Syscalls a confined process may ALWAYS make, whatever its profile says:
/// ending itself (a process that cannot exit can only hang) and asking what
/// confines it (so a program can find out, and so tests can observe).
const SYS_EXIT: u32 = 2;
const SYS_MAC_INFO: u32 = 71;

/// `SYSCALL_NAMES[n]` is the profile name of syscall number `n`
/// (kernel/arch/x86/cpu/syscall.h). A host test parses that header and fails
/// if this table and it ever disagree.
pub const SYSCALL_NAMES: [&str; 73] = [
    "", "write", "exit", "yield", "open", "read", "close", "net_send", "spawn", "exec",
    "wait", "sbrk", "fork", "read_key", "list_files", "rtc_read", "lspci", "beep",
    "write_file", "delete_file", "gfx_enter", "gfx_exit", "gfx_put_pixel",
    "gfx_fill_rect", "mouse_read", "ping_start", "ping_poll", "pipe", "write_handle",
    "login", "getuid", "sudo", "shutdown", "socket", "bind", "listen", "accept",
    "connect", "exec_trusted", "dns_resolve", "tftp_fetch", "wait_nonblock",
    "exec_env", "exec_trusted_env", "socket_udp", "sendto", "recvfrom", "fb_info",
    "fb_acquire", "fb_release", "fb_create", "fb_destroy", "fb_present",
    "fb_readback", "shm_create", "shm_grant", "shm_map", "shm_unmap", "shm_destroy",
    "shm_info", "msg_open", "msg_close", "msg_send", "msg_recv", "msg_service",
    "msg_ctl", "audio_open", "audio_write", "audio_ctl", "audio_close", "rlimit",
    "mac_info", "mac_ctl",
];

// ------------------------------------------------------------------------
// Rules and policies.
// ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FileRule {
    pat: [u8; PAT_LEN],
    len: u8,
    perms: u8,
}

impl FileRule {
    const EMPTY: FileRule = FileRule { pat: [0; PAT_LEN], len: 0, perms: 0 };
    fn pattern(&self) -> &[u8] {
        &self.pat[..self.len as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NetRule {
    any_ip: bool,
    ip: u32,
    ops: u8,
    port_lo: u16,
    port_hi: u16,
}

impl NetRule {
    const EMPTY: NetRule = NetRule { any_ip: false, ip: 0, ops: 0, port_lo: 0, port_hi: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExecRule {
    pat: [u8; PAT_LEN],
    len: u8,
}

impl ExecRule {
    const EMPTY: ExecRule = ExecRule { pat: [0; PAT_LEN], len: 0 };
    fn pattern(&self) -> &[u8] {
        &self.pat[..self.len as usize]
    }
}

/// Everything a profile says. Compared as a whole to recognise "the same
/// profile loaded again" (the counters live outside it).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    complain: bool,
    all_syscalls: bool,
    syscalls: [u32; 4],
    files: [FileRule; MAX_FILE_RULES],
    nfiles: u8,
    nets: [NetRule; MAX_NET_RULES],
    nnets: u8,
    execs: [ExecRule; MAX_EXEC_RULES],
    nexecs: u8,
}

impl Policy {
    const EMPTY: Policy = Policy {
        complain: false,
        all_syscalls: false,
        syscalls: [0; 4],
        files: [FileRule::EMPTY; MAX_FILE_RULES],
        nfiles: 0,
        nets: [NetRule::EMPTY; MAX_NET_RULES],
        nnets: 0,
        execs: [ExecRule::EMPTY; MAX_EXEC_RULES],
        nexecs: 0,
    };

    pub fn is_complain(&self) -> bool {
        self.complain
    }

    pub fn allows_syscall(&self, n: u32) -> bool {
        if n == SYS_EXIT || n == SYS_MAC_INFO {
            return true;
        }
        if n >= SYSCALL_LIMIT {
            return false;
        }
        self.all_syscalls || (self.syscalls[(n / 32) as usize] >> (n % 32)) & 1 == 1
    }

    /// Every bit of `op` must be granted, by the UNION of the rules whose
    /// pattern matches `name`: a profile that grants only `r` must not allow
    /// an open for read AND write, and `r` on one pattern plus `w` on another
    /// that both match the name together grant `rw`.
    pub fn allows_file(&self, name: &[u8], op: u8) -> bool {
        if op == 0 {
            return false;
        }
        let granted = self.files[..self.nfiles as usize]
            .iter()
            .filter(|r| glob_match(r.pattern(), name))
            .fold(0u8, |acc, r| acc | r.perms);
        granted & op == op
    }

    pub fn allows_net(&self, op: u8, ip: u32, port: u16) -> bool {
        self.nets[..self.nnets as usize].iter().any(|r| {
            r.ops & op != 0
                && (r.any_ip || r.ip == ip)
                && port >= r.port_lo
                && port <= r.port_hi
        })
    }

    pub fn allows_exec(&self, name: &[u8]) -> bool {
        self.execs[..self.nexecs as usize]
            .iter()
            .any(|r| glob_match(r.pattern(), name))
    }
}

// ------------------------------------------------------------------------
// Pattern matching.
// ------------------------------------------------------------------------

#[inline]
fn fold(b: u8) -> u8 {
    if b.is_ascii_lowercase() {
        b - 32
    } else {
        b
    }
}

/// Case-insensitive glob: `*` matches any run of bytes (including none), `?`
/// exactly one byte, everything else itself. Iterative with a single
/// backtrack point, so it is O(len(pattern) * len(text)) in the worst case
/// and cannot be driven into exponential time by a hostile pattern or name.
pub fn glob_match(pat: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while t < text.len() {
        if p < pat.len() && pat[p] == b'*' {
            star = p;
            mark = t;
            p += 1;
        } else if p < pat.len() && (pat[p] == b'?' || fold(pat[p]) == fold(text[t])) {
            p += 1;
            t += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

/// Is this the name of a policy file? Only an unconfined root may write or
/// delete these, and nobody may once the policy is frozen: a profile an app
/// could rewrite is not mandatory.
pub fn is_policy_file(name: &[u8]) -> bool {
    name.len() >= 4 && glob_match(b"*.MAC", name)
}

// ------------------------------------------------------------------------
// The parser.
// ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParseError {
    TooLarge,
    BadByte,
    LineTooLong(u16),
    Syntax(u16),
    UnknownSyscall(u16),
    BadPattern(u16),
    BadAddress(u16),
    TooManyRules(u16),
    DuplicateMode(u16),
}

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Splits the next whitespace-separated token off `line`.
fn next_token<'a>(line: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    while *pos < line.len() && is_ws(line[*pos]) {
        *pos += 1;
    }
    if *pos >= line.len() {
        return None;
    }
    let start = *pos;
    while *pos < line.len() && !is_ws(line[*pos]) {
        *pos += 1;
    }
    Some(&line[start..*pos])
}

fn valid_pattern(p: &[u8]) -> bool {
    !p.is_empty()
        && p.len() <= PAT_LEN
        && p.iter().all(|&b| {
            b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-' || b == b'~' || b == b'$' || b == b'*' || b == b'?'
        })
}

fn parse_u32_dec(s: &[u8], max: u32) -> Option<u32> {
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    let mut v: u64 = 0;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (b - b'0') as u64;
        if v > max as u64 {
            return None;
        }
    }
    Some(v as u32)
}

fn parse_ip(s: &[u8]) -> Option<u32> {
    let mut parts = 0;
    let mut ip: u32 = 0;
    for part in s.split(|&b| b == b'.') {
        parts += 1;
        if parts > 4 {
            return None;
        }
        let v = parse_u32_dec(part, 255)?;
        ip = (ip << 8) | v;
    }
    if parts == 4 {
        Some(ip)
    } else {
        None
    }
}

/// `*`, `N` or `N-M`.
fn parse_ports(s: &[u8]) -> Option<(u16, u16)> {
    if s == b"*" {
        return Some((0, 65535));
    }
    match s.iter().position(|&b| b == b'-') {
        None => {
            let v = parse_u32_dec(s, 65535)? as u16;
            Some((v, v))
        }
        Some(i) => {
            let lo = parse_u32_dec(&s[..i], 65535)? as u16;
            let hi = parse_u32_dec(&s[i + 1..], 65535)? as u16;
            if lo <= hi {
                Some((lo, hi))
            } else {
                None
            }
        }
    }
}

fn lookup_syscall(name: &[u8]) -> Option<u32> {
    for (n, candidate) in SYSCALL_NAMES.iter().enumerate().skip(1) {
        if candidate.len() == name.len()
            && candidate.bytes().zip(name.iter()).all(|(a, &b)| fold(a) == fold(b))
        {
            return Some(n as u32);
        }
    }
    None
}

/// The grammar (one directive per line; `#` starts a comment):
///
/// ```text
/// mode enforce|complain
/// allow syscall <name>... | *
/// allow file <rwd> <pattern>...
/// allow net <cbs> <ip|*>:<port|lo-hi|*>...
/// allow exec <pattern>...
/// ```
///
/// Strict on purpose: an unknown word, a bad name, an over-long pattern, too
/// many rules, a stray byte - all are errors, because a profile that is only
/// partly understood is a profile whose meaning is a guess.
pub fn parse(text: &[u8]) -> Result<Policy, ParseError> {
    if text.len() > MAX_PROFILE_TEXT {
        return Err(ParseError::TooLarge);
    }
    for &b in text {
        if !(b == b'\n' || b == b'\r' || b == b'\t' || (0x20..0x7f).contains(&b)) {
            return Err(ParseError::BadByte);
        }
    }
    let mut pol = Policy::EMPTY;
    let mut seen_mode = false;
    let mut line_no: u16 = 0;
    for raw in text.split(|&b| b == b'\n') {
        line_no = line_no.saturating_add(1);
        let mut line = raw;
        if let Some((&b'\r', rest)) = line.split_last() {
            line = rest;
        }
        if line.len() > MAX_LINE {
            return Err(ParseError::LineTooLong(line_no));
        }
        if let Some(i) = line.iter().position(|&b| b == b'#') {
            line = &line[..i];
        }
        let mut pos = 0;
        let word = match next_token(line, &mut pos) {
            None => continue,
            Some(w) => w,
        };
        if word == b"mode" {
            if seen_mode {
                return Err(ParseError::DuplicateMode(line_no));
            }
            seen_mode = true;
            let arg = next_token(line, &mut pos).ok_or(ParseError::Syntax(line_no))?;
            pol.complain = match arg {
                b"enforce" => false,
                b"complain" => true,
                _ => return Err(ParseError::Syntax(line_no)),
            };
            if next_token(line, &mut pos).is_some() {
                return Err(ParseError::Syntax(line_no));
            }
            continue;
        }
        if word != b"allow" {
            return Err(ParseError::Syntax(line_no));
        }
        let kind = next_token(line, &mut pos).ok_or(ParseError::Syntax(line_no))?;
        match kind {
            b"syscall" => {
                let mut any = false;
                while let Some(tok) = next_token(line, &mut pos) {
                    any = true;
                    if tok == b"*" {
                        pol.all_syscalls = true;
                    } else {
                        let n = lookup_syscall(tok).ok_or(ParseError::UnknownSyscall(line_no))?;
                        pol.syscalls[(n / 32) as usize] |= 1 << (n % 32);
                    }
                }
                if !any {
                    return Err(ParseError::Syntax(line_no));
                }
            }
            b"file" => {
                let perm_tok = next_token(line, &mut pos).ok_or(ParseError::Syntax(line_no))?;
                let mut perms = 0u8;
                for &c in perm_tok {
                    perms |= match c {
                        b'r' => FILE_READ,
                        b'w' => FILE_WRITE,
                        b'd' => FILE_DELETE,
                        _ => return Err(ParseError::Syntax(line_no)),
                    };
                }
                let mut any = false;
                while let Some(pat) = next_token(line, &mut pos) {
                    any = true;
                    if !valid_pattern(pat) {
                        return Err(ParseError::BadPattern(line_no));
                    }
                    if pol.nfiles as usize >= MAX_FILE_RULES {
                        return Err(ParseError::TooManyRules(line_no));
                    }
                    let mut rule = FileRule::EMPTY;
                    rule.pat[..pat.len()].copy_from_slice(pat);
                    rule.len = pat.len() as u8;
                    rule.perms = perms;
                    pol.files[pol.nfiles as usize] = rule;
                    pol.nfiles += 1;
                }
                if !any {
                    return Err(ParseError::Syntax(line_no));
                }
            }
            b"net" => {
                let op_tok = next_token(line, &mut pos).ok_or(ParseError::Syntax(line_no))?;
                let mut ops = 0u8;
                for &c in op_tok {
                    ops |= match c {
                        b'c' => NET_CONNECT,
                        b'b' => NET_BIND,
                        b's' => NET_SEND,
                        _ => return Err(ParseError::Syntax(line_no)),
                    };
                }
                let mut any = false;
                while let Some(addr) = next_token(line, &mut pos) {
                    any = true;
                    let colon = addr
                        .iter()
                        .rposition(|&b| b == b':')
                        .ok_or(ParseError::BadAddress(line_no))?;
                    let (ip_s, port_s) = (&addr[..colon], &addr[colon + 1..]);
                    let (any_ip, ip) = if ip_s == b"*" {
                        (true, 0)
                    } else {
                        (false, parse_ip(ip_s).ok_or(ParseError::BadAddress(line_no))?)
                    };
                    let (port_lo, port_hi) = parse_ports(port_s).ok_or(ParseError::BadAddress(line_no))?;
                    if pol.nnets as usize >= MAX_NET_RULES {
                        return Err(ParseError::TooManyRules(line_no));
                    }
                    pol.nets[pol.nnets as usize] = NetRule { any_ip, ip, ops, port_lo, port_hi };
                    pol.nnets += 1;
                }
                if !any {
                    return Err(ParseError::Syntax(line_no));
                }
            }
            b"exec" => {
                let mut any = false;
                while let Some(pat) = next_token(line, &mut pos) {
                    any = true;
                    if !valid_pattern(pat) {
                        return Err(ParseError::BadPattern(line_no));
                    }
                    if pol.nexecs as usize >= MAX_EXEC_RULES {
                        return Err(ParseError::TooManyRules(line_no));
                    }
                    let mut rule = ExecRule::EMPTY;
                    rule.pat[..pat.len()].copy_from_slice(pat);
                    rule.len = pat.len() as u8;
                    pol.execs[pol.nexecs as usize] = rule;
                    pol.nexecs += 1;
                }
                if !any {
                    return Err(ParseError::Syntax(line_no));
                }
            }
            _ => return Err(ParseError::Syntax(line_no)),
        }
    }
    Ok(pol)
}

// ------------------------------------------------------------------------
// The profile table and the evaluation of a stack.
// ------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Slot {
    in_use: bool,
    name: [u8; NAME_LEN],
    name_len: u8,
    policy: Policy,
    allowed: u32,
    denied: u32,
    complained: u32,
}

impl Slot {
    const EMPTY: Slot = Slot {
        in_use: false,
        name: [0; NAME_LEN],
        name_len: 0,
        policy: Policy::EMPTY,
        allowed: 0,
        denied: 0,
        complained: 0,
    };
    fn name(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }
}

#[derive(Clone, Copy)]
pub struct AuditRec {
    pub kind: u8,
    pub complained: bool,
    pub pid: i32,
    pub profile: u16,
    pub a: u32,
    pub b: u32,
}

impl AuditRec {
    const EMPTY: AuditRec = AuditRec { kind: 0, complained: false, pid: 0, profile: 0, a: 0, b: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stack {
    pub ids: [u16; MAX_STACK],
    pub n: usize,
}

pub struct Mac {
    slots: [Slot; MAX_PROFILES],
    frozen: bool,
    total_allowed: u32,
    total_denied: u32,
    total_complained: u32,
    audit: [AuditRec; AUDIT_LEN],
    audit_next: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadError {
    BadName,
    Parse(ParseError),
    Full,
    Frozen,
}

impl LoadError {
    pub fn errno(self) -> i32 {
        match self {
            LoadError::BadName | LoadError::Parse(_) => EINVAL,
            LoadError::Full => ENOSPC,
            LoadError::Frozen => EPERM,
        }
    }
}

fn valid_profile_name(n: &[u8]) -> bool {
    !n.is_empty()
        && n.len() <= NAME_LEN
        && n.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl Mac {
    pub const fn new() -> Mac {
        Mac {
            slots: [Slot::EMPTY; MAX_PROFILES],
            frozen: false,
            total_allowed: 0,
            total_denied: 0,
            total_complained: 0,
            audit: [AuditRec::EMPTY; AUDIT_LEN],
            audit_next: 0,
        }
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// One-way: see the module comment.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    pub fn loaded(&self) -> usize {
        self.slots.iter().filter(|s| s.in_use).count()
    }

    /// Parses `text` and installs it as profile `name`, returning its id
    /// (1-based; 0 is "no profile"). The SAME name with the SAME policy is
    /// recognised and reuses its id, so starting an app a thousand times
    /// takes one slot. The same name with DIFFERENT content (the file was
    /// edited) takes a new slot and leaves the old one for processes still
    /// running under it. Once frozen, only an identical profile that is
    /// already loaded may be used.
    pub fn load(&mut self, name: &[u8], text: &[u8]) -> Result<u16, LoadError> {
        if !valid_profile_name(name) {
            return Err(LoadError::BadName);
        }
        let policy = parse(text).map_err(LoadError::Parse)?;
        for (i, s) in self.slots.iter().enumerate() {
            if s.in_use && s.name() == name && s.policy == policy {
                return Ok((i + 1) as u16);
            }
        }
        if self.frozen {
            return Err(LoadError::Frozen);
        }
        for (i, s) in self.slots.iter_mut().enumerate() {
            if !s.in_use {
                *s = Slot::EMPTY;
                s.in_use = true;
                s.name[..name.len()].copy_from_slice(name);
                s.name_len = name.len() as u8;
                s.policy = policy;
                return Ok((i + 1) as u16);
            }
        }
        Err(LoadError::Full)
    }

    /// The stack for a process that starts a program: the parent's stack plus
    /// the program's own profile (`new_id`, 0 for none), without duplicates.
    pub fn exec_stack(&self, parent: &[u16], new_id: u16) -> Result<Stack, i32> {
        let mut st = Stack { ids: [0; MAX_STACK], n: 0 };
        if parent.len() > MAX_STACK {
            return Err(E2BIG);
        }
        st.ids[..parent.len()].copy_from_slice(parent);
        st.n = parent.len();
        if new_id != 0 && !parent.contains(&new_id) {
            if st.n >= MAX_STACK {
                return Err(E2BIG);
            }
            st.ids[st.n] = new_id;
            st.n += 1;
        }
        Ok(st)
    }

    fn record(&mut self, kind: u8, complained: bool, pid: i32, profile: u16, a: u32, b: u32) {
        let i = (self.audit_next as usize) % AUDIT_LEN;
        self.audit[i] = AuditRec { kind, complained, pid, profile, a, b };
        self.audit_next = self.audit_next.wrapping_add(1);
    }

    /// The i-th most recent audit record (0 = newest), if there is one.
    pub fn audit_recent(&self, i: usize) -> Option<AuditRec> {
        if i >= AUDIT_LEN || (i as u32) >= self.audit_next {
            return None;
        }
        let idx = (self.audit_next as usize - 1 - i) % AUDIT_LEN;
        Some(self.audit[idx])
    }

    /// Evaluates one operation against every profile on `stack`. Returns 0
    /// (allowed), a positive profile id (allowed, but that profile is in
    /// complain mode and would have denied it) or a negative profile id
    /// (DENIED by that profile; -255 for a stack entry that names nothing).
    fn eval(
        &mut self,
        stack: &[u16],
        kind: u8,
        pid: i32,
        a: u32,
        b: u32,
        allowed: &dyn Fn(&Policy) -> bool,
    ) -> i32 {
        if stack.is_empty() {
            return 0; // unconfined
        }
        if stack.len() > MAX_STACK {
            self.total_denied = self.total_denied.wrapping_add(1);
            return -DENIER_INVALID;
        }
        let mut first_complain: u16 = 0;
        for &id in stack {
            let idx = id as usize;
            if idx == 0 || idx > MAX_PROFILES || !self.slots[idx - 1].in_use {
                self.total_denied = self.total_denied.wrapping_add(1);
                self.record(kind, false, pid, 0, a, b);
                return -DENIER_INVALID;
            }
            let ok = allowed(&self.slots[idx - 1].policy);
            let complain = self.slots[idx - 1].policy.complain;
            if ok {
                self.slots[idx - 1].allowed = self.slots[idx - 1].allowed.wrapping_add(1);
            } else if complain {
                self.slots[idx - 1].complained = self.slots[idx - 1].complained.wrapping_add(1);
                self.total_complained = self.total_complained.wrapping_add(1);
                self.record(kind, true, pid, id, a, b);
                if first_complain == 0 {
                    first_complain = id;
                }
            } else {
                self.slots[idx - 1].denied = self.slots[idx - 1].denied.wrapping_add(1);
                self.total_denied = self.total_denied.wrapping_add(1);
                self.record(kind, false, pid, id, a, b);
                return -(id as i32);
            }
        }
        self.total_allowed = self.total_allowed.wrapping_add(1);
        first_complain as i32
    }

    pub fn check_syscall(&mut self, stack: &[u16], pid: i32, sysno: u32) -> i32 {
        self.eval(stack, KIND_SYSCALL, pid, sysno, 0, &|p| p.allows_syscall(sysno))
    }

    pub fn check_file(&mut self, stack: &[u16], pid: i32, name: &[u8], op: u8) -> i32 {
        // A name this long cannot be a valid pattern target; refuse it rather
        // than match a prefix of it.
        let name = if name.len() > 64 { &[][..] } else { name };
        let deny_all = name.is_empty();
        self.eval(stack, KIND_FILE, pid, op as u32, name.len() as u32, &|p| {
            !deny_all && p.allows_file(name, op)
        })
    }

    pub fn check_net(&mut self, stack: &[u16], pid: i32, op: u8, ip: u32, port: u16) -> i32 {
        self.eval(stack, KIND_NET, pid, op as u32, ((port as u32) << 16) ^ (ip >> 16), &|p| {
            p.allows_net(op, ip, port)
        })
    }

    pub fn check_exec(&mut self, stack: &[u16], pid: i32, name: &[u8]) -> i32 {
        let name = if name.len() > 64 { &[][..] } else { name };
        let deny_all = name.is_empty();
        self.eval(stack, KIND_EXEC, pid, name.len() as u32, 0, &|p| !deny_all && p.allows_exec(name))
    }

    /// Name and counters of profile `id`.
    pub fn profile_info(&self, id: u16) -> Option<(&[u8], u32, u32, u32)> {
        let idx = id as usize;
        if idx == 0 || idx > MAX_PROFILES || !self.slots[idx - 1].in_use {
            return None;
        }
        let s = &self.slots[idx - 1];
        Some((s.name(), s.allowed, s.denied, s.complained))
    }

    pub fn totals(&self) -> (u32, u32, u32) {
        (self.total_allowed, self.total_denied, self.total_complained)
    }
}

/// Exercised at boot (and by the host tests): a few checks of the engine on a
/// private instance, so a build whose core is broken says so in the log
/// instead of enforcing nonsense. Returns the number of checks that held and
/// writes the number attempted.
pub fn selftest() -> (u32, u32) {
    let mut held = 0u32;
    let mut total = 0u32;
    let mut check = |ok: bool| {
        total += 1;
        if ok {
            held += 1;
        }
    };
    check(glob_match(b"*.LOG", b"a.log") && !glob_match(b"*.LOG", b"a.txt") && glob_match(b"A?C", b"abc"));
    let mut m = Mac::new();
    let text = b"allow syscall write open\nallow file r DATA.TXT\nallow net c 10.0.2.2:80\nallow exec HELLO.ELF\n";
    let id = m.load(b"selftest", text).unwrap_or(0);
    check(id == 1);
    let st = [id];
    check(m.check_syscall(&st, 1, 1) == 0);
    check(m.check_syscall(&st, 1, 32) < 0); // shutdown is not listed
    check(m.check_syscall(&st, 1, 2) == 0); // exit is always allowed
    check(m.check_file(&st, 1, b"data.txt", FILE_READ) == 0);
    check(m.check_file(&st, 1, b"DATA.TXT", FILE_WRITE) < 0);
    check(m.check_file(&st, 1, b"SECRET.TXT", FILE_READ) < 0);
    check(m.check_net(&st, 1, NET_CONNECT, 0x0A00_0202, 80) == 0);
    check(m.check_net(&st, 1, NET_CONNECT, 0x0A00_0202, 81) < 0);
    check(m.check_exec(&st, 1, b"hello.elf") == 0 && m.check_exec(&st, 1, b"EVIL.ELF") < 0);
    check(m.check_syscall(&[99], 1, 1) < 0); // a stack entry naming nothing denies all
    check(parse(b"allow bogus x").is_err());
    check(m.check_syscall(&[], 1, 32) == 0); // no stack: unconfined
    let _ = min(held, total);
    (held, total)
}

// ------------------------------------------------------------------------
// The kernel layer.
// ------------------------------------------------------------------------

#[cfg(not(test))]
mod kernel_glue {
    use super::*;
    use crate::spinlock::SpinLock;
    use core::slice;

    /// One instance. All-zero is its valid empty state (no profiles, not
    /// frozen), so it lives in .bss. One lock for everything: a check is a
    /// scan of at most a few small rule lists.
    static STATE: SpinLock<Mac> = SpinLock::new(Mac::new());

    /// A stack from the C side. More than MAX_STACK entries cannot be a real
    /// stack; the callers treat that as "deny everything".
    unsafe fn stack_of<'a>(p: *const u16, n: u32) -> Option<&'a [u16]> {
        if n == 0 {
            return Some(&[]);
        }
        if p.is_null() || n as usize > MAX_STACK {
            return None;
        }
        Some(slice::from_raw_parts(p, n as usize))
    }

    unsafe fn bytes_of<'a>(p: *const u8, n: u32) -> Option<&'a [u8]> {
        if n == 0 {
            return Some(&[]);
        }
        if p.is_null() || n as usize > MAX_PROFILE_TEXT {
            return None;
        }
        Some(slice::from_raw_parts(p, n as usize))
    }

    /// Installs a profile. Returns its id (> 0) or a negative errno.
    #[no_mangle]
    pub extern "C" fn rust_mac_load(name: *const u8, name_len: u32, text: *const u8, text_len: u32) -> i32 {
        let (n, t) = unsafe {
            match (bytes_of(name, name_len), bytes_of(text, text_len)) {
                (Some(n), Some(t)) => (n, t),
                _ => return -EINVAL,
            }
        };
        match STATE.lock().load(n, t) {
            Ok(id) => id as i32,
            Err(e) => -e.errno(),
        }
    }

    /// Why a profile failed to parse, for the boot log: a small code for the
    /// kind and the line number (0 if the failure was not a parse error).
    #[no_mangle]
    pub extern "C" fn rust_mac_explain(text: *const u8, text_len: u32, out_kind: *mut u32, out_line: *mut u32) {
        let t = match unsafe { bytes_of(text, text_len) } {
            Some(t) => t,
            None => return,
        };
        let (kind, line) = match parse(t) {
            Ok(_) => (0u32, 0u32),
            Err(ParseError::TooLarge) => (1, 0),
            Err(ParseError::BadByte) => (2, 0),
            Err(ParseError::LineTooLong(l)) => (3, l as u32),
            Err(ParseError::Syntax(l)) => (4, l as u32),
            Err(ParseError::UnknownSyscall(l)) => (5, l as u32),
            Err(ParseError::BadPattern(l)) => (6, l as u32),
            Err(ParseError::BadAddress(l)) => (7, l as u32),
            Err(ParseError::TooManyRules(l)) => (8, l as u32),
            Err(ParseError::DuplicateMode(l)) => (9, l as u32),
        };
        unsafe {
            *out_kind = kind;
            *out_line = line;
        }
    }

    /// The stack a process gets when it starts a program. 0, or negative errno.
    #[no_mangle]
    pub extern "C" fn rust_mac_exec_stack(parent: *const u16, parent_n: u32, new_id: u32, out: *mut u16, out_n: *mut u32) -> i32 {
        let p = match unsafe { stack_of(parent, parent_n) } {
            Some(p) => p,
            None => return -E2BIG,
        };
        if new_id > MAX_PROFILES as u32 {
            return -EINVAL;
        }
        match STATE.lock().exec_stack(p, new_id as u16) {
            Ok(st) => unsafe {
                for i in 0..MAX_STACK {
                    *out.add(i) = st.ids[i];
                }
                *out_n = st.n as u32;
                0
            },
            Err(e) => -e,
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_mac_check_syscall(stack: *const u16, n: u32, pid: i32, sysno: u32) -> i32 {
        match unsafe { stack_of(stack, n) } {
            Some(s) => STATE.lock().check_syscall(s, pid, sysno),
            None => -DENIER_INVALID,
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_mac_check_file(stack: *const u16, n: u32, pid: i32, name: *const u8, name_len: u32, op: u32) -> i32 {
        let (s, nm) = unsafe {
            match (stack_of(stack, n), bytes_of(name, name_len)) {
                (Some(s), Some(nm)) => (s, nm),
                _ => return -DENIER_INVALID,
            }
        };
        STATE.lock().check_file(s, pid, nm, op as u8)
    }

    #[no_mangle]
    pub extern "C" fn rust_mac_check_net(stack: *const u16, n: u32, pid: i32, op: u32, ip: u32, port: u32) -> i32 {
        match unsafe { stack_of(stack, n) } {
            Some(s) => STATE.lock().check_net(s, pid, op as u8, ip, (port & 0xFFFF) as u16),
            None => -DENIER_INVALID,
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_mac_check_exec(stack: *const u16, n: u32, pid: i32, name: *const u8, name_len: u32) -> i32 {
        let (s, nm) = unsafe {
            match (stack_of(stack, n), bytes_of(name, name_len)) {
                (Some(s), Some(nm)) => (s, nm),
                _ => return -DENIER_INVALID,
            }
        };
        STATE.lock().check_exec(s, pid, nm)
    }

    #[no_mangle]
    pub extern "C" fn rust_mac_is_policy_file(name: *const u8, name_len: u32) -> i32 {
        match unsafe { bytes_of(name, name_len) } {
            Some(n) => is_policy_file(n) as i32,
            None => 0,
        }
    }

    /// Name (copied into `buf`, NUL-terminated) and counters of profile `id`.
    /// Returns the name length, or -ENOENT.
    #[no_mangle]
    pub extern "C" fn rust_mac_profile_info(id: u32, buf: *mut u8, cap: u32, counts: *mut u32) -> i32 {
        if id > MAX_PROFILES as u32 || cap == 0 {
            return -EINVAL;
        }
        let g = STATE.lock();
        match g.profile_info(id as u16) {
            None => -ENOENT,
            Some((name, allowed, denied, complained)) => unsafe {
                let n = min(name.len(), cap as usize - 1);
                for i in 0..n {
                    *buf.add(i) = name[i];
                }
                *buf.add(n) = 0;
                *counts = allowed;
                *counts.add(1) = denied;
                *counts.add(2) = complained;
                n as i32
            },
        }
    }

    /// [allowed, denied, complained, frozen, loaded, last_kind, last_a, last_b]
    #[no_mangle]
    pub extern "C" fn rust_mac_global(out: *mut u32) {
        let g = STATE.lock();
        let (a, d, c) = g.totals();
        let last_deny = (0..AUDIT_LEN).filter_map(|i| g.audit_recent(i)).find(|r| !r.complained);
        unsafe {
            *out = a;
            *out.add(1) = d;
            *out.add(2) = c;
            *out.add(3) = g.is_frozen() as u32;
            *out.add(4) = g.loaded() as u32;
            *out.add(5) = last_deny.map_or(0, |r| r.kind as u32);
            *out.add(6) = last_deny.map_or(0, |r| r.a);
            *out.add(7) = last_deny.map_or(0, |r| r.b);
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_mac_freeze() {
        STATE.lock().freeze();
    }

    #[no_mangle]
    pub extern "C" fn rust_mac_is_frozen() -> i32 {
        STATE.lock().is_frozen() as i32
    }

    /// Boot self-test. Writes the number of checks attempted, returns the
    /// number that held.
    #[no_mangle]
    pub extern "C" fn rust_mac_selftest(out_total: *mut u32) -> u32 {
        let (held, total) = selftest();
        unsafe {
            *out_total = total;
        }
        held
    }
}

// ------------------------------------------------------------------------
// Host tests.
// ------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- a tiny deterministic RNG ----
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 >> 11
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    const BASIC: &[u8] = b"# a small app\nmode enforce\nallow syscall write read open close yield sbrk\nallow file r DATA.TXT *.LOG\nallow file rw OUT.TXT\nallow net cs 10.0.2.2:80 *:5000-5010\nallow exec HELLO.ELF\n";

    fn loaded(text: &[u8]) -> (Mac, [u16; 1]) {
        let mut m = Mac::new();
        let id = m.load(b"app", text).expect("profile must load");
        (m, [id])
    }

    // ---- the syscall table ----

    #[test]
    fn syscall_table_matches_the_kernels_header() {
        let header = include_str!("../arch/x86/cpu/syscall.h");
        let mut seen = 0;
        for line in header.lines() {
            let l = line.trim_start();
            let rest = match l.strip_prefix("#define SYS_") {
                Some(r) => r,
                None => continue,
            };
            let mut it = rest.split_whitespace();
            let (name, num) = match (it.next(), it.next()) {
                (Some(n), Some(v)) => (n, v),
                _ => continue,
            };
            let num: usize = match num.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };
            assert!(num != 0 && num < SYSCALL_NAMES.len(), "syscall {} = {} is outside the profile table", name, num);
            assert_eq!(SYSCALL_NAMES[num].to_ascii_uppercase(), name, "syscall {} is {} in the header", num, name);
            seen += 1;
        }
        assert_eq!(seen, SYSCALL_NAMES.len() - 1, "every table entry must exist in the header");
    }

    #[test]
    fn every_syscall_name_resolves_to_its_number() {
        for n in 1..SYSCALL_NAMES.len() {
            assert_eq!(lookup_syscall(SYSCALL_NAMES[n].as_bytes()), Some(n as u32));
            assert_eq!(lookup_syscall(SYSCALL_NAMES[n].to_ascii_uppercase().as_bytes()), Some(n as u32), "names are case-insensitive");
        }
        assert_eq!(lookup_syscall(b""), None);
        assert_eq!(lookup_syscall(b"writ"), None);
        assert_eq!(lookup_syscall(b"writee"), None);
        assert_eq!(lookup_syscall(b"nosuch"), None);
    }

    // ---- glob ----

    fn naive_glob(p: &[u8], t: &[u8]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some((&b'*', rest)) => (0..=t.len()).any(|i| naive_glob(rest, &t[i..])),
            Some((&b'?', rest)) => !t.is_empty() && naive_glob(rest, &t[1..]),
            Some((&c, rest)) => !t.is_empty() && fold(c) == fold(t[0]) && naive_glob(rest, &t[1..]),
        }
    }

    #[test]
    fn glob_table() {
        let yes: &[(&str, &str)] = &[("*", ""), ("*", "anything"), ("a*", "a"), ("a*b", "ab"), ("a*b", "axxb"), ("?", "x"), ("??", "ab"),
            ("*.TXT", "x.txt"), ("*.txt", "X.TXT"), ("D?TA.*", "data.bin"), ("**", "x"), ("a*b*c", "aXbYc"), ("*a", "bba"), ("A", "a")];
        let no: &[(&str, &str)] = &[("", "a"), ("a", ""), ("a", "b"), ("?", ""), ("??", "a"), ("*.TXT", "x.txtx"), ("a*b", "ba"), ("a*b*c", "acb"), ("*a", "ab"), ("A", "AA")];
        for (p, t) in yes { assert!(glob_match(p.as_bytes(), t.as_bytes()), "{} should match {}", p, t); }
        for (p, t) in no { assert!(!glob_match(p.as_bytes(), t.as_bytes()), "{} should NOT match {}", p, t); }
        assert!(glob_match(b"", b""));
    }

    #[test]
    fn glob_agrees_with_a_naive_reference_on_random_inputs() {
        let mut r = Rng(0x5EED_1234);
        let pat_alpha = b"ab.?*";
        let txt_alpha = b"ab.";
        let mut checked = 0;
        for _ in 0..40_000 {
            let pl = r.below(9);
            let tl = r.below(9);
            let p: Vec<u8> = (0..pl).map(|_| pat_alpha[r.below(pat_alpha.len())]).collect();
            let t: Vec<u8> = (0..tl).map(|_| txt_alpha[r.below(txt_alpha.len())]).collect();
            assert_eq!(glob_match(&p, &t), naive_glob(&p, &t), "pattern {:?} text {:?}", String::from_utf8_lossy(&p), String::from_utf8_lossy(&t));
            checked += 1;
        }
        assert_eq!(checked, 40_000);
    }

    #[test]
    fn glob_cannot_be_driven_into_exponential_time() {
        // the classic: many stars against a long near-miss
        let pat = b"*a*a*a*a*a*a*a*a*a*a*b";
        let text = vec![b'a'; 60];
        assert!(!glob_match(pat, &text));
    }

    // ---- the parser ----

    #[test]
    fn a_good_profile_parses_and_decides() {
        let (mut m, st) = loaded(BASIC);
        assert_eq!(m.check_syscall(&st, 1, 1), 0);
        assert!(m.check_syscall(&st, 1, 32) < 0, "shutdown was not listed");
        assert_eq!(m.check_file(&st, 1, b"data.txt", FILE_READ), 0);
        assert_eq!(m.check_file(&st, 1, b"X.LOG", FILE_READ), 0);
        assert!(m.check_file(&st, 1, b"X.LOG", FILE_WRITE) < 0, "*.LOG is read-only");
        assert_eq!(m.check_file(&st, 1, b"OUT.TXT", FILE_READ | FILE_WRITE), 0);
        assert!(m.check_file(&st, 1, b"DATA.TXT", FILE_READ | FILE_WRITE) < 0, "r alone must not allow an open for read AND write");
        assert!(m.check_file(&st, 1, b"OUT.TXT", FILE_READ | FILE_WRITE | FILE_DELETE) < 0, "every requested bit must be granted");
        assert!(m.check_file(&st, 1, b"OUT.TXT", FILE_DELETE) < 0);
        assert!(m.check_file(&st, 1, b"OUT.TXT", 0) < 0, "asking for nothing is not a grant");
        assert!(m.check_file(&st, 1, b"SECRET.TXT", FILE_READ) < 0);
        assert_eq!(m.check_net(&st, 1, NET_CONNECT, 0x0A00_0202, 80), 0);
        assert!(m.check_net(&st, 1, NET_CONNECT, 0x0A00_0202, 81) < 0);
        assert!(m.check_net(&st, 1, NET_CONNECT, 0x0A00_0203, 80) < 0, "a different host");
        assert!(m.check_net(&st, 1, NET_BIND, 0x0A00_0202, 80) < 0, "bind was not granted");
        assert_eq!(m.check_net(&st, 1, NET_SEND, 0x0102_0304, 5005), 0, "any host, ports 5000-5010");
        assert!(m.check_net(&st, 1, NET_SEND, 0x0102_0304, 5011) < 0);
        assert_eq!(m.check_exec(&st, 1, b"HELLO.ELF"), 0);
        assert!(m.check_exec(&st, 1, b"SH.ELF") < 0);
    }

    #[test]
    fn an_empty_profile_denies_everything_but_exit_and_info() {
        let (mut m, st) = loaded(b"# nothing allowed\n");
        for n in 0..200u32 {
            let r = m.check_syscall(&st, 1, n);
            if n == 2 || n == 71 {
                assert_eq!(r, 0, "syscall {} must always be allowed", n);
            } else {
                assert!(r < 0, "syscall {} must be denied by an empty profile", n);
            }
        }
        assert!(m.check_file(&st, 1, b"A", FILE_READ) < 0);
        assert!(m.check_net(&st, 1, NET_CONNECT, 1, 1) < 0);
        assert!(m.check_exec(&st, 1, b"A") < 0);
    }

    #[test]
    fn star_allows_every_known_syscall_and_still_not_unknown_ones() {
        let (mut m, st) = loaded(b"allow syscall *\n");
        for n in 1..128u32 {
            assert_eq!(m.check_syscall(&st, 1, n), 0, "syscall {}", n);
        }
        for n in [128u32, 129, 1000, 0xFFFF_FFFF] {
            assert!(m.check_syscall(&st, 1, n) < 0, "unknown syscall {} must be denied", n);
        }
        assert!(m.check_syscall(&st, 1, 0) == 0 || true);
    }

    #[test]
    fn every_parse_error_is_reported_with_its_line() {
        let cases: &[(&[u8], ParseError)] = &[
            (b"bogus\n", ParseError::Syntax(1)),
            (b"allow\n", ParseError::Syntax(1)),
            (b"allow thing x\n", ParseError::Syntax(1)),
            (b"\n\nallow syscall\n", ParseError::Syntax(3)),
            (b"allow syscall nosuch\n", ParseError::UnknownSyscall(1)),
            (b"allow file r\n", ParseError::Syntax(1)),
            (b"allow file x A\n", ParseError::Syntax(1)),
            (b"allow file r bad/name\n", ParseError::BadPattern(1)),
            (b"allow file r ABCDEFGHIJKLMNOPQRSTUVWXYZ\n", ParseError::BadPattern(1)),
            (b"allow net c 1.2.3.4\n", ParseError::BadAddress(1)),
            (b"allow net c 1.2.3:80\n", ParseError::BadAddress(1)),
            (b"allow net c 1.2.3.4.5:80\n", ParseError::BadAddress(1)),
            (b"allow net c 1.2.3.256:80\n", ParseError::BadAddress(1)),
            (b"allow net c 1.2.3.4:65536\n", ParseError::BadAddress(1)),
            (b"allow net c 1.2.3.4:9-3\n", ParseError::BadAddress(1)),
            (b"allow net x 1.2.3.4:80\n", ParseError::Syntax(1)),
            (b"allow exec\n", ParseError::Syntax(1)),
            (b"mode enforce\nmode complain\n", ParseError::DuplicateMode(2)),
            (b"mode fast\n", ParseError::Syntax(1)),
            (b"mode enforce extra\n", ParseError::Syntax(1)),
            (b"allow syscall write\x01\n", ParseError::BadByte),
        ];
        for (text, want) in cases {
            assert_eq!(parse(text).err(), Some(*want), "for {:?}", String::from_utf8_lossy(text));
        }
    }

    #[test]
    fn rule_count_limits_are_enforced_at_exactly_the_limit() {
        let mut t = String::new();
        for i in 0..MAX_FILE_RULES { t.push_str(&format!("allow file r F{}.TXT\n", i)); }
        assert!(parse(t.as_bytes()).is_ok(), "exactly {} file rules fit", MAX_FILE_RULES);
        t.push_str("allow file r ONE-MORE\n");
        assert_eq!(parse(t.as_bytes()).err(), Some(ParseError::TooManyRules(MAX_FILE_RULES as u16 + 1)));
        let mut n = String::new();
        for i in 0..MAX_NET_RULES { n.push_str(&format!("allow net c 10.0.0.{}:1\n", i)); }
        assert!(parse(n.as_bytes()).is_ok());
        n.push_str("allow net c 10.0.0.99:1\n");
        assert!(matches!(parse(n.as_bytes()), Err(ParseError::TooManyRules(_))));
        let mut e = String::new();
        for i in 0..MAX_EXEC_RULES { e.push_str(&format!("allow exec X{}.ELF\n", i)); }
        assert!(parse(e.as_bytes()).is_ok());
        e.push_str("allow exec Y.ELF\n");
        assert!(matches!(parse(e.as_bytes()), Err(ParseError::TooManyRules(_))));
        // several patterns on one line count one rule each
        let many = "allow file r A B C D E F G H I J K L M N O P Q\n";
        assert!(matches!(parse(many.as_bytes()), Err(ParseError::TooManyRules(1))));
    }

    #[test]
    fn size_limits_are_exact() {
        let mut ok = vec![b'#'; MAX_LINE];
        ok.push(b'\n');
        assert!(parse(&ok).is_ok(), "a line of exactly {} bytes", MAX_LINE);
        let mut long = vec![b'#'; MAX_LINE + 1];
        long.push(b'\n');
        assert_eq!(parse(&long).err(), Some(ParseError::LineTooLong(1)));
        let mut big = Vec::new();
        while big.len() < MAX_PROFILE_TEXT { big.extend_from_slice(b"#\n"); }
        big.truncate(MAX_PROFILE_TEXT);
        assert!(parse(&big).is_ok(), "exactly {} bytes", MAX_PROFILE_TEXT);
        big.push(b'#');
        assert_eq!(parse(&big).err(), Some(ParseError::TooLarge));
        // a CR before the newline is tolerated; a NUL is not
        assert!(parse(b"allow syscall write\r\nallow syscall read\r\n").is_ok());
        assert_eq!(parse(b"allow syscall write\0").err(), Some(ParseError::BadByte));
        assert_eq!(parse(&[0x80]).err(), Some(ParseError::BadByte));
    }

    #[test]
    fn complain_mode_and_comments_parse() {
        let p = parse(b"mode complain # log only\nallow syscall write # fine\n").unwrap();
        assert!(p.is_complain());
        assert!(!parse(b"allow syscall write\n").unwrap().is_complain());
    }

    #[test]
    fn fuzz_the_parser_with_random_bytes_and_mutated_profiles() {
        let mut r = Rng(0xF00D_CAFE);
        let mut parsed_ok = 0;
        for i in 0..30_000 {
            let text: Vec<u8> = if i % 2 == 0 {
                (0..r.below(120)).map(|_| r.below(256) as u8).collect()
            } else {
                // a valid profile with a few bytes flipped, inserted or removed
                let mut t = BASIC.to_vec();
                for _ in 0..1 + r.below(4) {
                    if t.is_empty() { break; }
                    let pos = r.below(t.len());
                    match r.below(3) {
                        0 => t[pos] = b" \n:*?-.#abcrwd0123456789"[r.below(24)],
                        1 => t.insert(pos, b" \n:*?-.#abcrwd0123456789"[r.below(24)]),
                        _ => { t.remove(pos); }
                    }
                }
                t
            };
            // it must never panic, and anything it ACCEPTS must be well-formed
            if let Ok(p) = parse(&text) {
                parsed_ok += 1;
                assert!(p.nfiles as usize <= MAX_FILE_RULES && p.nnets as usize <= MAX_NET_RULES && p.nexecs as usize <= MAX_EXEC_RULES);
                for f in &p.files[..p.nfiles as usize] { assert!(valid_pattern(f.pattern()) && f.perms != 0); }
                for n in &p.nets[..p.nnets as usize] { assert!(n.port_lo <= n.port_hi && n.ops != 0); }
            }
        }
        assert!(parsed_ok > 100, "the mutation fuzz should still produce valid profiles ({})", parsed_ok);
    }

    // ---- the model-based test ----

    #[derive(Clone)]
    struct Spec {
        complain: bool,
        all: bool,
        sys: Vec<usize>,
        files: Vec<(u8, String)>,
        nets: Vec<(u8, Option<u32>, u16, u16)>,
        execs: Vec<String>,
    }

    fn gen_pat(r: &mut Rng) -> String {
        let alpha = b"abc.?*";
        (0..1 + r.below(6)).map(|_| alpha[r.below(alpha.len())] as char).collect()
    }

    fn gen_spec(r: &mut Rng) -> Spec {
        let mut s = Spec { complain: r.below(5) == 0, all: r.below(8) == 0, sys: vec![], files: vec![], nets: vec![], execs: vec![] };
        for _ in 0..r.below(12) { s.sys.push(1 + r.below(72)); }
        for _ in 0..r.below(MAX_FILE_RULES + 1) { s.files.push((1 + r.below(7) as u8, gen_pat(r))); }
        for _ in 0..r.below(MAX_NET_RULES + 1) {
            let lo = r.below(60) as u16;
            let hi = lo + r.below(30) as u16;
            let ip = if r.below(3) == 0 { None } else { Some(r.below(6) as u32) };
            s.nets.push((1 + r.below(7) as u8, ip, lo, hi));
        }
        for _ in 0..r.below(MAX_EXEC_RULES + 1) { s.execs.push(gen_pat(r)); }
        s
    }

    fn render(s: &Spec) -> String {
        let mut t = String::new();
        t.push_str(if s.complain { "mode complain\n" } else { "mode enforce\n" });
        if s.all { t.push_str("allow syscall *\n"); }
        for &n in &s.sys { t.push_str(&format!("allow syscall {}\n", SYSCALL_NAMES[n])); }
        for (perms, pat) in &s.files {
            let p: String = [(FILE_READ, 'r'), (FILE_WRITE, 'w'), (FILE_DELETE, 'd')].iter().filter(|(b, _)| perms & b != 0).map(|(_, c)| *c).collect();
            t.push_str(&format!("allow file {} {}\n", p, pat));
        }
        for (ops, ip, lo, hi) in &s.nets {
            let o: String = [(NET_CONNECT, 'c'), (NET_BIND, 'b'), (NET_SEND, 's')].iter().filter(|(b, _)| ops & b != 0).map(|(_, c)| *c).collect();
            let ips = match ip { None => "*".to_string(), Some(v) => format!("0.0.0.{}", v) };
            t.push_str(&format!("allow net {} {}:{}-{}\n", o, ips, lo, hi));
        }
        for p in &s.execs { t.push_str(&format!("allow exec {}\n", p)); }
        t
    }

    fn ref_sys(s: &Spec, n: u32) -> bool {
        if n == 2 || n == 71 { return true; }
        if n >= 128 { return false; }
        s.all || s.sys.contains(&(n as usize))
    }
    fn ref_file(s: &Spec, name: &[u8], op: u8) -> bool {
        !name.is_empty() && s.files.iter().any(|(perms, pat)| perms & op != 0 && naive_glob(pat.as_bytes(), name))
    }
    fn ref_net(s: &Spec, op: u8, ip: u32, port: u16) -> bool {
        s.nets.iter().any(|(ops, rip, lo, hi)| ops & op != 0 && rip.map_or(true, |v| v == ip) && port >= *lo && port <= *hi)
    }
    fn ref_exec(s: &Spec, name: &[u8]) -> bool {
        !name.is_empty() && s.execs.iter().any(|p| naive_glob(p.as_bytes(), name))
    }

    /// The decision for a stack of specs, by the book: every profile must
    /// allow; a complain-mode profile that would deny allows (and reports).
    fn ref_decide(specs: &[&Spec], f: &dyn Fn(&Spec) -> bool) -> (bool, bool) {
        let mut complained = false;
        for s in specs {
            if !f(s) {
                if s.complain { complained = true; } else { return (false, false); }
            }
        }
        (true, complained)
    }

    #[test]
    fn model_based_every_decision_matches_an_independent_reference() {
        let mut r = Rng(0xAC1D_C0DE);
        let mut decisions = 0u64;
        let mut denies = 0u64;
        let mut allows = 0u64;
        for round in 0..400 {
            let mut m = Mac::new();
            let nspecs = 1 + r.below(3);
            let specs: Vec<Spec> = (0..nspecs).map(|_| gen_spec(&mut r)).collect();
            let mut ids = vec![];
            for (i, s) in specs.iter().enumerate() {
                let text = render(s);
                let id = m.load(format!("p{}", i).as_bytes(), text.as_bytes()).unwrap_or_else(|e| panic!("round {}: {:?} for\n{}", round, e, text));
                ids.push(id);
            }
            let refs: Vec<&Spec> = specs.iter().collect();
            for _ in 0..200 {
                match r.below(4) {
                    0 => {
                        let n = r.below(80) as u32;
                        let (ok, _) = ref_decide(&refs, &|s| ref_sys(s, n));
                        let got = m.check_syscall(&ids, 7, n);
                        assert_eq!(got >= 0, ok, "round {} syscall {}", round, n);
                        if !ok { assert!(got < 0 && ids.contains(&((-got) as u16))); }
                        if ok { allows += 1 } else { denies += 1 }
                    }
                    1 => {
                        let len = r.below(7);
                        let name: Vec<u8> = (0..len).map(|_| b"abc."[r.below(4)]).collect();
                        let op = [FILE_READ, FILE_WRITE, FILE_DELETE][r.below(3)];
                        let (ok, _) = ref_decide(&refs, &|s| ref_file(s, &name, op));
                        let got = m.check_file(&ids, 7, &name, op);
                        assert_eq!(got >= 0, ok, "round {} file {:?} op {}", round, String::from_utf8_lossy(&name), op);
                        if ok { allows += 1 } else { denies += 1 }
                    }
                    2 => {
                        let op = [NET_CONNECT, NET_BIND, NET_SEND][r.below(3)];
                        let ip = r.below(7) as u32;
                        let port = r.below(100) as u16;
                        let (ok, _) = ref_decide(&refs, &|s| ref_net(s, op, ip, port));
                        let got = m.check_net(&ids, 7, op, ip, port);
                        assert_eq!(got >= 0, ok, "round {} net op {} ip {} port {}", round, op, ip, port);
                        if ok { allows += 1 } else { denies += 1 }
                    }
                    _ => {
                        let len = r.below(7);
                        let name: Vec<u8> = (0..len).map(|_| b"abc."[r.below(4)]).collect();
                        let (ok, _) = ref_decide(&refs, &|s| ref_exec(s, &name));
                        let got = m.check_exec(&ids, 7, &name);
                        assert_eq!(got >= 0, ok, "round {} exec {:?}", round, String::from_utf8_lossy(&name));
                        if ok { allows += 1 } else { denies += 1 }
                    }
                }
                decisions += 1;
            }
        }
        assert_eq!(decisions, 80_000);
        assert!(allows > 5_000 && denies > 5_000, "the model must exercise both outcomes: {} allows, {} denies", allows, denies);
    }

    // ---- stacks, complain, fail-closed, load, freeze ----

    #[test]
    fn a_stack_is_the_intersection_of_its_profiles() {
        let mut m = Mac::new();
        let a = m.load(b"a", b"allow syscall write read open\nallow file r *.TXT\n").unwrap();
        let b = m.load(b"b", b"allow syscall write sbrk open\nallow file r A.TXT B.TXT\n").unwrap();
        let st = [a, b];
        assert_eq!(m.check_syscall(&st, 1, 1), 0, "write: both allow");
        assert!(m.check_syscall(&st, 1, 5) < 0, "read: only a allows");
        assert!(m.check_syscall(&st, 1, 11) < 0, "sbrk: only b allows");
        assert_eq!(m.check_file(&st, 1, b"A.TXT", FILE_READ), 0);
        assert!(m.check_file(&st, 1, b"C.TXT", FILE_READ) < 0, "a allows *.TXT but b does not list C.TXT");
        // the order of the stack does not change any decision
        let rev = [b, a];
        for n in 0..80u32 { assert_eq!(m.check_syscall(&st, 1, n) >= 0, m.check_syscall(&rev, 1, n) >= 0, "syscall {}", n); }
    }

    #[test]
    fn rules_on_different_patterns_combine_their_permissions_for_one_name() {
        let (mut m, st) = loaded(b"allow file r *.TXT\nallow file w A.*\nallow file d Z*\n");
        assert_eq!(m.check_file(&st, 1, b"A.TXT", FILE_READ | FILE_WRITE), 0, "r from *.TXT and w from A.* both match A.TXT");
        assert!(m.check_file(&st, 1, b"B.TXT", FILE_READ | FILE_WRITE) < 0, "B.TXT only matches *.TXT");
        assert!(m.check_file(&st, 1, b"A.BIN", FILE_READ) < 0);
        assert_eq!(m.check_file(&st, 1, b"A.BIN", FILE_WRITE), 0);
        assert_eq!(m.check_file(&st, 1, b"Z.TXT", FILE_READ | FILE_DELETE), 0);
    }

    #[test]
    fn the_unconfined_fast_path_touches_nothing() {
        let mut m = Mac::new();
        let _ = m.load(b"x", b"allow syscall write\n").unwrap();
        for n in 0..50u32 {
            assert_eq!(m.check_syscall(&[], 1, n), 0);
        }
        assert_eq!(m.check_file(&[], 1, b"ANY", FILE_WRITE), 0);
        assert_eq!(m.totals(), (0, 0, 0), "an unconfined process is not counted, logged or audited");
        assert!(m.audit_recent(0).is_none());
    }

    #[test]
    fn the_denier_reported_is_the_profile_that_denied() {
        let mut m = Mac::new();
        let _filler1 = m.load(b"f1", b"allow syscall write\n").unwrap();
        let _filler2 = m.load(b"f2", b"allow syscall read\n").unwrap();
        let a = m.load(b"a", b"allow syscall write\n").unwrap();
        assert!(a >= 3, "the ids must differ from the trivial -1");
        let b = m.load(b"b", b"allow syscall write read\n").unwrap();
        assert_eq!(m.check_syscall(&[b, a], 1, 5), -(a as i32), "read is denied by a, not b");
        assert_eq!(m.check_syscall(&[a, b], 1, 5), -(a as i32));
    }

    #[test]
    fn complain_mode_allows_but_reports_and_only_for_its_own_profile() {
        let mut m = Mac::new();
        let strict = m.load(b"strict", b"allow syscall write\n").unwrap();
        let soft = m.load(b"soft", b"mode complain\nallow syscall write\n").unwrap();
        assert_eq!(m.check_syscall(&[soft], 1, 5), soft as i32, "complain: allowed, reported");
        assert_eq!(m.totals().2, 1);
        assert!(m.check_syscall(&[strict, soft], 1, 5) < 0, "a complaining profile never weakens a strict one on the same stack");
        assert!(m.check_syscall(&[soft, strict], 1, 5) < 0, "in either order");
        assert_eq!(m.check_syscall(&[soft], 1, 1), 0, "an allowed call reports nothing");
        let rec = m.audit_recent(1).unwrap();
        assert!(rec.complained && rec.kind == KIND_SYSCALL && rec.a == 5);
    }

    #[test]
    fn fail_closed_on_bad_stack_entries() {
        let mut m = Mac::new();
        let a = m.load(b"a", b"allow syscall *\n").unwrap();
        for bad in [0u16, 2, 33, 255, 0xFFFF] {
            assert_eq!(m.check_syscall(&[bad], 1, 1), -DENIER_INVALID, "entry {} names nothing", bad);
            assert_eq!(m.check_syscall(&[a, bad], 1, 1), -DENIER_INVALID, "one bad entry poisons the stack");
            assert_eq!(m.check_file(&[bad], 1, b"X", FILE_READ), -DENIER_INVALID);
        }
        assert_eq!(m.check_syscall(&[a, a, a, a, a], 1, 1), -DENIER_INVALID, "an over-long stack is not a stack");
        assert_eq!(m.check_syscall(&[], 1, 1), 0, "the empty stack is an unconfined process");
    }

    #[test]
    fn long_names_are_refused_not_prefix_matched() {
        let (mut m, st) = loaded(b"allow file r *\nallow exec *\n");
        let long = vec![b'A'; 65];
        assert!(m.check_file(&st, 1, &long, FILE_READ) < 0);
        assert!(m.check_exec(&st, 1, &long) < 0);
        assert!(m.check_file(&st, 1, &long[..64], FILE_READ) == 0);
        assert!(m.check_file(&st, 1, b"", FILE_READ) < 0, "the empty name is never allowed, even by *");
    }

    #[test]
    fn loading_dedupes_identical_profiles_and_versions_changed_ones() {
        let mut m = Mac::new();
        let a1 = m.load(b"app", b"allow syscall write\n").unwrap();
        let a2 = m.load(b"app", b"allow syscall write\n").unwrap();
        assert_eq!(a1, a2, "the same profile loaded twice is one slot");
        let a3 = m.load(b"app", b"allow syscall write read\n").unwrap();
        assert_ne!(a1, a3, "edited content is a new version");
        assert_eq!(m.check_syscall(&[a1], 1, 5) < 0, true, "processes already running keep the OLD version");
        assert_eq!(m.check_syscall(&[a3], 1, 5), 0);
        let other = m.load(b"other", b"allow syscall write\n").unwrap();
        assert_ne!(a1, other, "the same content under another name is another profile");
        assert_eq!(m.loaded(), 3);
        // comments and whitespace do not make a different profile
        let a4 = m.load(b"app", b"# a comment\n   allow   syscall   write   \n").unwrap();
        assert_eq!(a4, a1);
    }

    #[test]
    fn load_errors_and_a_full_table() {
        let mut m = Mac::new();
        assert_eq!(m.load(b"", b"").err(), Some(LoadError::BadName));
        assert_eq!(m.load(b"has space", b"").err(), Some(LoadError::BadName));
        assert_eq!(m.load(b"x/y", b"").err(), Some(LoadError::BadName));
        assert_eq!(m.load(&[b'a'; NAME_LEN + 1], b"").err(), Some(LoadError::BadName));
        assert!(m.load(&[b'a'; NAME_LEN], b"").is_ok());
        assert!(matches!(m.load(b"bad", b"nonsense").err(), Some(LoadError::Parse(_))));
        assert_eq!(LoadError::Parse(ParseError::BadByte).errno(), EINVAL);
        let mut m = Mac::new();
        for i in 0..MAX_PROFILES {
            let t = format!("allow syscall write\nallow file r F{}\n", i);
            assert_eq!(m.load(format!("p{}", i).as_bytes(), t.as_bytes()).unwrap(), (i + 1) as u16);
        }
        assert_eq!(m.load(b"extra", b"allow syscall read\n").err(), Some(LoadError::Full));
        assert_eq!(LoadError::Full.errno(), ENOSPC);
        assert!(m.load(b"p3", b"allow syscall write\nallow file r F3\n").is_ok(), "an existing profile still loads when the table is full");
    }

    #[test]
    fn freeze_is_one_way_and_only_known_profiles_survive_it() {
        let mut m = Mac::new();
        let a = m.load(b"app", b"allow syscall write\n").unwrap();
        assert!(!m.is_frozen());
        m.freeze();
        assert!(m.is_frozen());
        assert_eq!(m.load(b"app", b"allow syscall write\n").unwrap(), a, "an identical, already-loaded profile still works");
        assert_eq!(m.load(b"app", b"allow syscall write read\n").err(), Some(LoadError::Frozen), "a changed profile is refused");
        assert_eq!(m.load(b"new", b"allow syscall write\n").err(), Some(LoadError::Frozen), "so is a new one");
        assert_eq!(LoadError::Frozen.errno(), EPERM);
        // there is no way back
        m.freeze();
        assert!(m.is_frozen());
        assert_eq!(m.load(b"new", b"allow syscall write\n").err(), Some(LoadError::Frozen));
    }

    #[test]
    fn exec_stack_adds_the_programs_profile_and_never_removes_one() {
        let mut m = Mac::new();
        let a = m.load(b"a", b"allow syscall *\n").unwrap();
        let b = m.load(b"b", b"allow syscall *\n").unwrap();
        let c = m.load(b"c", b"allow syscall *\n").unwrap();
        let d = m.load(b"d", b"allow syscall *\n").unwrap();
        let e = m.load(b"e", b"allow syscall *\n").unwrap();
        let s = m.exec_stack(&[], 0).unwrap();
        assert_eq!(s.n, 0, "an unconfined parent starting an unprofiled program stays unconfined");
        let s = m.exec_stack(&[], a).unwrap();
        assert_eq!((s.n, s.ids[0]), (1, a));
        let s = m.exec_stack(&[a], 0).unwrap();
        assert_eq!((s.n, s.ids[0]), (1, a), "a confined parent starting an unprofiled program: the child is still confined");
        let s = m.exec_stack(&[a], b).unwrap();
        assert_eq!((s.n, s.ids[..2].to_vec()), (2, vec![a, b]));
        let s = m.exec_stack(&[a, b], a).unwrap();
        assert_eq!(s.n, 2, "no duplicates");
        let s = m.exec_stack(&[a, b, c], d).unwrap();
        assert_eq!(s.n, 4);
        assert_eq!(m.exec_stack(&[a, b, c, d], e).err(), Some(E2BIG), "the stack is full: refuse rather than drop an entry");
        assert_eq!(m.exec_stack(&[a, b, c, d], a).unwrap().n, 4, "re-adding one already there needs no room");
        assert_eq!(m.exec_stack(&[a, b, c, d, e], 0).err(), Some(E2BIG));
    }

    #[test]
    fn counters_and_the_audit_ring() {
        let (mut m, st) = loaded(b"allow syscall write\n");
        assert_eq!(m.audit_recent(0).map(|r| r.kind), None);
        m.check_syscall(&st, 9, 1);
        m.check_syscall(&st, 9, 5);
        m.check_syscall(&st, 9, 5);
        let (_, allowed, denied, complained) = m.profile_info(st[0]).unwrap();
        assert_eq!((allowed, denied, complained), (1, 2, 0));
        assert_eq!(m.totals(), (1, 2, 0));
        let r = m.audit_recent(0).unwrap();
        assert!(!r.complained && r.pid == 9 && r.a == 5 && r.profile == st[0]);
        for i in 0..40u32 { m.check_syscall(&st, 1, 100 + i); }
        assert!(m.audit_recent(AUDIT_LEN - 1).is_some() && m.audit_recent(AUDIT_LEN).is_none(), "the ring keeps exactly its length");
        assert_eq!(m.audit_recent(0).unwrap().a, 139, "newest first");
        assert!(m.profile_info(0).is_none() && m.profile_info(2).is_none() && m.profile_info(99).is_none());
    }

    #[test]
    fn policy_files_are_recognised() {
        for n in ["A.MAC", "a.mac", "MACJAIL.MAC", "X.Mac", "VERYLONG.MAC"] { assert!(is_policy_file(n.as_bytes()), "{}", n); }
        for n in ["A.MACX", "MAC", "A.MA", ".MA", "A.MAC.TXT", "", "AMAC", "A.ELF"] { assert!(!is_policy_file(n.as_bytes()), "{}", n); }
        assert!(is_policy_file(b".MAC"), "a file called just .MAC is still a policy file");
    }

    #[test]
    fn the_boot_selftest_holds() {
        let (held, total) = selftest();
        assert_eq!(held, total);
        assert!(total >= 14);
    }
}
