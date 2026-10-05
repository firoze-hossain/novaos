//! kernel/rust/msg.rs - Phase 84: app-to-app messaging, beyond pipes.
//!
//! What this is. "The file manager tells the text editor to open a file."
//! Pipes (Phase 36) cannot carry that: they are byte streams (two messages
//! are indistinguishable from one), they carry no identity (the reader
//! cannot tell who wrote), they have no address (you need the handle, which
//! only fork() can give you), and a reader that wants one kind of message
//! must read them all. This module is the missing structure.
//!
//! The model (the full contract, with every error code, is
//! userland/libc/include/nova_msg_abi.h, which a host test below reads and
//! cross-checks against the constants here):
//!
//!  * INBOX. A process opts in to receiving with `open`. Its inbox is a
//!    bounded FIFO of whole messages - one send is one receive, whatever
//!    the size. A process without an inbox cannot be sent to, so no memory
//!    is ever committed on behalf of a process that did not ask to listen.
//!  * IDENTITY. The caller (kernel/ipc/msg.c) passes the sender's pid and
//!    uid from the process table; they are stamped on the message and a
//!    sender cannot choose them.
//!  * ADDRESS. By pid, or by SERVICE NAME: a process registers "editor"
//!    and anyone can send to "editor". Names are released when the owner
//!    exits, and a name whose owner is somehow gone is treated as free.
//!  * SELECTIVE RECEIVE. Oldest first, or the oldest from a given sender,
//!    of a given type, with a given tag - what request/response needs.
//!  * POLICY. The receiver chooses who may send: anyone, the same uid (or
//!    root), or an explicit allowlist. Refusals happen at send time and
//!    never occupy queue space.
//!  * FAIRNESS. No sender may hold more than MAX_PER_SENDER slots of one
//!    inbox, so one process flooding a service cannot lock out the others.
//!  * NOTHING BLOCKS. A syscall handler runs with interrupts disabled and
//!    this kernel has no blocked-process state, so a call that cannot
//!    complete returns EAGAIN and the caller waits with SYS_YIELD - the
//!    convention pipes, SYS_READ_KEY and SYS_WAIT already use. A receive
//!    whose buffer is too small does NOT consume the message: it reports
//!    the size needed, so a message is never silently truncated.
//!
//! Structure. Like kernel/rust/shm.rs, all policy is a plain state machine
//! over a small [`Hal`] trait (is this pid alive, what is its uid, what
//! time is it); the kernel supplies a `Hal` that calls into C and the host
//! tests supply a mock. Everything is fixed-size and zero-initialised (the
//! kernel's instance lives in .bss), matching pipe.rs's static tables, so
//! the module needs no allocator. Errors are POSITIVE errno values inside
//! the module and negated at the C boundary.

#![allow(dead_code)]

// ===================================================================
// Limits and ABI constants (mirrored in nova_msg_abi.h; a host test fails
// if the two ever disagree)
// ===================================================================

pub const MAX_PAYLOAD: usize = 1024;
pub const QUEUE_DEPTH: usize = 16;
pub const MAX_PER_SENDER: u32 = 8;
pub const MAX_INBOXES: usize = 32;
pub const MAX_SERVICES: usize = 32;
pub const MAX_NAMES_PER_PROC: u32 = 4;
pub const MAX_ALLOW: usize = 8;
pub const NAME_MAX: usize = 31;
pub const TICK_HZ: u32 = 100;

pub const ACCEPT_ANY: u32 = 0;
pub const ACCEPT_SAME_UID: u32 = 1;
pub const ACCEPT_ALLOWLIST: u32 = 2;

pub const RECV_PEEK: u32 = 1;
pub const RECV_MATCH_TAG: u32 = 2;

pub const SVC_REGISTER: u32 = 1;
pub const SVC_UNREGISTER: u32 = 2;
pub const SVC_LOOKUP: u32 = 3;

pub const CTL_STAT: u32 = 1;
pub const CTL_ALLOW: u32 = 2;
pub const CTL_DENY: u32 = 3;
pub const CTL_POLICY: u32 = 4;

pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const ESRCH: i32 = 3;
pub const E2BIG: i32 = 7;
pub const EBADF: i32 = 9;
pub const EAGAIN: i32 = 11;
pub const EACCES: i32 = 13;
pub const EFAULT: i32 = 14;
pub const EEXIST: i32 = 17;
pub const EINVAL: i32 = 22;
pub const ENOSPC: i32 = 28;

// ===================================================================
// The hardware abstraction
// ===================================================================

pub trait Hal {
    /// True for a process that exists and has not yet exited.
    fn pid_is_live(&mut self, pid: i32) -> bool;
    /// The uid of a live process.
    fn uid_of(&mut self, pid: i32) -> Option<u32>;
    /// The kernel tick counter (100Hz).
    fn ticks(&mut self) -> u32;
}

// ===================================================================
// State
// ===================================================================

/// One queued message. `used == false` marks a free slot, so the all-zero
/// bit pattern is a valid empty queue.
#[derive(Clone, Copy)]
struct Slot {
    used: bool,
    sender: i32,
    sender_uid: u32,
    mtype: u32,
    tag: u32,
    len: u32,
    tick: u32,
    /// Arrival order within the inbox (FIFO = smallest seq first). Messages
    /// are removed from the middle by selective receive, so a ring buffer's
    /// positions cannot express order; a sequence number can.
    seq: u64,
    data: [u8; MAX_PAYLOAD],
}

#[derive(Clone, Copy)]
struct Inbox {
    in_use: bool,
    pid: i32,
    policy: u32,
    /// Allowed sender pids (0 = empty slot; pid 0 is never a process).
    allow: [i32; MAX_ALLOW],
    count: u32,
    next_seq: u64,
    delivered: u32,
    refused: u32,
    slots: [Slot; QUEUE_DEPTH],
}

#[derive(Clone, Copy)]
struct Service {
    in_use: bool,
    pid: i32,
    name_len: u8,
    name: [u8; 32],
}

const EMPTY_SLOT: Slot = Slot { used: false, sender: 0, sender_uid: 0, mtype: 0, tag: 0, len: 0, tick: 0, seq: 0, data: [0; MAX_PAYLOAD] };
const EMPTY_INBOX: Inbox = Inbox {
    in_use: false,
    pid: 0,
    policy: 0,
    allow: [0; MAX_ALLOW],
    count: 0,
    next_seq: 0,
    delivered: 0,
    refused: 0,
    slots: [EMPTY_SLOT; QUEUE_DEPTH],
};
const EMPTY_SERVICE: Service = Service { in_use: false, pid: 0, name_len: 0, name: [0; 32] };

/// The whole subsystem: ~550KB, almost all of it the message slots.
pub struct Msg {
    inboxes: [Inbox; MAX_INBOXES],
    services: [Service; MAX_SERVICES],
}

/// Where a send goes.
#[derive(Clone, Copy)]
pub enum Dest<'a> {
    Pid(i32),
    Name(&'a [u8]),
}

/// What `recv` reports about the message it found.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RecvInfo {
    pub sender: i32,
    pub sender_uid: u32,
    pub mtype: u32,
    pub tag: u32,
    pub len: u32,
    pub tick: u32,
    pub pending: u32,
}

/// `stat` of one's own inbox.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stat {
    pub queued: u32,
    pub capacity: u32,
    pub max_payload: u32,
    pub policy: u32,
    pub delivered: u32,
    pub refused: u32,
    pub next_len: u32,
    pub next_type: u32,
    pub next_sender: i32,
    pub now_tick: u32,
    pub tick_hz: u32,
}

/// 1..=31 bytes of [A-Za-z0-9._-]. Anything else (spaces, slashes, control
/// characters, an empty name) is refused so a name is always printable and
/// can never be confused with a path or smuggle a NUL.
pub fn valid_name(n: &[u8]) -> bool {
    !n.is_empty() && n.len() <= NAME_MAX && n.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-')
}

impl Msg {
    pub const fn new() -> Msg {
        Msg { inboxes: [EMPTY_INBOX; MAX_INBOXES], services: [EMPTY_SERVICE; MAX_SERVICES] }
    }

    fn find_inbox(&self, pid: i32) -> Option<usize> {
        if pid <= 0 {
            return None;
        }
        self.inboxes.iter().position(|b| b.in_use && b.pid == pid)
    }

    fn find_service(&self, name: &[u8]) -> Option<usize> {
        self.services.iter().position(|s| s.in_use && &s.name[..s.name_len as usize] == name)
    }

    fn names_of(&self, pid: i32) -> u32 {
        self.services.iter().filter(|s| s.in_use && s.pid == pid).count() as u32
    }

    // ---- inbox lifetime ---------------------------------------------

    /// Opens the caller's inbox. Returns (queue depth, max payload).
    pub fn open(&mut self, pid: i32, policy: u32, flags: u32) -> Result<(u32, u32), i32> {
        if flags != 0 || policy > ACCEPT_ALLOWLIST || pid <= 0 {
            return Err(EINVAL);
        }
        if self.find_inbox(pid).is_some() {
            return Err(EEXIST);
        }
        let slot = match self.inboxes.iter().position(|b| !b.in_use) {
            Some(s) => s,
            None => return Err(ENOSPC),
        };
        self.inboxes[slot] = EMPTY_INBOX;
        self.inboxes[slot].in_use = true;
        self.inboxes[slot].pid = pid;
        self.inboxes[slot].policy = policy;
        Ok((QUEUE_DEPTH as u32, MAX_PAYLOAD as u32))
    }

    /// Drops the inbox, everything queued in it, and the names it owns.
    fn drop_process(&mut self, pid: i32) {
        for s in self.services.iter_mut() {
            if s.in_use && s.pid == pid {
                *s = EMPTY_SERVICE;
            }
        }
        for b in self.inboxes.iter_mut() {
            if b.in_use && b.pid == pid {
                *b = EMPTY_INBOX;
            } else if b.in_use {
                for a in b.allow.iter_mut() {
                    if *a == pid {
                        *a = 0;
                    }
                }
            }
        }
    }

    pub fn close(&mut self, pid: i32) -> Result<(), i32> {
        if self.find_inbox(pid).is_none() {
            return Err(EBADF);
        }
        self.drop_process(pid);
        Ok(())
    }

    /// A process is exiting: everything of its goes. Safe for a process
    /// that never used messaging.
    pub fn process_exit(&mut self, pid: i32) {
        self.drop_process(pid);
    }

    // ---- sending ----------------------------------------------------

    /// Sends one message. `from` / `from_uid` are the kernel's own record of
    /// the caller, never anything the caller supplied.
    pub fn send<H: Hal>(&mut self, h: &mut H, from: i32, from_uid: u32, dest: Dest, mtype: u32, tag: u32, flags: u32, data: &[u8]) -> Result<(), i32> {
        if flags != 0 || mtype == 0 {
            return Err(EINVAL);
        }
        if data.len() > MAX_PAYLOAD {
            return Err(E2BIG);
        }
        let to = match dest {
            Dest::Pid(p) => {
                if p <= 0 {
                    return Err(EINVAL);
                }
                p
            }
            Dest::Name(n) => {
                if !valid_name(n) {
                    return Err(EINVAL);
                }
                let si = match self.find_service(n) {
                    Some(si) => si,
                    None => return Err(ENOENT),
                };
                let owner = self.services[si].pid;
                if !h.pid_is_live(owner) {
                    // the owner is gone without its exit hook having run:
                    // treat the name as free rather than deliver into a void
                    self.services[si] = EMPTY_SERVICE;
                    return Err(ENOENT);
                }
                owner
            }
        };
        if !h.pid_is_live(to) {
            return Err(ESRCH);
        }
        let ii = match self.find_inbox(to) {
            Some(i) => i,
            None => return Err(ENOENT),
        };

        // The receiver's policy, checked before anything is queued.
        match self.inboxes[ii].policy {
            ACCEPT_SAME_UID => {
                let ru = match h.uid_of(to) {
                    Some(u) => u,
                    None => return Err(ESRCH),
                };
                if from_uid != ru && from_uid != 0 {
                    self.inboxes[ii].refused = self.inboxes[ii].refused.wrapping_add(1);
                    return Err(EACCES);
                }
            }
            ACCEPT_ALLOWLIST => {
                if !self.inboxes[ii].allow.iter().any(|&a| a == from) {
                    self.inboxes[ii].refused = self.inboxes[ii].refused.wrapping_add(1);
                    return Err(EACCES);
                }
            }
            _ => {}
        }

        let now = h.ticks();
        let ib = &mut self.inboxes[ii];
        let mut mine = 0u32;
        let mut free = None;
        for (i, s) in ib.slots.iter().enumerate() {
            if s.used {
                if s.sender == from {
                    mine += 1;
                }
            } else if free.is_none() {
                free = Some(i);
            }
        }
        let fi = match free {
            Some(i) if mine < MAX_PER_SENDER => i,
            _ => {
                ib.refused = ib.refused.wrapping_add(1);
                return Err(EAGAIN);
            }
        };
        let s = &mut ib.slots[fi];
        s.used = true;
        s.sender = from;
        s.sender_uid = from_uid;
        s.mtype = mtype;
        s.tag = tag;
        s.len = data.len() as u32;
        s.tick = now;
        s.seq = ib.next_seq;
        s.data[..data.len()].copy_from_slice(data);
        ib.next_seq += 1;
        ib.count += 1;
        Ok(())
    }

    // ---- receiving --------------------------------------------------

    /// Takes (or with RECV_PEEK, looks at) the OLDEST message matching the
    /// filters: `match_sender` 0 = any sender, `match_type` 0 = any type,
    /// `match_tag` only with RECV_MATCH_TAG. `info` is filled whenever a
    /// matching message exists - including when `out` is too small, in which
    /// case nothing is consumed and the error is E2BIG: a message is never
    /// silently truncated, and the caller learns the size it needs.
    pub fn recv(&mut self, pid: i32, flags: u32, match_sender: i32, match_type: u32, match_tag: u32, out: &mut [u8], info: &mut RecvInfo) -> Result<(), i32> {
        if flags & !(RECV_PEEK | RECV_MATCH_TAG) != 0 {
            return Err(EINVAL);
        }
        let ii = match self.find_inbox(pid) {
            Some(i) => i,
            None => return Err(EBADF),
        };
        let ib = &mut self.inboxes[ii];
        let mut best: Option<usize> = None;
        for (i, s) in ib.slots.iter().enumerate() {
            if !s.used
                || (match_sender != 0 && s.sender != match_sender)
                || (match_type != 0 && s.mtype != match_type)
                || (flags & RECV_MATCH_TAG != 0 && s.tag != match_tag)
            {
                continue;
            }
            if best.map_or(true, |b| s.seq < ib.slots[b].seq) {
                best = Some(i);
            }
        }
        let bi = match best {
            Some(b) => b,
            None => return Err(EAGAIN),
        };
        let len = ib.slots[bi].len as usize;
        *info = RecvInfo {
            sender: ib.slots[bi].sender,
            sender_uid: ib.slots[bi].sender_uid,
            mtype: ib.slots[bi].mtype,
            tag: ib.slots[bi].tag,
            len: len as u32,
            tick: ib.slots[bi].tick,
            pending: ib.count,
        };
        if len > out.len() {
            return Err(E2BIG);
        }
        out[..len].copy_from_slice(&ib.slots[bi].data[..len]);
        if flags & RECV_PEEK == 0 {
            ib.slots[bi].used = false;
            ib.count -= 1;
            ib.delivered = ib.delivered.wrapping_add(1);
            info.pending = ib.count;
        }
        Ok(())
    }

    // ---- service names ----------------------------------------------

    /// REGISTER / UNREGISTER / LOOKUP. LOOKUP returns the owner's pid;
    /// the others return 0.
    pub fn service<H: Hal>(&mut self, h: &mut H, pid: i32, op: u32, name: &[u8]) -> Result<i32, i32> {
        if !valid_name(name) {
            return Err(EINVAL);
        }
        match op {
            SVC_REGISTER => {
                if self.find_inbox(pid).is_none() {
                    return Err(EBADF); // a name nobody can send to is a bug
                }
                if let Some(si) = self.find_service(name) {
                    let owner = self.services[si].pid;
                    if owner == pid {
                        return Ok(0); // idempotent
                    }
                    if h.pid_is_live(owner) {
                        return Err(EEXIST);
                    }
                    // stale (owner vanished without cleanup): take it over
                    if self.names_of(pid) >= MAX_NAMES_PER_PROC {
                        return Err(ENOSPC);
                    }
                    self.services[si].pid = pid;
                    return Ok(0);
                }
                if self.names_of(pid) >= MAX_NAMES_PER_PROC {
                    return Err(ENOSPC);
                }
                let free = match self.services.iter().position(|s| !s.in_use) {
                    Some(f) => f,
                    None => return Err(ENOSPC),
                };
                let mut buf = [0u8; 32];
                buf[..name.len()].copy_from_slice(name);
                self.services[free] = Service { in_use: true, pid, name_len: name.len() as u8, name: buf };
                Ok(0)
            }
            SVC_UNREGISTER => match self.find_service(name) {
                None => Err(ENOENT),
                Some(si) if self.services[si].pid != pid => Err(EPERM),
                Some(si) => {
                    self.services[si] = EMPTY_SERVICE;
                    Ok(0)
                }
            },
            SVC_LOOKUP => match self.find_service(name) {
                None => Err(ENOENT),
                Some(si) => {
                    let owner = self.services[si].pid;
                    if h.pid_is_live(owner) {
                        Ok(owner)
                    } else {
                        self.services[si] = EMPTY_SERVICE;
                        Err(ENOENT)
                    }
                }
            },
            _ => Err(EINVAL),
        }
    }

    // ---- control ----------------------------------------------------

    pub fn stat<H: Hal>(&self, h: &mut H, pid: i32) -> Result<Stat, i32> {
        let ii = match self.find_inbox(pid) {
            Some(i) => i,
            None => return Err(EBADF),
        };
        let ib = &self.inboxes[ii];
        let mut st = Stat {
            queued: ib.count,
            capacity: QUEUE_DEPTH as u32,
            max_payload: MAX_PAYLOAD as u32,
            policy: ib.policy,
            delivered: ib.delivered,
            refused: ib.refused,
            next_len: 0,
            next_type: 0,
            next_sender: 0,
            now_tick: h.ticks(),
            tick_hz: TICK_HZ,
        };
        let mut best: Option<usize> = None;
        for (i, s) in ib.slots.iter().enumerate() {
            if s.used && best.map_or(true, |b| s.seq < ib.slots[b].seq) {
                best = Some(i);
            }
        }
        if let Some(b) = best {
            st.next_len = ib.slots[b].len;
            st.next_type = ib.slots[b].mtype;
            st.next_sender = ib.slots[b].sender;
        }
        Ok(st)
    }

    /// Adds (`allow = true`) or removes a pid from the allowlist. Entries
    /// only matter under ACCEPT_ALLOWLIST but may be edited under any policy,
    /// so a receiver can prepare its list before switching to it.
    pub fn allow(&mut self, pid: i32, target: i32, allow: bool) -> Result<(), i32> {
        let ii = match self.find_inbox(pid) {
            Some(i) => i,
            None => return Err(EBADF),
        };
        if target <= 0 {
            return Err(EINVAL);
        }
        let list = &mut self.inboxes[ii].allow;
        if allow {
            if list.iter().any(|&a| a == target) {
                return Ok(());
            }
            match list.iter().position(|&a| a == 0) {
                Some(f) => {
                    list[f] = target;
                    Ok(())
                }
                None => Err(ENOSPC),
            }
        } else {
            for a in list.iter_mut() {
                if *a == target {
                    *a = 0;
                }
            }
            Ok(())
        }
    }

    pub fn set_policy(&mut self, pid: i32, policy: u32) -> Result<(), i32> {
        let ii = match self.find_inbox(pid) {
            Some(i) => i,
            None => return Err(EBADF),
        };
        if policy > ACCEPT_ALLOWLIST {
            return Err(EINVAL);
        }
        self.inboxes[ii].policy = policy;
        Ok(())
    }

    // ---- self-checking ----------------------------------------------

    /// Structural invariants, usable in the kernel as well as the tests.
    pub fn check_counts(&self) -> Result<(), &'static str> {
        for (i, b) in self.inboxes.iter().enumerate() {
            let used = b.slots.iter().filter(|s| s.used).count() as u32;
            if !b.in_use {
                if used != 0 || b.count != 0 {
                    return Err("a free inbox holds messages");
                }
                continue;
            }
            if b.pid <= 0 {
                return Err("an inbox belongs to no valid pid");
            }
            if b.count != used {
                return Err("an inbox's count disagrees with its used slots");
            }
            if b.policy > ACCEPT_ALLOWLIST {
                return Err("an inbox has an invalid policy");
            }
            for (j, o) in self.inboxes.iter().enumerate() {
                if j != i && o.in_use && o.pid == b.pid {
                    return Err("two inboxes belong to one pid");
                }
            }
            for (j, a) in b.allow.iter().enumerate() {
                if *a < 0 {
                    return Err("a negative pid in an allowlist");
                }
                if *a != 0 && b.allow[j + 1..].iter().any(|x| x == a) {
                    return Err("a duplicate allowlist entry");
                }
            }
            for (j, s) in b.slots.iter().enumerate() {
                if !s.used {
                    continue;
                }
                if s.mtype == 0 || s.len as usize > MAX_PAYLOAD || s.sender <= 0 {
                    return Err("a queued message has impossible fields");
                }
                if b.slots[j + 1..].iter().any(|t| t.used && t.seq == s.seq) {
                    return Err("two queued messages share an arrival number");
                }
                if s.seq >= b.next_seq {
                    return Err("a queued message is newer than the arrival counter");
                }
            }
            for sender in b.slots.iter().filter(|s| s.used).map(|s| s.sender) {
                let n = b.slots.iter().filter(|s| s.used && s.sender == sender).count() as u32;
                if n > MAX_PER_SENDER {
                    return Err("a sender holds more than its share of an inbox");
                }
            }
        }
        for (i, s) in self.services.iter().enumerate() {
            if !s.in_use {
                continue;
            }
            if !valid_name(&s.name[..s.name_len as usize]) {
                return Err("a service has an invalid name");
            }
            if self.find_inbox(s.pid).is_none() {
                return Err("a service belongs to a process with no inbox");
            }
            if self.services[i + 1..].iter().any(|t| t.in_use && t.name_len == s.name_len && t.name[..t.name_len as usize] == s.name[..s.name_len as usize]) {
                return Err("two services share a name");
            }
            if self.names_of(s.pid) > MAX_NAMES_PER_PROC {
                return Err("a process holds more names than the limit");
            }
        }
        Ok(())
    }

    /// (open inboxes, queued messages, registered names).
    pub fn stats(&self) -> (u32, u32, u32) {
        let ib = self.inboxes.iter().filter(|b| b.in_use).count() as u32;
        let q: u32 = self.inboxes.iter().filter(|b| b.in_use).map(|b| b.count).sum();
        let sv = self.services.iter().filter(|s| s.in_use).count() as u32;
        (ib, q, sv)
    }
}

// ===================================================================
// Kernel glue (not compiled for host tests)
// ===================================================================

#[cfg(not(test))]
mod kernel_glue {
    use super::*;
    use crate::spinlock::SpinLock;

    extern "C" {
        fn msg_hal_pid_is_live(pid: i32) -> i32;
        fn msg_hal_uid_of(pid: i32, out_uid: *mut u32) -> i32;
        fn msg_hal_ticks() -> u32;
    }

    struct KernelHal;

    impl Hal for KernelHal {
        fn pid_is_live(&mut self, pid: i32) -> bool {
            unsafe { msg_hal_pid_is_live(pid) != 0 }
        }
        fn uid_of(&mut self, pid: i32) -> Option<u32> {
            let mut uid = 0u32;
            if unsafe { msg_hal_uid_of(pid, &mut uid) } != 0 {
                Some(uid)
            } else {
                None
            }
        }
        fn ticks(&mut self) -> u32 {
            unsafe { msg_hal_ticks() }
        }
    }

    /// The one instance (all-zero is its valid empty state, so it lives in
    /// .bss). One lock for everything: a send or receive is a scan of a
    /// 16-slot queue plus at most a 1KB copy.
    static STATE: SpinLock<Msg> = SpinLock::new(Msg::new());

    #[inline]
    fn neg(r: Result<(), i32>) -> i32 {
        match r {
            Ok(()) => 0,
            Err(e) => -e,
        }
    }

    /// Every pointer below is a KERNEL pointer: msg.c has already validated
    /// the user's addresses and copied the data. Lengths are still checked
    /// here before any slice is built.
    #[no_mangle]
    pub extern "C" fn rust_msg_open(pid: i32, policy: u32, flags: u32, out_depth: *mut u32, out_max: *mut u32) -> i32 {
        match STATE.lock().open(pid, policy, flags) {
            Ok((d, m)) => unsafe {
                *out_depth = d;
                *out_max = m;
                0
            },
            Err(e) => -e,
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_msg_close(pid: i32) -> i32 {
        neg(STATE.lock().close(pid))
    }

    /// `name_len == 0` means "send to dest_pid"; otherwise `name` points at
    /// that many bytes of service name.
    #[no_mangle]
    pub extern "C" fn rust_msg_send(from: i32, from_uid: u32, dest_pid: i32, name: *const u8, name_len: u32, mtype: u32, tag: u32, flags: u32, data: *const u8, len: u32) -> i32 {
        if len as usize > MAX_PAYLOAD {
            return -E2BIG;
        }
        if name_len as usize > NAME_MAX + 1 {
            return -EINVAL;
        }
        let payload: &[u8] = if len == 0 { &[] } else { unsafe { core::slice::from_raw_parts(data, len as usize) } };
        let dest = if name_len == 0 {
            Dest::Pid(dest_pid)
        } else {
            Dest::Name(unsafe { core::slice::from_raw_parts(name, name_len as usize) })
        };
        let mut hal = KernelHal;
        neg(STATE.lock().send(&mut hal, from, from_uid, dest, mtype, tag, flags, payload))
    }

    /// `info` points at 8 u32s: sender pid, sender uid, type, tag, length,
    /// sent tick, pending, reserved. Filled whenever a message matched, even
    /// when the answer is -E2BIG.
    #[no_mangle]
    pub extern "C" fn rust_msg_recv(pid: i32, flags: u32, match_sender: i32, match_type: u32, match_tag: u32, buf: *mut u8, cap: u32, info: *mut u32) -> i32 {
        let cap = (cap as usize).min(MAX_PAYLOAD);
        let out: &mut [u8] = if cap == 0 { &mut [] } else { unsafe { core::slice::from_raw_parts_mut(buf, cap) } };
        let mut ri = RecvInfo::default();
        let r = STATE.lock().recv(pid, flags, match_sender, match_type, match_tag, out, &mut ri);
        if matches!(r, Ok(()) | Err(E2BIG)) {
            unsafe {
                *info.add(0) = ri.sender as u32;
                *info.add(1) = ri.sender_uid;
                *info.add(2) = ri.mtype;
                *info.add(3) = ri.tag;
                *info.add(4) = ri.len;
                *info.add(5) = ri.tick;
                *info.add(6) = ri.pending;
                *info.add(7) = 0;
            }
        }
        neg(r)
    }

    #[no_mangle]
    pub extern "C" fn rust_msg_service(pid: i32, op: u32, name: *const u8, name_len: u32, out_pid: *mut i32) -> i32 {
        if name_len as usize > NAME_MAX + 1 {
            return -EINVAL;
        }
        let n: &[u8] = if name_len == 0 { &[] } else { unsafe { core::slice::from_raw_parts(name, name_len as usize) } };
        let mut hal = KernelHal;
        match STATE.lock().service(&mut hal, pid, op, n) {
            Ok(p) => unsafe {
                *out_pid = p;
                0
            },
            Err(e) => -e,
        }
    }

    /// `out` points at 12 u32s: queued, capacity, max payload, policy,
    /// delivered, refused, next length, next type, next sender, now tick,
    /// tick Hz, reserved. Only filled for STAT.
    #[no_mangle]
    pub extern "C" fn rust_msg_ctl(pid: i32, op: u32, arg: u32, out: *mut u32) -> i32 {
        let mut hal = KernelHal;
        let mut st = STATE.lock();
        match op {
            CTL_STAT => match st.stat(&mut hal, pid) {
                Ok(s) => unsafe {
                    *out.add(0) = s.queued;
                    *out.add(1) = s.capacity;
                    *out.add(2) = s.max_payload;
                    *out.add(3) = s.policy;
                    *out.add(4) = s.delivered;
                    *out.add(5) = s.refused;
                    *out.add(6) = s.next_len;
                    *out.add(7) = s.next_type;
                    *out.add(8) = s.next_sender as u32;
                    *out.add(9) = s.now_tick;
                    *out.add(10) = s.tick_hz;
                    *out.add(11) = 0;
                    0
                },
                Err(e) => -e,
            },
            CTL_ALLOW => neg(st.allow(pid, arg as i32, true)),
            CTL_DENY => neg(st.allow(pid, arg as i32, false)),
            CTL_POLICY => neg(st.set_policy(pid, arg)),
            _ => -EINVAL,
        }
    }

    /// Called from process_exit_current(), before the process is marked
    /// terminated (same discipline and reason as the framebuffer's and
    /// shared memory's exit hooks).
    #[no_mangle]
    pub extern "C" fn rust_msg_process_exit(pid: i32) {
        STATE.lock().process_exit(pid);
    }

    /// Boot self-test: the books balance on the live state. Returns 0 if
    /// they do and writes (inboxes, queued messages, names).
    #[no_mangle]
    pub extern "C" fn rust_msg_selftest(out: *mut u32) -> u32 {
        let st = STATE.lock();
        let (i, q, s) = st.stats();
        unsafe {
            *out.add(0) = i;
            *out.add(1) = q;
            *out.add(2) = s;
        }
        match st.check_counts() {
            Ok(()) => 0,
            Err(_) => 1,
        }
    }
}

// ===================================================================
// Host tests: `rustc --edition 2021 --test kernel/rust/msg.rs`
// ===================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::vec::Vec;

    struct Mock {
        live: HashSet<i32>,
        uids: HashMap<i32, u32>,
        tick: u32,
    }

    impl Mock {
        fn new() -> Mock {
            let mut m = Mock { live: HashSet::new(), uids: HashMap::new(), tick: 1000 };
            for p in 1..=12 {
                m.live.insert(p);
                m.uids.insert(p, 100); // everyone is the same user unless a test says otherwise
            }
            m
        }
    }

    impl Hal for Mock {
        fn pid_is_live(&mut self, pid: i32) -> bool {
            self.live.contains(&pid)
        }
        fn uid_of(&mut self, pid: i32) -> Option<u32> {
            if self.live.contains(&pid) { self.uids.get(&pid).copied() } else { None }
        }
        fn ticks(&mut self) -> u32 {
            self.tick
        }
    }

    fn boxed() -> Box<Msg> {
        // All-zero is the valid empty state; allocate it directly rather
        // than building ~550KB on the test thread's stack.
        unsafe {
            let layout = std::alloc::Layout::new::<Msg>();
            let p = std::alloc::alloc_zeroed(layout) as *mut Msg;
            assert!(!p.is_null());
            Box::from_raw(p)
        }
    }

    fn setup() -> (Box<Msg>, Mock) {
        (boxed(), Mock::new())
    }

    fn open(m: &mut Msg, pid: i32) {
        m.open(pid, ACCEPT_ANY, 0).unwrap();
    }

    fn send(m: &mut Msg, h: &mut Mock, from: i32, to: i32, mtype: u32, tag: u32, data: &[u8]) -> Result<(), i32> {
        let uid = h.uids.get(&from).copied().unwrap_or(0);
        m.send(h, from, uid, Dest::Pid(to), mtype, tag, 0, data)
    }

    /// Receives with no filters into a 1KB buffer.
    fn recv(m: &mut Msg, pid: i32) -> Result<(RecvInfo, Vec<u8>), i32> {
        recv_f(m, pid, 0, 0, 0, 0)
    }

    fn recv_f(m: &mut Msg, pid: i32, flags: u32, sender: i32, mtype: u32, tag: u32) -> Result<(RecvInfo, Vec<u8>), i32> {
        let mut buf = vec![0u8; MAX_PAYLOAD];
        let mut info = RecvInfo::default();
        m.recv(pid, flags, sender, mtype, tag, &mut buf, &mut info)?;
        buf.truncate(info.len as usize);
        Ok((info, buf))
    }

    // ---- inbox lifetime -------------------------------------------------

    #[test]
    fn inbox_open_close_and_validation() {
        let (mut m, mut h) = setup();
        assert_eq!(m.open(1, ACCEPT_ANY, 0), Ok((QUEUE_DEPTH as u32, MAX_PAYLOAD as u32)));
        assert_eq!(m.open(1, ACCEPT_ANY, 0), Err(EEXIST));
        assert_eq!(m.open(2, 3, 0), Err(EINVAL), "unknown policy");
        assert_eq!(m.open(2, ACCEPT_ANY, 1), Err(EINVAL), "flags must be 0");
        assert_eq!(m.open(0, ACCEPT_ANY, 0), Err(EINVAL));
        assert_eq!(m.open(-5, ACCEPT_ANY, 0), Err(EINVAL));
        assert_eq!(m.close(2), Err(EBADF), "closing an inbox that is not open");
        // a process that has not opened an inbox cannot be sent to
        assert_eq!(send(&mut m, &mut h, 1, 2, 1, 0, b"x"), Err(ENOENT));
        open(&mut m, 2);
        assert_eq!(send(&mut m, &mut h, 1, 2, 1, 0, b"x"), Ok(()));
        // closing drops the queue; the pid can be sent to no longer, and may reopen
        assert_eq!(m.close(2), Ok(()));
        assert_eq!(send(&mut m, &mut h, 1, 2, 1, 0, b"x"), Err(ENOENT));
        open(&mut m, 2);
        assert_eq!(recv(&mut m, 2), Err(EAGAIN), "a reopened inbox starts empty");
        m.check_counts().unwrap();
    }

    #[test]
    fn the_inbox_table_is_bounded_and_close_frees_a_slot() {
        let (mut m, mut h) = setup();
        for p in 1..=MAX_INBOXES as i32 {
            h.live.insert(p);
            m.open(p, ACCEPT_ANY, 0).unwrap();
        }
        h.live.insert(1000);
        assert_eq!(m.open(1000, ACCEPT_ANY, 0), Err(ENOSPC));
        m.close(7).unwrap();
        assert_eq!(m.open(1000, ACCEPT_ANY, 0).map(|_| ()), Ok(()));
        m.check_counts().unwrap();
    }

    // ---- framing, identity, validation ---------------------------------------

    #[test]
    fn one_send_is_exactly_one_receive_whatever_the_size() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        let big: Vec<u8> = (0..MAX_PAYLOAD).map(|i| (i * 7 + 1) as u8).collect();
        send(&mut m, &mut h, 1, 2, 5, 0, b"").unwrap();
        send(&mut m, &mut h, 1, 2, 5, 0, b"a").unwrap();
        send(&mut m, &mut h, 1, 2, 5, 0, b"hello world").unwrap();
        send(&mut m, &mut h, 1, 2, 5, 0, &big).unwrap();
        // unlike a byte stream, nothing merges and nothing needs delimiting
        assert_eq!(recv(&mut m, 2).unwrap().1, b"");
        assert_eq!(recv(&mut m, 2).unwrap().1, b"a");
        assert_eq!(recv(&mut m, 2).unwrap().1, b"hello world");
        assert_eq!(recv(&mut m, 2).unwrap().1, big);
        assert_eq!(recv(&mut m, 2), Err(EAGAIN));
    }

    #[test]
    fn the_kernel_stamps_who_sent_it_and_when() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        h.uids.insert(5, 777);
        h.tick = 4242;
        m.send(&mut h, 5, 777, Dest::Pid(2), 9, 31337, 0, b"xyz").unwrap();
        let (i, d) = recv(&mut m, 2).unwrap();
        assert_eq!((i.sender, i.sender_uid, i.mtype, i.tag, i.len, i.tick, i.pending), (5, 777, 9, 31337, 3, 4242, 0));
        assert_eq!(d, b"xyz");
    }

    #[test]
    fn send_validation() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        assert_eq!(send(&mut m, &mut h, 1, 2, 0, 0, b"x"), Err(EINVAL), "type 0 is reserved for 'any' in receive filters");
        assert_eq!(m.send(&mut h, 1, 100, Dest::Pid(2), 1, 0, 1, b"x"), Err(EINVAL), "flags must be 0");
        assert_eq!(send(&mut m, &mut h, 1, 2, 1, 0, &vec![0u8; MAX_PAYLOAD + 1]), Err(E2BIG));
        assert_eq!(send(&mut m, &mut h, 1, 2, 1, 0, &vec![0u8; MAX_PAYLOAD]), Ok(()));
        assert_eq!(send(&mut m, &mut h, 1, 0, 1, 0, b"x"), Err(EINVAL));
        assert_eq!(send(&mut m, &mut h, 1, -3, 1, 0, b"x"), Err(EINVAL));
        assert_eq!(send(&mut m, &mut h, 1, 999, 1, 0, b"x"), Err(ESRCH), "no such process");
        assert_eq!(send(&mut m, &mut h, 1, 3, 1, 0, b"x"), Err(ENOENT), "a live process with no inbox");
        for bad in [&b""[..], b"has space", b"slash/y", b"nul\0inside", b"tab\t", &[b'a'; 32][..], "caf\u{e9}".as_bytes()] {
            assert_eq!(m.send(&mut h, 1, 100, Dest::Name(bad), 1, 0, 0, b"x"), Err(EINVAL), "name {:?}", bad);
        }
        assert_eq!(m.send(&mut h, 1, 100, Dest::Name(b"nobody"), 1, 0, 0, b"x"), Err(ENOENT));
        // the 31-character name limit is inclusive
        let max = [b'z'; NAME_MAX];
        assert_eq!(m.send(&mut h, 1, 100, Dest::Name(&max), 1, 0, 0, b"x"), Err(ENOENT));
        // nothing was queued by any of the failures except the one 1KB message
        assert_eq!(m.stats().1, 1);
        m.check_counts().unwrap();
    }

    // ---- ordering, capacity, fairness ---------------------------------------------

    #[test]
    fn delivery_is_first_in_first_out_even_after_partial_draining() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        // several senders: any ONE of them is capped at MAX_PER_SENDER slots
        for i in 0..10u8 {
            send(&mut m, &mut h, 1 + (i % 2) as i32, 2, 1, 0, &[i]).unwrap();
        }
        for i in 0..4u8 {
            assert_eq!(recv(&mut m, 2).unwrap().1, vec![i]);
        }
        for i in 10..16u8 {
            send(&mut m, &mut h, 3 + (i % 2) as i32, 2, 1, 0, &[i]).unwrap();
        }
        // slots freed from the middle are reused, but order is by arrival, not by slot
        for i in 4..16u8 {
            assert_eq!(recv(&mut m, 2).unwrap().1, vec![i], "message {}", i);
        }
        assert_eq!(recv(&mut m, 2), Err(EAGAIN));
    }

    #[test]
    fn a_full_queue_says_again_and_one_sender_cannot_take_more_than_its_share() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        // sender 1 floods: it may hold 8 of the 16 slots, no more
        for i in 0..MAX_PER_SENDER {
            send(&mut m, &mut h, 1, 2, 1, 0, &[i as u8]).unwrap();
        }
        assert_eq!(send(&mut m, &mut h, 1, 2, 1, 0, b"x"), Err(EAGAIN), "over its share, with room in the queue");
        assert_eq!(m.stats().1, MAX_PER_SENDER, "the refused message was not queued");
        // ...which is what keeps everyone else able to reach this service
        for i in 0..MAX_PER_SENDER {
            send(&mut m, &mut h, 3, 2, 1, 0, &[i as u8]).unwrap();
        }
        assert_eq!(send(&mut m, &mut h, 4, 2, 1, 0, b"x"), Err(EAGAIN), "now the whole queue is full");
        assert_eq!(send(&mut m, &mut h, 3, 2, 1, 0, b"x"), Err(EAGAIN));
        // taking one of sender 1's messages gives sender 1 its slot back, and only its
        let (i, _) = recv(&mut m, 2).unwrap();
        assert_eq!(i.sender, 1);
        assert_eq!(send(&mut m, &mut h, 3, 2, 1, 0, b"x"), Err(EAGAIN), "the freed slot is not sender 3's to take (it is at its share)");
        assert_eq!(send(&mut m, &mut h, 1, 2, 1, 0, b"x"), Ok(()));
        assert_eq!(m.stats().1, QUEUE_DEPTH as u32);
        assert_eq!(m.stat(&mut h, 2).unwrap().refused, 4, "refusals are counted");
        m.check_counts().unwrap();
    }

    // ---- receive semantics --------------------------------------------------------------

    #[test]
    fn receive_never_truncates_never_blocks_and_peek_does_not_consume() {
        let (mut m, mut h) = setup();
        assert_eq!(recv(&mut m, 2), Err(EBADF), "no inbox");
        open(&mut m, 2);
        assert_eq!(recv(&mut m, 2), Err(EAGAIN), "empty");
        send(&mut m, &mut h, 1, 2, 6, 77, b"0123456789").unwrap();
        send(&mut m, &mut h, 1, 2, 6, 78, b"second").unwrap();

        // a buffer that is too small: E2BIG, the message stays, and the size is reported
        let mut small = [0u8; 4];
        let mut info = RecvInfo::default();
        assert_eq!(m.recv(2, 0, 0, 0, 0, &mut small, &mut info), Err(E2BIG));
        assert_eq!((info.len, info.mtype, info.tag, info.sender), (10, 6, 77, 1));
        assert_eq!(m.stats().1, 2, "E2BIG must not consume");
        // a zero-length buffer is the way to ask 'how big is the next message?'
        let mut none: [u8; 0] = [];
        assert_eq!(m.recv(2, RECV_PEEK, 0, 0, 0, &mut none, &mut info), Err(E2BIG));
        assert_eq!(info.len, 10);

        // peek returns the message and leaves it
        let (pi, pd) = recv_f(&mut m, 2, RECV_PEEK, 0, 0, 0).unwrap();
        assert_eq!(pd, b"0123456789");
        assert_eq!(pi.pending, 2, "peek reports the queue including this message");
        assert_eq!(m.stats().1, 2);
        // a real receive removes it and reports what is left
        let (ri, rd) = recv(&mut m, 2).unwrap();
        assert_eq!(rd, b"0123456789");
        assert_eq!(ri.pending, 1);
        assert_eq!(m.stat(&mut h, 2).unwrap().delivered, 1, "peeks and E2BIG are not deliveries");
        assert_eq!(recv_f(&mut m, 2, 4, 0, 0, 0), Err(EINVAL), "unknown flag");
        m.check_counts().unwrap();
    }

    #[test]
    fn selective_receive_takes_the_oldest_match_and_leaves_the_rest() {
        let (mut m, mut h) = setup();
        open(&mut m, 9);
        //            from  type tag
        send(&mut m, &mut h, 1, 9, 1, 10, b"a").unwrap();
        send(&mut m, &mut h, 2, 9, 2, 20, b"b").unwrap();
        send(&mut m, &mut h, 3, 9, 1, 20, b"c").unwrap();
        send(&mut m, &mut h, 1, 9, 2, 10, b"d").unwrap();
        send(&mut m, &mut h, 2, 9, 1, 0, b"e").unwrap();
        // by sender
        assert_eq!(recv_f(&mut m, 9, 0, 3, 0, 0).unwrap().1, b"c");
        // by type: oldest of type 2 is 'b'
        assert_eq!(recv_f(&mut m, 9, 0, 0, 2, 0).unwrap().1, b"b");
        // by tag (needs the flag)
        assert_eq!(recv_f(&mut m, 9, RECV_MATCH_TAG, 0, 0, 10).unwrap().1, b"a");
        // tag 0 is a real tag when the flag is given...
        assert_eq!(recv_f(&mut m, 9, RECV_MATCH_TAG, 0, 0, 0).unwrap().1, b"e");
        // ...and a filter that matches nothing is EAGAIN, not an error about the queue
        assert_eq!(recv_f(&mut m, 9, 0, 3, 0, 0), Err(EAGAIN));
        assert_eq!(recv_f(&mut m, 9, 0, 1, 1, 0), Err(EAGAIN));
        // whatever is left is still there, in order
        assert_eq!(recv(&mut m, 9).unwrap().1, b"d");
        assert_eq!(recv(&mut m, 9), Err(EAGAIN));
        // combinations: sender AND type AND tag
        send(&mut m, &mut h, 1, 9, 5, 1, b"x").unwrap();
        send(&mut m, &mut h, 1, 9, 5, 2, b"y").unwrap();
        send(&mut m, &mut h, 2, 9, 5, 2, b"z").unwrap();
        assert_eq!(recv_f(&mut m, 9, RECV_MATCH_TAG, 2, 5, 2).unwrap().1, b"z");
        assert_eq!(recv_f(&mut m, 9, RECV_MATCH_TAG, 1, 5, 2).unwrap().1, b"y");
        m.check_counts().unwrap();
    }

    // ---- policy ---------------------------------------------------------------------------

    #[test]
    fn accept_same_uid_admits_the_same_user_and_root_only() {
        let (mut m, mut h) = setup();
        h.uids.insert(2, 500);
        h.uids.insert(3, 500);
        h.uids.insert(4, 600);
        h.uids.insert(5, 0);
        m.open(2, ACCEPT_SAME_UID, 0).unwrap();
        assert_eq!(send(&mut m, &mut h, 3, 2, 1, 0, b"same"), Ok(()));
        assert_eq!(send(&mut m, &mut h, 4, 2, 1, 0, b"other"), Err(EACCES));
        assert_eq!(send(&mut m, &mut h, 5, 2, 1, 0, b"root"), Ok(()));
        assert_eq!(m.stats().1, 2, "a refused message must not occupy a slot");
        // the receiver's uid is read when the message is SENT: a uid change is honoured
        h.uids.insert(2, 600);
        assert_eq!(send(&mut m, &mut h, 4, 2, 1, 0, b"now same"), Ok(()));
        assert_eq!(send(&mut m, &mut h, 3, 2, 1, 0, b"now other"), Err(EACCES));
        assert_eq!(m.stat(&mut h, 2).unwrap().refused, 2);
    }

    #[test]
    fn accept_allowlist_admits_only_listed_pids_and_the_list_is_bounded() {
        let (mut m, mut h) = setup();
        m.open(2, ACCEPT_ALLOWLIST, 0).unwrap();
        assert_eq!(send(&mut m, &mut h, 3, 2, 1, 0, b"x"), Err(EACCES), "an empty allowlist admits nobody");
        m.allow(2, 3, true).unwrap();
        assert_eq!(send(&mut m, &mut h, 3, 2, 1, 0, b"x"), Ok(()));
        assert_eq!(send(&mut m, &mut h, 4, 2, 1, 0, b"x"), Err(EACCES));
        m.allow(2, 3, true).unwrap(); // idempotent: no second entry
        m.allow(2, 3, false).unwrap();
        assert_eq!(send(&mut m, &mut h, 3, 2, 1, 0, b"x"), Err(EACCES), "denied again");
        m.allow(2, 3, false).unwrap(); // removing an absent pid is harmless
        // the list holds MAX_ALLOW entries
        for p in 20..20 + MAX_ALLOW as i32 {
            m.allow(2, p, true).unwrap();
        }
        assert_eq!(m.allow(2, 99, true), Err(ENOSPC));
        assert_eq!(m.allow(2, 0, true), Err(EINVAL));
        assert_eq!(m.allow(2, -1, false), Err(EINVAL));
        assert_eq!(m.allow(8, 1, true), Err(EBADF), "no inbox, no list");
        // the policy can be changed at runtime, and the list survives a switch away and back
        m.set_policy(2, ACCEPT_ANY).unwrap();
        assert_eq!(send(&mut m, &mut h, 4, 2, 1, 0, b"x"), Ok(()));
        m.set_policy(2, ACCEPT_ALLOWLIST).unwrap();
        assert_eq!(send(&mut m, &mut h, 4, 2, 1, 0, b"x"), Err(EACCES));
        h.live.insert(20);
        assert_eq!(send(&mut m, &mut h, 20, 2, 1, 0, b"x"), Ok(()));
        assert_eq!(m.set_policy(2, 3), Err(EINVAL));
        assert_eq!(m.set_policy(8, 0), Err(EBADF));
        m.check_counts().unwrap();
    }

    // ---- services ---------------------------------------------------------------------------

    #[test]
    fn service_names_register_lookup_unregister_and_deliver() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        open(&mut m, 3);
        assert_eq!(m.service(&mut h, 4, SVC_REGISTER, b"x"), Err(EBADF), "a name nobody can send to is a bug");
        assert_eq!(m.service(&mut h, 2, SVC_LOOKUP, b"editor"), Err(ENOENT));
        assert_eq!(m.service(&mut h, 2, SVC_REGISTER, b"editor"), Ok(0));
        assert_eq!(m.service(&mut h, 2, SVC_REGISTER, b"editor"), Ok(0), "re-registering your own name is a no-op");
        assert_eq!(m.service(&mut h, 3, SVC_REGISTER, b"editor"), Err(EEXIST));
        assert_eq!(m.service(&mut h, 3, SVC_LOOKUP, b"editor"), Ok(2));
        assert_eq!(m.service(&mut h, 9, SVC_LOOKUP, b"editor"), Ok(2), "anyone may look a name up");
        assert_eq!(m.service(&mut h, 3, SVC_UNREGISTER, b"editor"), Err(EPERM), "only the owner");
        assert_eq!(m.service(&mut h, 2, SVC_UNREGISTER, b"nope"), Err(ENOENT));
        for bad in [&b""[..], b"a b", b"a/b", &[b'q'; 32][..]] {
            for op in [SVC_REGISTER, SVC_UNREGISTER, SVC_LOOKUP] {
                assert_eq!(m.service(&mut h, 2, op, bad), Err(EINVAL), "op {} name {:?}", op, bad);
            }
        }
        assert_eq!(m.service(&mut h, 2, 4, b"editor"), Err(EINVAL), "unknown op");
        // a name is an address: sending to it reaches the owner
        m.send(&mut h, 3, 100, Dest::Name(b"editor"), 1, 0, 0, b"open /tmp/x").unwrap();
        let (i, d) = recv(&mut m, 2).unwrap();
        assert_eq!((i.sender, d.as_slice()), (3, &b"open /tmp/x"[..]));
        // names are case-sensitive and exact
        assert_eq!(m.service(&mut h, 3, SVC_LOOKUP, b"Editor"), Err(ENOENT));
        assert_eq!(m.service(&mut h, 2, SVC_UNREGISTER, b"editor"), Ok(0));
        assert_eq!(m.send(&mut h, 3, 100, Dest::Name(b"editor"), 1, 0, 0, b"x"), Err(ENOENT));
        // it can now be taken by someone else
        assert_eq!(m.service(&mut h, 3, SVC_REGISTER, b"editor"), Ok(0));
        m.check_counts().unwrap();
    }

    #[test]
    fn service_limits_per_process_and_in_total() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        for i in 0..MAX_NAMES_PER_PROC {
            assert_eq!(m.service(&mut h, 2, SVC_REGISTER, format!("svc{}", i).as_bytes()), Ok(0));
        }
        assert_eq!(m.service(&mut h, 2, SVC_REGISTER, b"onetoomany"), Err(ENOSPC));
        // total table: 8 processes x 4 names = the 32-name table
        let (mut m, mut h) = setup();
        for p in 1..=8 {
            open(&mut m, p);
            for i in 0..MAX_NAMES_PER_PROC {
                m.service(&mut h, p, SVC_REGISTER, format!("p{}n{}", p, i).as_bytes()).unwrap();
            }
        }
        open(&mut m, 9);
        assert_eq!(m.service(&mut h, 9, SVC_REGISTER, b"nospace"), Err(ENOSPC), "name table full");
        m.service(&mut h, 3, SVC_UNREGISTER, b"p3n0").unwrap();
        assert_eq!(m.service(&mut h, 9, SVC_REGISTER, b"nospace"), Ok(0), "unregistering frees an entry");
        m.check_counts().unwrap();
    }

    #[test]
    fn a_name_whose_owner_vanished_without_cleanup_is_not_a_black_hole() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        open(&mut m, 3);
        m.service(&mut h, 2, SVC_REGISTER, b"ghost").unwrap();
        h.live.remove(&2); // died, and the exit hook never ran
        assert_eq!(m.send(&mut h, 3, 100, Dest::Name(b"ghost"), 1, 0, 0, b"x"), Err(ENOENT), "no delivery into a void");
        assert_eq!(m.service(&mut h, 3, SVC_LOOKUP, b"ghost"), Err(ENOENT));
        assert_eq!(m.service(&mut h, 3, SVC_REGISTER, b"ghost"), Ok(0), "the name was freed");
        // and registering over a stale name takes it over directly
        m.service(&mut h, 3, SVC_REGISTER, b"second").unwrap();
        h.live.remove(&3);
        h.live.insert(2);
        open_again(&mut m, 4);
        assert_eq!(m.service(&mut h, 4, SVC_REGISTER, b"second"), Ok(0), "takeover of a stale name");
        assert_eq!(m.service(&mut h, 2, SVC_LOOKUP, b"second"), Ok(4));
    }

    #[test]
    fn lookup_is_itself_not_fooled_by_a_dead_owner() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        open(&mut m, 3);
        m.service(&mut h, 2, SVC_REGISTER, b"gone").unwrap();
        assert_eq!(m.service(&mut h, 3, SVC_LOOKUP, b"gone"), Ok(2));
        h.live.remove(&2); // died; the exit hook never ran
        // LOOKUP must be the first thing to touch the name (nothing has purged it yet)
        assert_eq!(m.service(&mut h, 3, SVC_LOOKUP, b"gone"), Err(ENOENT), "a lookup must not hand out a dead process's pid");
        assert_eq!(m.stats().2, 0, "and the stale name is purged as a side effect");
    }

    fn open_again(m: &mut Msg, pid: i32) {
        m.open(pid, ACCEPT_ANY, 0).unwrap();
    }

    // ---- process exit ------------------------------------------------------------------------------

    #[test]
    fn an_exiting_process_loses_its_inbox_its_names_and_its_place_in_allowlists() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        m.open(3, ACCEPT_ALLOWLIST, 0).unwrap();
        m.allow(3, 2, true).unwrap();
        m.service(&mut h, 2, SVC_REGISTER, b"svc-a").unwrap();
        m.service(&mut h, 2, SVC_REGISTER, b"svc-b").unwrap();
        send(&mut m, &mut h, 3, 2, 1, 0, b"queued for 2").unwrap();
        send(&mut m, &mut h, 2, 3, 1, 0, b"sent by 2, queued for 3").unwrap();
        assert_eq!(m.stats(), (2, 2, 2));

        m.process_exit(2);
        h.live.remove(&2);
        assert_eq!(m.stats(), (1, 1, 0), "inbox, its queue and both names are gone");
        assert_eq!(send(&mut m, &mut h, 3, 2, 1, 0, b"x"), Err(ESRCH));
        assert_eq!(m.service(&mut h, 3, SVC_LOOKUP, b"svc-a"), Err(ENOENT));
        // a message the exiting process had ALREADY sent stays deliverable
        let (i, d) = recv(&mut m, 3).unwrap();
        assert_eq!((i.sender, d.as_slice()), (2, &b"sent by 2, queued for 3"[..]));
        // 3's allowlist no longer contains 2
        h.live.insert(2);
        assert_eq!(send(&mut m, &mut h, 2, 3, 1, 0, b"x"), Err(EACCES), "a reused pid must not inherit a stale grant");
        // the names can be re-registered, exit is idempotent, and exiting a stranger is harmless
        open(&mut m, 2);
        assert_eq!(m.service(&mut h, 2, SVC_REGISTER, b"svc-a"), Ok(0));
        m.process_exit(2);
        m.process_exit(2);
        m.process_exit(777);
        m.check_counts().unwrap();
    }

    // ---- stat ---------------------------------------------------------------------------------------------

    #[test]
    fn stat_reports_the_inbox_and_the_clock() {
        let (mut m, mut h) = setup();
        assert_eq!(m.stat(&mut h, 2), Err(EBADF));
        m.open(2, ACCEPT_SAME_UID, 0).unwrap();
        let s0 = m.stat(&mut h, 2).unwrap();
        assert_eq!((s0.queued, s0.capacity, s0.max_payload, s0.policy, s0.tick_hz, s0.next_len), (0, 16, 1024, 1, 100, 0));
        h.tick = 5000;
        send(&mut m, &mut h, 3, 2, 8, 0, b"first!").unwrap();
        send(&mut m, &mut h, 4, 2, 9, 0, b"x").unwrap();
        h.tick = 5010;
        let s = m.stat(&mut h, 2).unwrap();
        assert_eq!((s.queued, s.next_len, s.next_type, s.next_sender, s.now_tick), (2, 6, 8, 3, 5010));
        recv(&mut m, 2).unwrap();
        let s = m.stat(&mut h, 2).unwrap();
        assert_eq!((s.queued, s.next_len, s.next_type, s.next_sender, s.delivered), (1, 1, 9, 4, 1));
    }

    // ---- self-checking has teeth -----------------------------------------------------------------------------

    #[test]
    fn check_counts_catches_corrupted_bookkeeping() {
        let (mut m, mut h) = setup();
        open(&mut m, 2);
        m.service(&mut h, 2, SVC_REGISTER, b"svc").unwrap();
        send(&mut m, &mut h, 1, 2, 1, 0, b"x").unwrap();
        send(&mut m, &mut h, 1, 2, 1, 0, b"y").unwrap();
        assert!(m.check_counts().is_ok());
        let ii = m.find_inbox(2).unwrap();

        m.inboxes[ii].count += 1;
        assert!(m.check_counts().is_err(), "count off by one");
        m.inboxes[ii].count -= 1;

        let u = m.inboxes[ii].slots.iter().position(|s| s.used).unwrap();
        let (a, b) = (u, m.inboxes[ii].slots.iter().rposition(|s| s.used).unwrap());
        let saved = m.inboxes[ii].slots[b].seq;
        m.inboxes[ii].slots[b].seq = m.inboxes[ii].slots[a].seq;
        assert!(m.check_counts().is_err(), "two messages with one arrival number");
        m.inboxes[ii].slots[b].seq = saved;

        m.inboxes[ii].slots[u].mtype = 0;
        assert!(m.check_counts().is_err(), "a queued message with type 0");
        m.inboxes[ii].slots[u].mtype = 1;

        m.inboxes[ii].policy = 9;
        assert!(m.check_counts().is_err(), "an invalid policy");
        m.inboxes[ii].policy = 0;

        let si = m.services.iter().position(|s| s.in_use).unwrap();
        m.services[si].name[0] = b' ';
        assert!(m.check_counts().is_err(), "an invalid service name");
        m.services[si].name[0] = b's';

        m.services[si].pid = 5; // owner has no inbox
        assert!(m.check_counts().is_err(), "a service owned by a process with no inbox");
        m.services[si].pid = 2;

        m.inboxes[ii].allow = [4, 4, 0, 0, 0, 0, 0, 0];
        assert!(m.check_counts().is_err(), "duplicate allowlist entries");
        m.inboxes[ii].allow = [0; MAX_ALLOW];
        assert!(m.check_counts().is_ok(), "all repairs restore a clean state");
    }

    // ---- ABI header cross-check ----------------------------------------------------------------------------------

    fn abi(name: &str) -> u64 {
        let text = include_str!("../../userland/libc/include/nova_msg_abi.h");
        for line in text.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("#define ") {
                let mut it = rest.splitn(2, char::is_whitespace);
                if it.next() == Some(name) {
                    let mut val = it.next().unwrap_or("").trim();
                    if let Some(c) = val.find("/*") {
                        val = val[..c].trim();
                    }
                    let p = val.trim_end_matches('u').trim_end_matches('U');
                    return p.parse::<u64>().unwrap_or_else(|_| panic!("cannot parse {} = {:?}", name, val));
                }
            }
        }
        panic!("{} not found in nova_msg_abi.h", name);
    }

    #[test]
    fn the_c_abi_header_agrees_with_the_kernel_constants() {
        assert_eq!(abi("NOVA_MSG_MAX_PAYLOAD"), MAX_PAYLOAD as u64);
        assert_eq!(abi("NOVA_MSG_QUEUE_DEPTH"), QUEUE_DEPTH as u64);
        assert_eq!(abi("NOVA_MSG_MAX_PER_SENDER"), MAX_PER_SENDER as u64);
        assert_eq!(abi("NOVA_MSG_MAX_INBOXES"), MAX_INBOXES as u64);
        assert_eq!(abi("NOVA_MSG_MAX_SERVICES"), MAX_SERVICES as u64);
        assert_eq!(abi("NOVA_MSG_MAX_NAMES_PER_PROC"), MAX_NAMES_PER_PROC as u64);
        assert_eq!(abi("NOVA_MSG_MAX_ALLOW"), MAX_ALLOW as u64);
        assert_eq!(abi("NOVA_MSG_NAME_MAX"), NAME_MAX as u64);
        assert_eq!(abi("NOVA_MSG_TICK_HZ"), TICK_HZ as u64);
        assert_eq!(abi("NOVA_MSG_ACCEPT_ANY"), ACCEPT_ANY as u64);
        assert_eq!(abi("NOVA_MSG_ACCEPT_SAME_UID"), ACCEPT_SAME_UID as u64);
        assert_eq!(abi("NOVA_MSG_ACCEPT_ALLOWLIST"), ACCEPT_ALLOWLIST as u64);
        assert_eq!(abi("NOVA_MSG_RECV_PEEK"), RECV_PEEK as u64);
        assert_eq!(abi("NOVA_MSG_RECV_MATCH_TAG"), RECV_MATCH_TAG as u64);
        assert_eq!(abi("NOVA_MSG_SVC_REGISTER"), SVC_REGISTER as u64);
        assert_eq!(abi("NOVA_MSG_SVC_UNREGISTER"), SVC_UNREGISTER as u64);
        assert_eq!(abi("NOVA_MSG_SVC_LOOKUP"), SVC_LOOKUP as u64);
        assert_eq!(abi("NOVA_MSG_CTL_STAT"), CTL_STAT as u64);
        assert_eq!(abi("NOVA_MSG_CTL_ALLOW"), CTL_ALLOW as u64);
        assert_eq!(abi("NOVA_MSG_CTL_DENY"), CTL_DENY as u64);
        assert_eq!(abi("NOVA_MSG_CTL_POLICY"), CTL_POLICY as u64);
        for (name, v) in [("PERM", EPERM), ("NOENT", ENOENT), ("SRCH", ESRCH), ("2BIG", E2BIG), ("BADF", EBADF), ("AGAIN", EAGAIN),
                          ("ACCES", EACCES), ("FAULT", EFAULT), ("EXIST", EEXIST), ("INVAL", EINVAL), ("NOSPC", ENOSPC)] {
            assert_eq!(abi(&std::format!("NOVA_MSG_ERR_{}", name)), v as u64, "errno {}", name);
        }
        for (i, name) in ["OPEN", "CLOSE", "SEND", "RECV", "SERVICE", "CTL"].iter().enumerate() {
            assert_eq!(abi(&std::format!("NOVA_SYS_MSG_{}", name)), 60 + i as u64);
        }
    }

    // ---- the randomized, model-checked stress test -------------------------------------------------------------------

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn pick<T: Copy>(&mut self, v: &[T]) -> T {
            v[self.below(v.len() as u64) as usize]
        }
    }

    #[derive(Clone)]
    struct MMsg { sender: i32, uid: u32, mtype: u32, tag: u32, tick: u32, data: Vec<u8> }

    struct MIn { policy: u32, allow: Vec<i32>, q: Vec<MMsg>, delivered: u32, refused: u32 }

    /// An independent, deliberately simple restatement of the rules: queues
    /// are Vecs in arrival order, services a map. It predicts the exact
    /// outcome - result code, delivered bytes, sender/type/tag, counters -
    /// of every operation in the random run below.
    #[derive(Default)]
    struct Model {
        inboxes: BTreeMap<i32, MIn>,
        services: BTreeMap<Vec<u8>, i32>,
    }

    impl Model {
        fn names_of(&self, pid: i32) -> usize {
            self.services.values().filter(|&&p| p == pid).count()
        }
    }

    #[test]
    fn randomized_operations_agree_with_an_independent_model() {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let (mut m, mut h) = setup();
        let mut md = Model::default();
        let mut active: Vec<i32> = (1..=6).collect();
        for p in 1..=60 {
            h.live.insert(p);
            h.uids.insert(p, (p % 3) as u32); // pid%3 == 0 is root
        }
        let mut next_pid = 7;
        // processes that died WITHOUT their exit hook having run yet: their
        // inbox and names linger until the (delayed) cleanup, which is the
        // situation every liveness check in the module exists for
        let mut zombies: Vec<i32> = Vec::new();
        let names: [&[u8]; 8] = [b"editor", b"files", b"mixer", b"a.b-c_d", b"", b"bad name", b"x/y", b"w"];
        let mut compared_deliveries = 0u32;

        for step in 0..100_000 {
            h.tick = 1000 + step as u32;
            let pid = rng.pick(&active);
            let uid = h.uids[&pid];
            match rng.below(100) {
                // open
                0..=9 => {
                    let policy = rng.pick(&[0u32, 0, 0, 0, 1, 2, 3]);
                    let flags = if rng.below(20) == 0 { 1 } else { 0 };
                    let got = m.open(pid, policy, flags).map(|_| ());
                    let want = if flags != 0 || policy > 2 { Err(EINVAL) }
                        else if md.inboxes.contains_key(&pid) { Err(EEXIST) }
                        else if md.inboxes.len() >= MAX_INBOXES { Err(ENOSPC) }
                        else {
                            md.inboxes.insert(pid, MIn { policy, allow: vec![], q: vec![], delivered: 0, refused: 0 });
                            Ok(())
                        };
                    assert_eq!(got, want, "step {}: open", step);
                }
                // send
                10..=44 => {
                    let mtype = if rng.below(12) == 0 { 0 } else { 1 + rng.below(3) as u32 };
                    let tag = rng.below(3) as u32;
                    let flags = if rng.below(40) == 0 { 1 } else { 0 };
                    let len = match rng.below(20) { 0 => MAX_PAYLOAD + 1, 1 => MAX_PAYLOAD, 2..=4 => rng.below(200) as usize, _ => rng.below(12) as usize };
                    let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
                    let by_name = rng.below(3) == 0;
                    let (dest, dest_pid, dest_name): (Dest, i32, Vec<u8>) = if by_name {
                        let n = rng.pick(&names);
                        (Dest::Name(n), 0, n.to_vec())
                    } else {
                        let p = match rng.below(10) { 0 => 0, 1 => 999, 2 => -4, _ => rng.pick(&active) };
                        (Dest::Pid(p), p, vec![])
                    };
                    let got = m.send(&mut h, pid, uid, dest, mtype, tag, flags, &data);
                    // --- the oracle ---
                    let want: Result<(), i32> = (|| {
                        if flags != 0 || mtype == 0 { return Err(EINVAL); }
                        if len > MAX_PAYLOAD { return Err(E2BIG); }
                        let to = if by_name {
                            if !valid_name(&dest_name) { return Err(EINVAL); }
                            match md.services.get(&dest_name) {
                                None => return Err(ENOENT),
                                Some(&o) => {
                                    if !h.live.contains(&o) { md.services.remove(&dest_name); return Err(ENOENT); }
                                    o
                                }
                            }
                        } else {
                            if dest_pid <= 0 { return Err(EINVAL); }
                            dest_pid
                        };
                        if !h.live.contains(&to) { return Err(ESRCH); }
                        let ib = match md.inboxes.get_mut(&to) { Some(i) => i, None => return Err(ENOENT) };
                        match ib.policy {
                            ACCEPT_SAME_UID => {
                                let ru = h.uids[&to];
                                if uid != ru && uid != 0 { ib.refused += 1; return Err(EACCES); }
                            }
                            ACCEPT_ALLOWLIST => {
                                if !ib.allow.contains(&pid) { ib.refused += 1; return Err(EACCES); }
                            }
                            _ => {}
                        }
                        let mine = ib.q.iter().filter(|x| x.sender == pid).count() as u32;
                        if ib.q.len() >= QUEUE_DEPTH || mine >= MAX_PER_SENDER { ib.refused += 1; return Err(EAGAIN); }
                        ib.q.push(MMsg { sender: pid, uid, mtype, tag, tick: h.tick, data: data.clone() });
                        Ok(())
                    })();
                    assert_eq!(got, want, "step {}: send {:?} -> {:?}", step, pid, if by_name { String::from_utf8_lossy(&dest_name).to_string() } else { dest_pid.to_string() });
                }
                // receive / peek, with filters
                45..=79 => {
                    let flags = rng.pick(&[0, 0, 0, 0, 0, 0, RECV_PEEK, RECV_PEEK, RECV_MATCH_TAG, RECV_PEEK | RECV_MATCH_TAG, 4]);
                    let ms = if rng.below(8) == 0 { rng.pick(&active) } else { 0 };
                    let mt = if rng.below(8) == 0 { 1 + rng.below(3) as u32 } else { 0 };
                    let mtag = rng.below(3) as u32;
                    let cap = rng.pick(&[0usize, 3, MAX_PAYLOAD, MAX_PAYLOAD, MAX_PAYLOAD, MAX_PAYLOAD, MAX_PAYLOAD]);
                    let mut buf = vec![0xEEu8; cap];
                    let mut info = RecvInfo::default();
                    let got = m.recv(pid, flags, ms, mt, mtag, &mut buf, &mut info);
                    let want: Result<Option<MMsg>, i32> = (|| {
                        if flags & !(RECV_PEEK | RECV_MATCH_TAG) != 0 { return Err(EINVAL); }
                        let ib = match md.inboxes.get_mut(&pid) { Some(i) => i, None => return Err(EBADF) };
                        let idx = ib.q.iter().position(|x| (ms == 0 || x.sender == ms) && (mt == 0 || x.mtype == mt) && (flags & RECV_MATCH_TAG == 0 || x.tag == mtag));
                        let idx = match idx { Some(i) => i, None => return Err(EAGAIN) };
                        let msg = ib.q[idx].clone();
                        if msg.data.len() > cap { return Ok(None); } // E2BIG, info still filled
                        if flags & RECV_PEEK == 0 { ib.q.remove(idx); ib.delivered += 1; }
                        Ok(Some(msg))
                    })();
                    match (got, want) {
                        (Ok(()), Ok(Some(w))) => {
                            compared_deliveries += 1;
                            assert_eq!(&buf[..info.len as usize], &w.data[..], "step {}: payload", step);
                            assert_eq!((info.sender, info.sender_uid, info.mtype, info.tag, info.len, info.tick),
                                       (w.sender, w.uid, w.mtype, w.tag, w.data.len() as u32, w.tick), "step {}: metadata", step);
                            let left = md.inboxes[&pid].q.len() as u32 + if flags & RECV_PEEK != 0 { 0 } else { 0 };
                            assert_eq!(info.pending, left, "step {}: pending", step);
                        }
                        (Err(E2BIG), Ok(None)) => {
                            assert!(info.len as usize > cap, "step {}: E2BIG must report a length that does not fit", step);
                        }
                        (Err(a), Err(b)) => assert_eq!(a, b, "step {}: recv", step),
                        (g, w) => panic!("step {}: recv returned {:?}, oracle expected {:?}", step, g, w.map(|o| o.map(|x| x.data.len()))),
                    }
                }
                // services
                80..=85 => {
                    let name = rng.pick(&names);
                    let op = rng.pick(&[1u32, 1, 2, 3, 3, 4]);
                    let got = m.service(&mut h, pid, op, name);
                    let want: Result<i32, i32> = (|| {
                        if !valid_name(name) { return Err(EINVAL); }
                        match op {
                            SVC_REGISTER => {
                                if !md.inboxes.contains_key(&pid) { return Err(EBADF); }
                                if let Some(&o) = md.services.get(name) {
                                    if o == pid { return Ok(0); }
                                    if h.live.contains(&o) { return Err(EEXIST); }
                                }
                                if md.names_of(pid) >= MAX_NAMES_PER_PROC as usize { return Err(ENOSPC); }
                                if !md.services.contains_key(name) && md.services.len() >= MAX_SERVICES { return Err(ENOSPC); }
                                md.services.insert(name.to_vec(), pid);
                                Ok(0)
                            }
                            SVC_UNREGISTER => match md.services.get(name) {
                                None => Err(ENOENT),
                                Some(&o) if o != pid => Err(EPERM),
                                Some(_) => { md.services.remove(name); Ok(0) }
                            },
                            SVC_LOOKUP => match md.services.get(name) {
                                None => Err(ENOENT),
                                Some(&o) if h.live.contains(&o) => Ok(o),
                                Some(_) => { md.services.remove(name); Err(ENOENT) }
                            },
                            _ => Err(EINVAL),
                        }
                    })();
                    assert_eq!(got, want, "step {}: service op {} {:?}", step, op, String::from_utf8_lossy(name));
                }
                // control: allow / deny / policy
                86..=90 => {
                    let which = rng.below(3);
                    // mostly pids that really send in this run (so allowlists get hit),
                    // sometimes junk and filler (so the list fills up and ENOSPC happens)
                    let target = if rng.below(2) == 0 { rng.pick(&active) } else { rng.pick(&[0, -1, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49]) };
                    let policy = rng.below(4) as u32;
                    match which {
                        0 | 1 => {
                            let allow = which == 0;
                            let got = m.allow(pid, target, allow);
                            let want = (|| {
                                let ib = match md.inboxes.get_mut(&pid) { Some(i) => i, None => return Err(EBADF) };
                                if target <= 0 { return Err(EINVAL); }
                                if allow {
                                    if ib.allow.contains(&target) { return Ok(()); }
                                    if ib.allow.len() >= MAX_ALLOW { return Err(ENOSPC); }
                                    ib.allow.push(target);
                                } else {
                                    ib.allow.retain(|&a| a != target);
                                }
                                Ok(())
                            })();
                            assert_eq!(got, want, "step {}: allow/deny", step);
                        }
                        _ => {
                            let got = m.set_policy(pid, policy);
                            let want = (|| {
                                let ib = match md.inboxes.get_mut(&pid) { Some(i) => i, None => return Err(EBADF) };
                                if policy > 2 { return Err(EINVAL); }
                                ib.policy = policy;
                                Ok(())
                            })();
                            assert_eq!(got, want, "step {}: set_policy", step);
                        }
                    }
                }
                // stat
                91..=93 => {
                    let got = m.stat(&mut h, pid);
                    match md.inboxes.get(&pid) {
                        None => assert_eq!(got, Err(EBADF), "step {}: stat without inbox", step),
                        Some(ib) => {
                            let s = got.unwrap();
                            assert_eq!((s.queued, s.policy, s.delivered, s.refused), (ib.q.len() as u32, ib.policy, ib.delivered, ib.refused), "step {}: stat", step);
                            match ib.q.first() {
                                Some(f) => assert_eq!((s.next_len, s.next_type, s.next_sender), (f.data.len() as u32, f.mtype, f.sender), "step {}: stat next", step),
                                None => assert_eq!((s.next_len, s.next_type, s.next_sender), (0, 0, 0)),
                            }
                        }
                    }
                }
                // close
                94..=95 => {
                    let got = m.close(pid);
                    if md.inboxes.remove(&pid).is_some() {
                        md.services.retain(|_, &mut o| o != pid);
                        for ib in md.inboxes.values_mut() { ib.allow.retain(|&a| a != pid); }
                        assert_eq!(got, Ok(()), "step {}: close", step);
                    } else {
                        assert_eq!(got, Err(EBADF), "step {}: close without inbox", step);
                    }
                }
                // exit (or a silent death), and a new process appears
                _ if active.len() > 3 => {
                    if rng.below(3) == 0 {
                        zombies.push(pid); // dies; cleanup comes later (below)
                    } else {
                        m.process_exit(pid);
                        md.inboxes.remove(&pid);
                        md.services.retain(|_, &mut o| o != pid);
                        for ib in md.inboxes.values_mut() { ib.allow.retain(|&a| a != pid); }
                    }
                    h.live.remove(&pid);
                    active.retain(|&p| p != pid);
                    h.live.insert(next_pid);
                    h.uids.insert(next_pid, (next_pid % 3) as u32);
                    active.push(next_pid);
                    next_pid += 1;
                }
                _ => {}
            }
            // the delayed exit hook of a silently-dead process finally runs
            if !zombies.is_empty() && rng.below(40) == 0 {
                let z = zombies.remove(0);
                m.process_exit(z);
                md.inboxes.remove(&z);
                md.services.retain(|_, &mut o| o != z);
                for ib in md.inboxes.values_mut() { ib.allow.retain(|&a| a != z); }
            }
            m.check_counts().unwrap_or_else(|e| panic!("step {}: {}", step, e));
            // the books agree with the model about what is queued where
            if step % 50 == 0 {
                let (ib, q, sv) = m.stats();
                assert_eq!(ib as usize, md.inboxes.len(), "step {}: inbox count", step);
                assert_eq!(q as usize, md.inboxes.values().map(|i| i.q.len()).sum::<usize>(), "step {}: queued total", step);
                assert_eq!(sv as usize, md.services.len(), "step {}: service count", step);
            }
        }
        eprintln!("oracle compared {} deliveries", compared_deliveries);
        assert!(compared_deliveries > 1500, "too few deliveries were compared ({}) for the run to mean anything", compared_deliveries);
        // everything exits: nothing remains
        for p in active.iter().chain(zombies.iter()).copied().collect::<Vec<_>>() {
            m.process_exit(p);
        }
        assert_eq!(m.stats(), (0, 0, 0));
        m.check_counts().unwrap();
    }
}
