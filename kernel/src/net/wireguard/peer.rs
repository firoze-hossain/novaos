//! kernel/src/net/wireguard/peer.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Native In-Kernel WireGuard VPN
//!
//! WireGuard Peer Representation, Automatic Roaming, Session Key Life Cycle,
//! and Traffic Accounting.

use super::crypto::Tai64n;
use super::noise::{HandshakeInitiationState, TransportSession};

pub const MAX_PEERS: usize = 8;

/// A remote WireGuard cryptographic peer.
#[derive(Copy, Clone, Debug)]
pub struct WireGuardPeer {
    pub public_key: [u8; 32],
    pub preshared_key: [u8; 32],
    pub endpoint_ip: u32,
    pub endpoint_port: u16,
    pub persistent_keepalive: u16,
    pub active_session: Option<TransportSession>,
    pub previous_session: Option<TransportSession>,
    pub handshake_initiation: Option<HandshakeInitiationState>,
    pub last_handshake_timestamp: Tai64n,
    pub last_handshake_ticks: u64,
    pub last_tx_ticks: u64,
    pub last_rx_ticks: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub is_configured: bool,
}

impl WireGuardPeer {
    pub const fn empty() -> Self {
        WireGuardPeer {
            public_key: [0; 32],
            preshared_key: [0; 32],
            endpoint_ip: 0,
            endpoint_port: 0,
            persistent_keepalive: 0,
            active_session: None,
            previous_session: None,
            handshake_initiation: None,
            last_handshake_timestamp: Tai64n::ZERO,
            last_handshake_ticks: 0,
            last_tx_ticks: 0,
            last_rx_ticks: 0,
            tx_bytes: 0,
            rx_bytes: 0,
            is_configured: false,
        }
    }

    pub fn new(public_key: [u8; 32], endpoint_ip: u32, endpoint_port: u16) -> Self {
        WireGuardPeer {
            public_key,
            preshared_key: [0; 32],
            endpoint_ip,
            endpoint_port,
            persistent_keepalive: 0,
            active_session: None,
            previous_session: None,
            handshake_initiation: None,
            last_handshake_timestamp: Tai64n::ZERO,
            last_handshake_ticks: 0,
            last_tx_ticks: 0,
            last_rx_ticks: 0,
            tx_bytes: 0,
            rx_bytes: 0,
            is_configured: true,
        }
    }

    /// Automatic Roaming: Update the peer's external IP:port endpoint
    /// upon receiving an authenticated packet from an updated network path.
    pub fn update_endpoint(&mut self, src_ip: u32, src_port: u16) {
        if self.endpoint_ip != src_ip || self.endpoint_port != src_port {
            self.endpoint_ip = src_ip;
            self.endpoint_port = src_port;
        }
    }

    /// Rotate sessions when a new handshake completes successfully.
    pub fn install_session(&mut self, session: TransportSession, now_ticks: u64) {
        self.previous_session = self.active_session;
        self.active_session = Some(session);
        self.handshake_initiation = None;
        self.last_handshake_ticks = now_ticks;
    }

    pub fn record_tx(&mut self, bytes: u64, now_ticks: u64) {
        self.tx_bytes = self.tx_bytes.wrapping_add(bytes);
        self.last_tx_ticks = now_ticks;
    }

    pub fn record_rx(&mut self, bytes: u64, now_ticks: u64) {
        self.rx_bytes = self.rx_bytes.wrapping_add(bytes);
        self.last_rx_ticks = now_ticks;
    }
}
