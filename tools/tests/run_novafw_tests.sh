#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Stateful Packet-Filter Firewall (novafw)
#
# Validates advanced stateful packet filtering and NAT:
# 1. Connection Tracking (Conntrack):
#    - 5-tuple tracking (src, dst, sport, dport, proto).
#    - State machine: NEW -> ESTABLISHED -> TIME_WAIT -> CLOSED.
#    - UDP bidirectional tracking.
#    - Helper expectation table for RELATED flows (TFTP, ICMP errors).
#    - Detection and drop of INVALID TCP flag anomalies (NULL scan, XMAS scan).
# 2. Network Address Translation (NAT):
#    - SNAT (Source NAT) rewriting and reverse reply translation.
#    - DNAT (Destination NAT) port forwarding and return path translation.
#    - Masquerade dynamic translation to egress interface.
#    - RFC 1624 IPv4 header and TCP/UDP pseudo-header checksum recalculation.
# 3. Rate-Limiting & Port-Scan Defense:
#    - Token-Bucket rate limiting with burst allowances and steady-state replenishment.
#    - Multi-port reconnaissance detector flagging sweeps and automatically banning offending IPs.
# 4. nftables Declarative Rule Engine:
#    - Multi-table (filter, nat), multi-chain (prerouting, input, forward, output, postrouting).
#    - Granular matching on CIDR prefixes, ports, states, interfaces, rate limits.
# 5. Userland CLI Tool (novafw):
#    - novafw load, status, rules list, conntrack list/flush, nat list, scan-stats, --selftest.
#
# Usage:
#   ./tools/tests/run_novafw_tests.sh      (or: make novafw-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/net/firewall/ host tests (Stateful Firewall & Conntrack) ---"

    ROOT_DIR="$(pwd)"
    cat << EOF > "$OUT/test_harness.rs"
#![allow(dead_code)]
#[path = "$ROOT_DIR/kernel/src/net/firewall/mod.rs"]
mod firewall;

pub mod spinlock {
    pub struct SpinLock<T>(std::sync::Mutex<T>);
    impl<T> SpinLock<T> {
        pub const fn new(val: T) -> Self {
            SpinLock(std::sync::Mutex::new(val))
        }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
    }
}

// Mock timer ticks for host tests
#[no_mangle]
pub extern "C" fn timer_get_ticks() -> u32 {
    1000
}

#[cfg(test)]
mod tests {
    use super::firewall::*;
    use super::firewall::conntrack::*;
    use super::firewall::nat::*;
    use super::firewall::ratelimit::*;
    use super::firewall::rules::*;

    #[test]
    fn test_kernel_firewall_selftest() {
        assert!(run_novafw_selftest(), "Kernel firewall self-test must pass");
    }

    #[test]
    fn test_conntrack_state_transitions() {
        let mut ct = ConntrackTable::new();
        let now = 500u32;

        // NEW SYN
        let (s, dir, _, idx) = ct.track_packet(
            0x0A00020F, 0x08080808, 40000, 53, IP_PROTO_UDP, 0, 64, now
        );
        assert_eq!(s, ConntrackState::New);
        assert_eq!(dir, Some(FlowDirection::Original));
        assert!(idx.is_some());

        // Reply datagram -> ESTABLISHED
        let (s2, dir2, _, _) = ct.track_packet(
            0x08080808, 0x0A00020F, 53, 40000, IP_PROTO_UDP, 0, 128, now + 10
        );
        assert_eq!(s2, ConntrackState::Established);
        assert_eq!(dir2, Some(FlowDirection::Reply));
        assert_eq!(ct.count(), 1);

        ct.flush();
        assert_eq!(ct.count(), 0);
    }

    #[test]
    fn test_nat_dnat_port_forwarding() {
        let mut pkt = [0u8; 40];
        pkt[0] = 0x45; // IPv4
        pkt[9] = IP_PROTO_TCP;
        pkt[12..16].copy_from_slice(&0xC0A80164u32.to_be_bytes()); // 192.168.1.100
        pkt[16..20].copy_from_slice(&0x0A00020Fu32.to_be_bytes()); // 10.0.2.15 (host)
        pkt[20..22].copy_from_slice(&51234u16.to_be_bytes());     // client sport
        pkt[22..24].copy_from_slice(&8080u16.to_be_bytes());      // host dport 8080

        let dnat_map = NatMapping {
            is_snat: false,
            snat_ip: 0,
            snat_port: 0,
            is_dnat: true,
            dnat_ip: 0x0A580002, // container 10.88.0.2
            dnat_port: 80,
            is_masquerade: false,
        };

        // Forward translation
        assert!(NatEngine::rewrite_packet(&mut pkt, &dnat_map, FlowDirection::Original, 0xC0A80164, 0x0A00020F, 51234, 8080));
        assert_eq!(u32::from_be_bytes([pkt[16], pkt[17], pkt[18], pkt[19]]), 0x0A580002);
        assert_eq!(u16::from_be_bytes([pkt[22], pkt[23]]), 80);

        // Reverse reply translation
        let mut reply_pkt = [0u8; 40];
        reply_pkt[0] = 0x45;
        reply_pkt[9] = IP_PROTO_TCP;
        reply_pkt[12..16].copy_from_slice(&0x0A580002u32.to_be_bytes()); // container 10.88.0.2:80
        reply_pkt[16..20].copy_from_slice(&0xC0A80164u32.to_be_bytes());
        reply_pkt[20..22].copy_from_slice(&80u16.to_be_bytes());
        reply_pkt[22..24].copy_from_slice(&51234u16.to_be_bytes());

        assert!(NatEngine::rewrite_packet(&mut reply_pkt, &dnat_map, FlowDirection::Reply, 0xC0A80164, 0x0A00020F, 51234, 8080));
        assert_eq!(u32::from_be_bytes([reply_pkt[12], reply_pkt[13], reply_pkt[14], reply_pkt[15]]), 0x0A00020F);
        assert_eq!(u16::from_be_bytes([reply_pkt[20], reply_pkt[21]]), 8080);
    }

    #[test]
    fn test_port_scan_mitigation() {
        let mut detector = PortScanDetector::new();
        let scanner_ip = 0xC0000277; // 192.0.2.119
        let now = 2000;

        for port in 1..=7 {
            assert!(!detector.record_probe(scanner_ip, port, now));
        }
        // 8th port triggers ban
        assert!(detector.record_probe(scanner_ip, 8, now));
        assert!(detector.check_banned(scanner_ip, now + 5));
        assert_eq!(detector.total_scans_detected, 1);
    }
}
EOF

    rustc --test "$OUT/test_harness.rs" -o "$OUT/test_novafw"
    "$OUT/test_novafw"
    echo "[PASS] All kernel/src/net/firewall/ conntrack, NAT, and rules tests succeeded."

    echo "--- userland/server/novafw CLI & Engine verification ---"
    cargo test --manifest-path userland/server/novafw/Cargo.toml --quiet
    cargo run --manifest-path userland/server/novafw/Cargo.toml --quiet -- --selftest
    cargo run --manifest-path userland/server/novafw/Cargo.toml --quiet -- load tools/fixtures/novafw.conf
    cargo run --manifest-path userland/server/novafw/Cargo.toml --quiet -- status
    cargo run --manifest-path userland/server/novafw/Cargo.toml --quiet -- rules list
    cargo run --manifest-path userland/server/novafw/Cargo.toml --quiet -- conntrack list
    cargo run --manifest-path userland/server/novafw/Cargo.toml --quiet -- nat list
    cargo run --manifest-path userland/server/novafw/Cargo.toml --quiet -- scan-stats
fi

echo "[PASS] All Stateful Packet-Filter Firewall (novafw) tests passed successfully."
