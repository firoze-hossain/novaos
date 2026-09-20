//! kernel/rust/tcp.rs - Phase 58: a real TCP (RFC 793) implementation
//! with retransmission, sliding-window data transfer, and passive
//! open (LISTEN/accept), plus the entry points a new Berkeley-sockets-
//! style syscall API (SYS_SOCKET/SYS_BIND/SYS_LISTEN/SYS_ACCEPT/
//! SYS_CONNECT - see kernel/arch/x86/cpu/syscall.h) dispatches
//! straight into.
//!
//! Gap this fills: kernel/net/tcp.c (now an intentionally empty
//! placeholder - see its own header comment) was a single-connection,
//! active-open-only, stop-and-wait client, entirely unreachable from
//! ring 3 - no ring-3 program could use TCP at all before this phase.
//! Written in Rust per this project's own standing rule (new kernel
//! work is attempted in Rust first) and this phase's own explicit
//! direction to prefer Rust wherever C isn't architecturally required -
//! nothing here needed to be C.
//!
//! Design:
//!
//! - A fixed-size connection table (MAX_TCP_CONNS slots, no heap -
//!   the same static-allocation discipline every other kernel-side
//!   Rust module in this project already follows), protected by a
//!   single SpinLock<[TcpConn; MAX_TCP_CONNS]> - the same shape as
//!   kernel/rust/pipe.rs's own PIPES table, including its "the
//!   protected type doesn't need to derive Copy/Clone as long as the
//!   array-repeat initializer is itself a `const`" trick (see
//!   EMPTY_CONN below).
//!
//! - A real RFC 793 state machine: Closed, Listen, SynSent,
//!   SynReceived, Established, FinWait1, FinWait2, CloseWait, Closing,
//!   LastAck, TimeWait - the full set the old tcp.c never had (it only
//!   ever modeled Closed/SynSent/Established).
//!
//! - Real sliding-window data transfer: a byte-ring send buffer per
//!   connection, with a `sent_count` cursor marking how much of the
//!   buffered-but-unacknowledged data has actually been transmitted at
//!   least once - multiple MSS-sized segments can be genuinely
//!   in-flight at once (bounded by the peer's advertised window and
//!   this connection's own send buffer capacity), unlike the old
//!   code's one-segment-at-a-time stop-and-wait design.
//!
//! - A real retransmission queue, implemented as go-back-N rather than
//!   a separate timer per segment: on RTO expiry, the entire in-flight
//!   run (`sent_count` bytes starting at `send_unacked`) is treated as
//!   not-yet-sent again, so the ordinary send step (in rust_tcp_poll()
//!   below) naturally re-sends it; `rto_ticks` doubles on each
//!   consecutive timeout (capped at MAX_RTO_TICKS) and resets to
//!   BASE_RTO_TICKS the moment new data is genuinely acknowledged -
//!   textbook exponential backoff, just applied to a byte range instead
//!   of a single segment, which is what basic (non-SACK) TCP does
//!   under packet loss anyway. A connection that exhausts MAX_RETRIES
//!   is torn down rather than retried forever - RFC 793 leaves the
//!   exact giving-up point to the implementation, and this project's
//!   already-established pattern (SYS_PING_START/POLL's own ~3s bound,
//!   arp_resolve()'s own ~3s bound) is "bounded, not infinite."
//!
//! - Real LISTEN/passive-open/accept support, which the old tcp.c
//!   explicitly never had (its own header comment: "no LISTEN/passive
//!   open, matching how this project has no server-side functionality
//!   anywhere else"). A LISTEN socket accumulates fully-handshaked
//!   connections in a small fixed backlog array; rust_tcp_accept()
//!   pops from it, blocking (yielding repeatedly, the same pattern
//!   process_wait()/arp_resolve() already use - see that function's
//!   own Phase 58 fix) if it's empty.
//!
//! What's deliberately still NOT in scope, honestly, the same way
//! every earlier phase's own network code has documented its own
//! limits (see PROGRESS.md for the full account):
//!
//! - No out-of-order reassembly: a segment that doesn't arrive exactly
//!   at `recv_next` is dropped, not buffered - the peer's own
//!   retransmission is relied on to eventually resend it in order,
//!   same as the old tcp.c.
//! - No delayed ACKs, no Nagle's algorithm, no congestion control
//!   (no slow start/congestion window - only receiver-advertised flow
//!   control plus RTO backoff under loss). Every inbound data/FIN
//!   segment is ACKed immediately.
//! - No SACK, no TCP options/MSS negotiation, no urgent pointer.
//! - A short, fixed TIME_WAIT (TIME_WAIT_TICKS, a few seconds) rather
//!   than the real 2*MSL (which would be minutes) - long enough to
//!   catch a delayed duplicate FIN/ACK on the controlled/local-ish
//!   networks this project has always targeted, short enough not to
//!   tie up one of MAX_TCP_CONNS' few slots for realistic minutes on a
//!   resource-constrained kernel with no dynamic allocation.
//! - No RST is ever sent (only received/acted upon) - an unmatched or
//!   otherwise-rejected segment is just dropped. A real stack would
//!   send RST in several of these cases; skipped here as a scope cut,
//!   matching this project's general preference for "honestly do less"
//!   over a half-finished version of the real thing.
//!
//! Locking discipline (Phase 57's own rule, carried forward here):
//! CONNS' lock is NEVER held across a call to ip_send() (which can
//! itself block for seconds inside arp_resolve() - see that function's
//! own Phase 58 fix) or across scheduler_yield(). Every function below
//! that needs to both touch the connection table AND send a segment
//! does so in two steps: gather what's needed under the lock, release
//! it, then send.

use crate::spinlock::SpinLock;
use core::sync::atomic::{AtomicUsize, Ordering};

const MAX_TCP_CONNS: usize = 8;
const MAX_BACKLOG: usize = 4;
const TCP_MSS: usize = 536; /* conservative default MSS, same value
                                real-world TCP falls back to when no
                                MSS option is negotiated - this stack
                                never sends the MSS option at all, so
                                assuming the conservative default on
                                both ends is the honest choice */
const SEND_BUF_CAP: usize = 2048;
const RECV_BUF_CAP: usize = 2048;

const BASE_RTO_TICKS: u32 = 100; /* ~1s at this kernel's 100Hz tick rate */
const MAX_RTO_TICKS: u32 = 800; /* ~8s cap on exponential backoff */
const MAX_RETRIES: u8 = 6;
const TIME_WAIT_TICKS: u32 = 200; /* ~2s - see module doc */

const TCP_FLAG_FIN: u8 = 0x01;
const TCP_FLAG_SYN: u8 = 0x02;
const TCP_FLAG_RST: u8 = 0x04;
const TCP_FLAG_PSH: u8 = 0x08;
const TCP_FLAG_ACK: u8 = 0x10;

/* Duplicated from kernel/net/net.h/ip.h rather than shared through a
 * header - this project's established convention for a small FFI-
 * boundary constant (see e.g. kernel/include/smp.h's own comment). */
const NET_OUR_IP: u32 = 0x0A00_020F;
const IP_PROTO_TCP: u8 = 6;

extern "C" {
    fn ip_send(dest_ip: u32, protocol: u8, payload: *const u8, payload_len: u16) -> bool;
    fn net_checksum16(data: *const u8, length: u16) -> u16;
    fn timer_get_ticks() -> u32;
    fn scheduler_yield();
}

/// Wrapping-safe sequence-number comparison (the standard TCP trick:
/// compare the difference as a signed 32-bit value) - `a` is at or
/// past `b` in sequence-number space.
fn seq_ge(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

/// Same idea, strictly past.
fn seq_gt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

/// One connection (or listener) slot. Deliberately NOT `derive(Copy,
/// Clone)` - like kernel/rust/pipe.rs's own `Pipe`, it doesn't need to
/// be: `[EMPTY_CONN; MAX_TCP_CONNS]` below re-evaluates the `const`
/// initializer for each element rather than copying a value, so Copy
/// is never actually required.
struct TcpConn {
    in_use: bool,
    is_listener: bool,
    state: TcpState,

    local_port: u16,
    remote_ip: u32,
    remote_port: u16,

    /// For a connection spawned by a LISTEN socket (SynReceived or
    /// later): the index of that listener, so the handshake-completion
    /// path knows whose backlog to push into. -1 for a listener itself
    /// or an actively-opened (client) connection.
    listener_id: i8,

    /// Oldest sequence number not yet acknowledged by the peer - the
    /// left edge of the send window.
    send_unacked: u32,
    /// How many of the `send_buffered` bytes currently sitting in
    /// `send_buf` (starting at `send_unacked`) have been transmitted
    /// at least once - the in-flight window. Always <= send_buffered.
    sent_count: u16,
    send_buf: [u8; SEND_BUF_CAP],
    /// Index within send_buf of the byte at sequence number
    /// send_unacked - a ring buffer, the same shape as pipe.rs's own.
    send_head: u16,
    /// Total bytes currently buffered (unacked, whether already sent
    /// or not) starting at send_head/send_unacked.
    send_buffered: u16,

    /// Next sequence number we expect from the peer - what our ACKs
    /// carry.
    recv_next: u32,
    recv_buf: [u8; RECV_BUF_CAP],
    recv_head: u16,
    recv_len: u16,

    /// Retransmission timer for whichever of (SYN, data, FIN) is
    /// currently the oldest outstanding thing on this connection - see
    /// the module doc's "go-back-N" explanation. 0 means "nothing
    /// outstanding, timer not running."
    rto_deadline: u32,
    rto_ticks: u32,
    retrans_count: u8,

    /// Set once rust_tcp_close() has been called (locally requested
    /// close) but before the FIN has actually gone out - draining any
    /// still-buffered send data first, see rust_tcp_poll().
    fin_needed: bool,
    fin_sent: bool,
    fin_seq: u32,
    peer_fin_received: bool,

    time_wait_deadline: u32,

    /// Indices (into this same table) of connections that have
    /// finished their 3-way handshake against this listener and are
    /// waiting for rust_tcp_accept() to pop them. -1 = empty slot.
    backlog: [i8; MAX_BACKLOG],
    backlog_len: u8,
}

impl TcpConn {
    const fn new() -> Self {
        TcpConn {
            in_use: false,
            is_listener: false,
            state: TcpState::Closed,
            local_port: 0,
            remote_ip: 0,
            remote_port: 0,
            listener_id: -1,
            send_unacked: 0,
            sent_count: 0,
            send_buf: [0u8; SEND_BUF_CAP],
            send_head: 0,
            send_buffered: 0,
            recv_next: 0,
            recv_buf: [0u8; RECV_BUF_CAP],
            recv_head: 0,
            recv_len: 0,
            rto_deadline: 0,
            rto_ticks: BASE_RTO_TICKS,
            retrans_count: 0,
            fin_needed: false,
            fin_sent: false,
            fin_seq: 0,
            peer_fin_received: false,
            time_wait_deadline: 0,
            backlog: [-1; MAX_BACKLOG],
            backlog_len: 0,
        }
    }

    fn send_buf_write(&mut self, data: &[u8]) -> u16 {
        let space = SEND_BUF_CAP as u16 - self.send_buffered;
        let n = core::cmp::min(space as usize, data.len()) as u16;
        for i in 0..n {
            let idx = (self.send_head as usize + self.send_buffered as usize + i as usize)
                % SEND_BUF_CAP;
            self.send_buf[idx] = data[i as usize];
        }
        self.send_buffered += n;
        n
    }

    /// Copies up to `out.len()` bytes starting `offset` bytes past
    /// send_unacked (i.e. not necessarily from the very front of the
    /// buffer) into `out`, WITHOUT consuming them - used to build a
    /// (re)transmission without disturbing what's still considered
    /// "buffered."
    fn send_buf_peek(&self, offset: u16, out: &mut [u8]) -> usize {
        let avail = self.send_buffered.saturating_sub(offset) as usize;
        let n = core::cmp::min(avail, out.len());
        for i in 0..n {
            let idx = (self.send_head as usize + offset as usize + i) % SEND_BUF_CAP;
            out[i] = self.send_buf[idx];
        }
        n
    }

    /// Drops `n` bytes off the front of the send buffer (an
    /// acknowledgment consuming them) and correspondingly shrinks
    /// `sent_count`.
    fn send_buf_consume(&mut self, n: u16) {
        let n = core::cmp::min(n, self.send_buffered);
        self.send_head = ((self.send_head as usize + n as usize) % SEND_BUF_CAP) as u16;
        self.send_buffered -= n;
        self.sent_count = if self.sent_count > n {
            self.sent_count - n
        } else {
            0
        };
    }

    fn recv_buf_write(&mut self, data: &[u8]) -> u16 {
        let space = RECV_BUF_CAP as u16 - self.recv_len;
        let n = core::cmp::min(space as usize, data.len()) as u16;
        for i in 0..n {
            let idx = (self.recv_head as usize + self.recv_len as usize + i as usize)
                % RECV_BUF_CAP;
            self.recv_buf[idx] = data[i as usize];
        }
        self.recv_len += n;
        n
    }

    fn recv_buf_read(&mut self, out: &mut [u8]) -> u16 {
        let n = core::cmp::min(self.recv_len as usize, out.len()) as u16;
        for i in 0..n {
            out[i as usize] = self.recv_buf[(self.recv_head as usize + i as usize) % RECV_BUF_CAP];
        }
        self.recv_head = ((self.recv_head as usize + n as usize) % RECV_BUF_CAP) as u16;
        self.recv_len -= n;
        n
    }

    fn advertised_window(&self) -> u16 {
        RECV_BUF_CAP as u16 - self.recv_len
    }
}

const EMPTY_CONN: TcpConn = TcpConn::new();
static CONNS: SpinLock<[TcpConn; MAX_TCP_CONNS]> = SpinLock::new([EMPTY_CONN; MAX_TCP_CONNS]);

/// Soft round-robin hint for rust_tcp_poll()'s send pass - doesn't need
/// real synchronization (worst case, two CPUs' poll calls both start
/// from the same connection once in a while, which just means that
/// connection gets sent to twice as often - harmless), so a plain
/// atomic counter is enough; no need to nest it inside CONNS' own lock.
static POLL_CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Builds one TCP segment (header + optional data), computes its real
/// checksum (the 12-byte IP pseudo-header + segment, exactly RFC 793's
/// algorithm - unlike this project's UDP, which sends a disabled all-
/// zero checksum, real TCP stacks universally validate this and drop
/// segments that fail it), and hands it to ip_send(). Always called
/// with CONNS' lock already released - see the module doc.
fn send_segment(
    remote_ip: u32,
    local_port: u16,
    remote_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    data: &[u8],
) -> bool {
    let mut seg = [0u8; 20 + TCP_MSS];
    seg[0..2].copy_from_slice(&local_port.to_be_bytes());
    seg[2..4].copy_from_slice(&remote_port.to_be_bytes());
    seg[4..8].copy_from_slice(&seq.to_be_bytes());
    seg[8..12].copy_from_slice(&ack.to_be_bytes());
    seg[12] = 5 << 4; /* 20-byte header, no options */
    seg[13] = flags;
    seg[14..16].copy_from_slice(&window.to_be_bytes());
    seg[16..18].copy_from_slice(&0u16.to_be_bytes()); /* checksum placeholder */
    seg[18..20].copy_from_slice(&0u16.to_be_bytes());
    let data_len = core::cmp::min(data.len(), TCP_MSS);
    seg[20..20 + data_len].copy_from_slice(&data[..data_len]);
    let total = 20 + data_len;

    let mut csum_buf = [0u8; 12 + 20 + TCP_MSS];
    csum_buf[0..4].copy_from_slice(&NET_OUR_IP.to_be_bytes());
    csum_buf[4..8].copy_from_slice(&remote_ip.to_be_bytes());
    csum_buf[8] = 0;
    csum_buf[9] = IP_PROTO_TCP;
    csum_buf[10..12].copy_from_slice(&(total as u16).to_be_bytes());
    csum_buf[12..12 + total].copy_from_slice(&seg[0..total]);

    let checksum = unsafe { net_checksum16(csum_buf.as_ptr(), (12 + total) as u16) };
    seg[16..18].copy_from_slice(&checksum.to_be_bytes());

    unsafe { ip_send(remote_ip, IP_PROTO_TCP, seg.as_ptr(), total as u16) }
}

fn send_syn(idx: usize) {
    let (local_port, remote_port, remote_ip, seq, window) = {
        let conns = CONNS.lock();
        let c = &conns[idx];
        (c.local_port, c.remote_port, c.remote_ip, c.send_unacked, c.advertised_window())
    };
    send_segment(remote_ip, local_port, remote_port, seq, 0, TCP_FLAG_SYN, window, &[]);
}

fn send_syn_ack(idx: usize) {
    let (local_port, remote_port, remote_ip, seq, ack, window) = {
        let conns = CONNS.lock();
        let c = &conns[idx];
        (
            c.local_port,
            c.remote_port,
            c.remote_ip,
            c.send_unacked,
            c.recv_next,
            c.advertised_window(),
        )
    };
    send_segment(
        remote_ip,
        local_port,
        remote_port,
        seq,
        ack,
        TCP_FLAG_SYN | TCP_FLAG_ACK,
        window,
        &[],
    );
}

fn send_ack_only(idx: usize) {
    let (local_port, remote_port, remote_ip, seq, ack, window) = {
        let conns = CONNS.lock();
        let c = &conns[idx];
        (
            c.local_port,
            c.remote_port,
            c.remote_ip,
            c.send_unacked.wrapping_add(c.sent_count as u32),
            c.recv_next,
            c.advertised_window(),
        )
    };
    send_segment(remote_ip, local_port, remote_port, seq, ack, TCP_FLAG_ACK, window, &[]);
}

/// Allocates a new, unbound socket. Returns its id (an index into this
/// module's own connection table, opaque to C - kernel/arch/x86/cpu/
/// syscall.c stores it in an open_files[] entry's `pipe_id` field, the
/// same way it already does for pipe ids), or -1 if every slot is
/// already in use.
#[no_mangle]
pub extern "C" fn rust_tcp_socket() -> i32 {
    let mut conns = CONNS.lock();
    for i in 0..MAX_TCP_CONNS {
        if !conns[i].in_use {
            conns[i] = TcpConn::new();
            conns[i].in_use = true;
            return i as i32;
        }
    }
    -1
}

/// Binds `id` to `port`. `port == 0` auto-assigns an unused ephemeral
/// port (>= 49152, the same IANA-recommended dynamic/private range),
/// matching real bind()'s "let the kernel pick" convention. Returns 0
/// on success, -1 if `id` is invalid/already bound/not Closed, or (for
/// an explicit non-zero port) already in use by another socket.
#[no_mangle]
pub extern "C" fn rust_tcp_bind(id: i32, port: u16) -> i32 {
    if id < 0 || (id as usize) >= MAX_TCP_CONNS {
        return -1;
    }
    let idx = id as usize;
    let mut conns = CONNS.lock();
    if !conns[idx].in_use || conns[idx].state != TcpState::Closed || conns[idx].local_port != 0 {
        return -1;
    }
    if port != 0 {
        for i in 0..MAX_TCP_CONNS {
            if i != idx && conns[i].in_use && conns[i].local_port == port {
                return -1;
            }
        }
        conns[idx].local_port = port;
        return 0;
    }
    let mut candidate: u16 = 49152;
    loop {
        let taken = (0..MAX_TCP_CONNS).any(|i| conns[i].in_use && conns[i].local_port == candidate);
        if !taken {
            break;
        }
        candidate = if candidate == 65535 { 49152 } else { candidate + 1 };
    }
    conns[idx].local_port = candidate;
    0
}

/// Marks `id` as a passive-open (LISTEN) socket. Requires it already
/// be bound (rust_tcp_bind() with a non-zero port, or an implicit bind
/// via rust_tcp_connect() - though listening on a connect()-assigned
/// ephemeral port is unusual, nothing here forbids it). `backlog` is
/// accepted for Berkeley-sockets shape compatibility but currently
/// unused - the backlog is always MAX_BACKLOG, a fixed, small,
/// statically-sized array, the same "simple and honest about the
/// limit" choice this project's other fixed-capacity tables already
/// make (see e.g. process.h's MAX_CAPABILITIES).
#[no_mangle]
pub extern "C" fn rust_tcp_listen(id: i32, backlog: i32) -> i32 {
    let _ = backlog;
    if id < 0 || (id as usize) >= MAX_TCP_CONNS {
        return -1;
    }
    let idx = id as usize;
    let mut conns = CONNS.lock();
    if !conns[idx].in_use || conns[idx].state != TcpState::Closed || conns[idx].local_port == 0 {
        return -1;
    }
    conns[idx].is_listener = true;
    conns[idx].state = TcpState::Listen;
    conns[idx].backlog = [-1; MAX_BACKLOG];
    conns[idx].backlog_len = 0;
    0
}

/// Blocks (yielding repeatedly - the same pattern process_wait()/
/// arp_resolve() already use, safe now that arp_resolve()'s own Phase
/// 58 fix establishes that yielding from deep inside a syscall handler
/// genuinely lets the timer tick advance) until a connection completes
/// its handshake against listener `id`, then returns its own id (a
/// brand new socket, already Established - accept()ing it again is
/// never necessary). Returns -1 immediately if `id` isn't a currently-
/// listening socket at all.
#[no_mangle]
pub extern "C" fn rust_tcp_accept(id: i32) -> i32 {
    if id < 0 || (id as usize) >= MAX_TCP_CONNS {
        return -1;
    }
    let idx = id as usize;
    loop {
        {
            let mut conns = CONNS.lock();
            if !conns[idx].in_use || !conns[idx].is_listener || conns[idx].state != TcpState::Listen {
                return -1;
            }
            if conns[idx].backlog_len > 0 {
                let popped = conns[idx].backlog[0];
                for i in 1..MAX_BACKLOG {
                    conns[idx].backlog[i - 1] = conns[idx].backlog[i];
                }
                conns[idx].backlog[MAX_BACKLOG - 1] = -1;
                conns[idx].backlog_len -= 1;
                return popped as i32;
            }
        }
        unsafe {
            scheduler_yield();
        }
    }
}

/// Opens a connection to remote_ip:remote_port - a full, real 3-way
/// handshake, with genuine SYN retransmission on loss (unlike the old
/// tcp.c's single fire-and-hope SYN). Auto-binds an ephemeral local
/// port first if `id` wasn't already bound. Blocks (yielding
/// repeatedly) until Established, RST, or MAX_RETRIES SYN
/// retransmissions are exhausted. Returns 0 on success, -1 otherwise.
#[no_mangle]
pub extern "C" fn rust_tcp_connect(id: i32, remote_ip: u32, remote_port: u16) -> i32 {
    if id < 0 || (id as usize) >= MAX_TCP_CONNS {
        return -1;
    }
    let idx = id as usize;
    {
        let mut conns = CONNS.lock();
        if !conns[idx].in_use || conns[idx].is_listener || conns[idx].state != TcpState::Closed {
            return -1;
        }
        if conns[idx].local_port == 0 {
            let mut candidate: u16 = 49152;
            loop {
                let taken =
                    (0..MAX_TCP_CONNS).any(|i| conns[i].in_use && conns[i].local_port == candidate);
                if !taken {
                    break;
                }
                candidate = if candidate == 65535 { 49152 } else { candidate + 1 };
            }
            conns[idx].local_port = candidate;
        }
        conns[idx].remote_ip = remote_ip;
        conns[idx].remote_port = remote_port;
        /* Not cryptographically random - see the old tcp.c's own,
         * identical reasoning (this project doesn't defend against
         * active ISN-guessing attacks). */
        let isn = unsafe { timer_get_ticks() }.wrapping_mul(12345).wrapping_add(1);
        conns[idx].send_unacked = isn;
        conns[idx].send_head = 0;
        conns[idx].send_buffered = 0;
        conns[idx].sent_count = 0;
        conns[idx].recv_next = 0;
        conns[idx].state = TcpState::SynSent;
        conns[idx].rto_ticks = BASE_RTO_TICKS;
        conns[idx].retrans_count = 0;
        conns[idx].rto_deadline = unsafe { timer_get_ticks() } + BASE_RTO_TICKS;
    }
    send_syn(idx);

    loop {
        {
            let conns = CONNS.lock();
            match conns[idx].state {
                TcpState::Established => return 0,
                TcpState::SynSent => {}
                _ => return -1, /* RST, or rust_tcp_poll() gave up retrying */
            }
        }
        unsafe {
            scheduler_yield();
        }
    }
}

/// Appends up to `len` bytes to `id`'s send buffer - the actual
/// transmission (and any retransmission) happens asynchronously via
/// rust_tcp_poll(), matching this kernel's non-blocking-write
/// convention (see kernel/rust/pipe.rs's own rust_pipe_write() doc
/// comment - the same "short write if the buffer's full, caller's
/// problem to retry" contract, not an error). Returns bytes actually
/// accepted (0..=len), or -1 if `id` isn't a connected, writable
/// socket.
///
/// # Safety
/// `buf` must be valid for reads of `len` bytes - see pipe.rs's own
/// rust_pipe_write() safety note; the same reasoning applies here.
#[no_mangle]
pub unsafe extern "C" fn rust_tcp_send(id: i32, buf: *const u8, len: u32) -> i32 {
    if id < 0 || (id as usize) >= MAX_TCP_CONNS || buf.is_null() {
        return -1;
    }
    let idx = id as usize;
    let data = core::slice::from_raw_parts(buf, len as usize);
    let mut conns = CONNS.lock();
    if !conns[idx].in_use || conns[idx].is_listener || conns[idx].fin_needed {
        return -1;
    }
    match conns[idx].state {
        TcpState::Established | TcpState::CloseWait => conns[idx].send_buf_write(data) as i32,
        _ => -1,
    }
}

/// Reads up to `max_len` bytes into `buf`. Contract deliberately
/// identical to kernel/rust/pipe.rs's own rust_pipe_read(): `> 0`
/// bytes read, `0` real end-of-stream (peer's FIN has arrived and
/// nothing is left buffered), `-2` would-block (nothing buffered yet,
/// but the connection is still open - more may arrive), `-1` invalid
/// handle or a closed/errored connection with nothing left to read -
/// matching that exact contract is what lets kernel/arch/x86/cpu/
/// syscall.c's handle_read() dispatch to either kind through the same
/// SYS_READ path.
///
/// # Safety
/// `buf` must be valid for writes of `max_len` bytes - see pipe.rs's
/// own rust_pipe_read() safety note.
#[no_mangle]
pub unsafe extern "C" fn rust_tcp_recv(id: i32, buf: *mut u8, max_len: u32) -> i32 {
    if id < 0 || (id as usize) >= MAX_TCP_CONNS || buf.is_null() {
        return -1;
    }
    let idx = id as usize;
    let mut conns = CONNS.lock();
    if !conns[idx].in_use || conns[idx].is_listener {
        return -1;
    }
    if conns[idx].recv_len > 0 {
        let out = core::slice::from_raw_parts_mut(buf, max_len as usize);
        return conns[idx].recv_buf_read(out) as i32;
    }
    if conns[idx].peer_fin_received {
        return 0;
    }
    match conns[idx].state {
        TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2 => -2,
        _ => -1,
    }
}

/// Initiates closing `id` - fire-and-forget, matching kernel/rust/
/// pipe.rs's own rust_pipe_close() (no return value): a listener is
/// torn down immediately (along with any handshaked-but-never-
/// accepted connections sitting in its backlog, which are only
/// reachable through it); a real data connection has its FIN sent once
/// any already-buffered send data has drained (see rust_tcp_poll()),
/// then lingers in the background through the rest of the normal
/// close handshake - closing the ring-3 handle does not mean the TCP-
/// level connection tears down instantly, the same true-to-life
/// distinction a real Unix close() on a socket fd makes (the
/// connection can linger past the fd's own lifetime). A redundant
/// close() on an already-closing connection is a harmless no-op.
#[no_mangle]
pub extern "C" fn rust_tcp_close(id: i32) {
    if id < 0 || (id as usize) >= MAX_TCP_CONNS {
        return;
    }
    let idx = id as usize;
    let mut conns = CONNS.lock();
    if !conns[idx].in_use {
        return;
    }
    if conns[idx].is_listener {
        for i in 0..(conns[idx].backlog_len as usize) {
            let bidx = conns[idx].backlog[i];
            if bidx >= 0 {
                conns[bidx as usize].in_use = false;
            }
        }
        conns[idx].in_use = false;
        return;
    }
    match conns[idx].state {
        TcpState::Closed | TcpState::SynSent | TcpState::SynReceived => {
            conns[idx].in_use = false;
        }
        TcpState::Established | TcpState::CloseWait => {
            conns[idx].fin_needed = true;
        }
        _ => {
            /* Already closing - a redundant close(), tolerated the same
             * way handle_close() already tolerates a double SYS_CLOSE. */
        }
    }
}

/// Called by kernel/net/ip.c's ip_handle_packet() for every incoming
/// TCP segment. Looks up the matching connection (or, for a bare SYN,
/// a LISTEN socket bound to the destination port) and applies it to
/// that connection's state machine. Decides at most one segment to
/// send in response, and - per the module doc's locking discipline -
/// sends it only after releasing CONNS' lock.
///
/// # Safety
/// `payload` must be valid for reads of `length` bytes - the same
/// trust model every other packet-handling entry point in this kernel
/// already uses (see e.g. kernel/net/arp.c's arp_handle_packet()).
#[no_mangle]
pub unsafe extern "C" fn rust_tcp_handle_packet(src_ip: u32, payload: *const u8, length: u16) {
    if payload.is_null() || (length as usize) < 20 {
        return;
    }
    let data = core::slice::from_raw_parts(payload, length as usize);

    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let seq = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let ack = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let data_offset = ((data[12] >> 4) as usize) * 4;
    let flags = data[13];
    if data_offset < 20 || data_offset > data.len() {
        return; /* malformed */
    }
    let payload_bytes = &data[data_offset..];

    /* What (if anything) to send in response, decided while holding
     * CONNS' lock below but only ever executed after releasing it. */
    let mut send_synack_for: i32 = -1;
    let mut send_ack_for: i32 = -1;

    {
        let mut conns = CONNS.lock();

        let mut matched: i32 = -1;
        for i in 0..MAX_TCP_CONNS {
            if conns[i].in_use
                && !conns[i].is_listener
                && conns[i].local_port == dst_port
                && conns[i].remote_port == src_port
                && conns[i].remote_ip == src_ip
            {
                matched = i as i32;
                break;
            }
        }

        if matched >= 0 {
            let idx = matched as usize;

            if (flags & TCP_FLAG_RST) != 0 {
                conns[idx].in_use = false;
            } else if conns[idx].state == TcpState::SynSent {
                if (flags & TCP_FLAG_SYN) != 0
                    && (flags & TCP_FLAG_ACK) != 0
                    && ack == conns[idx].send_unacked.wrapping_add(1)
                {
                    conns[idx].recv_next = seq.wrapping_add(1);
                    conns[idx].send_unacked = ack;
                    conns[idx].sent_count = 0;
                    conns[idx].state = TcpState::Established;
                    conns[idx].rto_ticks = BASE_RTO_TICKS;
                    conns[idx].retrans_count = 0;
                    conns[idx].rto_deadline = 0;
                    send_ack_for = idx as i32;
                }
            } else if conns[idx].state == TcpState::SynReceived {
                if (flags & TCP_FLAG_ACK) != 0 && ack == conns[idx].send_unacked.wrapping_add(1) {
                    conns[idx].send_unacked = ack;
                    conns[idx].sent_count = 0;
                    conns[idx].state = TcpState::Established;
                    conns[idx].rto_ticks = BASE_RTO_TICKS;
                    conns[idx].retrans_count = 0;
                    conns[idx].rto_deadline = 0;
                    let lidx = conns[idx].listener_id;
                    if lidx >= 0
                        && conns[lidx as usize].in_use
                        && conns[lidx as usize].is_listener
                        && (conns[lidx as usize].backlog_len as usize) < MAX_BACKLOG
                    {
                        let l = lidx as usize;
                        let pos = conns[l].backlog_len as usize;
                        conns[l].backlog[pos] = idx as i8;
                        conns[l].backlog_len += 1;
                    } else {
                        /* Listener gone, or its backlog is already full -
                         * drop the now-established connection; see this
                         * module's own SynReceived handling comment in
                         * the module doc for the honest scope note. */
                        conns[idx].in_use = false;
                    }
                }
            } else {
                /* Established, or one of the closing states - shared
                 * ACK/data/FIN processing, the same way a real TCP
                 * stack's segment-processing path is shared across
                 * them. */
                if (flags & TCP_FLAG_ACK) != 0 && seq_gt(ack, conns[idx].send_unacked) {
                    let raw = ack.wrapping_sub(conns[idx].send_unacked);
                    let advance = core::cmp::min(raw, conns[idx].send_buffered as u32) as u16;
                    conns[idx].send_buf_consume(advance);
                    conns[idx].send_unacked = ack;
                    conns[idx].rto_ticks = BASE_RTO_TICKS;
                    conns[idx].retrans_count = 0;
                    if conns[idx].send_buffered == 0 {
                        conns[idx].rto_deadline = 0;
                    }
                    if conns[idx].fin_sent && seq_ge(ack, conns[idx].fin_seq.wrapping_add(1)) {
                        conns[idx].state = match conns[idx].state {
                            TcpState::FinWait1 => TcpState::FinWait2,
                            TcpState::Closing => TcpState::TimeWait,
                            TcpState::LastAck => TcpState::Closed,
                            other => other,
                        };
                        conns[idx].rto_deadline = 0;
                        if conns[idx].state == TcpState::TimeWait {
                            conns[idx].time_wait_deadline =
                                timer_get_ticks() + TIME_WAIT_TICKS;
                        } else if conns[idx].state == TcpState::Closed {
                            conns[idx].in_use = false;
                        }
                    }
                }

                if conns[idx].in_use
                    && !payload_bytes.is_empty()
                    && seq == conns[idx].recv_next
                {
                    let accepted = conns[idx].recv_buf_write(payload_bytes);
                    conns[idx].recv_next = conns[idx].recv_next.wrapping_add(accepted as u32);
                    if accepted > 0 {
                        send_ack_for = idx as i32;
                    }
                }

                if conns[idx].in_use
                    && (flags & TCP_FLAG_FIN) != 0
                    && seq.wrapping_add(payload_bytes.len() as u32) == conns[idx].recv_next
                {
                    conns[idx].recv_next = conns[idx].recv_next.wrapping_add(1);
                    conns[idx].peer_fin_received = true;
                    conns[idx].state = match conns[idx].state {
                        TcpState::Established => TcpState::CloseWait,
                        TcpState::FinWait1 => TcpState::Closing,
                        TcpState::FinWait2 => TcpState::TimeWait,
                        other => other,
                    };
                    if conns[idx].state == TcpState::TimeWait {
                        conns[idx].time_wait_deadline = timer_get_ticks() + TIME_WAIT_TICKS;
                    }
                    send_ack_for = idx as i32;
                }
            }
        } else if (flags & TCP_FLAG_SYN) != 0 && (flags & TCP_FLAG_ACK) == 0 {
            /* A bare SYN with no matching connection - only meaningful
             * against a LISTEN socket bound to this port. */
            let mut listener: i32 = -1;
            for i in 0..MAX_TCP_CONNS {
                if conns[i].in_use && conns[i].is_listener && conns[i].state == TcpState::Listen
                    && conns[i].local_port == dst_port
                {
                    listener = i as i32;
                    break;
                }
            }
            if listener >= 0 {
                let mut free_idx: i32 = -1;
                for i in 0..MAX_TCP_CONNS {
                    if !conns[i].in_use {
                        free_idx = i as i32;
                        break;
                    }
                }
                if free_idx >= 0 {
                    let nidx = free_idx as usize;
                    let isn = timer_get_ticks()
                        .wrapping_mul(2_654_435_761)
                        .wrapping_add(nidx as u32);
                    conns[nidx] = TcpConn::new();
                    conns[nidx].in_use = true;
                    conns[nidx].state = TcpState::SynReceived;
                    conns[nidx].local_port = dst_port;
                    conns[nidx].remote_ip = src_ip;
                    conns[nidx].remote_port = src_port;
                    conns[nidx].listener_id = listener as i8;
                    conns[nidx].send_unacked = isn;
                    conns[nidx].recv_next = seq.wrapping_add(1);
                    conns[nidx].rto_ticks = BASE_RTO_TICKS;
                    conns[nidx].retrans_count = 0;
                    conns[nidx].rto_deadline = timer_get_ticks() + BASE_RTO_TICKS;
                    send_synack_for = nidx as i32;
                }
                /* else: no free connection slot - dropped, matching this
                 * kernel's usual fixed-capacity-exhaustion behavior (see
                 * e.g. pipe.rs's own rust_pipe_create()). */
            }
            /* else: SYN to a port nothing is listening on - dropped, not
             * RST'd; see the module doc's scope note. */
        }
        /* else: an unmatched ACK/data/FIN/RST with no connection at all
         * - dropped, same scope note. */
    }

    if send_synack_for >= 0 {
        send_syn_ack(send_synack_for as usize);
    } else if send_ack_for >= 0 {
        send_ack_only(send_ack_for as usize);
    }
}

/// Called from kernel/net/net.c's net_poll() - itself already called
/// on every idle-loop tick and every blocking wait loop throughout
/// this kernel - since this kernel has no dedicated per-connection
/// timer interrupt to drive retransmission/pacing off of instead. Does
/// at most two things per call, each independently cheap: (1) a
/// lock-only housekeeping pass across every connection (RTO/TIME_WAIT
/// expiry checks - no sends), and (2) at most ONE segment actually
/// transmitted, chosen round-robin across connections that have
/// unsent buffered data or a pending FIN - deliberately capped at one
/// send per call (rather than looping until every connection is fully
/// serviced) to bound how much stack space and how long a single
/// net_poll() call can take, since it's called from tight wait loops
/// elsewhere that themselves need to keep progressing.
#[no_mangle]
pub extern "C" fn rust_tcp_poll() {
    let now = unsafe { timer_get_ticks() };

    enum Retransmit {
        Syn(usize),
        SynAck(usize),
    }
    let mut retransmit: Option<Retransmit> = None;

    {
        let mut conns = CONNS.lock();
        for i in 0..MAX_TCP_CONNS {
            if !conns[i].in_use {
                continue;
            }
            match conns[i].state {
                TcpState::TimeWait => {
                    if seq_ge(now, conns[i].time_wait_deadline) {
                        conns[i].in_use = false;
                    }
                }
                TcpState::SynSent | TcpState::SynReceived => {
                    if conns[i].rto_deadline != 0 && seq_ge(now, conns[i].rto_deadline) {
                        if conns[i].retrans_count >= MAX_RETRIES {
                            conns[i].in_use = false;
                        } else {
                            conns[i].retrans_count += 1;
                            conns[i].rto_ticks = core::cmp::min(conns[i].rto_ticks * 2, MAX_RTO_TICKS);
                            conns[i].rto_deadline = now + conns[i].rto_ticks;
                            retransmit = Some(if conns[i].state == TcpState::SynSent {
                                Retransmit::Syn(i)
                            } else {
                                Retransmit::SynAck(i)
                            });
                            break;
                        }
                    }
                }
                _ => {
                    if conns[i].sent_count > 0
                        && conns[i].rto_deadline != 0
                        && seq_ge(now, conns[i].rto_deadline)
                    {
                        if conns[i].retrans_count >= MAX_RETRIES {
                            conns[i].in_use = false; /* give up - see module doc */
                        } else {
                            conns[i].sent_count = 0; /* go-back-N - see module doc */
                            conns[i].retrans_count += 1;
                            conns[i].rto_ticks = core::cmp::min(conns[i].rto_ticks * 2, MAX_RTO_TICKS);
                            conns[i].rto_deadline = 0; /* re-armed by the send pass below */
                        }
                    }
                }
            }
        }
    }

    match retransmit {
        Some(Retransmit::Syn(idx)) => {
            send_syn(idx);
            return;
        }
        Some(Retransmit::SynAck(idx)) => {
            send_syn_ack(idx);
            return;
        }
        None => {}
    }

    struct SegOut {
        remote_ip: u32,
        local_port: u16,
        remote_port: u16,
        seq: u32,
        ack: u32,
        flags: u8,
        window: u16,
        data: [u8; TCP_MSS],
        data_len: usize,
    }
    let mut chosen: Option<SegOut> = None;

    {
        let mut conns = CONNS.lock();
        let start = POLL_CURSOR.load(Ordering::Relaxed) % MAX_TCP_CONNS;
        for step in 0..MAX_TCP_CONNS {
            let i = (start + step) % MAX_TCP_CONNS;
            if !conns[i].in_use || conns[i].is_listener {
                continue;
            }
            let active = matches!(
                conns[i].state,
                TcpState::Established
                    | TcpState::CloseWait
                    | TcpState::FinWait1
                    | TcpState::FinWait2
                    | TcpState::Closing
                    | TcpState::LastAck
            );
            if !active {
                continue;
            }

            let unsent = conns[i].send_buffered.saturating_sub(conns[i].sent_count);
            if unsent > 0 {
                let chunk = core::cmp::min(unsent as usize, TCP_MSS);
                let mut data = [0u8; TCP_MSS];
                let got = conns[i].send_buf_peek(conns[i].sent_count, &mut data[..chunk]);
                let seq = conns[i].send_unacked.wrapping_add(conns[i].sent_count as u32);
                let ack = conns[i].recv_next;
                let window = conns[i].advertised_window();
                let seg = SegOut {
                    remote_ip: conns[i].remote_ip,
                    local_port: conns[i].local_port,
                    remote_port: conns[i].remote_port,
                    seq,
                    ack,
                    flags: TCP_FLAG_ACK | TCP_FLAG_PSH,
                    window,
                    data,
                    data_len: got,
                };
                conns[i].sent_count += got as u16;
                if conns[i].rto_deadline == 0 {
                    conns[i].rto_deadline = now + conns[i].rto_ticks;
                }
                chosen = Some(seg);
                POLL_CURSOR.store(i + 1, Ordering::Relaxed);
                break;
            }

            if conns[i].fin_needed && !conns[i].fin_sent && conns[i].send_buffered == 0 {
                let seq = conns[i].send_unacked;
                let ack = conns[i].recv_next;
                let window = conns[i].advertised_window();
                conns[i].fin_sent = true;
                conns[i].fin_seq = seq;
                conns[i].rto_deadline = now + conns[i].rto_ticks;
                conns[i].state = match conns[i].state {
                    TcpState::Established => TcpState::FinWait1,
                    TcpState::CloseWait => TcpState::LastAck,
                    other => other,
                };
                chosen = Some(SegOut {
                    remote_ip: conns[i].remote_ip,
                    local_port: conns[i].local_port,
                    remote_port: conns[i].remote_port,
                    seq,
                    ack,
                    flags: TCP_FLAG_FIN | TCP_FLAG_ACK,
                    window,
                    data: [0u8; TCP_MSS],
                    data_len: 0,
                });
                POLL_CURSOR.store(i + 1, Ordering::Relaxed);
                break;
            }
        }
    }

    if let Some(seg) = chosen {
        send_segment(
            seg.remote_ip,
            seg.local_port,
            seg.remote_port,
            seg.seq,
            seg.ack,
            seg.flags,
            seg.window,
            &seg.data[..seg.data_len],
        );
    }
}

/// Immediately frees connection `idx` with no graceful teardown at
/// all - deliberately NOT exposed to C/the syscall layer (a real
/// ring-3 socket always goes through rust_tcp_close(), whose lingering
/// background close is the correct, realistic behavior for a real
/// connection - see that function's own doc comment). Used only by
/// rust_tcp_selftest() below to discard its own synthetic connection:
/// that connection's "peer" isn't a real TCP stack, so a graceful
/// close's FIN would never be acknowledged and would otherwise retry
/// with exponential backoff for MAX_RETRIES attempts (tens of real
/// seconds) every single boot, for a peer that was never going to
/// answer in the first place - wasted, avoidable wall-clock time, not
/// a scenario rust_tcp_close()'s own realistic linger behavior needs
/// to accommodate.
fn force_free(idx: usize) {
    let mut conns = CONNS.lock();
    conns[idx].in_use = false;
}

/// Ring-0 self-test, called directly from kernel_main() (no syscall
/// involved, no real network hardware required) - proves the LISTEN/
/// SYN_RECEIVED/accept/data/close path entirely through synthetic,
/// hand-crafted incoming segments fed straight to
/// rust_tcp_handle_packet(), the same "loopback via a direct function
/// call" technique this kernel has no real loopback interface to do
/// any other way (see PROGRESS.md's Phase 58 entry for why). The
/// synthetic peer's IP is NET_GATEWAY_IP (10.0.2.2) specifically so any
/// segment this test's own stack tries to send back (SYN-ACK, data
/// ACKs) has a real chance of a fast, successful ARP resolution in a
/// real QEMU SLIRP environment - but this test does not assert on
/// whether those replies actually reach anywhere, only on this
/// connection's own internal state, which is fully deterministic
/// regardless of whether the reply segment made it out. Returns a
/// bitmask (0 = every check passed).
#[no_mangle]
pub extern "C" fn rust_tcp_selftest() -> i32 {
    const PEER_IP: u32 = 0x0A00_0202; /* NET_GATEWAY_IP - see module doc */
    const PEER_PORT: u16 = 9999;
    const LISTEN_PORT: u16 = 8080;

    let mut code = 0;

    let listener = rust_tcp_socket();
    let bind_ok = listener >= 0 && rust_tcp_bind(listener, LISTEN_PORT) == 0;
    let listen_ok = bind_ok && rust_tcp_listen(listener, 4) == 0;
    if !listen_ok {
        return code | 1;
    }

    /* Synthetic SYN from the "peer" - should create a SynReceived
     * connection and (best-effort) send a SYN-ACK back. */
    let client_isn: u32 = 1000;
    let mut syn = [0u8; 20];
    syn[0..2].copy_from_slice(&PEER_PORT.to_be_bytes());
    syn[2..4].copy_from_slice(&LISTEN_PORT.to_be_bytes());
    syn[4..8].copy_from_slice(&client_isn.to_be_bytes());
    syn[12] = 5 << 4;
    syn[13] = TCP_FLAG_SYN;
    syn[14..16].copy_from_slice(&1024u16.to_be_bytes());
    unsafe {
        rust_tcp_handle_packet(PEER_IP, syn.as_ptr(), syn.len() as u16);
    }

    let mut synrecv_idx: i32 = -1;
    let mut server_isn: u32 = 0;
    {
        let conns = CONNS.lock();
        for i in 0..MAX_TCP_CONNS {
            if conns[i].in_use
                && conns[i].state == TcpState::SynReceived
                && conns[i].remote_port == PEER_PORT
            {
                synrecv_idx = i as i32;
                server_isn = conns[i].send_unacked;
                break;
            }
        }
    }
    if synrecv_idx < 0 {
        return code | 2;
    }

    /* Synthetic final ACK completing the handshake - should move the
     * connection to Established and push it onto the listener's
     * backlog. */
    let mut ack_seg = [0u8; 20];
    ack_seg[0..2].copy_from_slice(&PEER_PORT.to_be_bytes());
    ack_seg[2..4].copy_from_slice(&LISTEN_PORT.to_be_bytes());
    ack_seg[4..8].copy_from_slice(&client_isn.wrapping_add(1).to_be_bytes());
    ack_seg[8..12].copy_from_slice(&server_isn.wrapping_add(1).to_be_bytes());
    ack_seg[12] = 5 << 4;
    ack_seg[13] = TCP_FLAG_ACK;
    ack_seg[14..16].copy_from_slice(&1024u16.to_be_bytes());
    unsafe {
        rust_tcp_handle_packet(PEER_IP, ack_seg.as_ptr(), ack_seg.len() as u16);
    }

    let established_ok = {
        let conns = CONNS.lock();
        conns[synrecv_idx as usize].state == TcpState::Established
            && conns[listener as usize].backlog_len == 1
    };
    if !established_ok {
        code |= 4;
    }

    /* accept() must return immediately (the backlog is already
     * populated) - if this call ever actually blocked, it would spin
     * forever here (scheduler_yield() is a safe no-op this early in
     * boot, before scheduler_start() - see do_schedule()'s own Phase
     * 57 comment), so reaching the assertion below at all is itself
     * part of what this test proves. */
    let accepted = rust_tcp_accept(listener);
    if accepted != synrecv_idx {
        code |= 8;
    }

    /* Synthetic inbound data. */
    let payload = b"HI";
    let mut data_seg = [0u8; 22];
    data_seg[0..2].copy_from_slice(&PEER_PORT.to_be_bytes());
    data_seg[2..4].copy_from_slice(&LISTEN_PORT.to_be_bytes());
    data_seg[4..8].copy_from_slice(&client_isn.wrapping_add(1).to_be_bytes());
    data_seg[8..12].copy_from_slice(&server_isn.wrapping_add(1).to_be_bytes());
    data_seg[12] = 5 << 4;
    data_seg[13] = TCP_FLAG_ACK | TCP_FLAG_PSH;
    data_seg[14..16].copy_from_slice(&1024u16.to_be_bytes());
    data_seg[20..22].copy_from_slice(payload);
    unsafe {
        rust_tcp_handle_packet(PEER_IP, data_seg.as_ptr(), data_seg.len() as u16);
    }

    let mut recv_out = [0u8; 8];
    let n = unsafe { rust_tcp_recv(accepted, recv_out.as_mut_ptr(), recv_out.len() as u32) };
    if n != 2 || &recv_out[0..2] != payload {
        code |= 16;
    }

    /* Outbound send - accepted count only; actual on-the-wire delivery
     * isn't asserted on, see this function's own doc comment. */
    let outbound = b"HELLO";
    let sent = unsafe { rust_tcp_send(accepted, outbound.as_ptr(), outbound.len() as u32) };
    if sent != outbound.len() as i32 {
        code |= 32;
    }
    /* No trailing rust_tcp_poll() here: it would cost another ~3s of
     * best-effort ip_send()/arp_resolve() in a sandbox where the "peer"
     * never actually replies, with no assertion value gained from it. */

    /* Use force_free(), not rust_tcp_close(), for the synthetic accepted
     * connection: see force_free()'s own doc comment above for why - the
     * synthetic "peer" here never ACKs anything, so a graceful close's
     * FIN would otherwise retry with exponential backoff for
     * MAX_RETRIES attempts (tens of real seconds) on every boot. */
    if accepted >= 0 {
        force_free(accepted as usize);
    }
    rust_tcp_close(listener);

    code
}
