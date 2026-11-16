//! userland/server/novactl/src/network.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Container Virtual Networking
//!
//! Configures isolated virtual networking for containers:
//!   - Bridge network (nova0 at 10.88.0.1/16).
//!   - IPAM (IP Address Management) pool for container IPs (10.88.0.2+).
//!   - Virtual Ethernet (veth) pair linking container eth0 to host bridge.
//!   - Dedicated loopback (lo) interface (127.0.0.1/8).
//!   - Port forwarding rules (e.g. 8080 -> 80).
//!   - Container /etc/resolv.conf and /etc/hosts generation.

#![allow(dead_code)]

use std::collections::HashSet;
use std::net::Ipv4Addr;

pub const DEFAULT_BRIDGE_NAME: &str = "nova0";
pub const DEFAULT_BRIDGE_IP: &str = "10.88.0.1";
pub const DEFAULT_SUBNET_MASK: &str = "255.255.0.0";

#[derive(Clone, Debug)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String, // "tcp" or "udp"
}

#[derive(Clone, Debug)]
pub struct ContainerNetworkConfig {
    pub container_id: String,
    pub hostname: String,
    pub ip_address: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub veth_host: String,
    pub veth_container: String,
    pub port_mappings: Vec<PortMapping>,
}

#[derive(Default)]
pub struct NetworkManager {
    allocated_ips: HashSet<Ipv4Addr>,
    next_ip_suffix: u8,
}

impl NetworkManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            allocated_ips: HashSet::new(),
            next_ip_suffix: 2, // 10.88.0.1 is gateway
        };
        // Reserve gateway
        mgr.allocated_ips.insert(Ipv4Addr::new(10, 88, 0, 1));
        mgr
    }

    /// Allocate a dedicated IPv4 address from the container pool.
    pub fn allocate_ip(&mut self) -> Ipv4Addr {
        loop {
            let ip = Ipv4Addr::new(10, 88, 0, self.next_ip_suffix);
            self.next_ip_suffix = self.next_ip_suffix.wrapping_add(1);
            if self.next_ip_suffix < 2 {
                self.next_ip_suffix = 2;
            }
            if !self.allocated_ips.contains(&ip) {
                self.allocated_ips.insert(ip);
                return ip;
            }
        }
    }

    /// Release an allocated IP address back to the pool.
    pub fn release_ip(&mut self, ip: Ipv4Addr) {
        self.allocated_ips.remove(&ip);
    }

    /// Mark an existing IP as allocated (used during state reload).
    pub fn mark_ip_allocated(&mut self, ip: Ipv4Addr) {
        self.allocated_ips.insert(ip);
    }

    /// Create network configuration for a new container.
    pub fn setup_container_network(
        &mut self,
        container_id: &str,
        hostname: &str,
        ports: &[(u16, u16)],
    ) -> ContainerNetworkConfig {
        let ip = self.allocate_ip();
        let short_id = if container_id.len() >= 7 { &container_id[..7] } else { container_id };
        let veth_host = format!("veth{}", short_id);
        let veth_container = "eth0".to_string();

        let mut port_mappings = Vec::new();
        for &(h, c) in ports {
            port_mappings.push(PortMapping {
                host_port: h,
                container_port: c,
                protocol: "tcp".to_string(),
            });
        }

        ContainerNetworkConfig {
            container_id: container_id.to_string(),
            hostname: hostname.to_string(),
            ip_address: ip,
            gateway: Ipv4Addr::new(10, 88, 0, 1),
            netmask: Ipv4Addr::new(255, 255, 0, 0),
            veth_host,
            veth_container,
            port_mappings,
        }
    }

    /// Generate container /etc/resolv.conf content.
    pub fn generate_resolv_conf(&self) -> String {
        "nameserver 10.88.0.1\nnameserver 1.1.1.1\nnameserver 8.8.8.8\nsearch novaos.internal\n".to_string()
    }

    /// Generate container /etc/hosts content.
    pub fn generate_hosts(&self, net_cfg: &ContainerNetworkConfig) -> String {
        format!(
            "127.0.0.1\tlocalhost\n::1\tlocalhost ip6-localhost ip6-loopback\n{}\t{}\n",
            net_cfg.ip_address, net_cfg.hostname
        )
    }
}
