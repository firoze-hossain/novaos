//! kernel/src/net/wireguard/device.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Native In-Kernel WireGuard VPN
//!
//! Virtual WireGuard Network Device (wg0):
//! Manages UDP encapsulation, cryptokey routing, Noise handshake state,
//! transport session lifecycle, and automatic roaming.

use super::crypto::{public_key, Tai64n};
use super::noise::{
    consume_handshake_initiation, consume_handshake_response, create_handshake_initiation,
    create_handshake_response, MESSAGE_INITIATION_SIZE, MESSAGE_RESPONSE_SIZE,
    MESSAGE_TRANSPORT_HEADER_SIZE, TYPE_INITIATION, TYPE_RESPONSE, TYPE_TRANSPORT_DATA,
};
use super::peer::{WireGuardPeer, MAX_PEERS};
use super::routing::CryptokeyRoutingTable;

pub const DEFAULT_WIREGUARD_PORT: u16 = 51820;

/// WireGuard virtual interface device (wg0).
pub struct WireGuardDevice {
    pub name: [u8; 16],
    pub private_key: [u8; 32],
    pub public_key: [u8; 32],
    pub listen_port: u16,
    pub is_up: bool,
    pub next_session_index: u32,
    pub peers: [WireGuardPeer; MAX_PEERS],
    pub routing_table: CryptokeyRoutingTable,
}

impl WireGuardDevice {
    pub const fn new() -> Self {
        let mut name = [0u8; 16];
        name[0] = b'w';
        name[1] = b'g';
        name[2] = b'0';

        WireGuardDevice {
            name,
            private_key: [0; 32],
            public_key: [0; 32],
            listen_port: DEFAULT_WIREGUARD_PORT,
            is_up: false,
            next_session_index: 0x1000,
            peers: [WireGuardPeer::empty(); MAX_PEERS],
            routing_table: CryptokeyRoutingTable::new(),
        }
    }

    pub fn set_private_key(&mut self, privkey: &[u8; 32]) {
        self.private_key = *privkey;
        self.public_key = public_key(privkey);
        self.is_up = true;
    }

    pub fn alloc_session_index(&mut self) -> u32 {
        let idx = self.next_session_index;
        self.next_session_index = self.next_session_index.wrapping_add(1);
        if self.next_session_index == 0 {
            self.next_session_index = 0x1000;
        }
        idx
    }

    pub fn add_peer(
        &mut self,
        public_key: [u8; 32],
        endpoint_ip: u32,
        endpoint_port: u16,
    ) -> Option<usize> {
        // If peer already exists, update endpoint
        for (i, p) in self.peers.iter_mut().enumerate() {
            if p.is_configured && p.public_key == public_key {
                p.endpoint_ip = endpoint_ip;
                p.endpoint_port = endpoint_port;
                return Some(i);
            }
        }

        // Find empty slot
        for (i, p) in self.peers.iter_mut().enumerate() {
            if !p.is_configured {
                *p = WireGuardPeer::new(public_key, endpoint_ip, endpoint_port);
                return Some(i);
            }
        }

        None
    }

    pub fn add_peer_allowed_ip(&mut self, peer_idx: usize, network: u32, cidr: u8) -> bool {
        if peer_idx >= MAX_PEERS || !self.peers[peer_idx].is_configured {
            return false;
        }
        self.routing_table.add_route(network, cidr, peer_idx)
    }

    pub fn find_peer_by_pubkey(&self, pubkey: &[u8; 32]) -> Option<usize> {
        for (i, p) in self.peers.iter().enumerate() {
            if p.is_configured && &p.public_key == pubkey {
                return Some(i);
            }
        }
        None
    }

    /// Initiate handshake to a remote peer.
    /// Fills `out_packet` with 148-byte Message 1.
    pub fn initiate_handshake(
        &mut self,
        peer_idx: usize,
        ephemeral_priv: &[u8; 32],
        timestamp: &Tai64n,
        out_packet: &mut [u8; MESSAGE_INITIATION_SIZE],
    ) -> Result<(u32, u16), ()> {
        if peer_idx >= MAX_PEERS || !self.peers[peer_idx].is_configured {
            return Err(());
        }

        let local_index = self.alloc_session_index();
        let peer_pub = self.peers[peer_idx].public_key;

        let state = create_handshake_initiation(
            local_index,
            &self.private_key,
            &peer_pub,
            ephemeral_priv,
            timestamp,
            out_packet,
        );

        let peer = &mut self.peers[peer_idx];
        peer.handshake_initiation = Some(state);

        Ok((peer.endpoint_ip, peer.endpoint_port))
    }

    /// Egress Path: Encapsulate an IPv4 packet for transmission across WireGuard tunnel.
    /// Returns `(packet_len, endpoint_ip, endpoint_port)`.
    pub fn encapsulate_ip_packet(
        &mut self,
        ip_packet: &[u8],
        now_ticks: u64,
        out_udp_packet: &mut [u8],
    ) -> Result<(usize, u32, u16), ()> {
        if ip_packet.len() < 20 {
            return Err(());
        }

        // Extract destination IPv4 from packet header (offset 16..20)
        let mut dst_bytes = [0u8; 4];
        dst_bytes.copy_from_slice(&ip_packet[16..20]);
        let dst_ip = u32::from_be_bytes(dst_bytes);

        // Longest Prefix Match lookup in AllowedIPs
        let peer_idx = self.routing_table.lookup_peer(dst_ip).ok_or(())?;
        let peer = &mut self.peers[peer_idx];

        if !peer.is_configured {
            return Err(());
        }

        let session = peer.active_session.as_mut().ok_or(())?;
        let len = session.encrypt_transport(ip_packet, out_udp_packet)?;

        peer.record_tx(len as u64, now_ticks);
        Ok((len, peer.endpoint_ip, peer.endpoint_port))
    }

    /// Ingress Path: Handle an incoming UDP packet on the WireGuard port.
    /// Performs decryption, replay protection, roaming endpoint update, and reverse-path filtering.
    /// Returns `Ok(Some(plaintext_len))` if an inner IPv4 packet was successfully decapsulated.
    pub fn handle_incoming_udp(
        &mut self,
        src_ip: u32,
        src_port: u16,
        payload: &[u8],
        now_ticks: u64,
        resp_ephemeral_priv: &[u8; 32],
        out_response: &mut [u8; MESSAGE_RESPONSE_SIZE],
        out_plaintext: &mut [u8],
    ) -> Result<IncomingResult, ()> {
        if payload.is_empty() {
            return Err(());
        }

        match payload[0] {
            TYPE_INITIATION => {
                if payload.len() != MESSAGE_INITIATION_SIZE {
                    return Err(());
                }
                let mut msg = [0u8; MESSAGE_INITIATION_SIZE];
                msg.copy_from_slice(payload);

                let (initiator_static_pub, sender_idx, e_pub, ck, h, timestamp) =
                    consume_handshake_initiation(&self.private_key, &self.public_key, &msg)?;

                let peer_idx = self.find_peer_by_pubkey(&initiator_static_pub).ok_or(())?;
                let local_index = self.alloc_session_index();
                let peer = &mut self.peers[peer_idx];

                // Replay check on timestamp
                if !timestamp.is_after(&peer.last_handshake_timestamp) {
                    return Err(());
                }
                peer.last_handshake_timestamp = timestamp;

                // Roaming update
                peer.update_endpoint(src_ip, src_port);

                let session = create_handshake_response(
                    local_index,
                    sender_idx,
                    &initiator_static_pub,
                    &e_pub,
                    &peer.preshared_key,
                    resp_ephemeral_priv,
                    ck,
                    h,
                    now_ticks,
                    out_response,
                );

                peer.install_session(session, now_ticks);
                peer.record_rx(payload.len() as u64, now_ticks);
                peer.record_tx(MESSAGE_RESPONSE_SIZE as u64, now_ticks);

                Ok(IncomingResult::HandshakeResponse {
                    endpoint_ip: peer.endpoint_ip,
                    endpoint_port: peer.endpoint_port,
                    response_len: MESSAGE_RESPONSE_SIZE,
                })
            }

            TYPE_RESPONSE => {
                if payload.len() != MESSAGE_RESPONSE_SIZE {
                    return Err(());
                }
                let mut msg = [0u8; MESSAGE_RESPONSE_SIZE];
                msg.copy_from_slice(payload);

                let mut recv_idx_bytes = [0u8; 4];
                recv_idx_bytes.copy_from_slice(&msg[8..12]);
                let receiver_index = u32::from_le_bytes(recv_idx_bytes);

                // Find peer by matching handshake initiation local_index
                let mut matched_peer = None;
                for (i, p) in self.peers.iter().enumerate() {
                    if p.is_configured {
                        if let Some(init) = &p.handshake_initiation {
                            if init.local_index == receiver_index {
                                matched_peer = Some(i);
                                break;
                            }
                        }
                    }
                }

                let peer_idx = matched_peer.ok_or(())?;
                let peer = &mut self.peers[peer_idx];
                let init_state = peer.handshake_initiation.take().ok_or(())?;

                let session = consume_handshake_response(
                    &init_state,
                    &self.private_key,
                    &self.public_key,
                    &peer.preshared_key,
                    &msg,
                    now_ticks,
                )?;

                peer.update_endpoint(src_ip, src_port);
                peer.install_session(session, now_ticks);
                peer.record_rx(payload.len() as u64, now_ticks);

                Ok(IncomingResult::HandshakeEstablished { peer_index: peer_idx })
            }

            TYPE_TRANSPORT_DATA => {
                if payload.len() < MESSAGE_TRANSPORT_HEADER_SIZE + 16 {
                    return Err(());
                }

                let mut recv_idx_bytes = [0u8; 4];
                recv_idx_bytes.copy_from_slice(&payload[4..8]);
                let receiver_index = u32::from_le_bytes(recv_idx_bytes);

                // Find peer with matching session receiver_index
                let mut target_peer_idx = None;
                for (i, p) in self.peers.iter_mut().enumerate() {
                    if p.is_configured {
                        if let Some(active) = &p.active_session {
                            if active.local_index == receiver_index {
                                target_peer_idx = Some(i);
                                break;
                            }
                        }
                        if let Some(prev) = &p.previous_session {
                            if prev.local_index == receiver_index {
                                target_peer_idx = Some(i);
                                break;
                            }
                        }
                    }
                }

                let peer_idx = target_peer_idx.ok_or(())?;
                let peer = &mut self.peers[peer_idx];

                let pt_len = if let Some(active) = &mut peer.active_session {
                    if active.local_index == receiver_index {
                        active.decrypt_transport(payload, out_plaintext)?
                    } else if let Some(prev) = &mut peer.previous_session {
                        prev.decrypt_transport(payload, out_plaintext)?
                    } else {
                        return Err(());
                    }
                } else if let Some(prev) = &mut peer.previous_session {
                    prev.decrypt_transport(payload, out_plaintext)?
                } else {
                    return Err(());
                };

                // Cryptokey Routing Reverse-Path Filtering:
                // Verify inner IPv4 packet source IP matches AllowedIPs for this peer!
                if pt_len < 20 {
                    return Err(());
                }
                let mut inner_src_bytes = [0u8; 4];
                inner_src_bytes.copy_from_slice(&out_plaintext[12..16]);
                let inner_src_ip = u32::from_be_bytes(inner_src_bytes);

                if !self.routing_table.allow_source(peer_idx, inner_src_ip) {
                    // Security Violation: inner IP not in AllowedIPs! Drop packet!
                    return Err(());
                }

                // Roaming update upon authentic transport packet
                peer.update_endpoint(src_ip, src_port);
                peer.record_rx(payload.len() as u64, now_ticks);

                Ok(IncomingResult::PacketDecapsulated {
                    peer_index: peer_idx,
                    length: pt_len,
                })
            }

            _ => Err(()),
        }
    }
}

pub enum IncomingResult {
    HandshakeResponse {
        endpoint_ip: u32,
        endpoint_port: u16,
        response_len: usize,
    },
    HandshakeEstablished {
        peer_index: usize,
    },
    PacketDecapsulated {
        peer_index: usize,
        length: usize,
    },
}
