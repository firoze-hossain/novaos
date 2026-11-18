//! userland/server/novainit/src/parser.rs
//!
//! Pure Rust YAML & JSON parser tailored for Cloud-Init #cloud-config, meta-data, and network-config.
//! Zero external crate dependencies (#![forbid(unsafe_code)]).

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<YamlValue>),
    Map(BTreeMap<String, YamlValue>),
}

impl YamlValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            YamlValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            YamlValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            YamlValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&Vec<YamlValue>> {
        match self {
            YamlValue::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, YamlValue>> {
        match self {
            YamlValue::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&YamlValue> {
        match self {
            YamlValue::Map(m) => m.get(key),
            _ => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| v.as_str())
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| v.as_bool())
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.as_int())
    }
}

// -----------------------------------------------------------------------------
// YAML Parser
// -----------------------------------------------------------------------------

pub struct YamlParser;

impl YamlParser {
    /// Strips leading/trailing whitespace and quotes
    fn clean_scalar(s: &str) -> YamlValue {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return YamlValue::Null;
        }

        // Quoted strings
        if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
        {
            let inner = &trimmed[1..trimmed.len() - 1];
            return YamlValue::String(inner.replace("\\n", "\n").replace("\\t", "\t"));
        }

        // Inline lists: [a, b, c]
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let inner = &trimmed[1..trimmed.len() - 1].trim();
            if inner.is_empty() {
                return YamlValue::List(Vec::new());
            }
            let mut list = Vec::new();
            for part in inner.split(',') {
                list.push(Self::clean_scalar(part));
            }
            return YamlValue::List(list);
        }

        // Booleans
        if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("yes") {
            return YamlValue::Bool(true);
        }
        if trimmed.eq_ignore_ascii_case("false") || trimmed.eq_ignore_ascii_case("no") {
            return YamlValue::Bool(false);
        }

        // Integers
        if let Ok(i) = trimmed.parse::<i64>() {
            return YamlValue::Int(i);
        }

        // Floats
        if let Ok(f) = trimmed.parse::<f64>() {
            return YamlValue::Float(f);
        }

        YamlValue::String(trimmed.to_string())
    }

    /// Parses YAML text into a root `YamlValue::Map`
    pub fn parse(input: &str) -> Result<YamlValue, String> {
        let mut lines = Vec::new();
        for (idx, line) in input.lines().enumerate() {
            // Strip comments unless inside quotes
            let mut clean_line = line;
            if let Some(pos) = line.find('#') {
                // If it's the header `#cloud-config`, ignore it
                if pos == 0 {
                    continue;
                }
                clean_line = &line[..pos];
            }
            if clean_line.trim().is_empty() {
                continue;
            }
            let indent = clean_line.chars().take_while(|c| *c == ' ').count();
            lines.push((idx + 1, indent, clean_line.trim().to_string()));
        }

        if lines.is_empty() {
            return Ok(YamlValue::Map(BTreeMap::new()));
        }

        let mut idx = 0;
        let root = Self::parse_block(&lines, &mut idx, 0)?;
        Ok(root)
    }

    fn parse_block(
        lines: &[(usize, usize, String)],
        idx: &mut usize,
        current_indent: usize,
    ) -> Result<YamlValue, String> {
        if *idx >= lines.len() {
            return Ok(YamlValue::Null);
        }

        // Determine if this block is a list or a map
        let (_, _first_indent, ref first_line) = lines[*idx];
        if first_line.starts_with("- ") || first_line == "-" {
            let mut list = Vec::new();
            while *idx < lines.len() {
                let (_, indent, ref line) = lines[*idx];
                if indent < current_indent {
                    break;
                }
                if indent == current_indent && (line.starts_with("- ") || line == "-") {
                    let content = if line.starts_with("- ") {
                        &line[2..]
                    } else {
                        ""
                    };

                    *idx += 1;
                    if content.contains(':') && !content.starts_with('[') {
                        // Single-line key-value starting a map inside a list item
                        let colon_pos = content.find(':').unwrap();
                        let key = content[..colon_pos].trim().to_string();
                        let val_part = content[colon_pos + 1..].trim();

                        let mut map = BTreeMap::new();
                        if val_part.is_empty() {
                            // Sub-block
                            if *idx < lines.len() && lines[*idx].1 > indent {
                                let sub_indent = lines[*idx].1;
                                let sub_val = Self::parse_block(lines, idx, sub_indent)?;
                                map.insert(key, sub_val);
                            } else {
                                map.insert(key, YamlValue::Null);
                            }
                        } else {
                            map.insert(key, Self::clean_scalar(val_part));
                        }

                        // Parse sibling fields belonging to this same list item
                        while *idx < lines.len() {
                            let (_, sub_i, ref sub_l) = lines[*idx];
                            if sub_i <= indent || sub_l.starts_with("- ") {
                                break;
                            }
                            if let Some(c_pos) = sub_l.find(':') {
                                let k = sub_l[..c_pos].trim().to_string();
                                let v_str = sub_l[c_pos + 1..].trim();
                                *idx += 1;
                                if v_str.is_empty() {
                                    if *idx < lines.len() && lines[*idx].1 > sub_i {
                                        let nested_i = lines[*idx].1;
                                        let nested_val = Self::parse_block(lines, idx, nested_i)?;
                                        map.insert(k, nested_val);
                                    } else {
                                        map.insert(k, YamlValue::Null);
                                    }
                                } else {
                                    map.insert(k, Self::clean_scalar(v_str));
                                }
                            } else {
                                *idx += 1;
                            }
                        }
                        list.push(YamlValue::Map(map));
                    } else if content.is_empty() {
                        // Empty '-' leading to nested block
                        if *idx < lines.len() && lines[*idx].1 > indent {
                            let sub_i = lines[*idx].1;
                            let sub_val = Self::parse_block(lines, idx, sub_i)?;
                            list.push(sub_val);
                        } else {
                            list.push(YamlValue::Null);
                        }
                    } else {
                        // Simple scalar list item
                        list.push(Self::clean_scalar(content));
                    }
                } else {
                    break;
                }
            }
            Ok(YamlValue::List(list))
        } else {
            // Map
            let mut map = BTreeMap::new();
            while *idx < lines.len() {
                let (_, indent, ref line) = lines[*idx];
                if indent < current_indent {
                    break;
                }
                if let Some(colon_pos) = line.find(':') {
                    let key = line[..colon_pos].trim().to_string();
                    let val_str = line[colon_pos + 1..].trim();
                    *idx += 1;

                    if val_str.is_empty() {
                        // Nested block
                        if *idx < lines.len() && lines[*idx].1 > indent {
                            let next_indent = lines[*idx].1;
                            let nested = Self::parse_block(lines, idx, next_indent)?;
                            map.insert(key, nested);
                        } else {
                            map.insert(key, YamlValue::Null);
                        }
                    } else if val_str == "|" {
                        // Literal block scalar
                        let mut multiline = String::new();
                        while *idx < lines.len() {
                            let (_, lit_i, ref lit_l) = lines[*idx];
                            if lit_i <= indent {
                                break;
                            }
                            multiline.push_str(lit_l);
                            multiline.push('\n');
                            *idx += 1;
                        }
                        map.insert(key, YamlValue::String(multiline));
                    } else {
                        map.insert(key, Self::clean_scalar(val_str));
                    }
                } else {
                    *idx += 1;
                }
            }
            Ok(YamlValue::Map(map))
        }
    }
}

// -----------------------------------------------------------------------------
// JSON Parser (Minimal subset for OpenStack metadata)
// -----------------------------------------------------------------------------

pub struct JsonParser;

impl JsonParser {
    pub fn parse(input: &str) -> Result<YamlValue, String> {
        let trimmed = input.trim();
        let chars: Vec<char> = trimmed.chars().collect();
        let mut idx = 0;
        Self::parse_value(&chars, &mut idx)
    }

    fn skip_whitespace(chars: &[char], idx: &mut usize) {
        while *idx < chars.len() && chars[*idx].is_whitespace() {
            *idx += 1;
        }
    }

    fn parse_value(chars: &[char], idx: &mut usize) -> Result<YamlValue, String> {
        Self::skip_whitespace(chars, idx);
        if *idx >= chars.len() {
            return Ok(YamlValue::Null);
        }

        match chars[*idx] {
            '{' => Self::parse_object(chars, idx),
            '[' => Self::parse_array(chars, idx),
            '"' => Self::parse_string(chars, idx),
            't' | 'f' => Self::parse_bool(chars, idx),
            'n' => Self::parse_null(chars, idx),
            c if c.is_ascii_digit() || c == '-' => Self::parse_number(chars, idx),
            other => Err(format!("Unexpected character in JSON: '{}' at index {}", other, idx)),
        }
    }

    fn parse_object(chars: &[char], idx: &mut usize) -> Result<YamlValue, String> {
        *idx += 1; // skip '{'
        let mut map = BTreeMap::new();

        loop {
            Self::skip_whitespace(chars, idx);
            if *idx >= chars.len() {
                return Err("Unterminated JSON object".into());
            }
            if chars[*idx] == '}' {
                *idx += 1;
                break;
            }

            let key = match Self::parse_string(chars, idx)? {
                YamlValue::String(s) => s,
                _ => return Err("Expected string key in object".into()),
            };

            Self::skip_whitespace(chars, idx);
            if *idx >= chars.len() || chars[*idx] != ':' {
                return Err("Expected ':' after object key".into());
            }
            *idx += 1; // skip ':'

            let value = Self::parse_value(chars, idx)?;
            map.insert(key, value);

            Self::skip_whitespace(chars, idx);
            if *idx < chars.len() && chars[*idx] == ',' {
                *idx += 1;
            } else if *idx < chars.len() && chars[*idx] == '}' {
                *idx += 1;
                break;
            }
        }
        Ok(YamlValue::Map(map))
    }

    fn parse_array(chars: &[char], idx: &mut usize) -> Result<YamlValue, String> {
        *idx += 1; // skip '['
        let mut list = Vec::new();

        loop {
            Self::skip_whitespace(chars, idx);
            if *idx >= chars.len() {
                return Err("Unterminated JSON array".into());
            }
            if chars[*idx] == ']' {
                *idx += 1;
                break;
            }

            let val = Self::parse_value(chars, idx)?;
            list.push(val);

            Self::skip_whitespace(chars, idx);
            if *idx < chars.len() && chars[*idx] == ',' {
                *idx += 1;
            } else if *idx < chars.len() && chars[*idx] == ']' {
                *idx += 1;
                break;
            }
        }
        Ok(YamlValue::List(list))
    }

    fn parse_string(chars: &[char], idx: &mut usize) -> Result<YamlValue, String> {
        *idx += 1; // skip opening quote
        let mut s = String::new();
        while *idx < chars.len() {
            let c = chars[*idx];
            *idx += 1;
            if c == '"' {
                return Ok(YamlValue::String(s));
            } else if c == '\\' {
                if *idx < chars.len() {
                    let esc = chars[*idx];
                    *idx += 1;
                    match esc {
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        other => s.push(other),
                    }
                }
            } else {
                s.push(c);
            }
        }
        Err("Unterminated JSON string".into())
    }

    fn parse_bool(chars: &[char], idx: &mut usize) -> Result<YamlValue, String> {
        if chars[*idx..].iter().collect::<String>().starts_with("true") {
            *idx += 4;
            Ok(YamlValue::Bool(true))
        } else if chars[*idx..].iter().collect::<String>().starts_with("false") {
            *idx += 5;
            Ok(YamlValue::Bool(false))
        } else {
            Err("Expected boolean".into())
        }
    }

    fn parse_null(chars: &[char], idx: &mut usize) -> Result<YamlValue, String> {
        if chars[*idx..].iter().collect::<String>().starts_with("null") {
            *idx += 4;
            Ok(YamlValue::Null)
        } else {
            Err("Expected null".into())
        }
    }

    fn parse_number(chars: &[char], idx: &mut usize) -> Result<YamlValue, String> {
        let mut num_str = String::new();
        while *idx < chars.len() && (chars[*idx].is_ascii_digit() || chars[*idx] == '-' || chars[*idx] == '.') {
            num_str.push(chars[*idx]);
            *idx += 1;
        }
        if num_str.contains('.') {
            num_str.parse::<f64>().map(YamlValue::Float).map_err(|e| e.to_string())
        } else {
            num_str.parse::<i64>().map(YamlValue::Int).map_err(|e| e.to_string())
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_cloud_config_parsing() {
        let yaml_text = r#"#cloud-config
hostname: novaserver-01
manage_etc_hosts: true
users:
  - name: novaadmin
    gecos: NovaOS Administrator
    sudo: ALL=(ALL) NOPASSWD:ALL
    groups: [wheel, sudo, admin]
    shell: /bin/novash
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIInw2BkW1+8n83fM5R admin@novaos
write_files:
  - path: /etc/motd
    content: "Welcome to NovaOS Server!\n"
    permissions: "0644"
runcmd:
  - [ novactl, service, enable, novasshd ]
  - [ novactl, service, start, novad ]
"#;

        let parsed = YamlParser::parse(yaml_text).expect("Parse valid cloud-config YAML");
        assert_eq!(parsed.get_str("hostname"), Some("novaserver-01"));
        assert_eq!(parsed.get_bool("manage_etc_hosts"), Some(true));

        let users = parsed.get("users").and_then(|u| u.as_list()).expect("Users list");
        assert_eq!(users.len(), 1);
        let user = &users[0];
        assert_eq!(user.get_str("name"), Some("novaadmin"));
        assert_eq!(user.get_str("shell"), Some("/bin/novash"));

        let groups = user.get("groups").and_then(|g| g.as_list()).expect("Groups list");
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].as_str(), Some("wheel"));

        let ssh_keys = user.get("ssh_authorized_keys").and_then(|k| k.as_list()).expect("SSH keys");
        assert_eq!(ssh_keys.len(), 1);
        assert!(ssh_keys[0].as_str().unwrap().starts_with("ssh-ed25519"));

        let write_files = parsed.get("write_files").and_then(|f| f.as_list()).expect("write_files");
        assert_eq!(write_files[0].get_str("path"), Some("/etc/motd"));

        let runcmd = parsed.get("runcmd").and_then(|c| c.as_list()).expect("runcmd");
        assert_eq!(runcmd.len(), 2);
    }

    #[test]
    fn test_json_openstack_metadata_parsing() {
        let json_text = r#"{
            "instance_id": "i-0987654321fedcba0",
            "hostname": "cloud-node-openstack",
            "public_keys": {
                "admin_key": "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC... user@cloud"
            },
            "network_config": {
                "deb_net": "auto eth0\niface eth0 inet dhcp\n"
            }
        }"#;

        let parsed = JsonParser::parse(json_text).expect("Parse JSON metadata");
        assert_eq!(parsed.get_str("instance_id"), Some("i-0987654321fedcba0"));
        assert_eq!(parsed.get_str("hostname"), Some("cloud-node-openstack"));

        let pkeys = parsed.get("public_keys").and_then(|k| k.as_map()).expect("public_keys map");
        assert!(pkeys.get("admin_key").unwrap().as_str().unwrap().starts_with("ssh-rsa"));
    }
}
