//! kernel/src/net/tcp/window_scale.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): RFC 7323 TCP Window Scaling
//!
//! Extends TCP window size up to 1GB for high Bandwidth-Delay Product (BDP) links:
//!   - Option Kind 3: Window Scale (3 bytes: Kind=3, Length=3, Shift Count=S)
//!   - Negotiated strictly during SYN and SYN-ACK
//!   - Translates 16-bit TCP header window to/from 32-bit effective window
//!   - Shift counts 0..14 supported (RFC 7323 maximum shift factor is 14)

#![allow(dead_code)]

pub const TCP_OPT_WINDOW_SCALE: u8 = 3;
pub const MAX_WINDOW_SCALE_SHIFT: u8 = 14;
pub const DEFAULT_LOCAL_SCALE_SHIFT: u8 = 7; // Shift 7 = multiplier 128 (window up to 8.3MB)

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TcpWindowScale {
    pub enabled: bool,
    pub snd_scale: u8,
    pub rcv_scale: u8,
}

impl Default for TcpWindowScale {
    fn default() -> Self {
        Self::new()
    }
}

impl TcpWindowScale {
    pub const fn new() -> Self {
        Self {
            enabled: false,
            snd_scale: 0,
            rcv_scale: DEFAULT_LOCAL_SCALE_SHIFT,
        }
    }

    /// Enable window scaling upon successful two-way negotiation.
    pub fn negotiate(&mut self, peer_scale: Option<u8>) {
        if let Some(peer_shift) = peer_scale {
            self.enabled = true;
            self.snd_scale = peer_shift.min(MAX_WINDOW_SCALE_SHIFT);
            self.rcv_scale = DEFAULT_LOCAL_SCALE_SHIFT;
        } else {
            // Peer did not send Window Scale option: disable scaling per RFC 7323
            self.enabled = false;
            self.snd_scale = 0;
            self.rcv_scale = 0;
        }
    }

    /// Decode raw 16-bit window from incoming TCP header into effective 32-bit send window.
    pub fn decode_window(&self, header_window: u16) -> u32 {
        if self.enabled {
            (header_window as u32) << self.snd_scale
        } else {
            header_window as u32
        }
    }

    /// Encode real 32-bit receive window into 16-bit field for outgoing TCP header.
    pub fn encode_window(&self, real_window: u32) -> u16 {
        if self.enabled {
            let scaled = real_window >> self.rcv_scale;
            scaled.min(65535) as u16
        } else {
            real_window.min(65535) as u16
        }
    }

    /// Write Window Scale option (Kind 3, Length 3, Shift) into buffer. Returns bytes written (3).
    pub fn write_option(&self, out: &mut [u8]) -> usize {
        if out.len() < 3 {
            return 0;
        }
        out[0] = TCP_OPT_WINDOW_SCALE;
        out[1] = 3;
        out[2] = self.rcv_scale.min(MAX_WINDOW_SCALE_SHIFT);
        3
    }
}

/// Parse Window Scale option from raw TCP options payload.
pub fn parse_window_scale_option(payload: &[u8]) -> Option<u8> {
    let mut idx = 0;
    while idx < payload.len() {
        let kind = payload[idx];
        if kind == 0 {
            break;
        }
        if kind == 1 {
            idx += 1;
            continue;
        }
        if idx + 1 >= payload.len() {
            break;
        }
        let len = payload[idx + 1] as usize;
        if len < 2 || idx + len > payload.len() {
            break;
        }

        if kind == TCP_OPT_WINDOW_SCALE && len == 3 {
            return Some(payload[idx + 2].min(MAX_WINDOW_SCALE_SHIFT));
        }

        idx += len;
    }
    None
}
