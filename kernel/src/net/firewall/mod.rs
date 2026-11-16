//! kernel/src/net/firewall/mod.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Advanced Stateful Packet-Filter Firewall
//!
//! Inspired by Linux nftables:
//! - Stateful packet inspection with bidirectional connection tracking (NEW, ESTABLISHED, RELATED, INVALID)
//! - Network Address Translation (SNAT, DNAT, Masquerade) with RFC checksum updates
//! - Rate-limiting and active defense against port-scan and syn-flood reconnaissance
//! - Declarative multi-table, multi-chain rule engine (filter & nat)

pub mod conntrack;
pub mod nat;
pub mod ratelimit;
pub mod rules;

use crate::spinlock::SpinLock;
pub use self::conntrack::*;
pub use self::nat::NatEngine;
pub use self::ratelimit::{PortScanDetector, TokenBucket};
pub use self::rules::{
    ChainHook, Rule, RuleTable, Verdict, STATE_MASK_ESTABLISHED,
    STATE_MASK_NEW, STATE_MASK_RELATED,
};

pub const DEFAULT_EGRESS_IP: u32 = 0x0A00020F; // 10.0.2.15

pub struct NovaFirewall {
    pub enabled: bool,
    pub conntrack: ConntrackTable,
    pub filter_table: RuleTable,
    pub nat_table: RuleTable,
    pub scan_detector: PortScanDetector,
    pub egress_ip: u32,
    pub total_in_packets: u64,
    pub total_out_packets: u64,
    pub total_dropped: u64,
    pub total_accepted: u64,
    pub total_nat_translated: u64,
}

impl NovaFirewall {
    pub const fn new() -> Self {
        NovaFirewall {
            enabled: false, // Disabled initially until configured or selftest
            conntrack: ConntrackTable::new(),
            filter_table: RuleTable::new(),
            nat_table: RuleTable::new(),
            scan_detector: PortScanDetector::new(),
            egress_ip: DEFAULT_EGRESS_IP,
            total_in_packets: 0,
            total_out_packets: 0,
            total_dropped: 0,
            total_accepted: 0,
            total_nat_translated: 0,
        }
    }

    /// Initializes default enterprise server ruleset (nftables baseline).
    pub fn init_default_ruleset(&mut self) {
        self.filter_table.flush();
        self.nat_table.flush();

        // 1. Filter Table: INPUT chain (policy: Drop)
        if let Some(input) = self.filter_table.create_chain(b"input", ChainHook::Input, Verdict::Drop) {
            // Rule 1: Accept loopback interface
            let mut r_lo = Rule::empty();
            r_lo.iif[0..2].copy_from_slice(b"lo");
            r_lo.verdict = Verdict::Accept;
            input.add_rule(r_lo);

            // Rule 2: Accept established and related traffic
            let mut r_est = Rule::empty();
            r_est.state_mask = STATE_MASK_ESTABLISHED | STATE_MASK_RELATED;
            r_est.verdict = Verdict::Accept;
            input.add_rule(r_est);

            // Rule 3: Accept SSH (port 22)
            let mut r_ssh = Rule::empty();
            r_ssh.proto = Some(IP_PROTO_TCP);
            r_ssh.dport_min = 22;
            r_ssh.dport_max = 22;
            r_ssh.state_mask = STATE_MASK_NEW;
            r_ssh.verdict = Verdict::Accept;
            input.add_rule(r_ssh);

            // Rule 4: Accept HTTP/HTTPS (ports 80, 443)
            let mut r_web = Rule::empty();
            r_web.proto = Some(IP_PROTO_TCP);
            r_web.dport_min = 80;
            r_web.dport_max = 80;
            r_web.state_mask = STATE_MASK_NEW;
            r_web.verdict = Verdict::Accept;
            input.add_rule(r_web);

            let mut r_ssl = Rule::empty();
            r_ssl.proto = Some(IP_PROTO_TCP);
            r_ssl.dport_min = 443;
            r_ssl.dport_max = 443;
            r_ssl.state_mask = STATE_MASK_NEW;
            r_ssl.verdict = Verdict::Accept;
            input.add_rule(r_ssl);

            // Rule 5: Accept WireGuard VPN (port 51820 UDP)
            let mut r_wg = Rule::empty();
            r_wg.proto = Some(IP_PROTO_UDP);
            r_wg.dport_min = 51820;
            r_wg.dport_max = 51820;
            r_wg.state_mask = STATE_MASK_NEW;
            r_wg.verdict = Verdict::Accept;
            input.add_rule(r_wg);

            // Rule 6: ICMP rate-limited ping (5/sec, burst 10)
            let mut r_icmp = Rule::empty();
            r_icmp.proto = Some(IP_PROTO_ICMP);
            r_icmp.token_bucket = Some(TokenBucket::new(5, 10));
            r_icmp.verdict = Verdict::Accept;
            input.add_rule(r_icmp);
        }

        // 2. Filter Table: FORWARD chain (policy: Drop)
        if let Some(fwd) = self.filter_table.create_chain(b"forward", ChainHook::Forward, Verdict::Drop) {
            // Forward established/related
            let mut r_fwd_est = Rule::empty();
            r_fwd_est.state_mask = STATE_MASK_ESTABLISHED | STATE_MASK_RELATED;
            r_fwd_est.verdict = Verdict::Accept;
            fwd.add_rule(r_fwd_est);

            // Allow container bridge (nova0) to WAN (eth0)
            let mut r_bridge = Rule::empty();
            r_bridge.iif[0..5].copy_from_slice(b"nova0");
            r_bridge.oif[0..4].copy_from_slice(b"eth0");
            r_bridge.verdict = Verdict::Accept;
            fwd.add_rule(r_bridge);
        }

        // 3. Filter Table: OUTPUT chain (policy: Accept)
        self.filter_table.create_chain(b"output", ChainHook::Output, Verdict::Accept);

        // 4. NAT Table: POSTROUTING chain (Masquerade for container subnet 10.88.0.0/16)
        if let Some(post) = self.nat_table.create_chain(b"postrouting", ChainHook::Postrouting, Verdict::Accept) {
            let mut r_masq = Rule::empty();
            r_masq.src_net = 0x0A580000; // 10.88.0.0
            r_masq.src_mask = 0xFFFF0000; // /16
            r_masq.verdict = Verdict::Masquerade;
            post.add_rule(r_masq);
        }

        // 5. NAT Table: PREROUTING chain (Port forwarding host 8080 -> container 10.88.0.2:80)
        if let Some(pre) = self.nat_table.create_chain(b"prerouting", ChainHook::Prerouting, Verdict::Accept) {
            let mut r_dnat = Rule::empty();
            r_dnat.proto = Some(IP_PROTO_TCP);
            r_dnat.dport_min = 8080;
            r_dnat.dport_max = 8080;
            r_dnat.verdict = Verdict::Dnat {
                ip: 0x0A580002, // 10.88.0.2
                port: 80,
            };
            pre.add_rule(r_dnat);
        }
    }

    /// Inbound packet filter and inspection.
    pub fn inspect_inbound(
        &mut self,
        src_ip: u32,
        dst_ip: u32,
        proto: u8,
        fragment: bool,
        payload: &[u8],
        now_ticks: u32,
    ) -> bool {
        if !self.enabled {
            return true;
        }

        self.total_in_packets += 1;

        if fragment {
            self.total_dropped += 1;
            return false;
        }

        // Check if source IP is banned for port scanning
        if self.scan_detector.check_banned(src_ip, now_ticks) {
            self.total_dropped += 1;
            return false;
        }

        let (sport, dport, tcp_flags) = Self::parse_transport(proto, payload);

        // Port scan reconnaissance detection for TCP SYN probes
        if proto == IP_PROTO_TCP && (tcp_flags & TCP_FLAG_SYN) != 0 && (tcp_flags & TCP_FLAG_ACK) == 0 {
            if self.scan_detector.record_probe(src_ip, dport, now_ticks) {
                self.total_dropped += 1;
                return false;
            }
        }

        // Track packet state
        let (state, _dir, _nat_map, _slot) = self.conntrack.track_packet(
            src_ip,
            dst_ip,
            sport,
            dport,
            proto,
            tcp_flags,
            payload.len() as u32,
            now_ticks,
        );

        if state == ConntrackState::Invalid {
            self.total_dropped += 1;
            return false;
        }

        // Evaluate INPUT chain
        let verdict = if let Some(chain) = self.filter_table.get_chain_mut(ChainHook::Input) {
            chain.evaluate(src_ip, dst_ip, sport, dport, proto, state, b"eth0", b"", payload.len() as u32, now_ticks)
        } else {
            Verdict::Accept
        };

        match verdict {
            Verdict::Accept => {
                self.total_accepted += 1;
                true
            }
            Verdict::Drop | Verdict::Reject => {
                self.total_dropped += 1;
                false
            }
            _ => {
                self.total_accepted += 1;
                true
            }
        }
    }

    /// Outbound packet filter and inspection.
    pub fn inspect_outbound(
        &mut self,
        dest_ip: u32,
        proto: u8,
        payload: &[u8],
        now_ticks: u32,
    ) -> bool {
        if !self.enabled {
            return true;
        }

        self.total_out_packets += 1;
        let (sport, dport, tcp_flags) = Self::parse_transport(proto, payload);

        let (state, _dir, _nat_map, _slot) = self.conntrack.track_packet(
            self.egress_ip,
            dest_ip,
            sport,
            dport,
            proto,
            tcp_flags,
            payload.len() as u32,
            now_ticks,
        );

        if state == ConntrackState::Invalid {
            self.total_dropped += 1;
            return false;
        }

        // Evaluate OUTPUT chain
        let verdict = if let Some(chain) = self.filter_table.get_chain_mut(ChainHook::Output) {
            chain.evaluate(self.egress_ip, dest_ip, sport, dport, proto, state, b"", b"eth0", payload.len() as u32, now_ticks)
        } else {
            Verdict::Accept
        };

        match verdict {
            Verdict::Accept => {
                self.total_accepted += 1;
                true
            }
            _ => {
                self.total_dropped += 1;
                false
            }
        }
    }

    fn parse_transport(proto: u8, payload: &[u8]) -> (u16, u16, u8) {
        if proto == IP_PROTO_TCP && payload.len() >= 14 {
            let sport = u16::from_be_bytes([payload[0], payload[1]]);
            let dport = u16::from_be_bytes([payload[2], payload[3]]);
            let flags = payload[13];
            (sport, dport, flags)
        } else if proto == IP_PROTO_UDP && payload.len() >= 4 {
            let sport = u16::from_be_bytes([payload[0], payload[1]]);
            let dport = u16::from_be_bytes([payload[2], payload[3]]);
            (sport, dport, 0)
        } else if proto == IP_PROTO_ICMP && payload.len() >= 2 {
            let icmp_type = payload[0] as u16;
            let icmp_code = payload[1] as u16;
            (icmp_type, icmp_code, 0)
        } else {
            (0, 0, 0)
        }
    }
}

pub static GLOBAL_FIREWALL: SpinLock<NovaFirewall> = SpinLock::new(NovaFirewall::new());

extern "C" {
    fn timer_get_ticks() -> u32;
}

/// C-ABI dispatch hook: called from `kernel/net/ip.c` for inbound packets.
#[no_mangle]
pub extern "C" fn rust_novafw_filter_in(
    src_ip: u32,
    proto: u8,
    fragment: bool,
    payload: *const u8,
    length: u16,
) -> bool {
    let now = unsafe { timer_get_ticks() };
    let slice = if payload.is_null() || length == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(payload, length as usize) }
    };
    GLOBAL_FIREWALL.lock().inspect_inbound(src_ip, DEFAULT_EGRESS_IP, proto, fragment, slice, now)
}

/// C-ABI dispatch hook: called from `kernel/net/ip.c` for outbound packets.
#[no_mangle]
pub extern "C" fn rust_novafw_filter_out(
    dest_ip: u32,
    proto: u8,
    payload: *const u8,
    length: u16,
) -> bool {
    let now = unsafe { timer_get_ticks() };
    let slice = if payload.is_null() || length == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(payload, length as usize) }
    };
    GLOBAL_FIREWALL.lock().inspect_outbound(dest_ip, proto, slice, now)
}

/// Comprehensive self-test executed during kernel boot.
#[no_mangle]
pub extern "C" fn rust_novafw_selftest() -> i32 {
    if run_novafw_selftest() {
        0
    } else {
        1
    }
}

pub fn run_novafw_selftest() -> bool {
    let mut ct = ConntrackTable::new();
    let now = 1000u32;

    // 1. Conntrack TCP 3-way handshake verification
    let (s1, d1, _, idx1) = ct.track_packet(
        0x0A00020F, 0x98516401, 45000, 80, IP_PROTO_TCP, TCP_FLAG_SYN, 0, now,
    );
    if s1 != ConntrackState::New || d1 != Some(FlowDirection::Original) || idx1.is_none() {
        return false;
    }

    // SYN-ACK reply
    let (_s2, d2, _, _) = ct.track_packet(
        0x98516401, 0x0A00020F, 80, 45000, IP_PROTO_TCP, TCP_FLAG_SYN | TCP_FLAG_ACK, 0, now + 10,
    );
    if d2 != Some(FlowDirection::Reply) {
        return false;
    }

    // ACK completing handshake -> ESTABLISHED
    let (s3, d3, _, _) = ct.track_packet(
        0x0A00020F, 0x98516401, 45000, 80, IP_PROTO_TCP, TCP_FLAG_ACK, 0, now + 20,
    );
    if s3 != ConntrackState::Established || d3 != Some(FlowDirection::Original) {
        return false;
    }

    // 2. INVALID TCP flags rejection (XMAS scan: FIN+PSH+URG, NULL scan)
    let (s_inv1, _, _, _) = ct.track_packet(
        0xC0000205, 0x0A00020F, 50000, 80, IP_PROTO_TCP,
        TCP_FLAG_FIN | TCP_FLAG_PSH | TCP_FLAG_URG, 0, now,
    );
    if s_inv1 != ConntrackState::Invalid {
        return false;
    }

    let (s_inv2, _, _, _) = ct.track_packet(
        0xC0000205, 0x0A00020F, 50000, 80, IP_PROTO_TCP, 0, 0, now,
    );
    if s_inv2 != ConntrackState::Invalid {
        return false;
    }

    // 3. UDP connection tracking and RELATED helper expectation
    ct.add_expectation(0x0A000202, 0x0A00020F, 49152, IP_PROTO_UDP, 500, now);
    let (s_rel, _, _, _) = ct.track_packet(
        0x0A000202, 0x0A00020F, 69, 49152, IP_PROTO_UDP, 0, 512, now + 5,
    );
    if s_rel != ConntrackState::Related {
        return false;
    }

    // 4. NAT SNAT & Checksum verification
    let mut pkt = [0u8; 40];
    pkt[0] = 0x45; // IPv4, IHL=5
    pkt[9] = IP_PROTO_TCP;
    pkt[12..16].copy_from_slice(&0x0A580002u32.to_be_bytes()); // 10.88.0.2
    pkt[16..20].copy_from_slice(&0x98516401u32.to_be_bytes());
    pkt[20..22].copy_from_slice(&12345u16.to_be_bytes()); // sport
    pkt[22..24].copy_from_slice(&80u16.to_be_bytes());    // dport

    let snat_map = NatMapping {
        is_snat: true,
        snat_ip: 0x0A00020Fu32,
        snat_port: 54321,
        is_dnat: false,
        dnat_ip: 0,
        dnat_port: 0,
        is_masquerade: false,
    };

    if !NatEngine::rewrite_packet(&mut pkt, &snat_map, FlowDirection::Original, 0x0A580002, 0x98516401, 12345, 80) {
        return false;
    }
    // Verify source rewritten to 10.0.2.15:54321
    if u32::from_be_bytes([pkt[12], pkt[13], pkt[14], pkt[15]]) != 0x0A00020F {
        return false;
    }
    if u16::from_be_bytes([pkt[20], pkt[21]]) != 54321 {
        return false;
    }

    // Reverse reply translation
    let mut reply_pkt = [0u8; 40];
    reply_pkt[0] = 0x45;
    reply_pkt[9] = IP_PROTO_TCP;
    reply_pkt[12..16].copy_from_slice(&0x98516401u32.to_be_bytes());
    reply_pkt[16..20].copy_from_slice(&0x0A00020Fu32.to_be_bytes());
    reply_pkt[20..22].copy_from_slice(&80u16.to_be_bytes());
    reply_pkt[22..24].copy_from_slice(&54321u16.to_be_bytes());

    if !NatEngine::rewrite_packet(&mut reply_pkt, &snat_map, FlowDirection::Reply, 0x0A580002, 0x98516401, 12345, 80) {
        return false;
    }
    // Verify restored back to 10.88.0.2:12345
    if u32::from_be_bytes([reply_pkt[16], reply_pkt[17], reply_pkt[18], reply_pkt[19]]) != 0x0A580002 {
        return false;
    }
    if u16::from_be_bytes([reply_pkt[22], reply_pkt[23]]) != 12345 {
        return false;
    }

    // 5. Rate Limiting verification
    let mut tb = TokenBucket::new(10, 3); // 3 burst, 10/sec
    if !tb.try_consume(100) || !tb.try_consume(100) || !tb.try_consume(100) {
        return false;
    }
    // 4th consume at same tick must fail (burst exhausted)
    if tb.try_consume(100) {
        return false;
    }
    // After 10 ticks (0.1s -> 1 token added)
    if !tb.try_consume(110) {
        return false;
    }

    // 6. Port Scan Reconnaissance Detector verification
    let mut detector = PortScanDetector::new();
    let scanner_ip = 0xC0000263; // 192.0.2.99
    for p in 1..=7 {
        if detector.record_probe(scanner_ip, p, now) {
            return false; // Should not ban before threshold (8)
        }
    }
    // 8th distinct port triggers port scan detection & IP ban
    if !detector.record_probe(scanner_ip, 8, now) {
        return false;
    }
    // Subsequent probes from banned IP are rejected
    if !detector.check_banned(scanner_ip, now + 1) {
        return false;
    }

    // 7. Rule table evaluation
    let mut rt = RuleTable::new();
    let chain = rt.create_chain(b"input", ChainHook::Input, Verdict::Drop).unwrap();
    let mut r = Rule::empty();
    r.proto = Some(IP_PROTO_TCP);
    r.dport_min = 22;
    r.dport_max = 22;
    r.verdict = Verdict::Accept;
    chain.add_rule(r);

    let v_ssh = chain.evaluate(0x0A000201, 0x0A00020F, 50000, 22, IP_PROTO_TCP, ConntrackState::New, b"eth0", b"", 60, now);
    if v_ssh != Verdict::Accept {
        return false;
    }

    let v_blocked = chain.evaluate(0x0A000201, 0x0A00020F, 50000, 23, IP_PROTO_TCP, ConntrackState::New, b"eth0", b"", 60, now);
    if v_blocked != Verdict::Drop {
        return false;
    }

    true
}
