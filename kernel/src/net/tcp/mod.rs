//! kernel/src/net/tcp/mod.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Enterprise Cloud TCP Stack
//!
//! Upgrades the Rust TCP network stack to enterprise cloud standards:
//!   - RFC 8312 CUBIC Congestion Control
//!   - BBR (Bottleneck Bandwidth and RTT) Congestion Control
//!   - RFC 2018 Selective Acknowledgments (SACK)
//!   - RFC 7323 TCP Window Scaling (up to 1GB window)
//!   - RFC 4987 TCP SYN Cookies for SYN flood DDoS defense
//!   - RFC 1122 TCP Keepalive Timers

pub mod bbr;
pub mod cubic;
pub mod keepalive;
pub mod sack;
pub mod syncookie;
pub mod window_scale;

pub use bbr::{BbrCongestionControl, BbrMode};
pub use cubic::{CubicCongestionControl, CubicMode};
pub use keepalive::{KeepaliveAction, TcpKeepalive};
pub use sack::{
    parse_sack_options, SackBlock, SackReceiver, SackScoreboard,
    MAX_SACK_BLOCKS,
};
pub use syncookie::{SynCookieGenerator, SYN_COOKIE_MSS_TABLE};
pub use window_scale::{parse_window_scale_option, TcpWindowScale};

/// Supported Congestion Control Algorithms
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CongestionAlgorithm {
    Cubic,
    Bbr,
}

/// Unified container for connection-level congestion and enterprise TCP features
#[derive(Clone, Debug)]
pub struct ConnectionEnterpriseState {
    pub algorithm: CongestionAlgorithm,
    pub cubic: CubicCongestionControl,
    pub bbr: BbrCongestionControl,
    pub sack_receiver: SackReceiver,
    pub sack_scoreboard: SackScoreboard,
    pub window_scale: TcpWindowScale,
    pub keepalive: TcpKeepalive,
}

impl ConnectionEnterpriseState {
    pub const fn new(algo: CongestionAlgorithm, mss: u32) -> Self {
        Self {
            algorithm: algo,
            cubic: CubicCongestionControl::new(mss),
            bbr: BbrCongestionControl::new(mss),
            sack_receiver: SackReceiver::new(),
            sack_scoreboard: SackScoreboard::new(),
            window_scale: TcpWindowScale::new(),
            keepalive: TcpKeepalive::new(),
        }
    }

    /// Current effective congestion window in bytes.
    pub fn current_cwnd(&self) -> u32 {
        match self.algorithm {
            CongestionAlgorithm::Cubic => self.cubic.cwnd,
            CongestionAlgorithm::Bbr => self.bbr.cwnd,
        }
    }

    /// Process incoming ACK event across active congestion control and SACK engine.
    pub fn on_ack(
        &mut self,
        cumulative_ack: u32,
        acked_bytes: u32,
        rtt_ms: u32,
        now_ms: u32,
        sack_blocks: &[SackBlock],
    ) {
        // 1. Process SACK scoreboard
        self.sack_scoreboard.process_sack_blocks(cumulative_ack, sack_blocks);

        // 2. Feed active congestion controller
        let in_flight = self.sack_scoreboard.in_flight_bytes();
        match self.algorithm {
            CongestionAlgorithm::Cubic => {
                self.cubic.on_ack(acked_bytes, rtt_ms, now_ms);
            }
            CongestionAlgorithm::Bbr => {
                self.bbr.on_ack(acked_bytes, rtt_ms, in_flight, now_ms);
            }
        }

        // 3. Keepalive activity reset
        self.keepalive.on_activity(now_ms / 10);
    }

    /// Process loss event across active congestion controller.
    pub fn on_loss(&mut self, now_ms: u32) {
        let in_flight = self.sack_scoreboard.in_flight_bytes();
        match self.algorithm {
            CongestionAlgorithm::Cubic => {
                self.cubic.on_loss(now_ms);
            }
            CongestionAlgorithm::Bbr => {
                self.bbr.on_loss(in_flight);
            }
        }
    }
}

/// Comprehensive self-test for Enterprise TCP features.
pub fn run_enterprise_tcp_selftest() -> bool {
    // 1. Test RFC 8312 CUBIC
    let mut cubic = CubicCongestionControl::new(1460);
    assert_eq!(cubic.cwnd, 1460 * 10); // initial 10 MSS
    assert_eq!(cubic.mode, CubicMode::SlowStart);

    // Slow Start growth
    cubic.on_ack(1460, 20, 100);
    assert_eq!(cubic.cwnd, 1460 * 11);

    // Trigger loss -> enters Fast Recovery with beta=0.7 reduction
    cubic.on_loss(200);
    assert_eq!(cubic.mode, CubicMode::FastRecovery);
    assert!(cubic.cwnd < 1460 * 11);
    assert_eq!(cubic.ssthresh, cubic.cwnd);

    // ACK in FastRecovery -> Congestion Avoidance
    cubic.on_ack(1460, 20, 300);
    assert_eq!(cubic.mode, CubicMode::CongestionAvoidance);

    // 2. Test BBR
    let mut bbr = BbrCongestionControl::new(1460);
    assert_eq!(bbr.mode, BbrMode::Startup);
    assert!(bbr.pacing_rate_bps > 0);

    // Deliver ACK samples to advance BBR
    for i in 1..=5 {
        bbr.on_ack(1460 * 10, 20, 1460 * 5, i * 100);
    }
    assert!(bbr.btl_bw_bytes_per_sec > 0);
    assert_eq!(bbr.min_rtt_ms, 20);

    // 3. Test RFC 2018 SACK Receiver and Scoreboard
    let mut sack_rcv = SackReceiver::new();
    sack_rcv.sack_permitted = true;
    // Inbound out-of-order segment [5000, 6460)
    sack_rcv.add_received_range(5000, 1460);
    assert_eq!(sack_rcv.block_count, 1);
    assert_eq!(sack_rcv.blocks[0], SackBlock::new(5000, 6460));

    // Inbound second out-of-order segment [8000, 9460)
    sack_rcv.add_received_range(8000, 1460);
    assert_eq!(sack_rcv.block_count, 2);
    assert_eq!(sack_rcv.blocks[0], SackBlock::new(8000, 9460));

    // Serialize SACK option
    let mut sack_opt_buf = [0u8; 32];
    let written = sack_rcv.write_sack_option(&mut sack_opt_buf);
    assert_eq!(written, 2 + 16); // Kind(1) + Len(1) + 2 blocks * 8 bytes

    // Parse back SACK option
    let (_permitted, parsed_blocks, parsed_cnt) = parse_sack_options(&sack_opt_buf[..written]);
    assert_eq!(parsed_cnt, 2);
    assert_eq!(parsed_blocks[0], SackBlock::new(8000, 9460));
    assert_eq!(parsed_blocks[1], SackBlock::new(5000, 6460));

    // Test SACK Scoreboard hole detection
    let mut sb = SackScoreboard::new();
    sb.sack_enabled = true;
    sb.record_sent(1000, 2460);
    sb.record_sent(2460, 3920); // Lost hole!
    sb.record_sent(3920, 5380);

    // Peer ACKs 2460 and SACKs [3920, 5380)
    let s_blocks = [SackBlock::new(3920, 5380)];
    sb.process_sack_blocks(2460, &s_blocks);
    // Hole [2460, 3920) should be reported for fast retransmission
    let hole = sb.get_next_lost_hole();
    assert_eq!(hole, Some((2460, 3920)));

    // 4. Test RFC 7323 TCP Window Scaling
    let mut ws = TcpWindowScale::new();
    ws.negotiate(Some(7)); // Peer offers shift 7
    assert!(ws.enabled);
    assert_eq!(ws.snd_scale, 7);
    // Decode: 1000 in header -> 1000 << 7 = 128,000 bytes
    assert_eq!(ws.decode_window(1000), 128_000);
    // Encode: 128,000 bytes -> 1000 in header
    assert_eq!(ws.encode_window(128_000), 1000);

    // 5. Test RFC 4987 SYN Cookies
    let cookie_gen = SynCookieGenerator::new();
    let src_ip = 0x0A000202; // 10.0.2.2
    let dst_ip = 0x0A00020F; // 10.0.2.15
    let cookie = cookie_gen.generate_cookie(src_ip, 54321, dst_ip, 80, 1460, 1000);
    let verified = cookie_gen.verify_cookie(cookie, src_ip, 54321, dst_ip, 80, 1050);
    assert_eq!(verified, Some(1460));

    // Forged cookie should fail
    let forged = cookie_gen.verify_cookie(cookie ^ 0x00FF0000, src_ip, 54321, dst_ip, 80, 1050);
    assert_eq!(forged, None);

    // 6. Test RFC 1122 Keepalive
    let mut keepalive = TcpKeepalive::new();
    keepalive.idle_ticks = 100;
    keepalive.interval_ticks = 20;
    keepalive.probes_max = 3;
    keepalive.on_activity(0);

    // Advance to tick 100 -> triggers probe
    let action = keepalive.poll(5000, 100);
    assert_eq!(action, KeepaliveAction::SendProbe { probe_seq: 4999 });

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enterprise_tcp_full_suite() {
        assert!(run_enterprise_tcp_selftest());
    }
}
