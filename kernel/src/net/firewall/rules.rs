//! kernel/src/net/firewall/rules.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): nftables-inspired Declarative Rule Engine
//!
//! - Multi-table architecture: `ip filter` and `ip nat`
//! - Chain hooks: PREROUTING, INPUT, FORWARD, OUTPUT, POSTROUTING
//! - Granular matches: CIDR subnets, port ranges, conntrack states, interfaces, rate limits
//! - Actions: Accept, Drop, Reject, SNAT, DNAT, Masquerade
//! - Declarative configuration parser for `/etc/novafw.conf`

use super::conntrack::ConntrackState;
use super::ratelimit::TokenBucket;

pub const MAX_RULES_PER_CHAIN: usize = 32;
pub const MAX_CHAINS: usize = 8;

pub const STATE_MASK_NEW: u8 = 0x01;
pub const STATE_MASK_ESTABLISHED: u8 = 0x02;
pub const STATE_MASK_RELATED: u8 = 0x04;
pub const STATE_MASK_INVALID: u8 = 0x08;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ChainHook {
    Prerouting,
    Input,
    Forward,
    Output,
    Postrouting,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    Accept,
    Drop,
    Reject,
    Snat { ip: u32, port: u16 },
    Dnat { ip: u32, port: u16 },
    Masquerade,
}

#[derive(Copy, Clone, Debug)]
pub struct Rule {
    pub id: u32,
    pub proto: Option<u8>,
    pub src_net: u32,
    pub src_mask: u32,
    pub dst_net: u32,
    pub dst_mask: u32,
    pub sport_min: u16,
    pub sport_max: u16,
    pub dport_min: u16,
    pub dport_max: u16,
    pub state_mask: u8,
    pub iif: [u8; 16],
    pub oif: [u8; 16],
    pub token_bucket: Option<TokenBucket>,
    pub verdict: Verdict,
    pub packets: u64,
    pub bytes: u64,
    pub is_active: bool,
}

impl Rule {
    pub const fn empty() -> Self {
        Rule {
            id: 0,
            proto: None,
            src_net: 0,
            src_mask: 0,
            dst_net: 0,
            dst_mask: 0,
            sport_min: 0,
            sport_max: 0,
            dport_min: 0,
            dport_max: 0,
            state_mask: 0,
            iif: [0; 16],
            oif: [0; 16],
            token_bucket: None,
            verdict: Verdict::Accept,
            packets: 0,
            bytes: 0,
            is_active: false,
        }
    }

    /// Evaluates whether this rule matches the packet context.
    pub fn matches(
        &mut self,
        src_ip: u32,
        dst_ip: u32,
        sport: u16,
        dport: u16,
        proto: u8,
        state: ConntrackState,
        in_if: &[u8],
        out_if: &[u8],
        now_ticks: u32,
    ) -> bool {
        if !self.is_active {
            return false;
        }

        // Protocol match
        if let Some(p) = self.proto {
            if p != proto {
                return false;
            }
        }

        // CIDR subnet match
        if self.src_mask != 0 && (src_ip & self.src_mask) != (self.src_net & self.src_mask) {
            return false;
        }
        if self.dst_mask != 0 && (dst_ip & self.dst_mask) != (self.dst_net & self.dst_mask) {
            return false;
        }

        // Port matches
        if self.sport_min != 0 && (sport < self.sport_min || sport > self.sport_max) {
            return false;
        }
        if self.dport_min != 0 && (dport < self.dport_min || dport > self.dport_max) {
            return false;
        }

        // Conntrack state match
        if self.state_mask != 0 {
            let pkt_mask = match state {
                ConntrackState::New => STATE_MASK_NEW,
                ConntrackState::Established => STATE_MASK_ESTABLISHED,
                ConntrackState::Related => STATE_MASK_RELATED,
                ConntrackState::Invalid => STATE_MASK_INVALID,
            };
            if (self.state_mask & pkt_mask) == 0 {
                return false;
            }
        }

        // Interface matches
        if self.iif[0] != 0 && !Self::iface_match(&self.iif, in_if) {
            return false;
        }
        if self.oif[0] != 0 && !Self::iface_match(&self.oif, out_if) {
            return false;
        }

        // Rate limiting match
        if let Some(ref mut tb) = self.token_bucket {
            if !tb.try_consume(now_ticks) {
                return false;
            }
        }

        true
    }

    fn iface_match(expected: &[u8; 16], actual: &[u8]) -> bool {
        let mut exp_len = 0;
        while exp_len < 16 && expected[exp_len] != 0 {
            exp_len += 1;
        }
        if exp_len != actual.len() {
            return false;
        }
        &expected[..exp_len] == actual
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Chain {
    pub name: [u8; 16],
    pub hook: ChainHook,
    pub policy: Verdict,
    pub rules: [Rule; MAX_RULES_PER_CHAIN],
    pub rule_count: usize,
    pub is_active: bool,
}

impl Chain {
    pub const fn empty() -> Self {
        Chain {
            name: [0; 16],
            hook: ChainHook::Input,
            policy: Verdict::Accept,
            rules: [Rule::empty(); MAX_RULES_PER_CHAIN],
            rule_count: 0,
            is_active: false,
        }
    }

    pub fn add_rule(&mut self, rule: Rule) -> bool {
        if self.rule_count >= MAX_RULES_PER_CHAIN {
            return false;
        }
        self.rules[self.rule_count] = rule;
        self.rules[self.rule_count].is_active = true;
        self.rule_count += 1;
        true
    }

    /// Evaluates chain rules sequentially; returns winning verdict.
    pub fn evaluate(
        &mut self,
        src_ip: u32,
        dst_ip: u32,
        sport: u16,
        dport: u16,
        proto: u8,
        state: ConntrackState,
        in_if: &[u8],
        out_if: &[u8],
        pkt_len: u32,
        now_ticks: u32,
    ) -> Verdict {
        for rule in self.rules[..self.rule_count].iter_mut() {
            if rule.matches(src_ip, dst_ip, sport, dport, proto, state, in_if, out_if, now_ticks) {
                rule.packets += 1;
                rule.bytes += pkt_len as u64;
                return rule.verdict;
            }
        }
        self.policy
    }
}

pub struct RuleTable {
    pub chains: [Chain; MAX_CHAINS],
    pub chain_count: usize,
}

impl RuleTable {
    pub const fn new() -> Self {
        RuleTable {
            chains: [Chain::empty(); MAX_CHAINS],
            chain_count: 0,
        }
    }

    pub fn get_chain_mut(&mut self, hook: ChainHook) -> Option<&mut Chain> {
        for chain in self.chains[..self.chain_count].iter_mut() {
            if chain.is_active && chain.hook == hook {
                return Some(chain);
            }
        }
        None
    }

    pub fn create_chain(&mut self, name: &[u8], hook: ChainHook, policy: Verdict) -> Option<&mut Chain> {
        if self.chain_count >= MAX_CHAINS {
            return None;
        }
        let idx = self.chain_count;
        let chain = &mut self.chains[idx];
        chain.hook = hook;
        chain.policy = policy;
        chain.rule_count = 0;
        chain.is_active = true;

        let len = name.len().min(15);
        chain.name[..len].copy_from_slice(&name[..len]);
        chain.name[len] = 0;

        self.chain_count += 1;
        Some(chain)
    }

    pub fn flush(&mut self) {
        for chain in self.chains.iter_mut() {
            chain.is_active = false;
            chain.rule_count = 0;
        }
        self.chain_count = 0;
    }
}

/// Helper parsing IPv4 address string ("192.168.1.1") into u32.
pub fn parse_ipv4(s: &str) -> Option<u32> {
    let mut octets = [0u8; 4];
    let mut octet_idx = 0;
    let mut current_val = 0u32;
    let mut has_digits = false;

    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            current_val = current_val * 10 + (b - b'0') as u32;
            if current_val > 255 {
                return None;
            }
            has_digits = true;
        } else if b == b'.' {
            if !has_digits || octet_idx >= 3 {
                return None;
            }
            octets[octet_idx] = current_val as u8;
            octet_idx += 1;
            current_val = 0;
            has_digits = false;
        } else {
            return None;
        }
    }
    if !has_digits || octet_idx != 3 {
        return None;
    }
    octets[3] = current_val as u8;
    Some(u32::from_be_bytes(octets))
}

/// Helper parsing CIDR ("10.88.0.0/16") into (ip, mask).
pub fn parse_cidr(s: &str) -> Option<(u32, u32)> {
    if let Some(slash_pos) = s.find('/') {
        let ip_part = &s[..slash_pos];
        let prefix_part = &s[slash_pos + 1..];
        let ip = parse_ipv4(ip_part)?;
        let mut prefix = 0u32;
        for &b in prefix_part.as_bytes() {
            if b >= b'0' && b <= b'9' {
                prefix = prefix * 10 + (b - b'0') as u32;
            } else {
                return None;
            }
        }
        if prefix > 32 {
            return None;
        }
        let mask = if prefix == 0 {
            0
        } else {
            !0u32 << (32 - prefix)
        };
        Some((ip, mask))
    } else {
        let ip = parse_ipv4(s)?;
        Some((ip, 0xFFFFFFFF))
    }
}
