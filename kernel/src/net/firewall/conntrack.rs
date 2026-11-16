//! kernel/src/net/firewall/conntrack.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Connection Tracking (Conntrack) Subsystem
//!
//! Maintains an active bidirectional connection state table:
//! - 5-tuple tracking: (src_ip, dst_ip, sport, dport, proto)
//! - Connection states: NEW, ESTABLISHED, RELATED, INVALID
//! - TCP state machine: SYN_SENT, SYN_RECV, ESTABLISHED, FIN_WAIT, CLOSE_WAIT, LAST_ACK, TIME_WAIT, CLOSED
//! - Helper expectation table for RELATED connections (TFTP data, ICMP error payloads)
//! - NAT metadata attachment for symmetric reverse-translation on reply paths
//! - Connection timeout expiration and active flow metrics

pub const MAX_CONNTRACK_ENTRIES: usize = 256;
pub const MAX_EXPECTATIONS: usize = 32;

pub const TCP_FLAG_FIN: u8 = 0x01;
pub const TCP_FLAG_SYN: u8 = 0x02;
pub const TCP_FLAG_RST: u8 = 0x04;
pub const TCP_FLAG_PSH: u8 = 0x08;
pub const TCP_FLAG_ACK: u8 = 0x10;
pub const TCP_FLAG_URG: u8 = 0x20;

pub const IP_PROTO_ICMP: u8 = 1;
pub const IP_PROTO_TCP: u8 = 6;
pub const IP_PROTO_UDP: u8 = 17;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ConntrackState {
    New,
    Established,
    Related,
    Invalid,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TcpState {
    None,
    SynSent,
    SynRecv,
    Established,
    FinWait,
    CloseWait,
    LastAck,
    TimeWait,
    Closed,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FlowDirection {
    Original,
    Reply,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FlowTuple {
    pub src_ip: u32,
    pub dst_ip: u32,
    pub sport: u16,
    pub dport: u16,
    pub proto: u8,
}

impl FlowTuple {
    pub const fn empty() -> Self {
        FlowTuple {
            src_ip: 0,
            dst_ip: 0,
            sport: 0,
            dport: 0,
            proto: 0,
        }
    }

    pub fn matches(&self, src: u32, dst: u32, sp: u16, dp: u16, pr: u8) -> bool {
        self.proto == pr && self.src_ip == src && self.dst_ip == dst && self.sport == sp && self.dport == dp
    }

    pub fn reverse(&self) -> Self {
        FlowTuple {
            src_ip: self.dst_ip,
            dst_ip: self.src_ip,
            sport: self.dport,
            dport: self.sport,
            proto: self.proto,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct NatMapping {
    pub is_snat: bool,
    pub snat_ip: u32,
    pub snat_port: u16,
    pub is_dnat: bool,
    pub dnat_ip: u32,
    pub dnat_port: u16,
    pub is_masquerade: bool,
}

impl NatMapping {
    pub const fn none() -> Self {
        NatMapping {
            is_snat: false,
            snat_ip: 0,
            snat_port: 0,
            is_dnat: false,
            dnat_ip: 0,
            dnat_port: 0,
            is_masquerade: false,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ConntrackEntry {
    pub original: FlowTuple,
    pub reply: FlowTuple,
    pub state: ConntrackState,
    pub tcp_state: TcpState,
    pub packets_orig: u64,
    pub bytes_orig: u64,
    pub packets_reply: u64,
    pub bytes_reply: u64,
    pub last_seen_tick: u32,
    pub timeout_ticks: u32,
    pub nat: NatMapping,
    pub is_active: bool,
}

impl ConntrackEntry {
    pub const fn empty() -> Self {
        ConntrackEntry {
            original: FlowTuple::empty(),
            reply: FlowTuple::empty(),
            state: ConntrackState::Invalid,
            tcp_state: TcpState::None,
            packets_orig: 0,
            bytes_orig: 0,
            packets_reply: 0,
            bytes_reply: 0,
            last_seen_tick: 0,
            timeout_ticks: 0,
            nat: NatMapping::none(),
            is_active: false,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ExpectationEntry {
    pub src_ip: u32,
    pub dst_ip: u32,
    pub dport: u16,
    pub proto: u8,
    pub expires_tick: u32,
    pub is_active: bool,
}

impl ExpectationEntry {
    pub const fn empty() -> Self {
        ExpectationEntry {
            src_ip: 0,
            dst_ip: 0,
            dport: 0,
            proto: 0,
            expires_tick: 0,
            is_active: false,
        }
    }
}

pub struct ConntrackTable {
    pub entries: [ConntrackEntry; MAX_CONNTRACK_ENTRIES],
    pub expectations: [ExpectationEntry; MAX_EXPECTATIONS],
    pub total_allocated: u64,
    pub total_expired: u64,
}

impl ConntrackTable {
    pub const fn new() -> Self {
        ConntrackTable {
            entries: [ConntrackEntry::empty(); MAX_CONNTRACK_ENTRIES],
            expectations: [ExpectationEntry::empty(); MAX_EXPECTATIONS],
            total_allocated: 0,
            total_expired: 0,
        }
    }

    /// Check if TCP flags are fundamentally invalid (port scan flags, illegal combinations).
    pub fn is_invalid_tcp_flags(flags: u8) -> bool {
        // No flags (NULL scan)
        if flags == 0 {
            return true;
        }
        // SYN + FIN
        if (flags & (TCP_FLAG_SYN | TCP_FLAG_FIN)) == (TCP_FLAG_SYN | TCP_FLAG_FIN) {
            return true;
        }
        // SYN + RST
        if (flags & (TCP_FLAG_SYN | TCP_FLAG_RST)) == (TCP_FLAG_SYN | TCP_FLAG_RST) {
            return true;
        }
        // FIN without ACK (and not pure RST/SYN)
        if (flags & TCP_FLAG_FIN) != 0 && (flags & TCP_FLAG_ACK) == 0 {
            return true;
        }
        // XMAS scan: FIN + PSH + URG
        if (flags & (TCP_FLAG_FIN | TCP_FLAG_PSH | TCP_FLAG_URG)) == (TCP_FLAG_FIN | TCP_FLAG_PSH | TCP_FLAG_URG) {
            return true;
        }
        false
    }

    /// Look up an active entry matching original or reply tuple.
    pub fn lookup(&self, src: u32, dst: u32, sport: u16, dport: u16, proto: u8) -> Option<(usize, FlowDirection)> {
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.is_active {
                continue;
            }
            if entry.original.matches(src, dst, sport, dport, proto) {
                return Some((i, FlowDirection::Original));
            }
            if entry.reply.matches(src, dst, sport, dport, proto) {
                return Some((i, FlowDirection::Reply));
            }
        }
        None
    }

    /// Check expectation table for RELATED flows.
    pub fn check_expectation(&mut self, src: u32, dst: u32, dport: u16, proto: u8, now_ticks: u32) -> bool {
        for exp in self.expectations.iter_mut() {
            if !exp.is_active {
                continue;
            }
            if exp.expires_tick < now_ticks {
                exp.is_active = false;
                continue;
            }
            if (exp.src_ip == 0 || exp.src_ip == src)
                && (exp.dst_ip == 0 || exp.dst_ip == dst)
                && exp.proto == proto
                && exp.dport == dport
            {
                // Consumed expectation
                exp.is_active = false;
                return true;
            }
        }
        false
    }

    /// Register a helper expectation (e.g. TFTP data port reply).
    pub fn add_expectation(&mut self, src_ip: u32, dst_ip: u32, dport: u16, proto: u8, duration_ticks: u32, now_ticks: u32) -> bool {
        for exp in self.expectations.iter_mut() {
            if !exp.is_active || exp.expires_tick < now_ticks {
                exp.src_ip = src_ip;
                exp.dst_ip = dst_ip;
                exp.dport = dport;
                exp.proto = proto;
                exp.expires_tick = now_ticks.wrapping_add(duration_ticks);
                exp.is_active = true;
                return true;
            }
        }
        false
    }

    /// Allocates a new connection tracking entry slot.
    fn allocate_entry(&mut self, now_ticks: u32) -> Option<usize> {
        let mut oldest_idx = None;
        let mut oldest_tick = u32::MAX;

        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.is_active {
                return Some(i);
            }
            // Check for expired entries
            if entry.last_seen_tick.wrapping_add(entry.timeout_ticks) < now_ticks {
                return Some(i);
            }
            if entry.last_seen_tick < oldest_tick {
                oldest_tick = entry.last_seen_tick;
                oldest_idx = Some(i);
            }
        }

        // Table is full, reclaim oldest non-established or oldest entry
        if let Some(idx) = oldest_idx {
            self.total_expired += 1;
            Some(idx)
        } else {
            None
        }
    }

    /// Main packet tracking entry point.
    /// Updates or creates a conntrack entry for the packet.
    /// Returns (ConntrackState, FlowDirection, NatMapping, entry_index).
    pub fn track_packet(
        &mut self,
        src: u32,
        dst: u32,
        sport: u16,
        dport: u16,
        proto: u8,
        tcp_flags: u8,
        payload_len: u32,
        now_ticks: u32,
    ) -> (ConntrackState, Option<FlowDirection>, Option<NatMapping>, Option<usize>) {
        if proto == IP_PROTO_TCP && Self::is_invalid_tcp_flags(tcp_flags) {
            return (ConntrackState::Invalid, None, None, None);
        }

        if let Some((idx, dir)) = self.lookup(src, dst, sport, dport, proto) {
            let entry = &mut self.entries[idx];
            entry.last_seen_tick = now_ticks;

            match dir {
                FlowDirection::Original => {
                    entry.packets_orig += 1;
                    entry.bytes_orig += payload_len as u64;
                }
                FlowDirection::Reply => {
                    entry.packets_reply += 1;
                    entry.bytes_reply += payload_len as u64;
                    // First packet in reply direction transitions NEW to ESTABLISHED
                    if entry.state == ConntrackState::New {
                        entry.state = ConntrackState::Established;
                    }
                }
            }

            // TCP state machine progression
            if proto == IP_PROTO_TCP {
                match entry.tcp_state {
                    TcpState::SynSent => {
                        if dir == FlowDirection::Reply && (tcp_flags & (TCP_FLAG_SYN | TCP_FLAG_ACK)) == (TCP_FLAG_SYN | TCP_FLAG_ACK) {
                            entry.tcp_state = TcpState::SynRecv;
                            entry.timeout_ticks = 3000; // 30s
                        } else if (tcp_flags & TCP_FLAG_RST) != 0 {
                            entry.tcp_state = TcpState::Closed;
                            entry.timeout_ticks = 500; // 5s
                        }
                    }
                    TcpState::SynRecv => {
                        if dir == FlowDirection::Original && (tcp_flags & TCP_FLAG_ACK) != 0 {
                            entry.tcp_state = TcpState::Established;
                            entry.state = ConntrackState::Established;
                            entry.timeout_ticks = 30000; // 300s
                        } else if (tcp_flags & TCP_FLAG_RST) != 0 {
                            entry.tcp_state = TcpState::Closed;
                            entry.timeout_ticks = 500;
                        }
                    }
                    TcpState::Established => {
                        if (tcp_flags & TCP_FLAG_FIN) != 0 {
                            entry.tcp_state = TcpState::FinWait;
                            entry.timeout_ticks = 3000;
                        } else if (tcp_flags & TCP_FLAG_RST) != 0 {
                            entry.tcp_state = TcpState::Closed;
                            entry.timeout_ticks = 500;
                        }
                    }
                    TcpState::FinWait => {
                        if (tcp_flags & TCP_FLAG_ACK) != 0 || (tcp_flags & TCP_FLAG_FIN) != 0 {
                            entry.tcp_state = TcpState::TimeWait;
                            entry.timeout_ticks = 1000; // 10s
                        }
                    }
                    TcpState::TimeWait => {
                        if (tcp_flags & TCP_FLAG_RST) != 0 {
                            entry.tcp_state = TcpState::Closed;
                        }
                    }
                    _ => {}
                }
            } else if proto == IP_PROTO_UDP {
                entry.timeout_ticks = if entry.state == ConntrackState::Established { 12000 } else { 3000 };
            } else if proto == IP_PROTO_ICMP {
                entry.timeout_ticks = 1000;
            }

            let nat_map = entry.nat;
            return (entry.state, Some(dir), Some(nat_map), Some(idx));
        }

        // No existing flow found. Is this a valid NEW flow or RELATED flow?
        let is_related = self.check_expectation(src, dst, dport, proto, now_ticks);

        if proto == IP_PROTO_TCP {
            // New TCP flow MUST begin with SYN (and not ACK/FIN/RST)
            if (tcp_flags & TCP_FLAG_SYN) == 0 || (tcp_flags & TCP_FLAG_ACK) != 0 {
                return (ConntrackState::Invalid, None, None, None);
            }
        }

        // Allocate slot for new flow
        let slot = match self.allocate_entry(now_ticks) {
            Some(s) => s,
            None => return (ConntrackState::Invalid, None, None, None),
        };

        self.total_allocated += 1;
        let orig = FlowTuple { src_ip: src, dst_ip: dst, sport, dport, proto };
        let reply = orig.reverse();

        let initial_state = if is_related {
            ConntrackState::Related
        } else {
            ConntrackState::New
        };

        let initial_tcp_state = if proto == IP_PROTO_TCP {
            TcpState::SynSent
        } else {
            TcpState::None
        };

        let timeout = match proto {
            IP_PROTO_TCP => 3000,  // 30s for SYN_SENT
            IP_PROTO_UDP => 3000,  // 30s
            _ => 1000,            // 10s
        };

        self.entries[slot] = ConntrackEntry {
            original: orig,
            reply,
            state: initial_state,
            tcp_state: initial_tcp_state,
            packets_orig: 1,
            bytes_orig: payload_len as u64,
            packets_reply: 0,
            bytes_reply: 0,
            last_seen_tick: now_ticks,
            timeout_ticks: timeout,
            nat: NatMapping::none(),
            is_active: true,
        };

        (initial_state, Some(FlowDirection::Original), Some(NatMapping::none()), Some(slot))
    }

    /// Attaches a NAT mapping to an active connection tracking entry.
    pub fn attach_nat_mapping(&mut self, slot: usize, mapping: NatMapping) {
        if slot >= MAX_CONNTRACK_ENTRIES || !self.entries[slot].is_active {
            return;
        }

        let entry = &mut self.entries[slot];
        entry.nat = mapping;

        // If SNAT/Masquerade, the reply tuple expects destination to match the translated SNAT IP/port
        if mapping.is_snat || mapping.is_masquerade {
            entry.reply.dst_ip = mapping.snat_ip;
            entry.reply.dport = mapping.snat_port;
        }

        // If DNAT, the reply tuple expects source to match the destination container IP/port
        if mapping.is_dnat {
            entry.reply.src_ip = mapping.dnat_ip;
            entry.reply.sport = mapping.dnat_port;
        }
    }

    /// Flushes all active connections.
    pub fn flush(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.is_active = false;
        }
        for exp in self.expectations.iter_mut() {
            exp.is_active = false;
        }
    }

    /// Returns count of currently active connections.
    pub fn count(&self) -> usize {
        let mut count = 0;
        for entry in self.entries.iter() {
            if entry.is_active {
                count += 1;
            }
        }
        count
    }
}
