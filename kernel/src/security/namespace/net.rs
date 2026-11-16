//! kernel/src/security/namespace/net.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Network Isolation Namespace
//!
//! Provides isolated network stack instances:
//!   - Dedicated virtual loopback interface (lo 127.0.0.1/8) per network namespace.
//!   - Virtual Ethernet pairs (veth) connecting isolated container namespaces to host.
//!   - Isolated port namespaces (independent socket binding on ports 80, 443, etc.).
//!   - Independent IP routing tables and packet filtering chains.

#![allow(dead_code)]

pub const ROOT_NET_NS: u32 = 0;
pub const MAX_NET_NAMESPACES: usize = 16;
pub const MAX_INTERFACES_PER_NS: usize = 4;
pub const MAX_PORT_BINDINGS: usize = 16;
pub const MAX_ROUTES_PER_NS: usize = 4;

pub const IFF_UP: u32       = 0x0001;
pub const IFF_LOOPBACK: u32 = 0x0008;
pub const IFF_RUNNING: u32  = 0x0040;

pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

pub const EINVAL: isize = -22;
pub const ENOMEM: isize = -12;
pub const EEXIST: isize = -17;
pub const EADDRINUSE: isize = -98;
pub const ENODEV: isize = -19;
pub const ENOENT: isize = -2;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct NetDevice {
    pub name: [u8; 16],
    pub name_len: usize,
    pub ip_addr: [u8; 4],
    pub netmask: [u8; 4],
    pub mac: [u8; 6],
    pub flags: u32,
    pub peer_ns_id: Option<u32>,
    pub peer_dev_name: [u8; 16],
    pub peer_name_len: usize,
    pub active: bool,
}

impl NetDevice {
    pub const fn empty() -> Self {
        Self {
            name: [0u8; 16],
            name_len: 0,
            ip_addr: [0u8; 4],
            netmask: [0u8; 4],
            mac: [0u8; 6],
            flags: 0,
            peer_ns_id: None,
            peer_dev_name: [0u8; 16],
            peer_name_len: 0,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PortBinding {
    pub protocol: u8,
    pub port: u16,
    pub bound_pid: u32,
    pub active: bool,
}

impl PortBinding {
    pub const fn empty() -> Self {
        Self {
            protocol: 0,
            port: 0,
            bound_pid: 0,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RouteEntry {
    pub dest_ip: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
    pub dev_idx: usize,
    pub metric: u32,
    pub active: bool,
}

impl RouteEntry {
    pub const fn empty() -> Self {
        Self {
            dest_ip: [0u8; 4],
            netmask: [0u8; 4],
            gateway: [0u8; 4],
            dev_idx: 0,
            metric: 0,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct NetNamespace {
    pub id: u32,
    pub devices: [NetDevice; MAX_INTERFACES_PER_NS],
    pub device_count: usize,
    pub ports: [PortBinding; MAX_PORT_BINDINGS],
    pub port_count: usize,
    pub routes: [RouteEntry; MAX_ROUTES_PER_NS],
    pub route_count: usize,
    pub ref_count: u32,
    pub allocated: bool,
}

impl NetNamespace {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            devices: [NetDevice::empty(); MAX_INTERFACES_PER_NS],
            device_count: 0,
            ports: [PortBinding::empty(); MAX_PORT_BINDINGS],
            port_count: 0,
            routes: [RouteEntry::empty(); MAX_ROUTES_PER_NS],
            route_count: 0,
            ref_count: 0,
            allocated: false,
        }
    }
}

pub struct NetNamespaceManager {
    pub namespaces: [NetNamespace; MAX_NET_NAMESPACES],
    next_ns_id: u32,
}

impl NetNamespaceManager {
    pub const fn empty() -> Self {
        Self {
            namespaces: [NetNamespace::empty(); MAX_NET_NAMESPACES],
            next_ns_id: 1,
        }
    }

    pub fn new() -> Self {
        let mut mgr = Self::empty();
        mgr.init();
        mgr
    }

    pub fn init(&mut self) {
        *self = Self::empty();
        // Initialize Root Network Namespace
        let root = &mut self.namespaces[0];
        root.id = ROOT_NET_NS;
        root.ref_count = 1;
        root.allocated = true;

        // Loopback lo in root
        let lo = &mut root.devices[0];
        lo.name[..2].copy_from_slice(b"lo");
        lo.name_len = 2;
        lo.ip_addr = [127, 0, 0, 1];
        lo.netmask = [255, 0, 0, 0];
        lo.flags = IFF_UP | IFF_LOOPBACK | IFF_RUNNING;
        lo.active = true;
        root.device_count = 1;

        // Default root route
        let r = &mut root.routes[0];
        r.dest_ip = [127, 0, 0, 0];
        r.netmask = [255, 0, 0, 0];
        r.gateway = [0, 0, 0, 0];
        r.dev_idx = 0;
        r.metric = 0;
        r.active = true;
        root.route_count = 1;
    }

    /// Create a new, isolated network namespace with its own loopback device
    pub fn create_namespace(&mut self) -> Result<u32, isize> {
        let mut free_idx = None;
        for i in 1..MAX_NET_NAMESPACES {
            if !self.namespaces[i].allocated {
                free_idx = Some(i);
                break;
            }
        }
        let idx = free_idx.ok_or(ENOMEM)?;

        let new_id = self.next_ns_id;
        self.next_ns_id = self.next_ns_id.wrapping_add(1);

        let mut new_ns = NetNamespace::empty();
        new_ns.id = new_id;
        new_ns.ref_count = 1;
        new_ns.allocated = true;

        // Automatically provision isolated virtual loopback interface
        let lo = &mut new_ns.devices[0];
        lo.name[..2].copy_from_slice(b"lo");
        lo.name_len = 2;
        lo.ip_addr = [127, 0, 0, 1];
        lo.netmask = [255, 0, 0, 0];
        lo.flags = IFF_UP | IFF_LOOPBACK | IFF_RUNNING;
        lo.active = true;
        new_ns.device_count = 1;

        // Add loopback route
        let r = &mut new_ns.routes[0];
        r.dest_ip = [127, 0, 0, 0];
        r.netmask = [255, 0, 0, 0];
        r.gateway = [0, 0, 0, 0];
        r.dev_idx = 0;
        r.metric = 0;
        r.active = true;
        new_ns.route_count = 1;

        self.namespaces[idx] = new_ns;
        Ok(new_id)
    }

    /// Create a virtual Ethernet pair linking two network namespaces (e.g. host and container)
    pub fn create_veth_pair(
        &mut self,
        ns1_id: u32,
        dev1_name: &[u8],
        ip1: [u8; 4],
        mask1: [u8; 4],
        ns2_id: u32,
        dev2_name: &[u8],
        ip2: [u8; 4],
        mask2: [u8; 4],
    ) -> Result<(), isize> {
        if dev1_name.is_empty() || dev1_name.len() > 15 || dev2_name.is_empty() || dev2_name.len() > 15 {
            return Err(EINVAL);
        }

        let idx1 = self.find_ns_idx(ns1_id).ok_or(EINVAL)?;
        let idx2 = self.find_ns_idx(ns2_id).ok_or(EINVAL)?;

        if self.namespaces[idx1].device_count >= MAX_INTERFACES_PER_NS
            || self.namespaces[idx2].device_count >= MAX_INTERFACES_PER_NS
        {
            return Err(ENOMEM);
        }

        // Add dev1 into ns1
        let mut d1_slot = None;
        for (i, d) in self.namespaces[idx1].devices.iter().enumerate() {
            if !d.active {
                d1_slot = Some(i);
                break;
            }
        }
        let slot1 = d1_slot.ok_or(ENOMEM)?;

        // Add dev2 into ns2
        let mut d2_slot = None;
        for (i, d) in self.namespaces[idx2].devices.iter().enumerate() {
            if !d.active {
                d2_slot = Some(i);
                break;
            }
        }
        let slot2 = d2_slot.ok_or(ENOMEM)?;

        let d1 = &mut self.namespaces[idx1].devices[slot1];
        d1.name[..dev1_name.len()].copy_from_slice(dev1_name);
        d1.name_len = dev1_name.len();
        d1.ip_addr = ip1;
        d1.netmask = mask1;
        d1.mac = [0x02, 0x00, 0x00, 0x00, (ns1_id & 0xFF) as u8, slot1 as u8];
        d1.flags = IFF_UP | IFF_RUNNING;
        d1.peer_ns_id = Some(ns2_id);
        d1.peer_dev_name[..dev2_name.len()].copy_from_slice(dev2_name);
        d1.peer_name_len = dev2_name.len();
        d1.active = true;
        self.namespaces[idx1].device_count += 1;

        let d2 = &mut self.namespaces[idx2].devices[slot2];
        d2.name[..dev2_name.len()].copy_from_slice(dev2_name);
        d2.name_len = dev2_name.len();
        d2.ip_addr = ip2;
        d2.netmask = mask2;
        d2.mac = [0x02, 0x00, 0x00, 0x00, (ns2_id & 0xFF) as u8, slot2 as u8];
        d2.flags = IFF_UP | IFF_RUNNING;
        d2.peer_ns_id = Some(ns1_id);
        d2.peer_dev_name[..dev1_name.len()].copy_from_slice(dev1_name);
        d2.peer_name_len = dev1_name.len();
        d2.active = true;
        self.namespaces[idx2].device_count += 1;

        Ok(())
    }

    /// Bind a TCP/UDP port in this specific network namespace.
    /// Notice: Port 80 in NetNS 1 does NOT conflict with Port 80 in NetNS 2!
    pub fn bind_port(&mut self, ns_id: u32, proto: u8, port: u16, pid: u32) -> Result<(), isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        // Check for collision within THIS namespace only
        for p in ns.ports.iter() {
            if p.active && p.protocol == proto && p.port == port {
                return Err(EADDRINUSE);
            }
        }

        for p in ns.ports.iter_mut() {
            if !p.active {
                *p = PortBinding {
                    protocol: proto,
                    port,
                    bound_pid: pid,
                    active: true,
                };
                ns.port_count += 1;
                return Ok(());
            }
        }

        Err(ENOMEM)
    }

    /// Unbind port in specified namespace
    pub fn unbind_port(&mut self, ns_id: u32, proto: u8, port: u16, pid: u32) -> Result<(), isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        for p in ns.ports.iter_mut() {
            if p.active && p.protocol == proto && p.port == port && p.bound_pid == pid {
                *p = PortBinding::empty();
                ns.port_count = ns.port_count.saturating_sub(1);
                return Ok(());
            }
        }

        Err(ENOENT)
    }

    /// Check if a port is in use in a given namespace
    pub fn is_port_bound(&self, ns_id: u32, proto: u8, port: u16) -> bool {
        if let Some(idx) = self.find_ns_idx(ns_id) {
            for p in self.namespaces[idx].ports.iter() {
                if p.active && p.protocol == proto && p.port == port {
                    return true;
                }
            }
        }
        false
    }

    /// Add a route into the network namespace
    pub fn add_route(
        &mut self,
        ns_id: u32,
        dest_ip: [u8; 4],
        netmask: [u8; 4],
        gateway: [u8; 4],
        dev_idx: usize,
        metric: u32,
    ) -> Result<(), isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        if dev_idx >= ns.device_count {
            return Err(ENODEV);
        }

        for r in ns.routes.iter_mut() {
            if !r.active {
                *r = RouteEntry {
                    dest_ip,
                    netmask,
                    gateway,
                    dev_idx,
                    metric,
                    active: true,
                };
                ns.route_count += 1;
                return Ok(());
            }
        }

        Err(ENOMEM)
    }

    pub fn retain_namespace(&mut self, ns_id: u32) {
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_add(1);
        }
    }

    pub fn release_namespace(&mut self, ns_id: u32) -> bool {
        if ns_id == ROOT_NET_NS {
            return false;
        }
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_sub(1);
            if self.namespaces[idx].ref_count == 0 {
                self.namespaces[idx] = NetNamespace::empty();
                return true;
            }
        }
        false
    }

    pub fn find_ns_idx(&self, ns_id: u32) -> Option<usize> {
        for (i, ns) in self.namespaces.iter().enumerate() {
            if ns.allocated && ns.id == ns_id {
                return Some(i);
            }
        }
        None
    }
}
