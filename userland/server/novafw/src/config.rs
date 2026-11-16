//! userland/server/novafw/src/config.rs
//!
//! Declarative nftables-inspired syntax parser for `/etc/novafw.conf`

pub const DEFAULT_CONFIG_PATH: &str = "/etc/novafw.conf";

#[derive(Clone, Debug, PartialEq)]
pub enum FwVerdict {
    Accept,
    Drop,
    Reject,
    Snat { ip: String, port: u16 },
    Dnat { ip: String, port: u16 },
    Masquerade,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedRule {
    pub proto: Option<String>,
    pub saddr: Option<String>,
    pub daddr: Option<String>,
    pub dports: Vec<u16>,
    pub sports: Vec<u16>,
    pub states: Vec<String>,
    pub iif: Option<String>,
    pub oif: Option<String>,
    pub rate_limit: Option<(u32, u32)>, // rate/sec, burst
    pub verdict: FwVerdict,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedChain {
    pub name: String,
    pub hook: String,
    pub policy: FwVerdict,
    pub rules: Vec<ParsedRule>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedTable {
    pub name: String,
    pub family: String,
    pub chains: Vec<ParsedChain>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FirewallConfig {
    pub tables: Vec<ParsedTable>,
}

impl FirewallConfig {
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut tables = Vec::new();
        let mut current_table: Option<ParsedTable> = None;
        let mut current_chain: Option<ParsedChain> = None;

        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if trimmed.starts_with("table ") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() < 3 {
                    return Err(format!("Invalid table definition: {}", trimmed));
                }
                let family = parts[1].to_string();
                let name = parts[2].trim_end_matches('{').trim().to_string();
                current_table = Some(ParsedTable {
                    name,
                    family,
                    chains: Vec::new(),
                });
                continue;
            }

            if trimmed.starts_with("chain ") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() < 2 {
                    return Err(format!("Invalid chain definition: {}", trimmed));
                }
                let name = parts[1].trim_end_matches('{').trim().to_string();
                current_chain = Some(ParsedChain {
                    name,
                    hook: "input".to_string(),
                    policy: FwVerdict::Accept,
                    rules: Vec::new(),
                });
                continue;
            }

            if trimmed == "}" {
                if let Some(chain) = current_chain.take() {
                    if let Some(ref mut table) = current_table {
                        table.chains.push(chain);
                    }
                } else if let Some(table) = current_table.take() {
                    tables.push(table);
                }
                continue;
            }

            // Chain properties or rules
            if let Some(ref mut chain) = current_chain {
                if trimmed.starts_with("type ") || trimmed.starts_with("policy ") {
                    // Chain configuration line
                    let items: Vec<&str> = trimmed.split(';').collect();
                    for item in items {
                        let item_trim = item.trim();
                        if item_trim.starts_with("policy ") {
                            let pol_str = item_trim.trim_start_matches("policy ").trim();
                            chain.policy = match pol_str {
                                "drop" => FwVerdict::Drop,
                                "reject" => FwVerdict::Reject,
                                _ => FwVerdict::Accept,
                            };
                        } else if item_trim.contains("hook ") {
                            let parts: Vec<&str> = item_trim.split_whitespace().collect();
                            for (idx, &part) in parts.iter().enumerate() {
                                if part == "hook" && idx + 1 < parts.len() {
                                    chain.hook = parts[idx + 1].to_string();
                                }
                            }
                        }
                    }
                    continue;
                }

                // Rule line
                let rule = Self::parse_rule(trimmed)?;
                chain.rules.push(rule);
            }
        }

        if let Some(chain) = current_chain {
            if let Some(ref mut table) = current_table {
                table.chains.push(chain);
            }
        }
        if let Some(table) = current_table {
            tables.push(table);
        }

        Ok(FirewallConfig { tables })
    }

    fn parse_rule(line: &str) -> Result<ParsedRule, String> {
        let mut proto = None;
        let mut saddr = None;
        let mut daddr = None;
        let mut dports = Vec::new();
        let mut sports = Vec::new();
        let mut states = Vec::new();
        let mut iif = None;
        let mut oif = None;
        let mut rate_limit = None;
        let mut verdict = FwVerdict::Accept;

        let tokens: Vec<&str> = line.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            let tok = tokens[i];
            match tok {
                "tcp" | "udp" | "icmp" => {
                    proto = Some(tok.to_string());
                    i += 1;
                }
                "ip" => {
                    if i + 2 < tokens.len() && tokens[i + 1] == "saddr" {
                        saddr = Some(tokens[i + 2].to_string());
                        i += 3;
                    } else if i + 2 < tokens.len() && tokens[i + 1] == "daddr" {
                        daddr = Some(tokens[i + 2].to_string());
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                "dport" => {
                    if i + 1 < tokens.len() {
                        let next = tokens[i + 1];
                        if next.starts_with('{') {
                            // Port set: e.g. { 80, 443 }
                            let mut j = i + 1;
                            while j < tokens.len() {
                                let clean = tokens[j].trim_matches(|c| c == '{' || c == '}' || c == ',');
                                if let Ok(p) = clean.parse::<u16>() {
                                    dports.push(p);
                                }
                                if tokens[j].ends_with('}') {
                                    break;
                                }
                                j += 1;
                            }
                            i = j + 1;
                        } else if let Ok(p) = next.parse::<u16>() {
                            dports.push(p);
                            i += 2;
                        } else {
                            i += 2;
                        }
                    } else {
                        i += 1;
                    }
                }
                "sport" => {
                    if i + 1 < tokens.len() {
                        if let Ok(p) = tokens[i + 1].parse::<u16>() {
                            sports.push(p);
                        }
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "ct" => {
                    if i + 2 < tokens.len() && tokens[i + 1] == "state" {
                        let state_str = tokens[i + 2];
                        for st in state_str.split(',') {
                            states.push(st.trim().to_string());
                        }
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                "iif" => {
                    if i + 1 < tokens.len() {
                        iif = Some(tokens[i + 1].trim_matches('"').to_string());
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "oif" => {
                    if i + 1 < tokens.len() {
                        oif = Some(tokens[i + 1].trim_matches('"').to_string());
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "limit" => {
                    // e.g. limit rate 5/second burst 10
                    let mut rate = 5u32;
                    let mut burst = 10u32;
                    if i + 2 < tokens.len() && tokens[i + 1] == "rate" {
                        let rate_tok = tokens[i + 2];
                        if let Some(slash) = rate_tok.find('/') {
                            if let Ok(r) = rate_tok[..slash].parse::<u32>() {
                                rate = r;
                            }
                        }
                    }
                    if i + 4 < tokens.len() && tokens[i + 3] == "burst" {
                        if let Ok(b) = tokens[i + 4].parse::<u32>() {
                            burst = b;
                        }
                    }
                    rate_limit = Some((rate, burst));
                    i += 5.min(tokens.len() - i);
                }
                "dnat" => {
                    // dnat to 10.88.0.2:80
                    if i + 2 < tokens.len() && tokens[i + 1] == "to" {
                        let target = tokens[i + 2];
                        if let Some(colon) = target.find(':') {
                            let ip = target[..colon].to_string();
                            let port = target[colon + 1..].parse::<u16>().unwrap_or(80);
                            verdict = FwVerdict::Dnat { ip, port };
                        }
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                "snat" => {
                    // snat to 10.0.2.15:10000
                    if i + 2 < tokens.len() && tokens[i + 1] == "to" {
                        let target = tokens[i + 2];
                        if let Some(colon) = target.find(':') {
                            let ip = target[..colon].to_string();
                            let port = target[colon + 1..].parse::<u16>().unwrap_or(0);
                            verdict = FwVerdict::Snat { ip, port };
                        }
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                "masquerade" => {
                    verdict = FwVerdict::Masquerade;
                    i += 1;
                }
                "accept" => {
                    verdict = FwVerdict::Accept;
                    i += 1;
                }
                "drop" => {
                    verdict = FwVerdict::Drop;
                    i += 1;
                }
                "reject" => {
                    verdict = FwVerdict::Reject;
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }

        Ok(ParsedRule {
            proto,
            saddr,
            daddr,
            dports,
            sports,
            states,
            iif,
            oif,
            rate_limit,
            verdict,
        })
    }
}
