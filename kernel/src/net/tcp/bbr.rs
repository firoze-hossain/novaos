//! kernel/src/net/tcp/bbr.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): BBR Congestion Control Engine
//!
//! Implements BBR (Bottleneck Bandwidth and RTT) Congestion Control:
//!   - Dual constraint model: BtlBw (max bottleneck bandwidth) & RTprop (min round-trip time)
//!   - 4-phase state machine: Startup -> Drain -> ProbeBW -> ProbeRTT
//!   - 8-phase ProbeBW pacing cycle: [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]
//!   - Pacing rate & dynamic Bandwidth-Delay Product (BDP) target window
//!   - Loss-tolerant: immune to shallow buffer drops and random packet losses

#![allow(dead_code)]

/// BBR State Machine
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BbrMode {
    Startup,
    Drain,
    ProbeBW,
    ProbeRTT,
}

#[derive(Clone, Debug)]
pub struct BbrCongestionControl {
    pub mode: BbrMode,
    pub btl_bw_bytes_per_sec: u64,
    pub min_rtt_ms: u32,
    pub min_rtt_stamp_ms: u32,
    pub pacing_gain_scaled: u32, // scaled by 256 (256 = 1.0)
    pub cwnd_gain_scaled: u32,   // scaled by 256 (256 = 1.0)
    pub cwnd: u32,
    pub pacing_rate_bps: u64,
    pub mss: u32,

    // ProbeBW cycling
    pub cycle_idx: usize,
    pub cycle_stamp_ms: u32,

    // Startup tracking
    pub prev_bw: u64,
    pub rounds_without_growth: u8,

    // ProbeRTT tracking
    pub probe_rtt_done_ms: u32,
    pub probe_rtt_round_done: bool,
}

// ProbeBW pacing gain cycle (scaled by 256):
// [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]
const PROBE_BW_GAINS: [u32; 8] = [320, 192, 256, 256, 256, 256, 256, 256];

impl BbrCongestionControl {
    pub const fn new(mss: u32) -> Self {
        let initial_cwnd = mss * 10;
        Self {
            mode: BbrMode::Startup,
            btl_bw_bytes_per_sec: 1_000_000, // Initial estimate: 1 MB/s
            min_rtt_ms: 50,
            min_rtt_stamp_ms: 0,
            pacing_gain_scaled: 740, // 2.885 * 256
            cwnd_gain_scaled: 740,   // 2.885 * 256
            cwnd: initial_cwnd,
            pacing_rate_bps: 2_885_000,
            mss,
            cycle_idx: 0,
            cycle_stamp_ms: 0,
            prev_bw: 0,
            rounds_without_growth: 0,
            probe_rtt_done_ms: 0,
            probe_rtt_round_done: false,
        }
    }

    /// Process incoming ACK delivery sample.
    pub fn on_ack(&mut self, acked_bytes: u32, sample_rtt_ms: u32, in_flight: u32, now_ms: u32) {
        // 1. Update Min RTT (RTprop filter over 10s window)
        let sample_rtt = sample_rtt_ms.max(1);
        if sample_rtt < self.min_rtt_ms || now_ms.saturating_sub(self.min_rtt_stamp_ms) > 10_000 {
            self.min_rtt_ms = sample_rtt;
            self.min_rtt_stamp_ms = now_ms;
        }

        // 2. Update Delivery Rate (BtlBw filter)
        // Rate = acked_bytes / sample_rtt_ms in bytes per millisecond -> * 1000 for bytes/sec
        let sample_rate = (acked_bytes as u64 * 1000) / sample_rtt as u64;
        if sample_rate > self.btl_bw_bytes_per_sec {
            self.btl_bw_bytes_per_sec = sample_rate;
        }

        // 3. State machine transitions
        match self.mode {
            BbrMode::Startup => {
                // Check if bandwidth plateaued (< 25% growth over last check)
                if self.prev_bw > 0 {
                    let threshold = self.prev_bw + (self.prev_bw / 4); // 1.25x
                    if self.btl_bw_bytes_per_sec < threshold {
                        self.rounds_without_growth += 1;
                    } else {
                        self.rounds_without_growth = 0;
                    }
                }
                self.prev_bw = self.btl_bw_bytes_per_sec;

                // If 3 rounds without substantial growth: pipe full detected, enter Drain
                if self.rounds_without_growth >= 3 {
                    self.mode = BbrMode::Drain;
                    self.pacing_gain_scaled = 89;  // 1 / 2.885 = 0.35 * 256
                    self.cwnd_gain_scaled = 740;  // 2.885
                }
            }
            BbrMode::Drain => {
                // Drain until in-flight data drops to target BDP
                let bdp = self.calculate_bdp();
                if (in_flight as u64) <= bdp {
                    self.mode = BbrMode::ProbeBW;
                    self.cycle_idx = 0;
                    self.cycle_stamp_ms = now_ms;
                    self.pacing_gain_scaled = PROBE_BW_GAINS[0];
                    self.cwnd_gain_scaled = 512; // 2.0 * 256
                }
            }
            BbrMode::ProbeBW => {
                // Advance 8-phase cycle once per RTT
                if now_ms.saturating_sub(self.cycle_stamp_ms) >= self.min_rtt_ms {
                    self.cycle_idx = (self.cycle_idx + 1) % 8;
                    self.cycle_stamp_ms = now_ms;
                    self.pacing_gain_scaled = PROBE_BW_GAINS[self.cycle_idx];
                }

                // Check if 10 seconds elapsed without ProbeRTT
                if now_ms.saturating_sub(self.min_rtt_stamp_ms) > 10_000 {
                    self.mode = BbrMode::ProbeRTT;
                    self.pacing_gain_scaled = 256; // 1.0
                    self.cwnd_gain_scaled = 256;   // 1.0
                    self.probe_rtt_done_ms = now_ms + 200; // 200ms probe duration
                }
            }
            BbrMode::ProbeRTT => {
                if now_ms >= self.probe_rtt_done_ms {
                    // Return to ProbeBW
                    self.min_rtt_stamp_ms = now_ms;
                    self.mode = BbrMode::ProbeBW;
                    self.cycle_idx = 0;
                    self.cycle_stamp_ms = now_ms;
                    self.pacing_gain_scaled = PROBE_BW_GAINS[0];
                    self.cwnd_gain_scaled = 512;
                }
            }
        }

        // 4. Update pacing rate and CWND
        self.update_pacing_and_cwnd();
    }

    /// Calculate estimated Bandwidth-Delay Product (BDP) in bytes.
    pub fn calculate_bdp(&self) -> u64 {
        // BDP = BtlBw (bytes/sec) * min_rtt (ms) / 1000
        (self.btl_bw_bytes_per_sec * self.min_rtt_ms as u64) / 1000
    }

    /// Update pacing rate and target congestion window based on current gains.
    fn update_pacing_and_cwnd(&mut self) {
        let bdp = self.calculate_bdp();

        // Pacing rate = BtlBw * pacing_gain
        self.pacing_rate_bps = (self.btl_bw_bytes_per_sec * self.pacing_gain_scaled as u64) / 256;

        // Target CWND
        if self.mode == BbrMode::ProbeRTT {
            // ProbeRTT restricts CWND to 4 packets
            self.cwnd = self.mss * 4;
        } else {
            let target = (bdp * self.cwnd_gain_scaled as u64) / 256;
            self.cwnd = (target as u32).max(self.mss * 4);
        }
    }

    /// Called on packet loss: BBR adjusts pacing without slashing CWND blindly.
    pub fn on_loss(&mut self, in_flight: u32) {
        // Under severe loss, prevent in-flight exceeding 1.25x BDP
        let bdp = self.calculate_bdp() as u32;
        if in_flight > bdp {
            self.cwnd = bdp.max(self.mss * 4);
        }
    }
}
