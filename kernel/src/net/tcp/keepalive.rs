//! kernel/src/net/tcp/keepalive.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): RFC 1122 TCP Keepalive Timers
//!
//! Detects dead/hung peers on idle TCP connections:
//!   - tcp_keepalive_time: idle duration before first probe (default 7200s / 720000 ticks)
//!   - tcp_keepalive_intvl: probe retry interval (default 75s / 7500 ticks)
//!   - tcp_keepalive_probes: max unacknowledged probes before aborting (default 9)
//!   - Probe packet transmission: empty ACK segment with seq = snd_una - 1
//!   - Automatic teardown (ETIMEDOUT) on probe exhaustion

#![allow(dead_code)]

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeepaliveAction {
    None,
    SendProbe { probe_seq: u32 },
    Timeout,
}

#[derive(Clone, Debug)]
pub struct TcpKeepalive {
    pub enabled: bool,
    pub idle_ticks: u32,
    pub interval_ticks: u32,
    pub probes_max: u8,
    pub last_activity_ticks: u32,
    pub probes_sent: u8,
    pub next_probe_ticks: u32,
}

impl Default for TcpKeepalive {
    fn default() -> Self {
        Self::new()
    }
}

impl TcpKeepalive {
    pub const fn new() -> Self {
        Self {
            enabled: true,
            idle_ticks: 720_000,    // 7200s at 100Hz
            interval_ticks: 7_500,  // 75s at 100Hz
            probes_max: 9,
            last_activity_ticks: 0,
            probes_sent: 0,
            next_probe_ticks: 720_000,
        }
    }

    /// Reset keepalive timers on any data or ACK reception/transmission.
    pub fn on_activity(&mut self, now_ticks: u32) {
        self.last_activity_ticks = now_ticks;
        self.probes_sent = 0;
        self.next_probe_ticks = now_ticks.wrapping_add(self.idle_ticks);
    }

    /// Check keepalive status on timer tick.
    pub fn poll(&mut self, send_unacked: u32, now_ticks: u32) -> KeepaliveAction {
        if !self.enabled {
            return KeepaliveAction::None;
        }

        // Check if next probe deadline reached
        if (now_ticks.wrapping_sub(self.next_probe_ticks) as i32) >= 0 {
            if self.probes_sent >= self.probes_max {
                // Keepalive probe limit exceeded: connection dead
                return KeepaliveAction::Timeout;
            }

            self.probes_sent += 1;
            self.next_probe_ticks = now_ticks.wrapping_add(self.interval_ticks);

            // RFC 1122: probe carries seq = send_unacked - 1 to elicit immediate ACK
            let probe_seq = send_unacked.wrapping_sub(1);
            return KeepaliveAction::SendProbe { probe_seq };
        }

        KeepaliveAction::None
    }
}
