//! kernel/src/net/wireguard/routing.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Native In-Kernel WireGuard VPN
//!
//! Cryptokey Routing Subsystem:
//! Maps IP network CIDR ranges (AllowedIPs) to specific cryptographic peers.
//! Provides Longest Prefix Match (LPM) routing for egress and reverse-path source
//! address validation for ingress.

pub const MAX_ROUTING_ENTRIES: usize = 32;

/// An allowed IP range associated with a cryptographic peer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AllowedIp {
    pub network: u32,
    pub cidr: u8,
    pub mask: u32,
    pub peer_index: usize,
    pub active: bool,
}

impl AllowedIp {
    pub const fn empty() -> Self {
        AllowedIp {
            network: 0,
            cidr: 0,
            mask: 0,
            peer_index: 0,
            active: false,
        }
    }

    pub fn new(network: u32, cidr: u8, peer_index: usize) -> Self {
        let mask = if cidr == 0 {
            0
        } else if cidr >= 32 {
            !0u32
        } else {
            !0u32 << (32 - cidr)
        };

        AllowedIp {
            network: network & mask,
            cidr,
            mask,
            peer_index,
            active: true,
        }
    }

    #[inline(always)]
    pub fn matches(&self, ip: u32) -> bool {
        self.active && (ip & self.mask) == self.network
    }
}

/// Cryptokey routing table.
#[derive(Copy, Clone, Debug)]
pub struct CryptokeyRoutingTable {
    pub routes: [AllowedIp; MAX_ROUTING_ENTRIES],
    pub count: usize,
}

impl CryptokeyRoutingTable {
    pub const fn new() -> Self {
        CryptokeyRoutingTable {
            routes: [AllowedIp::empty(); MAX_ROUTING_ENTRIES],
            count: 0,
        }
    }

    /// Add an AllowedIP route for a peer.
    pub fn add_route(&mut self, network: u32, cidr: u8, peer_index: usize) -> bool {
        let entry = AllowedIp::new(network, cidr, peer_index);

        // Check if route already exists and update
        for r in self.routes.iter_mut() {
            if r.active && r.network == entry.network && r.cidr == entry.cidr {
                *r = entry;
                return true;
            }
        }

        // Find empty slot
        for r in self.routes.iter_mut() {
            if !r.active {
                *r = entry;
                self.count += 1;
                return true;
            }
        }

        false // Table full
    }

    /// Remove all routes associated with a given peer.
    pub fn remove_peer_routes(&mut self, peer_index: usize) {
        for r in self.routes.iter_mut() {
            if r.active && r.peer_index == peer_index {
                r.active = false;
                if self.count > 0 {
                    self.count -= 1;
                }
            }
        }
    }

    /// Egress routing lookup: Longest Prefix Match (LPM) on destination IP.
    /// Returns the matching peer index with the highest CIDR prefix length.
    pub fn lookup_peer(&self, dst_ip: u32) -> Option<usize> {
        let mut best_peer = None;
        let mut best_cidr = -1i32;

        for r in self.routes.iter() {
            if r.matches(dst_ip) && (r.cidr as i32) > best_cidr {
                best_cidr = r.cidr as i32;
                best_peer = Some(r.peer_index);
            }
        }

        best_peer
    }

    /// Ingress validation: check if the source IP of a decapsulated packet
    /// is permitted for the peer from which it arrived (Reverse-Path Filter).
    pub fn allow_source(&self, peer_index: usize, src_ip: u32) -> bool {
        for r in self.routes.iter() {
            if r.active && r.peer_index == peer_index && r.matches(src_ip) {
                return true;
            }
        }
        false
    }
}
