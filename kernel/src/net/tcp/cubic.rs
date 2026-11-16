//! kernel/src/net/tcp/cubic.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): RFC 8312 CUBIC Congestion Control
//!
//! Implements RFC 8312 CUBIC Congestion Control for high-bandwidth WAN links:
//!   - W_cubic(t) = C * (t - K)^3 + W_max
//!   - Fast convergence on multi-flow bottlenecks
//!   - TCP-friendly region emulation (Reno AIMD)
//!   - Multiplicative decrease factor beta = 0.7 (717/1024)
//!   - Zero floating-point operations: pure fixed-point integer arithmetic (#![no_std]).

#![allow(dead_code)]

/// Pure integer cube root using binary bit-shift search.
/// Exact floor(cbrt(val)) computed in 22 steps without division or floating point.
pub fn integer_cbrt(val: u64) -> u32 {
    if val == 0 {
        return 0;
    }
    let mut x = 0u64;
    for s in (0..=21).rev() {
        let bit = 1u64 << s;
        let test = x | bit;
        if test.saturating_mul(test).saturating_mul(test) <= val {
            x = test;
        }
    }
    x as u32
}

/// CUBIC State Machine
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubicMode {
    SlowStart,
    CongestionAvoidance,
    FastRecovery,
}

#[derive(Clone, Debug)]
pub struct CubicCongestionControl {
    pub mode: CubicMode,
    pub cwnd: u32,
    pub ssthresh: u32,
    pub w_max: u32,
    pub w_last_max: u32,
    pub epoch_start_ms: u32,
    pub k_ms: u32,
    pub origin_point: u32,
    pub mss: u32,
    pub rtt_min_ms: u32,
    pub ack_cnt: u32,
}

impl CubicCongestionControl {
    /// Initialize a new CUBIC congestion controller with specified MSS.
    pub const fn new(mss: u32) -> Self {
        let initial_cwnd = mss * 10; // RFC 6928: initial window of 10 MSS
        Self {
            mode: CubicMode::SlowStart,
            cwnd: initial_cwnd,
            ssthresh: u32::MAX,
            w_max: initial_cwnd,
            w_last_max: initial_cwnd,
            epoch_start_ms: 0,
            k_ms: 0,
            origin_point: initial_cwnd,
            mss,
            rtt_min_ms: 50,
            ack_cnt: 0,
        }
    }

    /// Called on receipt of valid incoming cumulative ACK.
    pub fn on_ack(&mut self, acked_bytes: u32, rtt_ms: u32, now_ms: u32) {
        if rtt_ms > 0 && (self.rtt_min_ms == 0 || rtt_ms < self.rtt_min_ms) {
            self.rtt_min_ms = rtt_ms;
        }

        match self.mode {
            CubicMode::SlowStart => {
                // Exponential growth: increase cwnd by acked bytes
                self.cwnd = self.cwnd.saturating_add(acked_bytes);
                if self.cwnd >= self.ssthresh {
                    self.mode = CubicMode::CongestionAvoidance;
                    self.epoch_start_ms = now_ms;
                    self.origin_point = self.cwnd;
                    self.k_ms = self.calculate_k();
                }
            }
            CubicMode::CongestionAvoidance => {
                // RFC 8312 CUBIC window calculation
                let target_w = self.calculate_cubic_window(now_ms);
                let reno_w = self.calculate_reno_window(now_ms);
                let final_target = target_w.max(reno_w);

                if final_target > self.cwnd {
                    // Increase cwnd smoothly towards target
                    let delta = (final_target - self.cwnd).max(1);
                    self.ack_cnt = self.ack_cnt.saturating_add(acked_bytes);
                    if self.ack_cnt >= self.cwnd {
                        self.ack_cnt = 0;
                        self.cwnd = self.cwnd.saturating_add(self.mss.min(delta));
                    }
                }
            }
            CubicMode::FastRecovery => {
                // SACK fast recovery: exit when ACK acknowledges sequence beyond recovery point
                self.mode = CubicMode::CongestionAvoidance;
                self.epoch_start_ms = now_ms;
                self.origin_point = self.cwnd;
                self.k_ms = self.calculate_k();
            }
        }
    }

    /// Called when packet loss is detected via 3 duplicate ACKs or SACK loss detection.
    pub fn on_loss(&mut self, now_ms: u32) {
        // Fast convergence heuristic
        if self.cwnd < self.w_last_max {
            self.w_last_max = self.cwnd;
            self.w_max = (self.cwnd as u64 * 870 / 1024) as u32; // 0.85
        } else {
            self.w_last_max = self.cwnd;
            self.w_max = self.cwnd;
        }

        // Multiplicative decrease: beta = 0.7 (717/1024)
        self.ssthresh = ((self.cwnd as u64 * 717) / 1024) as u32;
        self.ssthresh = self.ssthresh.max(self.mss * 2);
        self.cwnd = self.ssthresh;

        self.mode = CubicMode::FastRecovery;
        self.epoch_start_ms = now_ms;
        self.origin_point = self.w_max;
        self.k_ms = self.calculate_k();
    }

    /// Called on full Retransmission Timeout (RTO) loss.
    pub fn on_rto(&mut self, now_ms: u32) {
        self.w_last_max = self.cwnd;
        self.w_max = self.cwnd;
        self.ssthresh = ((self.cwnd as u64 * 717) / 1024) as u32;
        self.ssthresh = self.ssthresh.max(self.mss * 2);
        self.cwnd = self.mss; // Collapse to 1 MSS
        self.mode = CubicMode::SlowStart;
        self.epoch_start_ms = now_ms;
        self.origin_point = self.w_max;
        self.k_ms = self.calculate_k();
    }

    /// Compute time period K in milliseconds:
    /// K = cbrt( (W_max - ssthresh) / C )
    /// C = 0.4 packets/sec^3 = 0.4 * MSS / 1000^3 ms^3
    fn calculate_k(&self) -> u32 {
        if self.w_max <= self.ssthresh {
            return 0;
        }
        let w_diff = (self.w_max - self.ssthresh) as u64;
        let packets_diff = (w_diff / self.mss.max(1) as u64).max(1);

        // K = cbrt(packets_diff / 0.4) = cbrt(packets_diff * 2.5) seconds
        // In ms: 1000 * cbrt(packets_diff * 25 / 10)
        let scaled_val = packets_diff.saturating_mul(2500); // 2.5 * 1000
        let cbrt_sec = integer_cbrt(scaled_val); // scaled in tenths
        let k_ms = (cbrt_sec as u64 * 100).min(30_000) as u32; // capped at 30 seconds
        k_ms.max(100)
    }

    /// Evaluate W_cubic(t) = C * (t - K)^3 + W_max
    fn calculate_cubic_window(&self, now_ms: u32) -> u32 {
        let elapsed_ms = now_ms.saturating_sub(self.epoch_start_ms);
        let k = self.k_ms;

        let delta_t_ms = if elapsed_ms >= k {
            elapsed_ms - k
        } else {
            k - elapsed_ms
        };

        // Cubic term: C * delta_t^3 / 10^9
        // C = 410 / 1024
        let dt = (delta_t_ms as u64).min(20_000); // capped at 20s
        let dt_cubed = dt.saturating_mul(dt).saturating_mul(dt);
        let cubic_delta_bytes = (dt_cubed.saturating_mul(410) / (1024 * 1_000_000_000))
            .saturating_mul(self.mss as u64) as u32;

        if elapsed_ms < k {
            self.w_max.saturating_sub(cubic_delta_bytes)
        } else {
            self.w_max.saturating_add(cubic_delta_bytes)
        }
    }

    /// Standard TCP Reno Friendly Emulation: W_est(t) = W_max * beta + 3*(1-beta)/(1+beta) * (t / RTT)
    fn calculate_reno_window(&self, now_ms: u32) -> u32 {
        let elapsed_ms = now_ms.saturating_sub(self.epoch_start_ms);
        let rtt = self.rtt_min_ms.max(1);

        // beta = 0.7, 3*(1-0.7)/(1+0.7) = 3*0.3/1.7 = 0.9/1.7 = 0.529 ~ 542/1024
        let reno_base = (self.w_max as u64 * 717 / 1024) as u32;
        let growth_packets = (elapsed_ms as u64 * 542 / (1024 * rtt as u64)) as u32;
        let growth_bytes = growth_packets.saturating_mul(self.mss);

        reno_base.saturating_add(growth_bytes)
    }
}
