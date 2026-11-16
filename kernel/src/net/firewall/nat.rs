//! kernel/src/net/firewall/nat.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Network Address Translation (NAT) Subsystem
//!
//! High-speed packet header rewriting and RFC checksum recalculation:
//! - SNAT: Source IP and port translation for container subnets & egress interfaces
//! - DNAT: Destination IP and port translation for container port-forwarding
//! - Masquerade: Dynamic SNAT using active interface IP
//! - Checksum computation: IPv4 header checksum & TCP/UDP pseudo-header checksum

use super::conntrack::{FlowDirection, NatMapping, IP_PROTO_TCP, IP_PROTO_UDP};

pub struct NatEngine;

impl NatEngine {
    /// Computes Internet 16-bit one's complement sum of byte slice.
    pub fn checksum16(data: &[u8]) -> u32 {
        let mut sum = 0u32;
        let mut i = 0;
        while i + 1 < data.len() {
            let word = u16::from_be_bytes([data[i], data[i + 1]]) as u32;
            sum = sum.wrapping_add(word);
            i += 2;
        }
        if i < data.len() {
            let word = (data[i] as u32) << 8;
            sum = sum.wrapping_add(word);
        }
        sum
    }

    /// Folds 32-bit sum into 16-bit one's complement.
    pub fn fold_checksum(mut sum: u32) -> u16 {
        while (sum >> 16) != 0 {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        !(sum as u16)
    }

    /// Recomputes IPv4 header checksum in-place.
    pub fn recompute_ipv4_checksum(header: &mut [u8]) {
        if header.len() < 20 {
            return;
        }
        let ihl = ((header[0] & 0x0F) * 4) as usize;
        if ihl < 20 || ihl > header.len() {
            return;
        }

        // Clear existing checksum field (bytes 10..12)
        header[10] = 0;
        header[11] = 0;

        let sum = Self::checksum16(&header[..ihl]);
        let cksum = Self::fold_checksum(sum);
        header[10..12].copy_from_slice(&cksum.to_be_bytes());
    }

    /// Recomputes TCP/UDP checksum in-place given src_ip, dst_ip, protocol, and transport slice.
    pub fn recompute_transport_checksum(
        src_ip: u32,
        dst_ip: u32,
        protocol: u8,
        transport: &mut [u8],
    ) {
        if protocol != IP_PROTO_TCP && protocol != IP_PROTO_UDP {
            return;
        }

        let cksum_offset = if protocol == IP_PROTO_TCP {
            if transport.len() < 20 { return; }
            16
        } else {
            if transport.len() < 8 { return; }
            6
        };

        // Zero out checksum field prior to calculation
        transport[cksum_offset] = 0;
        transport[cksum_offset + 1] = 0;

        // Pseudo-header sum:
        // src_ip (4B) + dst_ip (4B) + zero (1B) + proto (1B) + transport_len (2B)
        let mut sum = 0u32;
        sum = sum.wrapping_add((src_ip >> 16) & 0xFFFF);
        sum = sum.wrapping_add(src_ip & 0xFFFF);
        sum = sum.wrapping_add((dst_ip >> 16) & 0xFFFF);
        sum = sum.wrapping_add(dst_ip & 0xFFFF);
        sum = sum.wrapping_add(protocol as u32);
        sum = sum.wrapping_add(transport.len() as u32);

        // Transport header + payload sum
        sum = sum.wrapping_add(Self::checksum16(transport));
        let mut cksum = Self::fold_checksum(sum);

        // UDP RFC 768: If computed checksum is zero, transmit 0xFFFF
        if protocol == IP_PROTO_UDP && cksum == 0 {
            cksum = 0xFFFF;
        }

        transport[cksum_offset..cksum_offset + 2].copy_from_slice(&cksum.to_be_bytes());
    }

    /// Rewrites an IP packet in-place according to active NAT mapping and direction.
    /// `ip_packet` includes IPv4 header and payload.
    pub fn rewrite_packet(
        ip_packet: &mut [u8],
        mapping: &NatMapping,
        dir: FlowDirection,
        orig_src: u32,
        orig_dst: u32,
        orig_sport: u16,
        orig_dport: u16,
    ) -> bool {
        if ip_packet.len() < 20 {
            return false;
        }

        let ihl = ((ip_packet[0] & 0x0F) * 4) as usize;
        if ihl < 20 || ihl > ip_packet.len() {
            return false;
        }

        let proto = ip_packet[9];
        let (ip_hdr, transport) = ip_packet.split_at_mut(ihl);

        match dir {
            FlowDirection::Original => {
                // Outbound or forward original flow: apply SNAT or DNAT
                if mapping.is_snat || mapping.is_masquerade {
                    // Rewrite source IP
                    ip_hdr[12..16].copy_from_slice(&mapping.snat_ip.to_be_bytes());
                    // Rewrite source port in TCP/UDP
                    if (proto == IP_PROTO_TCP || proto == IP_PROTO_UDP) && transport.len() >= 4 {
                        transport[0..2].copy_from_slice(&mapping.snat_port.to_be_bytes());
                    }
                }
                if mapping.is_dnat {
                    // Rewrite destination IP
                    ip_hdr[16..20].copy_from_slice(&mapping.dnat_ip.to_be_bytes());
                    // Rewrite destination port in TCP/UDP
                    if (proto == IP_PROTO_TCP || proto == IP_PROTO_UDP) && transport.len() >= 4 {
                        transport[2..4].copy_from_slice(&mapping.dnat_port.to_be_bytes());
                    }
                }
            }
            FlowDirection::Reply => {
                // Return path: reverse translation back to client
                if mapping.is_snat || mapping.is_masquerade {
                    // In reply, the destination was the translated SNAT IP; restore to original client IP
                    ip_hdr[16..20].copy_from_slice(&orig_src.to_be_bytes());
                    if (proto == IP_PROTO_TCP || proto == IP_PROTO_UDP) && transport.len() >= 4 {
                        transport[2..4].copy_from_slice(&orig_sport.to_be_bytes());
                    }
                }
                if mapping.is_dnat {
                    // In reply from container, source is container IP; restore to host public IP
                    ip_hdr[12..16].copy_from_slice(&orig_dst.to_be_bytes());
                    if (proto == IP_PROTO_TCP || proto == IP_PROTO_UDP) && transport.len() >= 4 {
                        transport[0..2].copy_from_slice(&orig_dport.to_be_bytes());
                    }
                }
            }
        }

        // Recompute IP checksum
        Self::recompute_ipv4_checksum(ip_hdr);

        // Recompute transport checksum
        let current_src = u32::from_be_bytes([ip_hdr[12], ip_hdr[13], ip_hdr[14], ip_hdr[15]]);
        let current_dst = u32::from_be_bytes([ip_hdr[16], ip_hdr[17], ip_hdr[18], ip_hdr[19]]);
        Self::recompute_transport_checksum(current_src, current_dst, proto, transport);

        true
    }
}
