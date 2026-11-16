//! kernel/src/net/firewall/ratelimit.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Rate-Limiting & Port-Scan Defense
//!
//! - Token-Bucket rate limiter for ICMP and SYN-flood mitigation
//! - Multi-port reconnaissance detector flagging and banning stealth scanners (SYN/XMAS/NULL scans)

pub const MAX_SCANNER_TRACKED: usize = 64;
pub const MAX_DISTINCT_PORTS: usize = 16;
pub const PORT_SCAN_THRESHOLD: usize = 8;
pub const SCAN_WINDOW_TICKS: u32 = 100; // 1 second at 100Hz
pub const BAN_DURATION_TICKS: u32 = 3000; // 30 seconds

#[derive(Copy, Clone, Debug)]
pub struct TokenBucket {
    pub rate_per_sec: u32,
    pub burst_capacity: u32,
    pub tokens: u32,
    pub last_tick: u32,
}

impl TokenBucket {
    pub const fn new(rate_per_sec: u32, burst_capacity: u32) -> Self {
        TokenBucket {
            rate_per_sec,
            burst_capacity,
            tokens: burst_capacity,
            last_tick: 0,
        }
    }

    /// Attempts to consume 1 token. Returns true if permitted, false if rate limit exceeded.
    pub fn try_consume(&mut self, now_ticks: u32) -> bool {
        if self.last_tick == 0 {
            self.last_tick = now_ticks;
        }

        let elapsed = now_ticks.wrapping_sub(self.last_tick);
        if elapsed > 0 {
            // Replenish tokens: (elapsed * rate_per_sec) / 100 ticks per second
            let added = (elapsed.wrapping_mul(self.rate_per_sec)) / 100;
            if added > 0 {
                self.tokens = (self.tokens + added).min(self.burst_capacity);
                self.last_tick = now_ticks;
            }
        }

        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ScannerEntry {
    pub src_ip: u32,
    pub window_start_tick: u32,
    pub distinct_ports: [u16; MAX_DISTINCT_PORTS],
    pub port_count: usize,
    pub is_banned: bool,
    pub ban_expires_tick: u32,
    pub probes_dropped: u64,
    pub is_active: bool,
}

impl ScannerEntry {
    pub const fn empty() -> Self {
        ScannerEntry {
            src_ip: 0,
            window_start_tick: 0,
            distinct_ports: [0; MAX_DISTINCT_PORTS],
            port_count: 0,
            is_banned: false,
            ban_expires_tick: 0,
            probes_dropped: 0,
            is_active: false,
        }
    }
}

pub struct PortScanDetector {
    pub entries: [ScannerEntry; MAX_SCANNER_TRACKED],
    pub total_scans_detected: u64,
    pub total_banned_drops: u64,
}

impl PortScanDetector {
    pub const fn new() -> Self {
        PortScanDetector {
            entries: [ScannerEntry::empty(); MAX_SCANNER_TRACKED],
            total_scans_detected: 0,
            total_banned_drops: 0,
        }
    }

    /// Check if a source IP is actively banned. Drops packet if banned.
    pub fn check_banned(&mut self, src_ip: u32, now_ticks: u32) -> bool {
        for entry in self.entries.iter_mut() {
            if entry.is_active && entry.src_ip == src_ip {
                if entry.is_banned {
                    if now_ticks < entry.ban_expires_tick {
                        entry.probes_dropped += 1;
                        self.total_banned_drops += 1;
                        return true;
                    } else {
                        // Ban expired
                        entry.is_banned = false;
                        entry.port_count = 0;
                        entry.window_start_tick = now_ticks;
                    }
                }
                break;
            }
        }
        false
    }

    /// Records a probe to a destination port.
    /// If distinct probed ports exceed threshold in scan window, bans IP and returns true.
    pub fn record_probe(&mut self, src_ip: u32, dport: u16, now_ticks: u32) -> bool {
        let mut target_idx = None;
        let mut oldest_idx = 0;
        let mut oldest_tick = u32::MAX;

        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.is_active && entry.src_ip == src_ip {
                target_idx = Some(i);
                break;
            }
            if !entry.is_active && target_idx.is_none() {
                target_idx = Some(i);
            }
            if entry.window_start_tick < oldest_tick {
                oldest_tick = entry.window_start_tick;
                oldest_idx = i;
            }
        }

        let idx = target_idx.unwrap_or(oldest_idx);
        let entry = &mut self.entries[idx];

        if !entry.is_active || entry.src_ip != src_ip {
            // New tracker for this IP
            entry.src_ip = src_ip;
            entry.window_start_tick = now_ticks;
            entry.port_count = 1;
            entry.distinct_ports[0] = dport;
            entry.is_banned = false;
            entry.ban_expires_tick = 0;
            entry.probes_dropped = 0;
            entry.is_active = true;
            return false;
        }

        if entry.is_banned {
            if now_ticks < entry.ban_expires_tick {
                entry.probes_dropped += 1;
                self.total_banned_drops += 1;
                return true;
            } else {
                entry.is_banned = false;
                entry.port_count = 0;
                entry.window_start_tick = now_ticks;
            }
        }

        // Check if window has elapsed
        if now_ticks.wrapping_sub(entry.window_start_tick) > SCAN_WINDOW_TICKS {
            entry.window_start_tick = now_ticks;
            entry.port_count = 1;
            entry.distinct_ports[0] = dport;
            return false;
        }

        // Check if port already probed
        let mut already_seen = false;
        for &p in &entry.distinct_ports[..entry.port_count] {
            if p == dport {
                already_seen = true;
                break;
            }
        }

        if !already_seen && entry.port_count < MAX_DISTINCT_PORTS {
            entry.distinct_ports[entry.port_count] = dport;
            entry.port_count += 1;

            if entry.port_count >= PORT_SCAN_THRESHOLD {
                // Port scan detected! Trigger ban.
                entry.is_banned = true;
                entry.ban_expires_tick = now_ticks.wrapping_add(BAN_DURATION_TICKS);
                self.total_scans_detected += 1;
                return true;
            }
        }

        false
    }
}
