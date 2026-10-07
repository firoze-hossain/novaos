//! kernel/rust/firewall.rs - Phase 88: a real, stateful firewall in the
//! network stack.
//!
//! WHAT THIS IS
//!
//! Until now the stack accepted every IP packet addressed to it and sent every
//! packet it was asked to: "is this port open" was the whole policy. This is a
//! filtering stage that sits in the only two places IP traffic passes -
//! `ip_send()` (outbound) and `ip_handle_packet()` (inbound) - and decides each
//! packet from RULES and from CONNECTION STATE.
//!
//! STATE is what makes it a firewall and not a port list. Every flow is tracked
//! (a "conntrack" entry). A reply to something we sent is recognised as such
//! and let in; a packet that belongs to no flow and is not a valid way to start
//! one is INVALID and dropped, whatever the rules say. A rule can therefore say
//! "accept inbound traffic that is part of a connection we started" instead of
//! "accept inbound traffic from port 80".
//!
//! WHAT A PACKET IS CLASSIFIED AS
//!
//!   NEW          the first packet of a flow: a TCP SYN, a UDP datagram, an
//!                ICMP echo request, that matches no tracked flow;
//!   ESTABLISHED  part of a flow whose other side has been heard from;
//!   RELATED      not part of a flow but caused by one: an ICMP error about a
//!                tracked flow, or the other end of a flow a helper expected
//!                (TFTP answers from a port other than the one it was asked on);
//!   INVALID      anything else. Always dropped, not overridable by a rule.
//!
//! INVALID includes: a TCP segment with no connection that is not a SYN; a
//! flag combination no real TCP sends (none, SYN+FIN, SYN+RST, FIN without ACK,
//! the "xmas" set); a segment that is impossible in the connection's current
//! state; an ICMP echo reply nobody asked for, or a second reply to one request;
//! an ICMP error about nothing we track; a truncated or inconsistent header; any
//! IP fragment (this stack does not reassemble, so a fragment's ports cannot be
//! known); any protocol this stack does not handle.
//!
//! A packet that is dropped by a rule does NOT advance connection state.
//!
//! RULES are an ordered list, first match wins, and a per-direction default
//! POLICY applies when none matches. A rule matches on direction, protocol,
//! source and destination network, source and destination port range, packet
//! state, and ICMP type. The baseline (and the shipped FIREWALL.CFG) is
//!
//!     policy in drop
//!     policy out accept
//!     rule accept in state established,related
//!
//! i.e. nothing gets in unless we asked for it, and what we asked for gets back.
//!
//! FAILS CLOSED. An engine that has not been initialised drops everything
//! (all-zero is "policy drop, no rules, not disabled"). A configuration that does
//! not parse is rejected whole and the baseline stays. A full table refuses NEW
//! flows rather than evicting live ones. A flood of inbound half-open TCP
//! connections is capped so it cannot use up the table.
//!
//! LOCK. `lock()` is one-way until reboot: afterwards no rule, policy or enable
//! flag can be changed, so a compromise of an administrator process after boot
//! cannot open the firewall.
//!
//! LIMITS, stated plainly: TCP tracking is by FLAGS and direction, with no
//! sequence-number window, so an off-path attacker who can guess a flow's
//! 5-tuple can still inject a segment that is valid in the flow's state; only
//! one helper (TFTP) exists; IP options and fragments are not interpreted; there
//! is no NAT, no IPv6, no rate limiting beyond the half-open cap.
//!
//! Two layers, as in msg.rs and mac.rs: a PURE core that the host tests attack
//! directly, and a thin `kernel_glue` of `rust_fw_*` exports.

use core::cmp::min;

pub const MAX_RULES: usize = 32;
pub const MAX_CONNS: usize = 256;
pub const MAX_EXPECT: usize = 16;
/// Inbound-initiated TCP connections that have not completed their handshake.
pub const HALF_OPEN_MAX: usize = 64;
pub const TICK_HZ: u32 = 100;
pub const MAX_CONFIG_TEXT: usize = 4096;
pub const MAX_LINE: usize = 200;

pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const EINVAL: i32 = 22;
pub const ENOSPC: i32 = 28;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

pub const DIR_IN: u8 = 0;
pub const DIR_OUT: u8 = 1;
pub const DIR_ANY: u8 = 2;

pub const ST_NEW: u8 = 1;
pub const ST_ESTABLISHED: u8 = 2;
pub const ST_RELATED: u8 = 4;
pub const ST_INVALID: u8 = 8;

pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_RST: u8 = 0x04;
pub const TCP_PSH: u8 = 0x08;
pub const TCP_ACK: u8 = 0x10;
pub const TCP_URG: u8 = 0x20;

/// Why a verdict was reached.
pub const R_RULE: u8 = 1; // a rule matched (accept or drop, see `accept`)
pub const R_POLICY: u8 = 2; // no rule matched; the direction's policy applied
pub const R_INVALID: u8 = 3;
pub const R_FRAGMENT: u8 = 4;
pub const R_UNSUPPORTED: u8 = 5;
pub const R_MALFORMED: u8 = 6;
pub const R_TABLE_FULL: u8 = 7;
pub const R_HALF_OPEN: u8 = 8;
pub const R_DISABLED: u8 = 9; // the firewall is switched off: everything passes

/// Why a packet was INVALID.
pub const INV_NONE: u8 = 0;
pub const INV_FLAGS: u8 = 1;
pub const INV_NO_CONN: u8 = 2;
pub const INV_STATE: u8 = 3;
pub const INV_PORT0: u8 = 4;
pub const INV_ICMP: u8 = 5;
pub const INV_RELATED: u8 = 6;

// Conntrack states. TCP:
pub const TCP_SYN_SENT: u8 = 1;
pub const TCP_SYN_RECV: u8 = 2;
pub const TCP_ESTABLISHED: u8 = 3;
pub const TCP_FIN_WAIT: u8 = 4;
pub const TCP_LAST_ACK: u8 = 5;
pub const TCP_TIME_WAIT: u8 = 6;
pub const TCP_CLOSE: u8 = 7;
// UDP and ICMP:
pub const UDP_UNREPLIED: u8 = 1;
pub const UDP_REPLIED: u8 = 2;
pub const ICMP_ECHO: u8 = 1;

/// The destination port the TFTP helper watches (see `Expect`).
pub const TFTP_PORT: u16 = 69;

const T_SYN: u32 = 60 * TICK_HZ;
const T_ESTABLISHED: u32 = 3600 * TICK_HZ;
const T_FIN_WAIT: u32 = 120 * TICK_HZ;
const T_LAST_ACK: u32 = 30 * TICK_HZ;
const T_TIME_WAIT: u32 = 120 * TICK_HZ;
const T_CLOSE: u32 = 10 * TICK_HZ;
const T_UDP_UNREPLIED: u32 = 30 * TICK_HZ;
const T_UDP_REPLIED: u32 = 180 * TICK_HZ;
const T_ICMP: u32 = 30 * TICK_HZ;
const T_EXPECT: u32 = 30 * TICK_HZ;

// ------------------------------------------------------------------------
// Packets.
// ------------------------------------------------------------------------

/// The bit of a packet that could not be understood, if any.
pub const BAD_NONE: u8 = 0;
pub const BAD_MALFORMED: u8 = 1;
pub const BAD_UNSUPPORTED: u8 = 2;
pub const BAD_FRAGMENT: u8 = 3;

/// The packet an ICMP error message quotes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Inner {
    pub proto: u8,
    pub src: u32,
    pub dst: u32,
    pub sport: u16,
    pub dport: u16,
}

/// What the filter needs to know about one IP packet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pkt {
    pub dir: u8,
    pub proto: u8,
    pub src: u32,
    pub dst: u32,
    pub sport: u16,
    pub dport: u16,
    pub flags: u8,
    pub icmp_type: u8,
    pub icmp_code: u8,
    pub id: u16,
    /// Transport length, for the byte counters.
    pub len: u16,
    pub bad: u8,
    pub inner: Option<Inner>,
}

impl Pkt {
    pub const fn blank() -> Pkt {
        Pkt {
            dir: DIR_IN,
            proto: 0,
            src: 0,
            dst: 0,
            sport: 0,
            dport: 0,
            flags: 0,
            icmp_type: 0,
            icmp_code: 0,
            id: 0,
            len: 0,
            bad: BAD_NONE,
            inner: None,
        }
    }
}

fn be16(b: &[u8], i: usize) -> u16 {
    ((b[i] as u16) << 8) | b[i + 1] as u16
}

fn be32(b: &[u8], i: usize) -> u32 {
    ((b[i] as u32) << 24) | ((b[i + 1] as u32) << 16) | ((b[i + 2] as u32) << 8) | b[i + 3] as u32
}

fn is_icmp_error(t: u8) -> bool {
    matches!(t, 3 | 4 | 5 | 11 | 12)
}

/// Parses the transport header of one IP packet. NEVER panics and never reads
/// past `d`: anything that does not hold together comes back marked `bad`.
/// `frag` is true if the IP header said this is any part of a fragmented packet.
pub fn parse_packet(dir: u8, src: u32, dst: u32, proto: u8, frag: bool, d: &[u8]) -> Pkt {
    let mut p = Pkt::blank();
    p.dir = dir;
    p.proto = proto;
    p.src = src;
    p.dst = dst;
    p.len = min(d.len(), 0xFFFF) as u16;
    if frag {
        p.bad = BAD_FRAGMENT;
        return p;
    }
    match proto {
        PROTO_TCP => {
            if d.len() < 20 {
                p.bad = BAD_MALFORMED;
                return p;
            }
            let doff = (d[12] >> 4) as usize * 4;
            if doff < 20 || doff > d.len() {
                p.bad = BAD_MALFORMED;
                return p;
            }
            p.sport = be16(d, 0);
            p.dport = be16(d, 2);
            p.flags = d[13] & 0x3F;
        }
        PROTO_UDP => {
            if d.len() < 8 {
                p.bad = BAD_MALFORMED;
                return p;
            }
            let ulen = be16(d, 4) as usize;
            if ulen < 8 || ulen > d.len() {
                p.bad = BAD_MALFORMED;
                return p;
            }
            p.sport = be16(d, 0);
            p.dport = be16(d, 2);
        }
        PROTO_ICMP => {
            if d.len() < 8 {
                p.bad = BAD_MALFORMED;
                return p;
            }
            p.icmp_type = d[0];
            p.icmp_code = d[1];
            if p.icmp_type == 0 || p.icmp_type == 8 {
                p.id = be16(d, 4);
            } else if is_icmp_error(p.icmp_type) {
                let ip = &d[8..];
                if ip.len() < 20 || ip[0] >> 4 != 4 {
                    p.bad = BAD_MALFORMED;
                    return p;
                }
                let ihl = (ip[0] & 0x0F) as usize * 4;
                if ihl < 20 || ip.len() < ihl + 4 {
                    p.bad = BAD_MALFORMED;
                    return p;
                }
                let iproto = ip[9];
                let isrc = be32(ip, 12);
                let idst = be32(ip, 16);
                match iproto {
                    PROTO_TCP | PROTO_UDP => {
                        p.inner = Some(Inner {
                            proto: iproto,
                            src: isrc,
                            dst: idst,
                            sport: be16(ip, ihl),
                            dport: be16(ip, ihl + 2),
                        });
                    }
                    PROTO_ICMP => {
                        if ip.len() >= ihl + 8 && (ip[ihl] == 8 || ip[ihl] == 0) {
                            let id = be16(ip, ihl + 4);
                            p.inner = Some(Inner { proto: iproto, src: isrc, dst: idst, sport: id, dport: id });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {
            p.bad = BAD_UNSUPPORTED;
        }
    }
    p
}

// ------------------------------------------------------------------------
// Rules.
// ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rule {
    pub dir: u8,
    pub proto: u8,
    pub accept: bool,
    pub log: bool,
    pub src_ip: u32,
    pub src_mask: u32,
    pub dst_ip: u32,
    pub dst_mask: u32,
    pub sp_set: bool,
    pub sp_lo: u16,
    pub sp_hi: u16,
    pub dp_set: bool,
    pub dp_lo: u16,
    pub dp_hi: u16,
    pub states: u8,
    pub icmp_set: bool,
    pub icmp_type: u8,
}

impl Rule {
    pub const EMPTY: Rule = Rule {
        dir: DIR_IN,
        proto: 0,
        accept: false,
        log: false,
        src_ip: 0,
        src_mask: 0,
        dst_ip: 0,
        dst_mask: 0,
        sp_set: false,
        sp_lo: 0,
        sp_hi: 0,
        dp_set: false,
        dp_lo: 0,
        dp_hi: 0,
        states: 0,
        icmp_set: false,
        icmp_type: 0,
    };

    pub fn matches(&self, p: &Pkt, state: u8) -> bool {
        if self.dir != DIR_ANY && self.dir != p.dir {
            return false;
        }
        if self.proto != 0 && self.proto != p.proto {
            return false;
        }
        if p.src & self.src_mask != self.src_ip || p.dst & self.dst_mask != self.dst_ip {
            return false;
        }
        let has_ports = p.proto == PROTO_TCP || p.proto == PROTO_UDP;
        if self.sp_set && (!has_ports || p.sport < self.sp_lo || p.sport > self.sp_hi) {
            return false;
        }
        if self.dp_set && (!has_ports || p.dport < self.dp_lo || p.dport > self.dp_hi) {
            return false;
        }
        if self.states != 0 && self.states & state == 0 {
            return false;
        }
        if self.icmp_set && (p.proto != PROTO_ICMP || p.icmp_type != self.icmp_type) {
            return false;
        }
        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParseError {
    TooLarge,
    BadByte,
    LineTooLong(u16),
    Syntax(u16),
    BadAddress(u16),
    BadPort(u16),
    BadState(u16),
    BadCombination(u16),
    TooManyRules(u16),
}

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

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
        ip = (ip << 8) | parse_u32_dec(part, 255)?;
    }
    if parts == 4 {
        Some(ip)
    } else {
        None
    }
}

/// `any`, `a.b.c.d` or `a.b.c.d/n`. Returns (network, mask), with the network
/// already masked so that comparing `addr & mask == network` is the whole test.
fn parse_cidr(s: &[u8]) -> Option<(u32, u32)> {
    if s == b"any" {
        return Some((0, 0));
    }
    let (ip_s, bits) = match s.iter().position(|&b| b == b'/') {
        None => (s, 32u32),
        Some(i) => (&s[..i], parse_u32_dec(&s[i + 1..], 32)?),
    };
    let ip = parse_ip(ip_s)?;
    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
    Some((ip & mask, mask))
}

/// `any`, `N` or `lo-hi`.
fn parse_ports(s: &[u8]) -> Option<(bool, u16, u16)> {
    if s == b"any" {
        return Some((false, 0, 0));
    }
    match s.iter().position(|&b| b == b'-') {
        None => {
            let v = parse_u32_dec(s, 65535)? as u16;
            Some((true, v, v))
        }
        Some(i) => {
            let lo = parse_u32_dec(&s[..i], 65535)? as u16;
            let hi = parse_u32_dec(&s[i + 1..], 65535)? as u16;
            if lo <= hi {
                Some((true, lo, hi))
            } else {
                None
            }
        }
    }
}

fn parse_states(s: &[u8]) -> Option<u8> {
    let mut m = 0u8;
    for part in s.split(|&b| b == b',') {
        m |= match part {
            b"new" => ST_NEW,
            b"established" => ST_ESTABLISHED,
            b"related" => ST_RELATED,
            // INVALID packets are always dropped; a rule that claimed to treat
            // them would be a rule that does nothing, and nothing is worse
            // than an error when what is being written is a security policy.
            _ => return None,
        };
    }
    if m == 0 {
        None
    } else {
        Some(m)
    }
}

/// `rule <accept|drop> <in|out|any> [proto P] [from CIDR] [to CIDR] [sport R]
/// [dport R] [state LIST] [icmp-type N] [log]`, with `rule` already consumed.
fn parse_rule_tokens(line: &[u8], pos: &mut usize, ln: u16) -> Result<Rule, ParseError> {
    let mut r = Rule::EMPTY;
    r.accept = match next_token(line, pos).ok_or(ParseError::Syntax(ln))? {
        b"accept" => true,
        b"drop" => false,
        _ => return Err(ParseError::Syntax(ln)),
    };
    r.dir = match next_token(line, pos).ok_or(ParseError::Syntax(ln))? {
        b"in" => DIR_IN,
        b"out" => DIR_OUT,
        b"any" => DIR_ANY,
        _ => return Err(ParseError::Syntax(ln)),
    };
    let (mut seen_proto, mut seen_from, mut seen_to, mut seen_sp, mut seen_dp, mut seen_state, mut seen_icmp) =
        (false, false, false, false, false, false, false);
    while let Some(key) = next_token(line, pos) {
        if key == b"log" {
            if r.log {
                return Err(ParseError::Syntax(ln));
            }
            r.log = true;
            continue;
        }
        let val = next_token(line, pos).ok_or(ParseError::Syntax(ln))?;
        match key {
            b"proto" if !seen_proto => {
                seen_proto = true;
                r.proto = match val {
                    b"tcp" => PROTO_TCP,
                    b"udp" => PROTO_UDP,
                    b"icmp" => PROTO_ICMP,
                    b"any" => 0,
                    _ => return Err(ParseError::Syntax(ln)),
                };
            }
            b"from" if !seen_from => {
                seen_from = true;
                let (ip, m) = parse_cidr(val).ok_or(ParseError::BadAddress(ln))?;
                r.src_ip = ip;
                r.src_mask = m;
            }
            b"to" if !seen_to => {
                seen_to = true;
                let (ip, m) = parse_cidr(val).ok_or(ParseError::BadAddress(ln))?;
                r.dst_ip = ip;
                r.dst_mask = m;
            }
            b"sport" if !seen_sp => {
                seen_sp = true;
                let (set, lo, hi) = parse_ports(val).ok_or(ParseError::BadPort(ln))?;
                r.sp_set = set;
                r.sp_lo = lo;
                r.sp_hi = hi;
            }
            b"dport" if !seen_dp => {
                seen_dp = true;
                let (set, lo, hi) = parse_ports(val).ok_or(ParseError::BadPort(ln))?;
                r.dp_set = set;
                r.dp_lo = lo;
                r.dp_hi = hi;
            }
            b"state" if !seen_state => {
                seen_state = true;
                r.states = parse_states(val).ok_or(ParseError::BadState(ln))?;
            }
            b"icmp-type" if !seen_icmp => {
                seen_icmp = true;
                r.icmp_set = true;
                r.icmp_type = parse_u32_dec(val, 255).ok_or(ParseError::Syntax(ln))? as u8;
            }
            _ => return Err(ParseError::Syntax(ln)),
        }
    }
    // A port means TCP or UDP, an ICMP type means ICMP: say so rather than
    // write a rule that can never match what its author thinks it matches.
    if (r.sp_set || r.dp_set) && r.proto != PROTO_TCP && r.proto != PROTO_UDP {
        return Err(ParseError::BadCombination(ln));
    }
    if r.icmp_set && r.proto != PROTO_ICMP {
        return Err(ParseError::BadCombination(ln));
    }
    Ok(r)
}

/// One `rule ...` line (the whole line, including the word `rule`).
pub fn parse_rule(text: &[u8]) -> Result<Rule, ParseError> {
    let mut pos = 0;
    if next_token(text, &mut pos) != Some(b"rule") {
        return Err(ParseError::Syntax(1));
    }
    parse_rule_tokens(text, &mut pos, 1)
}

#[derive(Clone, Copy)]
pub struct Config {
    pub rules: [Rule; MAX_RULES],
    pub n: usize,
    pub policy_in: bool,
    pub policy_out: bool,
}

/// The built-in configuration: nothing gets in unless we asked for it, and what
/// we asked for gets back. Also what FIREWALL.CFG ships as.
pub const BASELINE: &[u8] =
    b"policy in drop\npolicy out accept\nrule accept in state established,related\n";

/// Parses a whole configuration. Strict on purpose: an unknown word, a bad
/// address, too many rules, a stray byte are all errors, because a firewall
/// policy that is only partly understood is a policy whose meaning is a guess.
/// The default stance, for a file that names none, is the baseline's.
pub fn parse_config(text: &[u8]) -> Result<Config, ParseError> {
    if text.len() > MAX_CONFIG_TEXT {
        return Err(ParseError::TooLarge);
    }
    for &b in text {
        if !(b == b'\n' || b == b'\r' || b == b'\t' || (0x20..0x7f).contains(&b)) {
            return Err(ParseError::BadByte);
        }
    }
    let mut c = Config { rules: [Rule::EMPTY; MAX_RULES], n: 0, policy_in: false, policy_out: true };
    let (mut seen_in, mut seen_out) = (false, false);
    let mut ln: u16 = 0;
    for raw in text.split(|&b| b == b'\n') {
        ln = ln.saturating_add(1);
        let mut line = raw;
        if let Some((&b'\r', rest)) = line.split_last() {
            line = rest;
        }
        if line.len() > MAX_LINE {
            return Err(ParseError::LineTooLong(ln));
        }
        if let Some(i) = line.iter().position(|&b| b == b'#') {
            line = &line[..i];
        }
        let mut pos = 0;
        let word = match next_token(line, &mut pos) {
            None => continue,
            Some(w) => w,
        };
        match word {
            b"policy" => {
                let dir = next_token(line, &mut pos).ok_or(ParseError::Syntax(ln))?;
                let act = next_token(line, &mut pos).ok_or(ParseError::Syntax(ln))?;
                let accept = match act {
                    b"accept" => true,
                    b"drop" => false,
                    _ => return Err(ParseError::Syntax(ln)),
                };
                if next_token(line, &mut pos).is_some() {
                    return Err(ParseError::Syntax(ln));
                }
                match dir {
                    b"in" if !seen_in => {
                        seen_in = true;
                        c.policy_in = accept;
                    }
                    b"out" if !seen_out => {
                        seen_out = true;
                        c.policy_out = accept;
                    }
                    _ => return Err(ParseError::Syntax(ln)),
                }
            }
            b"rule" => {
                if c.n >= MAX_RULES {
                    return Err(ParseError::TooManyRules(ln));
                }
                c.rules[c.n] = parse_rule_tokens(line, &mut pos, ln)?;
                c.n += 1;
            }
            _ => return Err(ParseError::Syntax(ln)),
        }
    }
    Ok(c)
}

// ------------------------------------------------------------------------
// Connection tracking.
// ------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Conn {
    used: bool,
    proto: u8,
    /// The direction of the packet that started the flow.
    orig_dir: u8,
    state: u8,
    /// For TCP FIN_WAIT/LAST_ACK: which side sent the first FIN (0 = the
    /// initiator, 1 = the responder).
    fin_from: u8,
    /// The other side has been heard from.
    replied: bool,
    /// ICMP echo: requests sent that have not been answered.
    pending: u8,
    a_ip: u32,
    b_ip: u32,
    a_port: u16,
    b_port: u16,
    expires: u32,
    created: u32,
    pkts: [u32; 2],
    bytes: [u32; 2],
}

impl Conn {
    const EMPTY: Conn = Conn {
        used: false,
        proto: 0,
        orig_dir: 0,
        state: 0,
        fin_from: 0,
        replied: false,
        pending: 0,
        a_ip: 0,
        b_ip: 0,
        a_port: 0,
        b_port: 0,
        expires: 0,
        created: 0,
        pkts: [0; 2],
        bytes: [0; 2],
    };
}

/// A flow a helper has been told to expect (see `TFTP_PORT`).
#[derive(Clone, Copy)]
struct Expect {
    used: bool,
    remote_ip: u32,
    local_ip: u32,
    local_port: u16,
    expires: u32,
}

impl Expect {
    const EMPTY: Expect = Expect { used: false, remote_ip: 0, local_ip: 0, local_port: 0, expires: 0 };
}

#[derive(Clone, Copy)]
struct Key {
    proto: u8,
    src: u32,
    sport: u16,
    dst: u32,
    dport: u16,
}

fn key_of(p: &Pkt) -> Key {
    if p.proto == PROTO_ICMP {
        // An echo's identifier stands in for BOTH ports, so the request and
        // its reply have the same "ports" and differ only in which way round
        // the addresses are.
        Key { proto: p.proto, src: p.src, sport: p.id, dst: p.dst, dport: p.id }
    } else {
        Key { proto: p.proto, src: p.src, sport: p.sport, dst: p.dst, dport: p.dport }
    }
}

fn side_of(c: &Conn, k: &Key) -> Option<u8> {
    if c.proto != k.proto {
        return None;
    }
    if c.a_ip == k.src && c.a_port == k.sport && c.b_ip == k.dst && c.b_port == k.dport {
        Some(0)
    } else if c.b_ip == k.src && c.b_port == k.sport && c.a_ip == k.dst && c.a_port == k.dport {
        Some(1)
    } else {
        None
    }
}

fn expired(expires: u32, now: u32) -> bool {
    (now.wrapping_sub(expires) as i32) >= 0
}

/// The flag classes a real TCP sends. Everything else is INVALID.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fc {
    Syn,
    SynAck,
    Ack,
    FinAck,
    Rst,
}

fn tcp_class(f: u8) -> Option<Fc> {
    let f = f & 0x3F;
    if f == TCP_SYN {
        Some(Fc::Syn)
    } else if f == TCP_SYN | TCP_ACK {
        Some(Fc::SynAck)
    } else if f & (TCP_SYN | TCP_FIN | TCP_RST) == 0 && f & TCP_ACK != 0 {
        Some(Fc::Ack)
    } else if f & (TCP_SYN | TCP_RST) == 0 && f & TCP_FIN != 0 && f & TCP_ACK != 0 {
        Some(Fc::FinAck)
    } else if f & (TCP_SYN | TCP_FIN) == 0 && f & TCP_RST != 0 {
        Some(Fc::Rst)
    } else {
        None
    }
}

/// The TCP state machine, by direction and flag class. `side` is 0 for a
/// packet from the connection's initiator, 1 for one from the responder.
/// Returns the next state and the side that sent the first FIN, or None if this
/// packet cannot happen in this state.
fn tcp_next(state: u8, side: u8, fc: Fc, fin_from: u8) -> Option<(u8, u8)> {
    use Fc::*;
    let s = |n: u8| Some((n, fin_from));
    match (state, side, fc) {
        (TCP_SYN_SENT, 0, Syn) => s(TCP_SYN_SENT),
        (TCP_SYN_SENT, 1, SynAck) => s(TCP_SYN_RECV),
        // simultaneous open: the responder started with a SYN of its own
        (TCP_SYN_SENT, 1, Syn) => s(TCP_SYN_RECV),
        (TCP_SYN_SENT, _, Rst) => s(TCP_CLOSE),

        (TCP_SYN_RECV, 1, SynAck) | (TCP_SYN_RECV, 0, Syn) | (TCP_SYN_RECV, 0, SynAck) => s(TCP_SYN_RECV),
        (TCP_SYN_RECV, 0, Ack) => s(TCP_ESTABLISHED),
        // the other end's ACK of a simultaneous open
        (TCP_SYN_RECV, 1, Ack) => s(TCP_ESTABLISHED),
        (TCP_SYN_RECV, sd, FinAck) => Some((TCP_FIN_WAIT, sd)),
        (TCP_SYN_RECV, _, Rst) => s(TCP_CLOSE),

        (TCP_ESTABLISHED, _, Ack) => s(TCP_ESTABLISHED),
        // our SYN-ACK was lost and is being retransmitted
        (TCP_ESTABLISHED, 1, SynAck) => s(TCP_ESTABLISHED),
        (TCP_ESTABLISHED, sd, FinAck) => Some((TCP_FIN_WAIT, sd)),
        (TCP_ESTABLISHED, _, Rst) => s(TCP_CLOSE),

        (TCP_FIN_WAIT, _, Ack) => s(TCP_FIN_WAIT),
        (TCP_FIN_WAIT, sd, FinAck) if sd == fin_from => s(TCP_FIN_WAIT),
        (TCP_FIN_WAIT, _, FinAck) => s(TCP_LAST_ACK),
        (TCP_FIN_WAIT, _, Rst) => s(TCP_CLOSE),

        (TCP_LAST_ACK, sd, Ack) => {
            if sd == fin_from {
                s(TCP_TIME_WAIT)
            } else {
                s(TCP_LAST_ACK)
            }
        }
        (TCP_LAST_ACK, _, FinAck) => s(TCP_LAST_ACK),
        (TCP_LAST_ACK, _, Rst) => s(TCP_CLOSE),

        (TCP_TIME_WAIT, _, Ack) | (TCP_TIME_WAIT, _, FinAck) => s(TCP_TIME_WAIT),
        (TCP_TIME_WAIT, _, Rst) => s(TCP_CLOSE),

        (TCP_CLOSE, _, Ack) | (TCP_CLOSE, _, Rst) | (TCP_CLOSE, _, FinAck) => s(TCP_CLOSE),

        _ => None,
    }
}

fn conn_timeout(c: &Conn) -> u32 {
    match c.proto {
        PROTO_TCP => match c.state {
            TCP_SYN_SENT | TCP_SYN_RECV => T_SYN,
            TCP_ESTABLISHED => T_ESTABLISHED,
            TCP_FIN_WAIT => T_FIN_WAIT,
            TCP_LAST_ACK => T_LAST_ACK,
            TCP_TIME_WAIT => T_TIME_WAIT,
            _ => T_CLOSE,
        },
        PROTO_UDP => {
            if c.replied {
                T_UDP_REPLIED
            } else {
                T_UDP_UNREPLIED
            }
        }
        _ => T_ICMP,
    }
}

/// How a packet relates to the tracked flows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cls {
    Invalid(u8),
    Existing { idx: usize, side: u8, next: u8, fin_from: u8 },
    Related { idx: usize },
    RelatedNew { ex: usize },
    New,
    /// A SYN for a tuple whose old flow is finished (TIME_WAIT / CLOSE).
    Replace { idx: usize },
}

// ------------------------------------------------------------------------
// The firewall.
// ------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Stats {
    pub accepted_in: u32,
    pub accepted_out: u32,
    pub dropped_in: u32,
    pub dropped_out: u32,
    pub invalid: u32,
    pub fragments: u32,
    pub table_full: u32,
    pub half_open: u32,
    pub new_conns: u32,
    pub related: u32,
    pub unsupported: u32,
    pub malformed: u32,
    pub logged: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Verdict {
    pub accept: bool,
    /// The packet's state as the rules saw it (ST_*; ST_INVALID if invalid).
    pub state: u8,
    /// The rule that decided it, or -1 if none (policy, or not rule-driven).
    pub rule: i32,
    pub reason: u8,
    /// If INVALID, why (INV_*).
    pub detail: u8,
    /// The tracked flow's state after this packet (0 if none).
    pub ct_state: u8,
    /// A log line is wanted (already rate limited).
    pub log: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdminError {
    Locked,
    BadIndex,
    Full,
    Parse(ParseError),
}

impl AdminError {
    pub fn errno(self) -> i32 {
        match self {
            AdminError::Locked => EPERM,
            AdminError::BadIndex => ENOENT,
            AdminError::Full => ENOSPC,
            AdminError::Parse(_) => EINVAL,
        }
    }
}

/// What one tracked flow looks like from outside.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConnInfo {
    pub proto: u8,
    pub state: u8,
    pub orig_dir: u8,
    pub replied: bool,
    pub a_ip: u32,
    pub b_ip: u32,
    pub a_port: u16,
    pub b_port: u16,
    pub remaining: u32,
    pub age: u32,
    pub pkts: [u32; 2],
    pub bytes: [u32; 2],
    pub pending: u8,
}

pub struct Firewall {
    rules: [Rule; MAX_RULES],
    nrules: u8,
    hits: [u32; MAX_RULES],
    hit_bytes: [u32; MAX_RULES],
    /// Stored as "accept": all-zero is DROP, so a firewall nobody initialised
    /// drops everything in both directions.
    policy_in: bool,
    policy_out: bool,
    disabled: bool,
    locked: bool,
    conns: [Conn; MAX_CONNS],
    expects: [Expect; MAX_EXPECT],
    stats: Stats,
    log_seq: u32,
}

impl Firewall {
    /// All-zero: policy drop in both directions, no rules, not disabled,
    /// not locked - so it lives in .bss and fails closed until `init()`.
    pub const fn new() -> Firewall {
        Firewall {
            rules: [Rule::EMPTY; MAX_RULES],
            nrules: 0,
            hits: [0; MAX_RULES],
            hit_bytes: [0; MAX_RULES],
            policy_in: false,
            policy_out: false,
            disabled: false,
            locked: false,
            conns: [Conn::EMPTY; MAX_CONNS],
            expects: [Expect::EMPTY; MAX_EXPECT],
            stats: Stats {
                accepted_in: 0,
                accepted_out: 0,
                dropped_in: 0,
                dropped_out: 0,
                invalid: 0,
                fragments: 0,
                table_full: 0,
                half_open: 0,
                new_conns: 0,
                related: 0,
                unsupported: 0,
                malformed: 0,
                logged: 0,
            },
            log_seq: 0,
        }
    }

    /// Back to the built-in baseline, with no tracked flows and the counters
    /// at zero. This is boot, so it also clears the lock.
    pub fn init(&mut self) {
        *self = Firewall::new();
        if let Ok(c) = parse_config(BASELINE) {
            self.load_config_unchecked(&c);
        }
    }

    fn load_config_unchecked(&mut self, c: &Config) {
        self.rules = [Rule::EMPTY; MAX_RULES];
        self.rules[..c.n].copy_from_slice(&c.rules[..c.n]);
        self.nrules = c.n as u8;
        self.hits = [0; MAX_RULES];
        self.hit_bytes = [0; MAX_RULES];
        self.policy_in = c.policy_in;
        self.policy_out = c.policy_out;
    }

    // ---- administration (all refused once locked) ----

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn is_enabled(&self) -> bool {
        !self.disabled
    }

    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// Replaces the rules and policies with a parsed configuration. Atomic: on
    /// a parse error nothing changes. Tracked flows are kept.
    pub fn apply_config(&mut self, text: &[u8]) -> Result<(), AdminError> {
        if self.locked {
            return Err(AdminError::Locked);
        }
        let c = parse_config(text).map_err(AdminError::Parse)?;
        self.load_config_unchecked(&c);
        Ok(())
    }

    /// Inserts a rule at `pos` (0 = first; `nrules` = last).
    pub fn add_rule(&mut self, pos: usize, rule: Rule) -> Result<(), AdminError> {
        if self.locked {
            return Err(AdminError::Locked);
        }
        let n = self.nrules as usize;
        if n >= MAX_RULES {
            return Err(AdminError::Full);
        }
        if pos > n {
            return Err(AdminError::BadIndex);
        }
        let mut i = n;
        while i > pos {
            self.rules[i] = self.rules[i - 1];
            self.hits[i] = self.hits[i - 1];
            self.hit_bytes[i] = self.hit_bytes[i - 1];
            i -= 1;
        }
        self.rules[pos] = rule;
        self.hits[pos] = 0;
        self.hit_bytes[pos] = 0;
        self.nrules += 1;
        Ok(())
    }

    pub fn add_rule_text(&mut self, pos: usize, text: &[u8]) -> Result<(), AdminError> {
        if self.locked {
            return Err(AdminError::Locked);
        }
        let r = parse_rule(text).map_err(AdminError::Parse)?;
        self.add_rule(pos, r)
    }

    pub fn del_rule(&mut self, idx: usize) -> Result<(), AdminError> {
        if self.locked {
            return Err(AdminError::Locked);
        }
        let n = self.nrules as usize;
        if idx >= n {
            return Err(AdminError::BadIndex);
        }
        for i in idx..n - 1 {
            self.rules[i] = self.rules[i + 1];
            self.hits[i] = self.hits[i + 1];
            self.hit_bytes[i] = self.hit_bytes[i + 1];
        }
        self.rules[n - 1] = Rule::EMPTY;
        self.hits[n - 1] = 0;
        self.hit_bytes[n - 1] = 0;
        self.nrules -= 1;
        Ok(())
    }

    pub fn flush_rules(&mut self) -> Result<(), AdminError> {
        if self.locked {
            return Err(AdminError::Locked);
        }
        self.rules = [Rule::EMPTY; MAX_RULES];
        self.hits = [0; MAX_RULES];
        self.hit_bytes = [0; MAX_RULES];
        self.nrules = 0;
        Ok(())
    }

    pub fn set_policy(&mut self, dir: u8, accept: bool) -> Result<(), AdminError> {
        if self.locked {
            return Err(AdminError::Locked);
        }
        match dir {
            DIR_IN => self.policy_in = accept,
            DIR_OUT => self.policy_out = accept,
            _ => return Err(AdminError::BadIndex),
        }
        Ok(())
    }

    pub fn set_enabled(&mut self, on: bool) -> Result<(), AdminError> {
        if self.locked {
            return Err(AdminError::Locked);
        }
        self.disabled = !on;
        Ok(())
    }

    /// Forgets every tracked flow and expectation. (Not a policy change, so it
    /// is allowed after the lock: it can only make the firewall stricter.)
    pub fn flush_conns(&mut self) {
        self.conns = [Conn::EMPTY; MAX_CONNS];
        self.expects = [Expect::EMPTY; MAX_EXPECT];
    }

    // ---- introspection ----

    pub fn rule_count(&self) -> usize {
        self.nrules as usize
    }

    pub fn rule_at(&self, i: usize) -> Option<(Rule, u32, u32)> {
        if i < self.nrules as usize {
            Some((self.rules[i], self.hits[i], self.hit_bytes[i]))
        } else {
            None
        }
    }

    pub fn policy(&self, dir: u8) -> bool {
        if dir == DIR_IN {
            self.policy_in
        } else {
            self.policy_out
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn active_conns(&self, now: u32) -> usize {
        self.conns.iter().filter(|c| c.used && !expired(c.expires, now)).count()
    }

    pub fn active_expects(&self, now: u32) -> usize {
        self.expects.iter().filter(|e| e.used && !expired(e.expires, now)).count()
    }

    /// The n-th live tracked flow, in table order.
    pub fn conn_nth(&self, n: usize, now: u32) -> Option<ConnInfo> {
        self.conns
            .iter()
            .filter(|c| c.used && !expired(c.expires, now))
            .nth(n)
            .map(|c| ConnInfo {
                proto: c.proto,
                state: c.state,
                orig_dir: c.orig_dir,
                replied: c.replied,
                a_ip: c.a_ip,
                b_ip: c.b_ip,
                a_port: c.a_port,
                b_port: c.b_port,
                remaining: c.expires.wrapping_sub(now),
                age: now.wrapping_sub(c.created),
                pkts: c.pkts,
                bytes: c.bytes,
                pending: c.pending,
            })
    }

    // ---- the filter ----

    fn find(&self, k: &Key, now: u32, want_side: Option<u8>) -> Option<(usize, u8)> {
        for (i, c) in self.conns.iter().enumerate() {
            if !c.used || expired(c.expires, now) {
                continue;
            }
            if let Some(s) = side_of(c, k) {
                if want_side.map_or(true, |w| w == s) {
                    return Some((i, s));
                }
            }
        }
        None
    }

    fn has_free_slot(&self, now: u32) -> bool {
        self.conns.iter().any(|c| !c.used || expired(c.expires, now))
    }

    fn half_open_inbound(&self, now: u32) -> usize {
        self.conns
            .iter()
            .filter(|c| {
                c.used
                    && !expired(c.expires, now)
                    && c.proto == PROTO_TCP
                    && c.orig_dir == DIR_IN
                    && (c.state == TCP_SYN_SENT || c.state == TCP_SYN_RECV)
            })
            .count()
    }

    fn find_expect(&self, p: &Pkt, now: u32) -> Option<usize> {
        if p.dir != DIR_IN || p.proto != PROTO_UDP {
            return None;
        }
        self.expects.iter().position(|e| {
            e.used && !expired(e.expires, now) && e.remote_ip == p.src && e.local_ip == p.dst && e.local_port == p.dport
        })
    }

    fn classify(&self, p: &Pkt, now: u32) -> Cls {
        match p.proto {
            PROTO_TCP => {
                if p.sport == 0 || p.dport == 0 {
                    return Cls::Invalid(INV_PORT0);
                }
                let fc = match tcp_class(p.flags) {
                    Some(f) => f,
                    None => return Cls::Invalid(INV_FLAGS),
                };
                let k = key_of(p);
                match self.find(&k, now, None) {
                    Some((idx, side)) => {
                        let c = &self.conns[idx];
                        if fc == Fc::Syn && (c.state == TCP_TIME_WAIT || c.state == TCP_CLOSE) {
                            return Cls::Replace { idx };
                        }
                        match tcp_next(c.state, side, fc, c.fin_from) {
                            Some((next, ff)) => Cls::Existing { idx, side, next, fin_from: ff },
                            None => Cls::Invalid(INV_STATE),
                        }
                    }
                    None => {
                        if fc == Fc::Syn {
                            Cls::New
                        } else {
                            Cls::Invalid(INV_NO_CONN)
                        }
                    }
                }
            }
            PROTO_UDP => {
                if p.dport == 0 {
                    return Cls::Invalid(INV_PORT0);
                }
                let k = key_of(p);
                match self.find(&k, now, None) {
                    Some((idx, side)) => Cls::Existing {
                        idx,
                        side,
                        next: UDP_REPLIED,
                        fin_from: 0,
                    },
                    None => match self.find_expect(p, now) {
                        Some(ex) => Cls::RelatedNew { ex },
                        None => Cls::New,
                    },
                }
            }
            PROTO_ICMP => {
                if p.icmp_code != 0 && (p.icmp_type == 0 || p.icmp_type == 8) {
                    return Cls::Invalid(INV_ICMP);
                }
                let k = key_of(p);
                match p.icmp_type {
                    8 => match self.find(&k, now, Some(0)) {
                        Some((idx, side)) => Cls::Existing { idx, side, next: ICMP_ECHO, fin_from: 0 },
                        None => Cls::New,
                    },
                    0 => match self.find(&k, now, Some(1)) {
                        Some((idx, side)) if self.conns[idx].pending > 0 => {
                            Cls::Existing { idx, side, next: ICMP_ECHO, fin_from: 0 }
                        }
                        _ => Cls::Invalid(INV_NO_CONN),
                    },
                    t if is_icmp_error(t) => match p.inner {
                        None => Cls::Invalid(INV_RELATED),
                        Some(inner) => {
                            let k = Key {
                                proto: inner.proto,
                                src: inner.src,
                                sport: inner.sport,
                                dst: inner.dst,
                                dport: inner.dport,
                            };
                            match self.find(&k, now, None) {
                                Some((idx, _)) => Cls::Related { idx },
                                None => Cls::Invalid(INV_RELATED),
                            }
                        }
                    },
                    _ => Cls::Invalid(INV_ICMP),
                }
            }
            _ => Cls::Invalid(INV_NONE),
        }
    }

    fn state_of(&self, cls: &Cls) -> u8 {
        match *cls {
            Cls::Invalid(_) => ST_INVALID,
            Cls::New | Cls::Replace { .. } => ST_NEW,
            Cls::Related { .. } | Cls::RelatedNew { .. } => ST_RELATED,
            Cls::Existing { idx, side, .. } => {
                if side == 1 || self.conns[idx].replied {
                    ST_ESTABLISHED
                } else {
                    ST_NEW
                }
            }
        }
    }

    fn alloc_slot(&mut self, now: u32) -> Option<usize> {
        let i = self.conns.iter().position(|c| !c.used || expired(c.expires, now))?;
        self.conns[i] = Conn::EMPTY;
        Some(i)
    }

    fn start_conn(&mut self, i: usize, p: &Pkt, now: u32, related: bool) {
        let k = key_of(p);
        let c = &mut self.conns[i];
        *c = Conn::EMPTY;
        c.used = true;
        c.proto = p.proto;
        c.orig_dir = p.dir;
        c.a_ip = k.src;
        c.a_port = k.sport;
        c.b_ip = k.dst;
        c.b_port = k.dport;
        c.created = now;
        c.state = match p.proto {
            PROTO_TCP => TCP_SYN_SENT,
            PROTO_UDP => UDP_UNREPLIED,
            _ => ICMP_ECHO,
        };
        if related {
            // the flow was expected, so it counts as answered from the start
            c.replied = true;
            c.state = UDP_REPLIED;
        }
        if p.proto == PROTO_ICMP {
            c.pending = 1;
        }
        c.pkts[0] = 1;
        c.bytes[0] = p.len as u32;
        c.expires = now.wrapping_add(conn_timeout(c));
    }

    fn add_expect(&mut self, remote_ip: u32, local_ip: u32, local_port: u16, now: u32) {
        let slot = self
            .expects
            .iter()
            .position(|e| e.used && e.remote_ip == remote_ip && e.local_ip == local_ip && e.local_port == local_port)
            .or_else(|| self.expects.iter().position(|e| !e.used || expired(e.expires, now)))
            // full of live expectations: the oldest-created is the first slot
            // we find, which is good enough for a helper this small
            .unwrap_or(0);
        self.expects[slot] = Expect {
            used: true,
            remote_ip,
            local_ip,
            local_port,
            expires: now.wrapping_add(T_EXPECT),
        };
    }

    fn commit(&mut self, p: &Pkt, cls: Cls, now: u32) -> u8 {
        match cls {
            Cls::Existing { idx, side, next, fin_from } => {
                let proto = p.proto;
                let c = &mut self.conns[idx];
                c.pkts[side as usize] = c.pkts[side as usize].wrapping_add(1);
                c.bytes[side as usize] = c.bytes[side as usize].wrapping_add(p.len as u32);
                match proto {
                    PROTO_TCP => {
                        c.state = next;
                        c.fin_from = fin_from;
                        if side == 1 {
                            c.replied = true;
                        }
                    }
                    PROTO_UDP => {
                        if side == 1 {
                            c.replied = true;
                            c.state = UDP_REPLIED;
                        }
                    }
                    _ => {
                        if side == 0 {
                            c.pending = c.pending.saturating_add(1);
                        } else {
                            c.pending = c.pending.saturating_sub(1);
                            c.replied = true;
                        }
                    }
                }
                c.expires = now.wrapping_add(conn_timeout(c));
                c.state
            }
            Cls::Related { idx } => {
                let c = &mut self.conns[idx];
                // an error about a flow keeps that flow alive and is counted,
                // but changes none of its state
                c.pkts[0] = c.pkts[0].wrapping_add(0);
                self.stats.related = self.stats.related.wrapping_add(1);
                self.conns[idx].state
            }
            Cls::RelatedNew { ex } => {
                self.expects[ex].used = false;
                self.stats.related = self.stats.related.wrapping_add(1);
                if let Some(i) = self.alloc_slot(now) {
                    self.start_conn(i, p, now, true);
                    self.stats.new_conns = self.stats.new_conns.wrapping_add(1);
                    return self.conns[i].state;
                }
                0
            }
            Cls::New => {
                if let Some(i) = self.alloc_slot(now) {
                    self.start_conn(i, p, now, false);
                    self.stats.new_conns = self.stats.new_conns.wrapping_add(1);
                    if p.proto == PROTO_UDP && p.dir == DIR_OUT && p.dport == TFTP_PORT {
                        self.add_expect(p.dst, p.src, p.sport, now);
                    }
                    return self.conns[i].state;
                }
                0
            }
            Cls::Replace { idx } => {
                self.conns[idx] = Conn::EMPTY;
                self.start_conn(idx, p, now, false);
                self.stats.new_conns = self.stats.new_conns.wrapping_add(1);
                self.conns[idx].state
            }
            Cls::Invalid(_) => 0,
        }
    }

    fn want_log(&mut self, wanted: bool) -> bool {
        if !wanted {
            return false;
        }
        let n = self.log_seq;
        self.log_seq = self.log_seq.wrapping_add(1);
        // the first few, then one in 256: a flood must not flood the log
        if n < 32 || n % 256 == 0 {
            self.stats.logged = self.stats.logged.wrapping_add(1);
            true
        } else {
            false
        }
    }

    /// Decides one packet. With `commit` false nothing at all changes (a dry
    /// run for diagnostics and tests); with it true the tracked state, counters
    /// and rule hit counts are updated - and tracked state is updated only if
    /// the packet is ACCEPTED, so a packet a rule drops cannot move a flow.
    pub fn filter(&mut self, p: &Pkt, now: u32, commit: bool) -> Verdict {
        let mut v = Verdict {
            accept: false,
            state: ST_INVALID,
            rule: -1,
            reason: R_INVALID,
            detail: INV_NONE,
            ct_state: 0,
            log: false,
        };
        if self.disabled {
            v.accept = true;
            v.reason = R_DISABLED;
            return v;
        }
        let inbound = p.dir == DIR_IN;
        let mut want_log = false;
        let mut cls = Cls::Invalid(INV_NONE);

        if p.bad != BAD_NONE {
            v.reason = match p.bad {
                BAD_FRAGMENT => R_FRAGMENT,
                BAD_UNSUPPORTED => R_UNSUPPORTED,
                _ => R_MALFORMED,
            };
            want_log = true;
        } else {
            cls = self.classify(p, now);
            if let Cls::Invalid(d) = cls {
                v.reason = R_INVALID;
                v.detail = d;
                want_log = true;
            } else {
                v.state = self.state_of(&cls);
                // first match wins; no match falls to the direction's policy
                let mut decided = false;
                for i in 0..self.nrules as usize {
                    if self.rules[i].matches(p, v.state) {
                        v.accept = self.rules[i].accept;
                        v.rule = i as i32;
                        v.reason = R_RULE;
                        want_log = self.rules[i].log;
                        decided = true;
                        break;
                    }
                }
                if !decided {
                    v.accept = if inbound { self.policy_in } else { self.policy_out };
                    v.reason = R_POLICY;
                }
                // a NEW flow needs room, and an inbound one must not be allowed
                // to use the table up with half-open connections
                if v.accept {
                    let needs_slot = matches!(cls, Cls::New | Cls::RelatedNew { .. });
                    if needs_slot && !self.has_free_slot(now) {
                        v.accept = false;
                        v.reason = R_TABLE_FULL;
                        want_log = true;
                    } else if matches!(cls, Cls::New)
                        && p.proto == PROTO_TCP
                        && inbound
                        && self.half_open_inbound(now) >= HALF_OPEN_MAX
                    {
                        v.accept = false;
                        v.reason = R_HALF_OPEN;
                        want_log = true;
                    }
                }
            }
        }

        if !commit {
            // report the state the flow would be in, without touching it
            if v.accept {
                if let Cls::Existing { next, .. } = cls {
                    v.ct_state = next;
                } else if matches!(cls, Cls::New | Cls::Replace { .. }) {
                    v.ct_state = match p.proto {
                        PROTO_TCP => TCP_SYN_SENT,
                        PROTO_UDP => UDP_UNREPLIED,
                        _ => ICMP_ECHO,
                    };
                }
            }
            return v;
        }

        if v.accept {
            v.ct_state = self.commit(p, cls, now);
        }
        // counters
        let s = &mut self.stats;
        match (v.accept, inbound) {
            (true, true) => s.accepted_in = s.accepted_in.wrapping_add(1),
            (true, false) => s.accepted_out = s.accepted_out.wrapping_add(1),
            (false, true) => s.dropped_in = s.dropped_in.wrapping_add(1),
            (false, false) => s.dropped_out = s.dropped_out.wrapping_add(1),
        }
        match v.reason {
            R_INVALID => s.invalid = s.invalid.wrapping_add(1),
            R_FRAGMENT => s.fragments = s.fragments.wrapping_add(1),
            R_UNSUPPORTED => s.unsupported = s.unsupported.wrapping_add(1),
            R_MALFORMED => s.malformed = s.malformed.wrapping_add(1),
            R_TABLE_FULL => s.table_full = s.table_full.wrapping_add(1),
            R_HALF_OPEN => s.half_open = s.half_open.wrapping_add(1),
            _ => {}
        }
        if v.rule >= 0 {
            let r = v.rule as usize;
            self.hits[r] = self.hits[r].wrapping_add(1);
            self.hit_bytes[r] = self.hit_bytes[r].wrapping_add(p.len as u32);
        }
        v.log = self.want_log(want_log);
        v
    }
}

/// Exercised at boot (and by the host tests): the engine, on a private
/// instance, doing the handful of things that matter most. A build whose core
/// is broken says so in the log instead of filtering nonsense. Returns
/// (checks that held, checks attempted).
pub fn selftest() -> (u32, u32) {
    let mut held = 0u32;
    let mut total = 0u32;
    let mut check = |ok: bool| {
        total += 1;
        if ok {
            held += 1;
        }
    };
    let mut z = Firewall::new();
    let probe_out = Pkt { dir: DIR_OUT, proto: PROTO_UDP, src: 1, dst: 2, sport: 5000, dport: 53, ..Pkt::blank() };
    check(!z.filter(&probe_out, 0, true).accept); // an uninitialised firewall drops everything
    let mut f = Firewall::new();
    f.init();
    let us = 0x0A00_020F;
    let peer = 0x0102_0304;
    let syn = Pkt { dir: DIR_OUT, proto: PROTO_TCP, src: us, dst: peer, sport: 40000, dport: 80, flags: TCP_SYN, ..Pkt::blank() };
    let synack = Pkt { dir: DIR_IN, proto: PROTO_TCP, src: peer, dst: us, sport: 80, dport: 40000, flags: TCP_SYN | TCP_ACK, ..Pkt::blank() };
    let ack = Pkt { dir: DIR_OUT, flags: TCP_ACK, ..syn };
    let stray = Pkt { dir: DIR_IN, proto: PROTO_TCP, src: peer, dst: us, sport: 80, dport: 40001, flags: TCP_ACK, ..Pkt::blank() };
    let unsolicited = Pkt { dir: DIR_IN, proto: PROTO_TCP, src: peer, dst: us, sport: 4444, dport: 22, flags: TCP_SYN, ..Pkt::blank() };
    check(f.rule_count() == 1 && f.policy(DIR_OUT) && !f.policy(DIR_IN));
    check(f.filter(&unsolicited, 0, true).accept == false); // nothing gets in unasked
    let v = f.filter(&syn, 1, true);
    check(v.accept && v.state == ST_NEW && v.ct_state == TCP_SYN_SENT);
    check(!f.filter(&stray, 1, true).accept); // an ACK for a flow we do not have
    let v = f.filter(&synack, 2, true);
    check(v.accept && v.state == ST_ESTABLISHED && v.rule == 0);
    let v = f.filter(&ack, 3, true);
    check(v.accept && v.ct_state == TCP_ESTABLISHED);
    check(f.active_conns(3) == 1);
    let bad = Pkt { flags: TCP_SYN | TCP_FIN, ..syn };
    check(!f.filter(&bad, 4, true).accept); // a flag combination no TCP sends
    check(parse_config(b"policy sideways\n").is_err());
    check(parse_rule(b"rule accept in dport 22").is_err()); // a port needs a protocol
    check(glob_free_cidr_ok());
    f.lock();
    check(f.add_rule(0, Rule::EMPTY).is_err() && f.set_enabled(false).is_err());
    (held, total)
}

fn glob_free_cidr_ok() -> bool {
    parse_cidr(b"10.0.2.0/24") == Some((0x0A00_0200, 0xFFFF_FF00))
        && parse_cidr(b"10.0.2.9/24") == Some((0x0A00_0200, 0xFFFF_FF00))
        && parse_cidr(b"any") == Some((0, 0))
        && parse_cidr(b"1.2.3.4") == Some((0x0102_0304, u32::MAX))
        && parse_cidr(b"1.2.3.4/33").is_none()
}

// ------------------------------------------------------------------------
// The kernel layer.
// ------------------------------------------------------------------------

#[cfg(not(test))]
mod kernel_glue {
    use super::*;
    use crate::spinlock::SpinLock;
    use core::slice;

    /// One instance, one lock. All-zero is its valid (fail-closed) state, so it
    /// lives in .bss. A check is a scan of at most MAX_RULES rules and
    /// MAX_CONNS flows.
    static STATE: SpinLock<Firewall> = SpinLock::new(Firewall::new());

    extern "C" {
        /// kernel/net/firewall.c: formats one log line. Called with no lock held.
        fn fw_hal_log(dir: u32, proto: u32, src: u32, dst: u32, sport: u32, dport: u32, accept: u32, reason: u32, rule: i32);
    }

    unsafe fn bytes_of<'a>(p: *const u8, n: u32) -> Option<&'a [u8]> {
        if n == 0 {
            return Some(&[]);
        }
        if p.is_null() || n as usize > 65535 {
            return None;
        }
        Some(slice::from_raw_parts(p, n as usize))
    }

    #[no_mangle]
    pub extern "C" fn rust_fw_init() {
        STATE.lock().init();
    }

    /// Boot self-test. Writes the number of checks attempted, returns the
    /// number that held.
    #[no_mangle]
    pub extern "C" fn rust_fw_selftest(out_total: *mut u32) -> u32 {
        let (held, total) = selftest();
        unsafe {
            *out_total = total;
        }
        held
    }

    /// Replaces rules and policies from a configuration file's text. 0, or a
    /// negative errno. On a parse error `out_kind`/`out_line` say where.
    #[no_mangle]
    pub extern "C" fn rust_fw_load_config(text: *const u8, len: u32, out_kind: *mut u32, out_line: *mut u32) -> i32 {
        let t = match unsafe { bytes_of(text, len) } {
            Some(t) => t,
            None => return -EINVAL,
        };
        match STATE.lock().apply_config(t) {
            Ok(()) => 0,
            Err(e) => {
                let (kind, line) = match e {
                    AdminError::Parse(ParseError::TooLarge) => (1u32, 0u32),
                    AdminError::Parse(ParseError::BadByte) => (2, 0),
                    AdminError::Parse(ParseError::LineTooLong(l)) => (3, l as u32),
                    AdminError::Parse(ParseError::Syntax(l)) => (4, l as u32),
                    AdminError::Parse(ParseError::BadAddress(l)) => (5, l as u32),
                    AdminError::Parse(ParseError::BadPort(l)) => (6, l as u32),
                    AdminError::Parse(ParseError::BadState(l)) => (7, l as u32),
                    AdminError::Parse(ParseError::BadCombination(l)) => (8, l as u32),
                    AdminError::Parse(ParseError::TooManyRules(l)) => (9, l as u32),
                    _ => (0, 0),
                };
                unsafe {
                    *out_kind = kind;
                    *out_line = line;
                }
                -e.errno()
            }
        }
    }

    /// The filter. `dir`: 0 inbound, 1 outbound. `payload` is the TRANSPORT
    /// bytes (the IP header is not passed), `frag` is nonzero if the IP header
    /// marked the packet as any part of a fragmented one. Returns 1 to accept,
    /// 0 to drop.
    #[no_mangle]
    pub extern "C" fn rust_fw_filter(dir: u32, src: u32, dst: u32, proto: u32, frag: u32, payload: *const u8, len: u32, now: u32) -> i32 {
        let data = match unsafe { bytes_of(payload, len) } {
            Some(d) => d,
            None => return 0,
        };
        let d = if dir == 0 { DIR_IN } else { DIR_OUT };
        let p = parse_packet(d, src, dst, proto as u8, frag != 0, data);
        let v = STATE.lock().filter(&p, now, true);
        if v.log {
            unsafe {
                fw_hal_log(d as u32, p.proto as u32, src, dst, p.sport as u32, p.dport as u32, v.accept as u32, v.reason as u32, v.rule);
            }
        }
        v.accept as i32
    }

    /// Probe: `inp` is 18 u32s, `out` 8. See nova_fw_abi.h for the layout.
    /// With commit != 0 it changes the real tracked state (administrator only -
    /// the caller has checked).
    #[no_mangle]
    pub extern "C" fn rust_fw_probe(inp: *const u32, out: *mut u32, now: u32) -> i32 {
        if inp.is_null() || out.is_null() {
            return -EINVAL;
        }
        let i = unsafe { slice::from_raw_parts(inp, 18) };
        let mut p = Pkt::blank();
        p.dir = if i[0] == 0 { DIR_IN } else { DIR_OUT };
        p.proto = i[1] as u8;
        p.src = i[2];
        p.dst = i[3];
        p.sport = i[4] as u16;
        p.dport = i[5] as u16;
        p.flags = i[6] as u8 & 0x3F;
        p.icmp_type = i[7] as u8;
        p.icmp_code = i[8] as u8;
        p.id = i[9] as u16;
        p.bad = match i[10] {
            0 => BAD_NONE,
            _ => BAD_FRAGMENT,
        };
        p.len = i[17] as u16;
        if i[12] != 0 {
            p.inner = Some(Inner { proto: i[12] as u8, src: i[13], dst: i[14], sport: i[15] as u16, dport: i[16] as u16 });
        }
        if p.proto != PROTO_TCP && p.proto != PROTO_UDP && p.proto != PROTO_ICMP && p.bad == BAD_NONE {
            p.bad = BAD_UNSUPPORTED;
        }
        let commit = i[11] != 0;
        let v = STATE.lock().filter(&p, now, commit);
        unsafe {
            let o = slice::from_raw_parts_mut(out, 8);
            o[0] = v.accept as u32;
            o[1] = v.state as u32;
            o[2] = v.rule as u32;
            o[3] = v.reason as u32;
            o[4] = v.detail as u32;
            o[5] = v.ct_state as u32;
            o[6] = 0;
            o[7] = 0;
        }
        0
    }

    #[no_mangle]
    pub extern "C" fn rust_fw_add_rule(text: *const u8, len: u32, pos: u32) -> i32 {
        let t = match unsafe { bytes_of(text, len) } {
            Some(t) => t,
            None => return -EINVAL,
        };
        let mut g = STATE.lock();
        // u32::MAX means "after the last rule"
        let at = if pos == u32::MAX { g.rule_count() } else { pos as usize };
        match g.add_rule_text(at, t) {
            Ok(()) => 0,
            Err(e) => -e.errno(),
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_fw_del_rule(idx: u32) -> i32 {
        match STATE.lock().del_rule(idx as usize) {
            Ok(()) => 0,
            Err(e) => -e.errno(),
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_fw_flush_rules() -> i32 {
        match STATE.lock().flush_rules() {
            Ok(()) => 0,
            Err(e) => -e.errno(),
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_fw_set_policy(dir: u32, accept: u32) -> i32 {
        let d = if dir == 0 { DIR_IN } else if dir == 1 { DIR_OUT } else { return -EINVAL };
        match STATE.lock().set_policy(d, accept != 0) {
            Ok(()) => 0,
            Err(e) => -e.errno(),
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_fw_set_enabled(on: u32) -> i32 {
        match STATE.lock().set_enabled(on != 0) {
            Ok(()) => 0,
            Err(e) => -e.errno(),
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_fw_lock() {
        STATE.lock().lock();
    }

    #[no_mangle]
    pub extern "C" fn rust_fw_flush_conns() {
        STATE.lock().flush_conns();
    }

    /// 24 u32s - see nova_fw_abi.h.
    #[no_mangle]
    pub extern "C" fn rust_fw_info(out: *mut u32, now: u32) {
        let g = STATE.lock();
        let s = g.stats();
        let o = unsafe { slice::from_raw_parts_mut(out, 24) };
        o[0] = g.is_enabled() as u32;
        o[1] = g.is_locked() as u32;
        o[2] = g.policy(DIR_IN) as u32;
        o[3] = g.policy(DIR_OUT) as u32;
        o[4] = g.rule_count() as u32;
        o[5] = g.active_conns(now) as u32;
        o[6] = g.active_expects(now) as u32;
        o[7] = s.accepted_in;
        o[8] = s.accepted_out;
        o[9] = s.dropped_in;
        o[10] = s.dropped_out;
        o[11] = s.invalid;
        o[12] = s.fragments;
        o[13] = s.table_full;
        o[14] = s.half_open;
        o[15] = s.new_conns;
        o[16] = s.related;
        o[17] = s.unsupported;
        o[18] = s.malformed;
        o[19] = s.logged;
        o[20] = MAX_RULES as u32;
        o[21] = MAX_CONNS as u32;
        o[22] = TICK_HZ;
        o[23] = 0;
    }

    /// 18 u32s. Returns -ENOENT if there is no such rule.
    #[no_mangle]
    pub extern "C" fn rust_fw_get_rule(idx: u32, out: *mut u32) -> i32 {
        let g = STATE.lock();
        match g.rule_at(idx as usize) {
            None => -ENOENT,
            Some((r, hits, bytes)) => {
                let o = unsafe { slice::from_raw_parts_mut(out, 18) };
                o[0] = r.dir as u32;
                o[1] = r.proto as u32;
                o[2] = r.accept as u32;
                o[3] = r.log as u32;
                o[4] = r.src_ip;
                o[5] = r.src_mask;
                o[6] = r.dst_ip;
                o[7] = r.dst_mask;
                o[8] = r.sp_set as u32;
                o[9] = r.sp_lo as u32;
                o[10] = r.sp_hi as u32;
                o[11] = r.dp_set as u32;
                o[12] = r.dp_lo as u32;
                o[13] = r.dp_hi as u32;
                o[14] = r.states as u32 | ((r.icmp_set as u32) << 8) | ((r.icmp_type as u32) << 16);
                o[15] = hits;
                o[16] = bytes;
                o[17] = 0;
                0
            }
        }
    }

    /// 16 u32s. The n-th live tracked flow, or -ENOENT.
    #[no_mangle]
    pub extern "C" fn rust_fw_get_conn(n: u32, out: *mut u32, now: u32) -> i32 {
        let g = STATE.lock();
        match g.conn_nth(n as usize, now) {
            None => -ENOENT,
            Some(c) => {
                let o = unsafe { slice::from_raw_parts_mut(out, 16) };
                o[0] = c.proto as u32;
                o[1] = c.state as u32;
                o[2] = c.orig_dir as u32;
                o[3] = c.replied as u32;
                o[4] = c.a_ip;
                o[5] = c.b_ip;
                o[6] = c.a_port as u32;
                o[7] = c.b_port as u32;
                o[8] = c.remaining;
                o[9] = c.pkts[0];
                o[10] = c.pkts[1];
                o[11] = c.bytes[0];
                o[12] = c.bytes[1];
                o[13] = c.age;
                o[14] = c.pending as u32;
                o[15] = 0;
                0
            }
        }
    }
}

// ------------------------------------------------------------------------
// Host tests.
// ------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const US: u32 = 0x0A00_020F; // 10.0.2.15
    const GW: u32 = 0x0A00_0202; // 10.0.2.2
    const PEER: u32 = 0x0102_0304;
    const PEER2: u32 = 0x0506_0708;

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

    fn fw() -> Firewall {
        let mut f = Firewall::new();
        f.init();
        f
    }

    // ---- raw packet builders ----

    fn tcp_bytes(sport: u16, dport: u16, flags: u8) -> Vec<u8> {
        let mut b = vec![0u8; 20];
        b[0] = (sport >> 8) as u8;
        b[1] = sport as u8;
        b[2] = (dport >> 8) as u8;
        b[3] = dport as u8;
        b[12] = 5 << 4;
        b[13] = flags;
        b
    }

    fn udp_bytes(sport: u16, dport: u16, payload: usize) -> Vec<u8> {
        let len = 8 + payload;
        let mut b = vec![0u8; len];
        b[0] = (sport >> 8) as u8;
        b[1] = sport as u8;
        b[2] = (dport >> 8) as u8;
        b[3] = dport as u8;
        b[4] = (len >> 8) as u8;
        b[5] = len as u8;
        b
    }

    fn icmp_echo_bytes(t: u8, id: u16) -> Vec<u8> {
        let mut b = vec![0u8; 8];
        b[0] = t;
        b[4] = (id >> 8) as u8;
        b[5] = id as u8;
        b
    }

    fn icmp_error_bytes(t: u8, iproto: u8, isrc: u32, idst: u32, isport: u16, idport: u16) -> Vec<u8> {
        let mut b = vec![0u8; 8];
        b[0] = t;
        let mut ip = vec![0u8; 20];
        ip[0] = 0x45;
        ip[9] = iproto;
        ip[12..16].copy_from_slice(&isrc.to_be_bytes());
        ip[16..20].copy_from_slice(&idst.to_be_bytes());
        b.extend_from_slice(&ip);
        let mut tr = vec![0u8; 8];
        if iproto == PROTO_ICMP {
            tr[0] = 8;
            tr[4] = (isport >> 8) as u8;
            tr[5] = isport as u8;
        } else {
            tr[0] = (isport >> 8) as u8;
            tr[1] = isport as u8;
            tr[2] = (idport >> 8) as u8;
            tr[3] = idport as u8;
        }
        b.extend_from_slice(&tr);
        b
    }

    fn send(f: &mut Firewall, dir: u8, src: u32, dst: u32, proto: u8, d: &[u8], now: u32) -> Verdict {
        let p = parse_packet(dir, src, dst, proto, false, d);
        f.filter(&p, now, true)
    }

    fn tcp_out(f: &mut Firewall, sport: u16, dport: u16, flags: u8, now: u32) -> Verdict {
        send(f, DIR_OUT, US, PEER, PROTO_TCP, &tcp_bytes(sport, dport, flags), now)
    }

    fn tcp_in(f: &mut Firewall, sport: u16, dport: u16, flags: u8, now: u32) -> Verdict {
        send(f, DIR_IN, PEER, US, PROTO_TCP, &tcp_bytes(sport, dport, flags), now)
    }

    // ---- rule and config parsing ----

    #[test]
    fn rule_parsing_table() {
        let r = parse_rule(b"rule accept in proto tcp from 10.0.2.0/24 to 10.0.2.15 sport 1024-65535 dport 22 state new,established icmp-type").err();
        assert!(r.is_some(), "icmp-type with tcp must be refused");
        let r = parse_rule(b"rule accept in proto tcp from 10.0.2.0/24 to 10.0.2.15 sport 1024-65535 dport 22 state new,established log").unwrap();
        assert!(r.accept && r.log && r.dir == DIR_IN && r.proto == PROTO_TCP);
        assert_eq!((r.src_ip, r.src_mask), (0x0A00_0200, 0xFFFF_FF00));
        assert_eq!((r.dst_ip, r.dst_mask), (US, u32::MAX));
        assert!(r.sp_set && r.sp_lo == 1024 && r.sp_hi == 65535 && r.dp_set && r.dp_lo == 22 && r.dp_hi == 22);
        assert_eq!(r.states, ST_NEW | ST_ESTABLISHED);
        let r = parse_rule(b"rule drop any").unwrap();
        assert!(!r.accept && r.dir == DIR_ANY && r.proto == 0 && r.src_mask == 0 && !r.sp_set && r.states == 0);
        let r = parse_rule(b"rule accept out proto icmp icmp-type 8").unwrap();
        assert!(r.icmp_set && r.icmp_type == 8);
        let r = parse_rule(b"rule accept in proto udp dport any sport 53").unwrap();
        assert!(!r.dp_set && r.sp_set);
        // order of the keywords is free
        assert_eq!(
            parse_rule(b"rule accept in dport 22 proto tcp").unwrap(),
            parse_rule(b"rule accept in proto tcp dport 22").unwrap()
        );
    }

    #[test]
    fn every_rule_error_is_reported() {
        let cases: &[(&[u8], ParseError)] = &[
            (b"rule", ParseError::Syntax(1)),
            (b"rule accept", ParseError::Syntax(1)),
            (b"rule permit in", ParseError::Syntax(1)),
            (b"rule accept sideways", ParseError::Syntax(1)),
            (b"rule accept in proto sctp", ParseError::Syntax(1)),
            (b"rule accept in proto", ParseError::Syntax(1)),
            (b"rule accept in proto tcp proto udp", ParseError::Syntax(1)),
            (b"rule accept in frobnicate 1", ParseError::Syntax(1)),
            (b"rule accept in log log", ParseError::Syntax(1)),
            (b"rule accept in from 1.2.3", ParseError::BadAddress(1)),
            (b"rule accept in from 1.2.3.4/33", ParseError::BadAddress(1)),
            (b"rule accept in from 1.2.3.256", ParseError::BadAddress(1)),
            (b"rule accept in to 1.2.3.4/", ParseError::BadAddress(1)),
            (b"rule accept in proto tcp dport 70000", ParseError::BadPort(1)),
            (b"rule accept in proto tcp dport 90-80", ParseError::BadPort(1)),
            (b"rule accept in proto tcp sport x", ParseError::BadPort(1)),
            (b"rule accept in state bogus", ParseError::BadState(1)),
            (b"rule accept in state invalid", ParseError::BadState(1)),
            (b"rule accept in state new,", ParseError::BadState(1)),
            (b"rule accept in dport 22", ParseError::BadCombination(1)),
            (b"rule accept in proto icmp dport 22", ParseError::BadCombination(1)),
            (b"rule accept in proto tcp icmp-type 8", ParseError::BadCombination(1)),
            (b"rule accept in icmp-type 8", ParseError::BadCombination(1)),
        ];
        for (text, want) in cases {
            assert_eq!(parse_rule(text).err(), Some(*want), "for {:?}", String::from_utf8_lossy(text));
        }
        assert_eq!(parse_rule(b"policy in drop").err(), Some(ParseError::Syntax(1)));
    }

    #[test]
    fn config_parsing() {
        let c = parse_config(BASELINE).unwrap();
        assert!(!c.policy_in && c.policy_out && c.n == 1);
        let c = parse_config(b"# nothing\n\n").unwrap();
        assert!(!c.policy_in && c.policy_out && c.n == 0, "no policy lines: the baseline stance");
        let c = parse_config(b"policy out drop # strict\npolicy in accept\r\nrule drop any\r\n").unwrap();
        assert!(c.policy_in && !c.policy_out && c.n == 1);
        let bad: &[(&[u8], ParseError)] = &[
            (b"policy in\n", ParseError::Syntax(1)),
            (b"policy sideways drop\n", ParseError::Syntax(1)),
            (b"policy in maybe\n", ParseError::Syntax(1)),
            (b"policy in drop extra\n", ParseError::Syntax(1)),
            (b"policy in drop\npolicy in accept\n", ParseError::Syntax(2)),
            (b"allow everything\n", ParseError::Syntax(1)),
            (b"\n\nrule accept in dport 22\n", ParseError::BadCombination(3)),
            (b"rule accept in\x01\n", ParseError::BadByte),
        ];
        for (t, want) in bad {
            assert_eq!(parse_config(t).err(), Some(*want), "for {:?}", String::from_utf8_lossy(t));
        }
    }

    #[test]
    fn config_limits_are_exact() {
        let mut t = String::new();
        for _ in 0..MAX_RULES {
            t.push_str("rule drop any\n");
        }
        assert_eq!(parse_config(t.as_bytes()).unwrap().n, MAX_RULES);
        t.push_str("rule drop any\n");
        assert_eq!(parse_config(t.as_bytes()).err(), Some(ParseError::TooManyRules(MAX_RULES as u16 + 1)));
        let mut ok = vec![b'#'; MAX_LINE];
        ok.push(b'\n');
        assert!(parse_config(&ok).is_ok());
        let mut long = vec![b'#'; MAX_LINE + 1];
        long.push(b'\n');
        assert_eq!(parse_config(&long).err(), Some(ParseError::LineTooLong(1)));
        let mut big = Vec::new();
        while big.len() < MAX_CONFIG_TEXT {
            big.extend_from_slice(b"#\n");
        }
        big.truncate(MAX_CONFIG_TEXT);
        assert!(parse_config(&big).is_ok());
        big.push(b'#');
        assert_eq!(parse_config(&big).err(), Some(ParseError::TooLarge));
    }

    #[test]
    fn cidr_edge_cases() {
        assert_eq!(parse_cidr(b"1.2.3.4/0"), Some((0, 0)), "/0 is every address, not one host");
        assert_eq!(parse_cidr(b"1.2.3.4/32"), Some((0x0102_0304, u32::MAX)));
        assert_eq!(parse_cidr(b"1.2.3.4/1"), Some((0, 0x8000_0000)));
        assert_eq!(parse_cidr(b"200.2.3.4/1"), Some((0x8000_0000, 0x8000_0000)));
        assert_eq!(parse_cidr(b"10.9.9.9/8"), Some((0x0A00_0000, 0xFF00_0000)));
        for bad in [&b""[..], b"/8", b"1.2.3.4/", b"1.2.3.4/-1", b"1.2.3.4/33", b"1.2.3.4//8", b"a.b.c.d", b"1.2.3.4.5"] {
            assert!(parse_cidr(bad).is_none(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn cidr_and_port_semantics() {
        let mut f = fw();
        f.flush_rules().unwrap();
        f.add_rule_text(0, b"rule accept in proto udp from 10.0.2.0/24 sport 1000-2000 dport 53").unwrap();
        let mk = |src: u32, sport: u16, dport: u16| Pkt { dir: DIR_IN, proto: PROTO_UDP, src, dst: US, sport, dport, ..Pkt::blank() };
        let hit = |f: &mut Firewall, p: Pkt| f.filter(&p, 0, false).accept;
        assert!(hit(&mut f, mk(0x0A00_0201, 1500, 53)));
        assert!(hit(&mut f, mk(0x0A00_02FF, 1000, 53)) && hit(&mut f, mk(0x0A00_0200, 2000, 53)), "both ends of the range");
        assert!(!hit(&mut f, mk(0x0A00_0301, 1500, 53)), "outside the /24");
        assert!(!hit(&mut f, mk(0x0A00_0201, 999, 53)) && !hit(&mut f, mk(0x0A00_0201, 2001, 53)), "just outside the range");
        assert!(!hit(&mut f, mk(0x0A00_0201, 1500, 54)));
    }

    // ---- packet parsing ----

    #[test]
    fn packet_parsing_and_malformed_headers() {
        let p = parse_packet(DIR_IN, PEER, US, PROTO_TCP, false, &tcp_bytes(80, 40000, TCP_SYN | TCP_ACK));
        assert!(p.bad == BAD_NONE && p.sport == 80 && p.dport == 40000 && p.flags == TCP_SYN | TCP_ACK);
        // the ECN bits (CWR, ECE) are not part of the flags the filter sees
        let p = parse_packet(DIR_IN, PEER, US, PROTO_TCP, false, &tcp_bytes(1, 2, TCP_SYN | 0x40 | 0x80));
        assert_eq!(p.flags, TCP_SYN);
        let p = parse_packet(DIR_IN, PEER, US, PROTO_UDP, false, &udp_bytes(53, 5000, 10));
        assert!(p.bad == BAD_NONE && p.sport == 53 && p.dport == 5000);
        // truncated
        for n in 0..20 {
            let b = tcp_bytes(1, 2, TCP_SYN);
            assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_TCP, false, &b[..n]).bad, BAD_MALFORMED, "tcp of {} bytes", n);
        }
        for n in 0..8 {
            let b = udp_bytes(1, 2, 0);
            assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_UDP, false, &b[..n]).bad, BAD_MALFORMED);
            let i = icmp_echo_bytes(8, 1);
            assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_ICMP, false, &i[..n]).bad, BAD_MALFORMED);
        }
        // a data offset that is too small, or points past the packet
        let mut b = tcp_bytes(1, 2, TCP_SYN);
        b[12] = 4 << 4;
        assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_TCP, false, &b).bad, BAD_MALFORMED);
        b[12] = 6 << 4;
        assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_TCP, false, &b).bad, BAD_MALFORMED);
        // a UDP length that disagrees with the datagram
        let mut u = udp_bytes(1, 2, 4);
        u[5] = 7;
        assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_UDP, false, &u).bad, BAD_MALFORMED);
        u[5] = 13;
        assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_UDP, false, &u).bad, BAD_MALFORMED);
        // fragments are never looked into, other protocols are unsupported
        assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_TCP, true, &tcp_bytes(1, 2, TCP_SYN)).bad, BAD_FRAGMENT);
        assert_eq!(parse_packet(DIR_IN, PEER, US, 47, false, &[0; 40]).bad, BAD_UNSUPPORTED);
        // an ICMP error that quotes too little
        let e = icmp_error_bytes(3, PROTO_UDP, US, PEER, 5000, 53);
        for n in 8..e.len() - 4 {
            assert_eq!(parse_packet(DIR_IN, PEER, US, PROTO_ICMP, false, &e[..n]).bad, BAD_MALFORMED, "error of {} bytes", n);
        }
        let ok = parse_packet(DIR_IN, GW, US, PROTO_ICMP, false, &e);
        assert_eq!(ok.inner, Some(Inner { proto: PROTO_UDP, src: US, dst: PEER, sport: 5000, dport: 53 }));
    }

    // ---- TCP ----

    #[test]
    fn outbound_tcp_lifecycle_every_state() {
        let mut f = fw();
        let v = tcp_out(&mut f, 40000, 80, TCP_SYN, 100);
        assert!(v.accept && v.state == ST_NEW && v.ct_state == TCP_SYN_SENT && v.reason == R_POLICY);
        let v = tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 101);
        assert!(v.accept && v.state == ST_ESTABLISHED && v.rule == 0 && v.ct_state == TCP_SYN_RECV);
        let v = tcp_out(&mut f, 40000, 80, TCP_ACK, 102);
        assert!(v.accept && v.state == ST_ESTABLISHED && v.ct_state == TCP_ESTABLISHED);
        for t in 103..110 {
            assert!(tcp_in(&mut f, 80, 40000, TCP_ACK | TCP_PSH, t).accept);
            assert!(tcp_out(&mut f, 40000, 80, TCP_ACK, t).accept);
        }
        let v = tcp_out(&mut f, 40000, 80, TCP_FIN | TCP_ACK, 120);
        assert!(v.accept && v.ct_state == TCP_FIN_WAIT);
        assert!(tcp_in(&mut f, 80, 40000, TCP_ACK, 121).accept);
        let v = tcp_in(&mut f, 80, 40000, TCP_FIN | TCP_ACK, 122);
        assert!(v.accept && v.ct_state == TCP_LAST_ACK);
        let v = tcp_out(&mut f, 40000, 80, TCP_ACK, 123);
        assert!(v.accept && v.ct_state == TCP_TIME_WAIT);
        let s = f.stats();
        assert_eq!((s.new_conns, s.dropped_in, s.dropped_out, s.invalid), (1, 0, 0, 0));
        assert_eq!(f.active_conns(123), 1);
    }

    #[test]
    fn inbound_tcp_needs_a_rule_and_then_works() {
        let mut f = fw();
        let v = tcp_in(&mut f, 50000, 22, TCP_SYN, 10);
        assert!(!v.accept && v.state == ST_NEW && v.reason == R_POLICY && v.rule == -1, "nothing gets in unasked");
        assert_eq!(f.active_conns(10), 0, "a dropped SYN must not create a flow");
        f.add_rule_text(0, b"rule accept in proto tcp dport 22 state new").unwrap();
        assert!(!tcp_in(&mut f, 50000, 80, TCP_SYN, 11).accept, "other ports stay closed");
        let v = tcp_in(&mut f, 50000, 22, TCP_SYN, 12);
        assert!(v.accept && v.rule == 0 && v.ct_state == TCP_SYN_SENT);
        // the responder's SYN-ACK (outbound) is a reply: policy out accept
        let v = tcp_out(&mut f, 22, 50000, TCP_SYN | TCP_ACK, 13);
        assert!(v.accept && v.state == ST_ESTABLISHED && v.ct_state == TCP_SYN_RECV);
        // the initiator's ACK is established traffic: the baseline rule
        let v = tcp_in(&mut f, 50000, 22, TCP_ACK, 14);
        assert!(v.accept && v.state == ST_ESTABLISHED && v.rule == 1 && v.ct_state == TCP_ESTABLISHED);
        assert!(tcp_in(&mut f, 50000, 22, TCP_ACK | TCP_PSH, 15).accept);
        // a retransmitted SYN while still half-open is still NEW and so needs the rule
        let mut g = fw();
        g.add_rule_text(0, b"rule accept in proto tcp dport 22 state new").unwrap();
        assert!(tcp_in(&mut g, 50000, 22, TCP_SYN, 1).accept);
        let v = tcp_in(&mut g, 50000, 22, TCP_SYN, 2);
        assert!(v.accept && v.state == ST_NEW && v.ct_state == TCP_SYN_SENT, "a SYN retransmit");
        assert_eq!(g.active_conns(2), 1);
    }

    #[test]
    fn invalid_tcp_is_dropped_and_changes_nothing() {
        let mut f = fw();
        // no connection, not a SYN: invalid
        for flags in [TCP_ACK, TCP_ACK | TCP_PSH, TCP_FIN | TCP_ACK, TCP_RST, TCP_RST | TCP_ACK, TCP_SYN | TCP_ACK] {
            let v = tcp_in(&mut f, 80, 40000, flags, 1);
            assert!(!v.accept && v.reason == R_INVALID && v.detail == INV_NO_CONN, "flags {:#x}", flags);
        }
        // impossible flag combinations, in both directions, even with an open policy
        f.set_policy(DIR_IN, true).unwrap();
        for flags in [0u8, TCP_SYN | TCP_FIN, TCP_SYN | TCP_RST, TCP_FIN, TCP_FIN | TCP_PSH | TCP_URG, TCP_FIN | TCP_RST, TCP_SYN | TCP_FIN | TCP_ACK | TCP_RST] {
            for dir in [DIR_IN, DIR_OUT] {
                let (s, d) = if dir == DIR_IN { (PEER, US) } else { (US, PEER) };
                let v = send(&mut f, dir, s, d, PROTO_TCP, &tcp_bytes(1000, 2000, flags), 2);
                assert!(!v.accept && v.reason == R_INVALID && v.detail == INV_FLAGS, "flags {:#x} dir {}", flags, dir);
            }
        }
        // port 0
        assert_eq!(tcp_out(&mut f, 0, 80, TCP_SYN, 3).detail, INV_PORT0);
        assert_eq!(tcp_out(&mut f, 40000, 0, TCP_SYN, 3).detail, INV_PORT0);
        assert_eq!(f.active_conns(3), 0);
        assert_eq!(f.stats().accepted_in + f.stats().accepted_out, 0);
        // ECN bits on a SYN are fine
        assert!(tcp_out(&mut f, 40001, 80, TCP_SYN | 0x40 | 0x80, 4).accept);
    }

    #[test]
    fn a_reply_must_come_from_the_exact_tuple() {
        let mut f = fw();
        assert!(tcp_out(&mut f, 40000, 80, TCP_SYN, 1).accept);
        // wrong port, wrong host, wrong destination port, wrong source host: none are replies
        assert!(!tcp_in(&mut f, 81, 40000, TCP_SYN | TCP_ACK, 2).accept);
        assert!(!tcp_in(&mut f, 80, 40001, TCP_SYN | TCP_ACK, 2).accept);
        assert!(!send(&mut f, DIR_IN, PEER2, US, PROTO_TCP, &tcp_bytes(80, 40000, TCP_SYN | TCP_ACK), 2).accept);
        // the right peer and ports but addressed to some OTHER local address
        assert!(!send(&mut f, DIR_IN, PEER, 0x0A00_0210, PROTO_TCP, &tcp_bytes(80, 40000, TCP_SYN | TCP_ACK), 2).accept);
        // none of those moved the flow, and none were counted as its packets
        let c = f.conn_nth(0, 3).unwrap();
        assert_eq!((c.state, c.pkts), (TCP_SYN_SENT, [1, 0]));
        // a bare ACK from the right tuple cannot complete a handshake that has not been answered
        let v = tcp_in(&mut f, 80, 40000, TCP_ACK, 3);
        assert!(!v.accept && v.detail == INV_STATE);
        let v = tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 4);
        assert!(v.accept && v.ct_state == TCP_SYN_RECV, "the right tuple is accepted");
    }

    #[test]
    fn rst_closes_and_a_new_syn_reuses_the_tuple() {
        let mut f = fw();
        assert!(tcp_out(&mut f, 40000, 80, TCP_SYN, 1).accept);
        let v = tcp_in(&mut f, 80, 40000, TCP_RST | TCP_ACK, 2);
        assert!(v.accept && v.ct_state == TCP_CLOSE, "a refused connection");
        // a retransmitted RST and stray ACKs in CLOSE are tolerated
        assert!(tcp_in(&mut f, 80, 40000, TCP_RST, 3).accept);
        // data is not
        assert!(!tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 3).accept);
        // the SAME tuple can be used again: a SYN replaces the finished flow
        let v = tcp_out(&mut f, 40000, 80, TCP_SYN, 4);
        assert!(v.accept && v.state == ST_NEW && v.ct_state == TCP_SYN_SENT);
        assert_eq!(f.active_conns(4), 1, "replaced, not duplicated");
        // an established flow reset from either side
        assert!(tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 5).accept);
        assert!(tcp_out(&mut f, 40000, 80, TCP_ACK, 6).accept);
        assert!(tcp_out(&mut f, 40000, 80, TCP_RST, 7).accept);
        assert_eq!(f.conn_nth(0, 7).unwrap().state, TCP_CLOSE);
        // TIME_WAIT also yields to a fresh SYN
        let mut g = fw();
        assert!(tcp_out(&mut g, 41000, 80, TCP_SYN, 1).accept);
        assert!(tcp_in(&mut g, 80, 41000, TCP_SYN | TCP_ACK, 2).accept);
        assert!(tcp_out(&mut g, 41000, 80, TCP_ACK, 3).accept);
        assert!(tcp_out(&mut g, 41000, 80, TCP_FIN | TCP_ACK, 4).accept);
        assert!(tcp_in(&mut g, 80, 41000, TCP_FIN | TCP_ACK, 5).accept);
        assert!(tcp_out(&mut g, 41000, 80, TCP_ACK, 6).accept);
        assert_eq!(g.conn_nth(0, 6).unwrap().state, TCP_TIME_WAIT);
        assert!(tcp_out(&mut g, 41000, 80, TCP_SYN, 7).accept);
        assert_eq!(g.conn_nth(0, 7).unwrap().state, TCP_SYN_SENT);
    }

    #[test]
    fn syn_is_invalid_inside_an_established_flow() {
        let mut f = fw();
        tcp_out(&mut f, 40000, 80, TCP_SYN, 1);
        tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 2);
        tcp_out(&mut f, 40000, 80, TCP_ACK, 3);
        for (inb, flags) in [(false, TCP_SYN), (true, TCP_SYN)] {
            let v = if inb { tcp_in(&mut f, 80, 40000, flags, 4) } else { tcp_out(&mut f, 40000, 80, flags, 4) };
            assert!(!v.accept && v.reason == R_INVALID && v.detail == INV_STATE);
        }
        // a late retransmitted SYN-ACK from the server is tolerated
        assert!(tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 5).accept);
        assert_eq!(f.conn_nth(0, 5).unwrap().state, TCP_ESTABLISHED);
    }

    #[test]
    fn simultaneous_open_completes() {
        let mut f = fw();
        f.set_policy(DIR_IN, true).unwrap();
        assert!(tcp_out(&mut f, 5000, 6000, TCP_SYN, 1).accept);
        let v = tcp_in(&mut f, 6000, 5000, TCP_SYN, 2);
        assert!(v.accept && v.ct_state == TCP_SYN_RECV);
        assert!(tcp_out(&mut f, 5000, 6000, TCP_SYN | TCP_ACK, 3).accept);
        assert!(tcp_in(&mut f, 6000, 5000, TCP_SYN | TCP_ACK, 4).accept);
        let v = tcp_out(&mut f, 5000, 6000, TCP_ACK, 5);
        assert!(v.accept && v.ct_state == TCP_ESTABLISHED);
    }

    #[test]
    fn half_close_keeps_the_flow_usable() {
        let mut f = fw();
        tcp_out(&mut f, 40000, 80, TCP_SYN, 1);
        tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 2);
        tcp_out(&mut f, 40000, 80, TCP_ACK, 3);
        assert!(tcp_in(&mut f, 80, 40000, TCP_FIN | TCP_ACK, 4).accept); // the server closes first
        assert_eq!(f.conn_nth(0, 4).unwrap().state, TCP_FIN_WAIT);
        for t in 5..20 {
            assert!(tcp_out(&mut f, 40000, 80, TCP_ACK | TCP_PSH, t).accept, "we may still send");
        }
        assert!(tcp_in(&mut f, 80, 40000, TCP_FIN | TCP_ACK, 21).accept, "a retransmitted FIN");
        assert!(tcp_out(&mut f, 40000, 80, TCP_FIN | TCP_ACK, 22).accept);
        assert_eq!(f.conn_nth(0, 22).unwrap().state, TCP_LAST_ACK);
        // the ACK from the side that sent the SECOND fin does not finish it ...
        assert!(tcp_out(&mut f, 40000, 80, TCP_ACK, 22).accept);
        assert_eq!(f.conn_nth(0, 22).unwrap().state, TCP_LAST_ACK);
        // ... the side that sent the FIRST fin sends the final ack
        assert!(tcp_in(&mut f, 80, 40000, TCP_ACK, 23).accept);
        assert_eq!(f.conn_nth(0, 23).unwrap().state, TCP_TIME_WAIT);
    }

    // ---- UDP ----

    #[test]
    fn udp_reply_matching() {
        let mut f = fw();
        let q = udp_bytes(5353, 53, 20);
        let v = send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &q, 10);
        assert!(v.accept && v.state == ST_NEW && v.ct_state == UDP_UNREPLIED);
        // the reply
        let r = udp_bytes(53, 5353, 100);
        let v = send(&mut f, DIR_IN, GW, US, PROTO_UDP, &r, 11);
        assert!(v.accept && v.state == ST_ESTABLISHED && v.ct_state == UDP_REPLIED);
        // more of the same conversation, either way
        assert!(send(&mut f, DIR_IN, GW, US, PROTO_UDP, &r, 12).accept);
        assert!(send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &q, 13).state == ST_ESTABLISHED);
        // an unsolicited datagram: not from the right host, not to the right port, from the wrong port
        assert!(!send(&mut f, DIR_IN, PEER, US, PROTO_UDP, &r, 14).accept);
        assert!(!send(&mut f, DIR_IN, GW, US, PROTO_UDP, &udp_bytes(53, 5354, 1), 14).accept);
        assert!(!send(&mut f, DIR_IN, GW, US, PROTO_UDP, &udp_bytes(54, 5353, 1), 14).accept);
        // destination port 0 is not a thing
        assert_eq!(send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &udp_bytes(1000, 0, 0), 15).detail, INV_PORT0);
    }

    #[test]
    fn udp_timeouts_are_exact_and_replies_extend_them() {
        let q = udp_bytes(7000, 7, 0);
        let r = udp_bytes(7, 7000, 0);
        let start = 1000u32;
        let mk = || {
            let mut f = fw();
            send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &q, start);
            f
        };
        // unreplied: 30 s, to the tick
        let edge = start + T_UDP_UNREPLIED;
        assert!(send(&mut mk(), DIR_IN, GW, US, PROTO_UDP, &r, edge - 1).accept, "one tick before expiry");
        assert!(!send(&mut mk(), DIR_IN, GW, US, PROTO_UDP, &r, edge).accept, "exactly at expiry the flow is gone");
        // a reply makes it replied: 180 s counted from the reply, to the tick
        let tr = start + 5;
        let replied = || {
            let mut f = mk();
            assert!(send(&mut f, DIR_IN, GW, US, PROTO_UDP, &r, tr).accept);
            f
        };
        assert!(send(&mut replied(), DIR_IN, GW, US, PROTO_UDP, &r, tr + T_UDP_REPLIED - 1).accept);
        assert!(!send(&mut replied(), DIR_IN, GW, US, PROTO_UDP, &r, tr + T_UDP_REPLIED).accept);
        // and every packet pushes the deadline out again
        let mut f = replied();
        let t2 = tr + T_UDP_REPLIED - 1;
        assert!(send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &q, t2).accept);
        assert!(send(&mut f, DIR_IN, GW, US, PROTO_UDP, &r, t2 + T_UDP_REPLIED - 1).accept, "refreshed by the packet at t2");
        assert_eq!(f.active_conns(t2 + 2 * T_UDP_REPLIED - 2), 1);
        assert_eq!(f.active_conns(t2 + 2 * T_UDP_REPLIED - 1), 0, "and then it does expire");
    }

    #[test]
    fn timeouts_survive_the_tick_counter_wrapping() {
        let start = u32::MAX - 100;
        let mut f = fw();
        let q = udp_bytes(7000, 7, 0);
        send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &q, start);
        let r = udp_bytes(7, 7000, 0);
        // still BEFORE the wrap (the deadline itself is already past it)
        assert!(send(&mut f, DIR_IN, GW, US, PROTO_UDP, &r, start.wrapping_add(50)).accept, "alive just before the wrap");
        let across = start.wrapping_add(500); // wrapped past zero
        assert!(across < 1000);
        assert!(send(&mut f, DIR_IN, GW, US, PROTO_UDP, &r, across).accept, "alive across the wrap");
        assert_eq!(f.active_conns(across), 1);
        let much_later = start.wrapping_add(T_UDP_REPLIED + 600);
        assert_eq!(f.active_conns(much_later), 0, "and gone after its timeout, also across the wrap");
        // a TCP flow too
        let mut g = fw();
        tcp_out(&mut g, 1111, 80, TCP_SYN, start);
        assert!(tcp_in(&mut g, 80, 1111, TCP_SYN | TCP_ACK, start.wrapping_add(T_SYN - 1)).accept);
        let mut h = fw();
        tcp_out(&mut h, 1111, 80, TCP_SYN, start);
        assert!(!tcp_in(&mut h, 80, 1111, TCP_SYN | TCP_ACK, start.wrapping_add(T_SYN)).accept);
    }

    // ---- ICMP ----

    #[test]
    fn icmp_echo_is_accepted_exactly_once_per_request() {
        let mut f = fw();
        let req = icmp_echo_bytes(8, 0x1234);
        let rep = icmp_echo_bytes(0, 0x1234);
        let v = send(&mut f, DIR_OUT, US, GW, PROTO_ICMP, &req, 1);
        assert!(v.accept && v.state == ST_NEW);
        let v = send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &rep, 2);
        assert!(v.accept && v.state == ST_ESTABLISHED);
        let v = send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &rep, 3);
        assert!(!v.accept && v.detail == INV_NO_CONN, "a second reply to one request");
        // two requests allow two replies
        send(&mut f, DIR_OUT, US, GW, PROTO_ICMP, &req, 4);
        send(&mut f, DIR_OUT, US, GW, PROTO_ICMP, &req, 5);
        assert!(send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &rep, 6).accept);
        assert!(send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &rep, 7).accept);
        assert!(!send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &rep, 8).accept);
        // an unsolicited reply, a reply from someone else, a reply with another id
        assert!(!send(&mut f, DIR_IN, PEER, US, PROTO_ICMP, &rep, 9).accept);
        send(&mut f, DIR_OUT, US, GW, PROTO_ICMP, &req, 10);
        assert!(!send(&mut f, DIR_IN, PEER, US, PROTO_ICMP, &rep, 11).accept);
        assert!(!send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &icmp_echo_bytes(0, 0x1235), 11).accept);
        // an inbound echo request needs a rule
        assert!(!send(&mut f, DIR_IN, PEER, US, PROTO_ICMP, &req, 12).accept);
        f.add_rule_text(0, b"rule accept in proto icmp icmp-type 8 state new").unwrap();
        assert!(send(&mut f, DIR_IN, PEER, US, PROTO_ICMP, &req, 13).accept);
        assert!(send(&mut f, DIR_OUT, US, PEER, PROTO_ICMP, &rep, 14).accept, "and our reply to it goes out");
        // other ICMP types are not tracked and not accepted
        let ts = icmp_echo_bytes(13, 1);
        assert_eq!(send(&mut f, DIR_IN, PEER, US, PROTO_ICMP, &ts, 15).detail, INV_ICMP);
        let mut bad_code = icmp_echo_bytes(8, 1);
        bad_code[1] = 5;
        assert_eq!(send(&mut f, DIR_OUT, US, GW, PROTO_ICMP, &bad_code, 16).detail, INV_ICMP);
    }

    #[test]
    fn icmp_errors_are_related_only_to_flows_we_track() {
        let mut f = fw();
        // we send a UDP datagram; the network says the port is unreachable
        send(&mut f, DIR_OUT, US, PEER, PROTO_UDP, &udp_bytes(5000, 9, 4), 1);
        let err = icmp_error_bytes(3, PROTO_UDP, US, PEER, 5000, 9);
        let v = send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &err, 2);
        assert!(v.accept && v.state == ST_RELATED && v.rule == 0);
        // an error about a flow that does not exist
        let other = icmp_error_bytes(3, PROTO_UDP, US, PEER, 5001, 9);
        let v = send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &other, 3);
        assert!(!v.accept && v.detail == INV_RELATED);
        let other = icmp_error_bytes(3, PROTO_UDP, US, PEER2, 5000, 9);
        assert!(!send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &other, 3).accept);
        // the same for TCP (time exceeded) and for a ping (destination unreachable)
        tcp_out(&mut f, 40000, 80, TCP_SYN, 4);
        assert!(send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &icmp_error_bytes(11, PROTO_TCP, US, PEER, 40000, 80), 5).accept);
        send(&mut f, DIR_OUT, US, PEER, PROTO_ICMP, &icmp_echo_bytes(8, 77), 6);
        assert!(send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &icmp_error_bytes(3, PROTO_ICMP, US, PEER, 77, 77), 7).accept);
        assert!(!send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &icmp_error_bytes(3, PROTO_ICMP, US, PEER, 78, 78), 7).accept);
        // an error quoting a protocol we cannot place
        let mut weird = icmp_error_bytes(3, PROTO_UDP, US, PEER, 5000, 9);
        weird[8 + 9] = 47;
        assert!(!send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &weird, 8).accept);
        assert!(f.stats().related >= 3);
    }

    #[test]
    fn an_icmp_type_in_a_rule_selects_that_type_only() {
        let mut f = fw();
        f.flush_rules().unwrap();
        f.add_rule_text(0, b"rule accept in proto icmp icmp-type 3 state related").unwrap();
        send(&mut f, DIR_OUT, US, PEER, PROTO_UDP, &udp_bytes(5000, 9, 4), 1);
        let unreachable = icmp_error_bytes(3, PROTO_UDP, US, PEER, 5000, 9);
        let exceeded = icmp_error_bytes(11, PROTO_UDP, US, PEER, 5000, 9);
        let v = send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &unreachable, 2);
        assert!(v.accept && v.rule == 0, "type 3 is what the rule names");
        let v = send(&mut f, DIR_IN, GW, US, PROTO_ICMP, &exceeded, 3);
        assert!(!v.accept && v.state == ST_RELATED && v.rule == -1, "type 11 is related too, but this rule is for type 3 only");
    }

    // ---- the TFTP helper ----

    #[test]
    fn tftp_reply_from_a_new_port_is_related() {
        let mut f = fw();
        // we ask the server on port 69 from our port 40000
        let rrq = udp_bytes(40000, TFTP_PORT, 20);
        assert!(send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &rrq, 100).accept);
        assert_eq!(f.active_expects(100), 1);
        // the server answers from a port of its own choosing
        let data = udp_bytes(51234, 40000, 516);
        let v = send(&mut f, DIR_IN, GW, US, PROTO_UDP, &data, 101);
        assert!(v.accept && v.state == ST_RELATED && v.ct_state == UDP_REPLIED);
        assert_eq!(f.active_expects(101), 0, "the expectation is used up");
        // a second block arrives before we have answered the first: the flow was
        // expected, so this is already ESTABLISHED, not a second NEW one
        let v = send(&mut f, DIR_IN, GW, US, PROTO_UDP, &data, 101);
        assert!(v.accept && v.state == ST_ESTABLISHED, "state {}", v.state);
        // the rest of the transfer is an ordinary established flow
        let ack = udp_bytes(40000, 51234, 4);
        let v = send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &ack, 102);
        assert!(v.accept && v.state == ST_ESTABLISHED);
        let v = send(&mut f, DIR_IN, GW, US, PROTO_UDP, &data, 103);
        assert!(v.accept && v.state == ST_ESTABLISHED);
        // a different server, or a different local port, is not expected
        let mut g = fw();
        send(&mut g, DIR_OUT, US, GW, PROTO_UDP, &rrq, 100);
        assert!(!send(&mut g, DIR_IN, PEER, US, PROTO_UDP, &data, 101).accept);
        assert!(!send(&mut g, DIR_IN, GW, US, PROTO_UDP, &udp_bytes(51234, 40001, 4), 101).accept);
        // and it expires
        let mut h = fw();
        send(&mut h, DIR_OUT, US, GW, PROTO_UDP, &rrq, 100);
        assert!(!send(&mut h, DIR_IN, GW, US, PROTO_UDP, &data, 100 + T_EXPECT).accept);
        let mut k = fw();
        send(&mut k, DIR_OUT, US, GW, PROTO_UDP, &rrq, 100);
        assert!(send(&mut k, DIR_IN, GW, US, PROTO_UDP, &data, 100 + T_EXPECT - 1).accept);
        // other destination ports create no expectation
        let mut m = fw();
        send(&mut m, DIR_OUT, US, GW, PROTO_UDP, &udp_bytes(40000, 70, 4), 1);
        assert_eq!(m.active_expects(1), 0);
        // only OUTBOUND requests do
        let mut n = fw();
        n.set_policy(DIR_IN, true).unwrap();
        send(&mut n, DIR_IN, GW, US, PROTO_UDP, &udp_bytes(40000, TFTP_PORT, 4), 1);
        assert_eq!(n.active_expects(1), 0);
    }

    #[test]
    fn expectations_are_bounded() {
        let mut f = fw();
        for i in 0..(MAX_EXPECT as u16 * 3) {
            send(&mut f, DIR_OUT, US, GW, PROTO_UDP, &udp_bytes(30000 + i, TFTP_PORT, 4), 1);
        }
        assert!(f.active_expects(1) <= MAX_EXPECT);
    }

    // ---- fragments and unsupported protocols ----

    #[test]
    fn fragments_and_unknown_protocols_are_dropped_even_with_open_policies() {
        let mut f = fw();
        f.set_policy(DIR_IN, true).unwrap();
        for dir in [DIR_IN, DIR_OUT] {
            let p = parse_packet(dir, PEER, US, PROTO_UDP, true, &udp_bytes(1, 2, 0));
            let v = f.filter(&p, 1, true);
            assert!(!v.accept && v.reason == R_FRAGMENT);
            let p = parse_packet(dir, PEER, US, 47, false, &[0; 24]);
            let v = f.filter(&p, 1, true);
            assert!(!v.accept && v.reason == R_UNSUPPORTED);
            let p = parse_packet(dir, PEER, US, PROTO_TCP, false, &[1, 2, 3]);
            let v = f.filter(&p, 1, true);
            assert!(!v.accept && v.reason == R_MALFORMED);
        }
        let s = f.stats();
        assert_eq!((s.fragments, s.unsupported, s.malformed), (2, 2, 2));
    }

    // ---- rules, policy, counters ----

    #[test]
    fn first_match_wins_and_policy_is_per_direction() {
        let mut f = fw();
        f.flush_rules().unwrap();
        f.add_rule_text(0, b"rule drop out proto udp dport 53").unwrap();
        f.add_rule_text(1, b"rule accept out proto udp").unwrap();
        f.add_rule_text(2, b"rule drop out").unwrap();
        let mut check = |proto: u8, d: Vec<u8>| {
            let p = parse_packet(DIR_OUT, US, GW, proto, false, &d);
            f.filter(&p, 1, false)
        };
        let v = check(PROTO_UDP, udp_bytes(5000, 53, 0));
        assert!(!v.accept && v.rule == 0);
        let v = check(PROTO_UDP, udp_bytes(5000, 123, 0));
        assert!(v.accept && v.rule == 1, "an earlier drop must not hide a later accept for other traffic");
        let v = check(PROTO_TCP, tcp_bytes(5000, 80, TCP_SYN));
        assert!(!v.accept && v.rule == 2);
        // no rule matches inbound: the inbound policy
        let p = parse_packet(DIR_IN, GW, US, PROTO_UDP, false, &udp_bytes(1, 2, 0));
        let v = f.filter(&p, 1, false);
        assert!(!v.accept && v.reason == R_POLICY && v.rule == -1);
        f.set_policy(DIR_IN, true).unwrap();
        assert!(f.filter(&p, 1, false).accept);
        // out policy applies when nothing matches outbound
        f.flush_rules().unwrap();
        f.set_policy(DIR_OUT, false).unwrap();
        let o = parse_packet(DIR_OUT, US, GW, PROTO_UDP, false, &udp_bytes(1, 2, 0));
        assert!(!f.filter(&o, 1, false).accept);
        // an "any" direction rule applies both ways
        f.add_rule_text(0, b"rule accept any proto udp").unwrap();
        assert!(f.filter(&o, 1, false).accept && f.filter(&p, 1, false).accept);
    }

    #[test]
    fn hit_counters_and_log_throttling() {
        let mut f = fw();
        f.flush_rules().unwrap();
        f.add_rule_text(0, b"rule drop in proto udp dport 9999 log").unwrap();
        f.add_rule_text(1, b"rule drop in proto udp dport 9998").unwrap();
        let mut logged = 0;
        for i in 0..1000 {
            let v = send(&mut f, DIR_IN, PEER, US, PROTO_UDP, &udp_bytes(1000 + (i % 50) as u16, 9999, 100), i);
            assert!(!v.accept && v.rule == 0);
            if v.log {
                logged += 1;
            }
        }
        // 32 up front, then 1 in 256 of the rest (sequence 256, 512, 768)
        assert_eq!(logged, 32 + 3, "{} lines logged for 1000 drops", logged);
        let (_, hits, bytes) = f.rule_at(0).unwrap();
        assert_eq!((hits, bytes), (1000, 1000 * 108));
        assert_eq!(f.rule_at(1).unwrap().1, 0);
        // a rule without `log` never logs ...
        let v = send(&mut f, DIR_IN, PEER, US, PROTO_UDP, &udp_bytes(1, 9998, 0), 2000);
        assert!(!v.log && v.rule == 1);
        // ... but invalid traffic is logged without any rule asking (a fresh firewall: the
        // throttle budget above is long spent)
        let mut g = fw();
        let v = tcp_in(&mut g, 1, 2, TCP_ACK, 1);
        assert!(v.log && v.reason == R_INVALID);
        let p = parse_packet(DIR_IN, PEER, US, PROTO_UDP, true, &udp_bytes(1, 2, 0));
        assert!(g.filter(&p, 1, true).log, "fragments too");
    }

    #[test]
    fn a_dropped_packet_does_not_advance_flow_state() {
        let mut f = fw();
        f.add_rule_text(0, b"rule drop in proto tcp sport 80 state established").unwrap();
        tcp_out(&mut f, 40000, 80, TCP_SYN, 1);
        // the SYN-ACK is dropped by the rule: the flow must stay SYN_SENT
        let v = tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 2);
        assert!(!v.accept && v.rule == 0);
        assert_eq!(f.conn_nth(0, 2).unwrap().state, TCP_SYN_SENT);
        assert_eq!(f.conn_nth(0, 2).unwrap().pkts, [1, 0]);
        f.del_rule(0).unwrap();
        assert!(tcp_in(&mut f, 80, 40000, TCP_SYN | TCP_ACK, 3).accept, "once the rule is gone the same packet is fine");
    }

    #[test]
    fn a_dry_run_changes_nothing() {
        let mut f = fw();
        tcp_out(&mut f, 40000, 80, TCP_SYN, 1);
        let before = (f.stats(), f.active_conns(1), f.conn_nth(0, 1), f.rule_at(0));
        let p = parse_packet(DIR_IN, PEER, US, PROTO_TCP, false, &tcp_bytes(80, 40000, TCP_SYN | TCP_ACK));
        let v = f.filter(&p, 2, false);
        assert!(v.accept && v.ct_state == TCP_SYN_RECV, "it reports what WOULD happen");
        let n = parse_packet(DIR_OUT, US, PEER, PROTO_TCP, false, &tcp_bytes(41000, 81, TCP_SYN));
        assert!(f.filter(&n, 2, false).accept);
        assert_eq!((f.stats(), f.active_conns(1), f.conn_nth(0, 1), f.rule_at(0)), before);
        assert_eq!(f.active_conns(2), 1, "the dry-run SYN created nothing");
        assert!(!f.filter(&Pkt { flags: TCP_ACK, ..p }, 2, false).accept);
    }

    // ---- capacity ----

    #[test]
    fn a_full_table_refuses_new_flows_but_not_established_ones() {
        let mut f = fw();
        for i in 0..MAX_CONNS as u16 {
            assert!(tcp_out(&mut f, 10000 + i, 80, TCP_SYN, 1).accept, "flow {}", i);
        }
        let v = tcp_out(&mut f, 20000, 80, TCP_SYN, 1);
        assert!(!v.accept && v.reason == R_TABLE_FULL, "the {}th flow", MAX_CONNS + 1);
        assert_eq!(f.stats().table_full, 1);
        // established traffic is unaffected
        assert!(tcp_in(&mut f, 80, 10000, TCP_SYN | TCP_ACK, 2).accept);
        assert!(tcp_out(&mut f, 10000, 80, TCP_ACK, 2).accept);
        // flows that time out give their slots back
        let later = 1 + T_SYN;
        assert_eq!(f.active_conns(later), 1, "all but the one that was refreshed have timed out");
        assert!(tcp_out(&mut f, 20000, 80, TCP_SYN, later).accept);
        // a UDP or ICMP flow needs room too
        let mut g = fw();
        for i in 0..MAX_CONNS as u16 {
            send(&mut g, DIR_OUT, US, GW, PROTO_UDP, &udp_bytes(10000 + i, 53, 0), 1);
        }
        assert_eq!(send(&mut g, DIR_OUT, US, GW, PROTO_UDP, &udp_bytes(20000, 53, 0), 1).reason, R_TABLE_FULL);
        assert_eq!(send(&mut g, DIR_OUT, US, GW, PROTO_ICMP, &icmp_echo_bytes(8, 1), 1).reason, R_TABLE_FULL);
    }

    #[test]
    fn inbound_syn_floods_are_capped_without_starving_our_own_connections() {
        let mut f = fw();
        f.add_rule_text(0, b"rule accept in proto tcp dport 22 state new").unwrap();
        let mut accepted = 0;
        for i in 0..200u16 {
            let v = send(&mut f, DIR_IN, PEER, US, PROTO_TCP, &tcp_bytes(20000 + i, 22, TCP_SYN), 1);
            if v.accept {
                accepted += 1;
            } else {
                assert_eq!(v.reason, R_HALF_OPEN);
            }
        }
        assert_eq!(accepted, HALF_OPEN_MAX, "exactly the cap");
        assert_eq!(f.stats().half_open as usize, 200 - HALF_OPEN_MAX);
        // our own outbound connection still works
        assert!(tcp_out(&mut f, 40000, 80, TCP_SYN, 2).accept);
        // a handshake that completes stops being half-open and frees room
        assert!(send(&mut f, DIR_OUT, US, PEER, PROTO_TCP, &tcp_bytes(22, 20000, TCP_SYN | TCP_ACK), 3).accept);
        assert!(send(&mut f, DIR_IN, PEER, US, PROTO_TCP, &tcp_bytes(20000, 22, TCP_ACK), 4).accept);
        assert!(send(&mut f, DIR_IN, PEER, US, PROTO_TCP, &tcp_bytes(30000, 22, TCP_SYN), 5).accept);
    }

    // ---- administration, lock, enable ----

    #[test]
    fn rule_administration() {
        let mut f = fw();
        assert_eq!(f.rule_count(), 1);
        f.add_rule_text(0, b"rule drop out proto icmp").unwrap();
        f.add_rule_text(2, b"rule accept in proto tcp dport 22").unwrap();
        assert_eq!(f.rule_count(), 3);
        assert!(!f.rule_at(0).unwrap().0.accept && f.rule_at(2).unwrap().0.dp_lo == 22);
        assert_eq!(f.add_rule_text(9, b"rule drop any").err(), Some(AdminError::BadIndex));
        assert_eq!(f.add_rule_text(0, b"bogus").err().map(|e| e.errno()), Some(EINVAL));
        f.del_rule(1).unwrap();
        assert_eq!(f.rule_count(), 2);
        assert!(f.rule_at(1).unwrap().0.dp_set, "rules after the deleted one moved up");
        assert_eq!(f.del_rule(5).err(), Some(AdminError::BadIndex));
        assert!(f.rule_at(2).is_none());
        while f.rule_count() < MAX_RULES {
            f.add_rule_text(0, b"rule drop any").unwrap();
        }
        assert_eq!(f.add_rule_text(0, b"rule drop any").err(), Some(AdminError::Full));
        assert_eq!(AdminError::Full.errno(), ENOSPC);
        f.flush_rules().unwrap();
        assert_eq!(f.rule_count(), 0);
        // hit counts travel with their rules when others are inserted or removed
        let mut g = fw();
        g.add_rule_text(0, b"rule drop in proto udp dport 1").unwrap();
        send(&mut g, DIR_IN, PEER, US, PROTO_UDP, &udp_bytes(5, 1, 0), 1);
        g.add_rule_text(0, b"rule drop in proto udp dport 2").unwrap();
        assert_eq!((g.rule_at(0).unwrap().1, g.rule_at(1).unwrap().1), (0, 1));
        g.del_rule(0).unwrap();
        assert_eq!(g.rule_at(0).unwrap().1, 1);
    }

    #[test]
    fn apply_config_is_atomic() {
        let mut f = fw();
        tcp_out(&mut f, 40000, 80, TCP_SYN, 1);
        f.apply_config(b"policy in accept\nrule drop out proto udp\n").unwrap();
        assert!(f.policy(DIR_IN) && f.rule_count() == 1);
        assert_eq!(f.active_conns(1), 1, "flows survive a reconfiguration");
        let r = f.apply_config(b"policy in drop\nrule drop any\nrule bogus\n");
        assert!(matches!(r, Err(AdminError::Parse(ParseError::Syntax(3)))));
        assert!(f.policy(DIR_IN) && f.rule_count() == 1, "a bad config changes NOTHING, not even the lines before the error");
    }

    #[test]
    fn disabling_passes_everything_and_enabling_restores_it() {
        let mut f = fw();
        assert!(f.is_enabled());
        f.set_enabled(false).unwrap();
        let v = tcp_in(&mut f, 1, 2, TCP_FIN, 1);
        assert!(v.accept && v.reason == R_DISABLED);
        assert_eq!(f.active_conns(1), 0, "a disabled firewall does not track");
        f.set_enabled(true).unwrap();
        assert!(!tcp_in(&mut f, 1, 2, TCP_FIN, 2).accept);
    }

    #[test]
    fn the_lock_is_one_way_and_blocks_every_policy_change() {
        let mut f = fw();
        assert!(!f.is_locked());
        f.lock();
        assert!(f.is_locked());
        let r = Rule::EMPTY;
        assert_eq!(f.add_rule(0, r).err(), Some(AdminError::Locked));
        assert_eq!(f.add_rule_text(0, b"rule drop any").err(), Some(AdminError::Locked));
        assert_eq!(f.del_rule(0).err(), Some(AdminError::Locked));
        assert_eq!(f.flush_rules().err(), Some(AdminError::Locked));
        assert_eq!(f.set_policy(DIR_IN, true).err(), Some(AdminError::Locked));
        assert_eq!(f.set_enabled(false).err(), Some(AdminError::Locked));
        assert_eq!(f.apply_config(BASELINE).err(), Some(AdminError::Locked));
        assert_eq!(AdminError::Locked.errno(), EPERM);
        assert_eq!(f.rule_count(), 1);
        assert!(f.is_enabled() && !f.policy(DIR_IN));
        // flushing tracked flows only makes it stricter, so it stays possible
        tcp_out(&mut f, 1, 2, TCP_SYN, 1);
        f.flush_conns();
        assert_eq!(f.active_conns(1), 0);
        // and filtering keeps working
        assert!(tcp_out(&mut f, 3, 4, TCP_SYN, 2).accept);
        // locking again is harmless; only a re-init (a reboot) clears it
        f.lock();
        assert!(f.is_locked());
        f.init();
        assert!(!f.is_locked());
    }

    #[test]
    fn an_uninitialised_firewall_drops_everything() {
        let mut f = Firewall::new();
        for dir in [DIR_IN, DIR_OUT] {
            let (s, d) = if dir == DIR_IN { (PEER, US) } else { (US, PEER) };
            let v = send(&mut f, dir, s, d, PROTO_TCP, &tcp_bytes(1000, 80, TCP_SYN), 1);
            assert!(!v.accept, "dir {}", dir);
            let v = send(&mut f, dir, s, d, PROTO_UDP, &udp_bytes(1000, 80, 0), 1);
            assert!(!v.accept);
        }
        assert_eq!(f.rule_count(), 0);
    }

    // ---- model-based and fuzz ----

    /// One legitimate conversation. `ia` starts it (address, port), `ib` answers.
    /// Which of the two is us decides the direction of each packet.
    #[derive(Clone)]
    struct Session {
        proto: u8,
        ia: (u32, u16),
        ib: (u32, u16),
        icmp_id: u16,
        script: Vec<(bool, u8)>, // (from the initiator?, tcp flags)
        at: usize,
    }

    fn tcp_script(r: &mut Rng, reset: bool) -> Vec<(bool, u8)> {
        let mut s = vec![(true, TCP_SYN), (false, TCP_SYN | TCP_ACK), (true, TCP_ACK)];
        for _ in 0..r.below(6) {
            s.push((r.below(2) == 0, TCP_ACK | if r.below(2) == 0 { TCP_PSH } else { 0 }));
        }
        if reset {
            s.push((r.below(2) == 0, TCP_RST));
        } else if r.below(2) == 0 {
            // the initiator closes first
            s.extend_from_slice(&[(true, TCP_FIN | TCP_ACK), (false, TCP_ACK), (false, TCP_FIN | TCP_ACK), (true, TCP_ACK)]);
        } else {
            s.extend_from_slice(&[(false, TCP_FIN | TCP_ACK), (true, TCP_ACK), (true, TCP_FIN | TCP_ACK), (false, TCP_ACK)]);
        }
        s
    }

    #[test]
    fn model_every_legitimate_session_passes_and_every_stray_packet_does_not() {
        let mut r = Rng(0xF1AE_0001);
        let mut packets = 0u64;
        let mut strays_dropped = 0u64;
        for round in 0..300u32 {
            let mut f = fw();
            f.add_rule_text(0, b"rule accept in proto tcp dport 22 state new").unwrap();
            f.add_rule_text(1, b"rule accept in proto udp dport 123 state new").unwrap();
            let mut sessions: Vec<Session> = Vec::new();
            for i in 0..1 + r.below(12) as u32 {
                let proto = [PROTO_TCP, PROTO_UDP, PROTO_ICMP][r.below(3)];
                // inbound-initiated sessions only where a rule lets them in
                let outbound = proto == PROTO_ICMP || r.below(3) != 0;
                let peer = 0x1000_0000 + i * 7 + 1;
                let ours = 30000 + ((round * 13 + i * 101) % 20000) as u16; // distinct per session
                let theirs = 40000 + ((round * 17 + i * 103) % 20000) as u16;
                let (svc_tcp, svc_udp, out_tcp, out_udp) = (22u16, 123u16, 80 + r.below(3) as u16, 53u16);
                let (ia, ib) = match (proto, outbound) {
                    (PROTO_TCP, true) => ((US, ours), (peer, out_tcp)),
                    (PROTO_TCP, false) => ((peer, theirs), (US, svc_tcp)),
                    (PROTO_UDP, true) => ((US, ours), (peer, out_udp)),
                    (PROTO_UDP, false) => ((peer, theirs), (US, svc_udp)),
                    _ => ((US, 0), (peer, 0)),
                };
                let reset = r.below(5) == 0;
                let script = match proto {
                    PROTO_TCP => tcp_script(&mut r, reset),
                    PROTO_UDP => vec![(true, 0), (false, 0), (true, 0), (false, 0)],
                    _ => vec![(true, 0), (false, 0)],
                };
                sessions.push(Session { proto, ia, ib, icmp_id: ours, script, at: 0 });
            }
            let mut now = 1000u32;
            let mut live: Vec<usize> = (0..sessions.len()).collect();
            let mut guard = 0;
            while !live.is_empty() {
                guard += 1;
                assert!(guard < 5000);
                now += r.below(3) as u32;
                // A stray packet from an address no session uses. It must be
                // dropped and must change nothing - unless it happens to be exactly
                // what a rule invites (a SYN to port 22, a datagram to port 123),
                // in which case it is a legitimate NEW flow and is allowed to open one.
                if r.below(3) == 0 {
                    let stray = 0xC0A8_0000 + r.below(200) as u32;
                    let (sp, dp) = (1 + r.below(60000) as u16, 1 + r.below(60000) as u16);
                    let before = f.active_conns(now);
                    let v = match r.below(3) {
                        0 => send(&mut f, DIR_IN, stray, US, PROTO_TCP, &tcp_bytes(sp, dp, r.below(64) as u8), now),
                        1 => send(&mut f, DIR_IN, stray, US, PROTO_UDP, &udp_bytes(sp, dp, r.below(30)), now),
                        _ => send(&mut f, DIR_IN, stray, US, PROTO_ICMP, &icmp_echo_bytes([0u8, 8, 3, 13][r.below(4)], r.below(65536) as u16), now),
                    };
                    packets += 1;
                    if v.accept {
                        assert!(v.state == ST_NEW && v.rule >= 0 && v.rule < 2, "a stray packet got through without a rule's invitation: {:?}", v);
                    } else {
                        strays_dropped += 1;
                        assert_eq!(f.active_conns(now), before, "a dropped stray must not create a flow");
                    }
                    continue;
                }
                let li = r.below(live.len());
                let si = live[li];
                let s = sessions[si].clone();
                let (from_init, flags) = s.script[s.at];
                let (src, dst) = if from_init { (s.ia, s.ib) } else { (s.ib, s.ia) };
                let dir = if src.0 == US { DIR_OUT } else { DIR_IN };
                let bytes = match s.proto {
                    PROTO_TCP => tcp_bytes(src.1, dst.1, flags),
                    PROTO_UDP => udp_bytes(src.1, dst.1, r.below(40)),
                    _ => icmp_echo_bytes(if from_init { 8 } else { 0 }, s.icmp_id),
                };
                let v = send(&mut f, dir, src.0, dst.0, s.proto, &bytes, now);
                packets += 1;
                assert!(
                    v.accept,
                    "round {}: a legitimate packet was dropped (proto {} step {} flags {:#x} dir {}): {:?}",
                    round, s.proto, s.at, flags, dir, v
                );
                sessions[si].at += 1;
                if sessions[si].at >= sessions[si].script.len() {
                    live.swap_remove(li);
                }
            }
            assert!(f.active_conns(now) <= sessions.len() + 200, "round {}: runaway flow count", round);
        }
        assert!(packets > 5000, "{} packets", packets);
        assert!(strays_dropped > 1000, "only {} strays were exercised", strays_dropped);
    }

    #[test]
    fn fuzz_random_packets_never_panic_and_never_break_the_table() {
        let mut r = Rng(0xDEC0_DE01);
        let mut f = fw();
        f.set_policy(DIR_IN, true).unwrap();
        for i in 0..60_000u32 {
            let len = r.below(80);
            let data: Vec<u8> = (0..len).map(|_| r.below(256) as u8).collect();
            let proto = [PROTO_TCP, PROTO_UDP, PROTO_ICMP, 47, 0][r.below(5)];
            let dir = if r.below(2) == 0 { DIR_IN } else { DIR_OUT };
            let src = if r.below(2) == 0 { US } else { 0x0100_0000 + r.below(8) as u32 };
            let dst = if r.below(2) == 0 { US } else { 0x0100_0000 + r.below(8) as u32 };
            let p = parse_packet(dir, src, dst, proto, r.below(20) == 0, &data);
            let v = f.filter(&p, i / 3, r.below(8) != 0);
            if p.bad != BAD_NONE {
                assert!(!v.accept);
            }
            assert!(f.active_conns(i / 3) <= MAX_CONNS && f.active_expects(i / 3) <= MAX_EXPECT);
        }
    }

    #[test]
    fn fuzz_random_rule_text_never_panics_and_accepted_rules_are_well_formed() {
        let mut r = Rng(0xFEED_FACE);
        let words: [&[u8]; 22] = [b"rule", b"accept", b"drop", b"in", b"out", b"any", b"proto", b"tcp", b"udp", b"icmp", b"from", b"to", b"sport", b"dport", b"state", b"new,established", b"10.0.2.0/24", b"22", b"1-100", b"icmp-type", b"8", b"log"];
        let valid: [&[u8]; 6] = [
            b"rule accept in proto tcp from 10.0.2.0/24 to 10.0.2.15 sport 1024-65535 dport 22 state new,established log",
            b"rule drop out proto udp dport 53",
            b"rule accept in proto icmp icmp-type 8 state new",
            b"rule drop any from 1.2.3.4/16",
            b"rule accept out proto tcp dport 80-443 state new,established,related",
            b"rule accept in",
        ];
        let alphabet: &[u8] = b" /,-.0123456789abcdefghijklmnopqrstuvwxyz";
        let mut ok = 0;
        for i in 0..60_000 {
            let mut t: Vec<u8> = Vec::new();
            if i % 2 == 0 {
                // word soup
                for _ in 0..r.below(14) {
                    t.extend_from_slice(words[r.below(words.len())]);
                    t.push(b' ');
                    if r.below(30) == 0 {
                        t.push(r.below(256) as u8);
                    }
                }
            } else {
                // a valid rule with a few bytes flipped, inserted or removed
                t.extend_from_slice(valid[r.below(valid.len())]);
                for _ in 0..r.below(4) {
                    if t.is_empty() {
                        break;
                    }
                    let pos = r.below(t.len());
                    match r.below(3) {
                        0 => t[pos] = alphabet[r.below(alphabet.len())],
                        1 => t.insert(pos, alphabet[r.below(alphabet.len())]),
                        _ => {
                            t.remove(pos);
                        }
                    }
                }
            }
            if let Ok(rule) = parse_rule(&t) {
                ok += 1;
                assert!(!(rule.sp_set || rule.dp_set) || rule.proto == PROTO_TCP || rule.proto == PROTO_UDP);
                assert!(!rule.icmp_set || rule.proto == PROTO_ICMP);
                assert!(rule.sp_lo <= rule.sp_hi && rule.dp_lo <= rule.dp_hi);
                assert!(rule.src_ip & rule.src_mask == rule.src_ip && rule.dst_ip & rule.dst_mask == rule.dst_ip);
                assert!(rule.states & ST_INVALID == 0);
            }
            let _ = parse_config(&t);
        }
        assert!(ok > 2000, "the fuzz should still produce valid rules ({})", ok);
    }

    #[test]
    fn the_boot_selftest_holds() {
        let (held, total) = selftest();
        assert_eq!(held, total);
        assert!(total >= 13, "{} checks", total);
    }
}
