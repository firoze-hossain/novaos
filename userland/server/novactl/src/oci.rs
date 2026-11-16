//! userland/server/novactl/src/oci.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): OCI Runtime Spec v1.0.2 Parser
//!
//! Pure Rust parser for OCI (Open Container Initiative) container bundles:
//!   - config.json schema: process, root, mounts, namespaces, linux resources.
//!   - Generates default compliant specifications.
//!   - Zero external dependencies.

#![allow(dead_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OciProcess {
    pub terminal: bool,
    pub user_uid: u32,
    pub user_gid: u32,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub cwd: String,
}

impl Default for OciProcess {
    fn default() -> Self {
        Self {
            terminal: false,
            user_uid: 0,
            user_gid: 0,
            args: vec!["/bin/sh".to_string()],
            env: vec![
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
                "TERM=xterm".to_string(),
                "HOME=/root".to_string(),
            ],
            cwd: "/".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OciRoot {
    pub path: String,
    pub readonly: bool,
}

impl Default for OciRoot {
    fn default() -> Self {
        Self {
            path: "rootfs".to_string(),
            readonly: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OciMount {
    pub destination: String,
    pub source: String,
    pub mount_type: String,
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OciLinuxResources {
    pub memory_max_bytes: Option<u64>,
    pub memory_high_bytes: Option<u64>,
    pub cpu_quota_usec: Option<u64>,
    pub cpu_period_usec: u64,
    pub cpu_weight: u32,
    pub pids_max: Option<u64>,
    pub io_max: Option<String>,
}

impl Default for OciLinuxResources {
    fn default() -> Self {
        Self {
            memory_max_bytes: None,
            memory_high_bytes: None,
            cpu_quota_usec: None,
            cpu_period_usec: 100_000,
            cpu_weight: 100,
            pids_max: None,
            io_max: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OciSpec {
    pub oci_version: String,
    pub process: OciProcess,
    pub root: OciRoot,
    pub hostname: String,
    pub mounts: Vec<OciMount>,
    pub linux_resources: OciLinuxResources,
}

impl Default for OciSpec {
    fn default() -> Self {
        Self {
            oci_version: "1.0.2".to_string(),
            process: OciProcess::default(),
            root: OciRoot::default(),
            hostname: "novaos-container".to_string(),
            mounts: vec![
                OciMount {
                    destination: "/proc".to_string(),
                    source: "proc".to_string(),
                    mount_type: "procfs".to_string(),
                    options: vec!["nosuid".to_string(), "noexec".to_string(), "nodev".to_string()],
                },
                OciMount {
                    destination: "/dev".to_string(),
                    source: "devfs".to_string(),
                    mount_type: "devfs".to_string(),
                    options: vec!["nosuid".to_string(), "strictatime".to_string(), "mode=755".to_string()],
                },
                OciMount {
                    destination: "/sys".to_string(),
                    source: "sysfs".to_string(),
                    mount_type: "sysfs".to_string(),
                    options: vec!["nosuid".to_string(), "noexec".to_string(), "nodev".to_string(), "ro".to_string()],
                },
                OciMount {
                    destination: "/tmp".to_string(),
                    source: "tmpfs".to_string(),
                    mount_type: "tmpfs".to_string(),
                    options: vec!["nosuid".to_string(), "nodev".to_string(), "mode=1777".to_string()],
                },
            ],
            linux_resources: OciLinuxResources::default(),
        }
    }
}

impl OciSpec {
    /// Parse an OCI config.json string into an OciSpec struct.
    pub fn parse_json(json_str: &str) -> Result<Self, String> {
        let mut spec = OciSpec::default();

        // Extract hostname
        if let Some(host) = extract_string_field(json_str, "hostname") {
            spec.hostname = host;
        }

        // Extract ociVersion
        if let Some(ver) = extract_string_field(json_str, "ociVersion") {
            spec.oci_version = ver;
        }

        // Extract process args
        if let Some(args) = extract_string_array(json_str, "args") {
            if !args.is_empty() {
                spec.process.args = args;
            }
        }

        // Extract process env
        if let Some(env) = extract_string_array(json_str, "env") {
            if !env.is_empty() {
                spec.process.env = env;
            }
        }

        // Extract process cwd
        if let Some(cwd) = extract_string_field(json_str, "cwd") {
            spec.process.cwd = cwd;
        }

        // Extract memory limit
        if let Some(mem_max) = extract_nested_number_field(json_str, "memory", "limit") {
            spec.linux_resources.memory_max_bytes = Some(mem_max);
        } else if let Some(mem_max) = extract_number_field(json_str, "limit") {
            spec.linux_resources.memory_max_bytes = Some(mem_max);
        }

        // Extract cpu quota and period
        if let Some(quota) = extract_number_field(json_str, "quota") {
            spec.linux_resources.cpu_quota_usec = Some(quota);
        }
        if let Some(period) = extract_number_field(json_str, "period") {
            spec.linux_resources.cpu_period_usec = period;
        }

        // Extract pids limit
        if let Some(pids) = extract_nested_number_field(json_str, "pids", "limit") {
            spec.linux_resources.pids_max = Some(pids);
        } else if let Some(pids) = extract_number_field(json_str, "pids") {
            spec.linux_resources.pids_max = Some(pids);
        }

        Ok(spec)
    }

    /// Serialize specification into formatted OCI config.json format.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("{\n");
        out.push_str(&format!("  \"ociVersion\": \"{}\",\n", self.oci_version));
        out.push_str(&format!("  \"hostname\": \"{}\",\n", self.hostname));
        out.push_str("  \"process\": {\n");
        out.push_str(&format!("    \"cwd\": \"{}\",\n", self.process.cwd));
        out.push_str("    \"args\": [\n");
        for (i, arg) in self.process.args.iter().enumerate() {
            let comma = if i + 1 < self.process.args.len() { "," } else { "" };
            out.push_str(&format!("      \"{}\"{}\n", arg, comma));
        }
        out.push_str("    ],\n");
        out.push_str("    \"env\": [\n");
        for (i, e) in self.process.env.iter().enumerate() {
            let comma = if i + 1 < self.process.env.len() { "," } else { "" };
            out.push_str(&format!("      \"{}\"{}\n", e, comma));
        }
        out.push_str("    ]\n");
        out.push_str("  },\n");
        out.push_str("  \"root\": {\n");
        out.push_str(&format!("    \"path\": \"{}\",\n", self.root.path));
        out.push_str(&format!("    \"readonly\": {}\n", self.root.readonly));
        out.push_str("  },\n");
        out.push_str("  \"linux\": {\n");
        out.push_str("    \"resources\": {\n");
        if let Some(mem) = self.linux_resources.memory_max_bytes {
            out.push_str(&format!("      \"memory\": {{ \"limit\": {} }},\n", mem));
        }
        if let Some(cpu) = self.linux_resources.cpu_quota_usec {
            out.push_str(&format!("      \"cpu\": {{ \"quota\": {}, \"period\": {} }},\n", cpu, self.linux_resources.cpu_period_usec));
        }
        if let Some(pids) = self.linux_resources.pids_max {
            out.push_str(&format!("      \"pids\": {{ \"limit\": {} }}\n", pids));
        }
        out.push_str("    }\n");
        out.push_str("  }\n");
        out.push_str("}\n");
        out
    }
}

// -----------------------------------------------------------------------------
// Pure Rust JSON String Matchers
// -----------------------------------------------------------------------------

fn extract_nested_number_field(json: &str, parent: &str, field: &str) -> Option<u64> {
    let key = format!("\"{}\"", parent);
    let key_pos = json.find(&key)?;
    let after_key = &json[key_pos + key.len()..];
    extract_number_field(after_key, field)
}

fn extract_string_field(json: &str, field_name: &str) -> Option<String> {
    let key = format!("\"{}\"", field_name);
    let key_pos = json.find(&key)?;
    let after_key = &json[key_pos + key.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let rest = &after_colon[1..];
    let end_quote = rest.find('"')?;
    Some(rest[..end_quote].to_string())
}

fn extract_number_field(json: &str, field_name: &str) -> Option<u64> {
    let key = format!("\"{}\"", field_name);
    let key_pos = json.find(&key)?;
    let after_key = &json[key_pos + key.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();

    let mut num_str = String::new();
    for ch in after_colon.chars() {
        if ch.is_ascii_digit() {
            num_str.push(ch);
        } else if !num_str.is_empty() {
            break;
        } else if !ch.is_whitespace() {
            return None;
        }
    }
    num_str.parse::<u64>().ok()
}

fn extract_string_array(json: &str, field_name: &str) -> Option<Vec<String>> {
    let key = format!("\"{}\"", field_name);
    let key_pos = json.find(&key)?;
    let after_key = &json[key_pos + key.len()..];
    let lbracket = after_key.find('[')?;
    let rbracket = after_key.find(']')?;
    if rbracket <= lbracket {
        return None;
    }
    let array_content = &after_key[lbracket + 1..rbracket];

    let mut items = Vec::new();
    let mut in_quote = false;
    let mut current = String::new();
    let mut prev_char = ' ';

    for ch in array_content.chars() {
        if ch == '"' && prev_char != '\\' {
            if in_quote {
                items.push(current.clone());
                current.clear();
                in_quote = false;
            } else {
                in_quote = true;
            }
        } else if in_quote {
            current.push(ch);
        }
        prev_char = ch;
    }

    Some(items)
}
