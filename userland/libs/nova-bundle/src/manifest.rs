//! userland/libs/nova-bundle/src/manifest.rs
//!
//! NovaOS Application Bundle Manifest (`Manifest.json`) parser, validator, and serializer.
//! Defines bundle metadata, declared capabilities, sandbox permissions, and execution parameters.

#[cfg(not(test))]
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

/// Errors that can occur during manifest parsing or validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    EmptyInput,
    UnexpectedToken(usize, char),
    UnexpectedEof,
    ExpectedKey(usize),
    ExpectedColon(usize),
    ExpectedCommaOrEnd(usize),
    MissingField(&'static str),
    InvalidFieldValue(&'static str),
    SemanticError(String),
}

/// Granular sandbox permission declarations required by an application bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPermissions {
    /// True if application requests outbound/inbound network access.
    pub network: bool,
    /// True if application requests audio input/output playback & recording.
    pub audio: bool,
    /// True if application requests webcam / video capture.
    pub camera: bool,
    /// True if application requires hardware GPU 3D acceleration.
    pub gpu: bool,
    /// Whitelisted read-only filesystem paths outside the bundle.
    pub filesystem_read: Vec<String>,
    /// Whitelisted read-write filesystem paths outside the bundle.
    pub filesystem_write: Vec<String>,
    /// True if application may run as an unattended background service / daemon.
    pub background_daemon: bool,
    /// Whitelisted system IPC service names (e.g., "novad", "audio_mixer", "compositor").
    pub ipc_services: Vec<String>,
}

impl Default for SandboxPermissions {
    fn default() -> Self {
        Self {
            network: false,
            audio: false,
            camera: false,
            gpu: false,
            filesystem_read: Vec::new(),
            filesystem_write: Vec::new(),
            background_daemon: false,
            ipc_services: Vec::new(),
        }
    }
}

/// Application Bundle Manifest metadata specification (`Manifest.json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleManifest {
    /// Reverse-domain application identifier (e.g., "org.novaos.calculator").
    pub bundle_id: String,
    /// Human-readable display name (e.g., "Nova Calculator").
    pub name: String,
    /// Semantic version string (e.g., "2.0.0").
    pub version: String,
    /// Relative path to primary 64-bit ELF executable (e.g., "bin/calculator").
    pub executable: String,
    /// Relative path to main bundle icon (e.g., "assets/icon_256.png").
    pub icon: String,
    /// Application categories (e.g., ["Productivity", "Utilities"]).
    pub categories: Vec<String>,
    /// Minimum compatible NovaOS kernel/system version (e.g., "2.0.0").
    pub min_os_version: String,
    /// Brief description of the application.
    pub description: String,
    /// Developer or organization author.
    pub author: String,
    /// Declared sandbox permissions and capability boundaries.
    pub sandbox: SandboxPermissions,
    /// Environment variables injected on bundle startup.
    pub env_vars: Vec<(String, String)>,
    /// File MIME types associated with this application (e.g., ["text/plain"]).
    pub mime_types: Vec<String>,
}

impl BundleManifest {
    /// Creates a minimal valid application bundle manifest.
    pub fn new(
        bundle_id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        executable: impl Into<String>,
    ) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            name: name.into(),
            version: version.into(),
            executable: executable.into(),
            icon: "assets/icon_256.png".to_string(),
            categories: vec!["Utilities".to_string()],
            min_os_version: "2.0.0".to_string(),
            description: "NovaOS Application".to_string(),
            author: "NovaOS Community".to_string(),
            sandbox: SandboxPermissions::default(),
            env_vars: Vec::new(),
            mime_types: Vec::new(),
        }
    }

    /// Validates the semantic constraints of the manifest.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.bundle_id.trim().is_empty() {
            return Err(ManifestError::InvalidFieldValue("bundle_id cannot be empty"));
        }
        if !self.bundle_id.contains('.') {
            return Err(ManifestError::InvalidFieldValue("bundle_id must follow reverse-DNS convention (e.g. org.novaos.app)"));
        }
        if self.name.trim().is_empty() {
            return Err(ManifestError::InvalidFieldValue("name cannot be empty"));
        }
        if self.version.trim().is_empty() {
            return Err(ManifestError::InvalidFieldValue("version cannot be empty"));
        }
        if self.executable.trim().is_empty() {
            return Err(ManifestError::InvalidFieldValue("executable cannot be empty"));
        }
        if !self.executable.starts_with("bin/") {
            return Err(ManifestError::InvalidFieldValue("executable path must be located in bin/ directory"));
        }
        Ok(())
    }

    /// Serializes the manifest into a formatted, indented JSON string.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("{\n");
        out.push_str(&format!("  \"bundle_id\": \"{}\",\n", escape_json(&self.bundle_id)));
        out.push_str(&format!("  \"name\": \"{}\",\n", escape_json(&self.name)));
        out.push_str(&format!("  \"version\": \"{}\",\n", escape_json(&self.version)));
        out.push_str(&format!("  \"executable\": \"{}\",\n", escape_json(&self.executable)));
        out.push_str(&format!("  \"icon\": \"{}\",\n", escape_json(&self.icon)));

        // Categories
        out.push_str("  \"categories\": [\n");
        for (i, cat) in self.categories.iter().enumerate() {
            let comma = if i + 1 < self.categories.len() { "," } else { "" };
            out.push_str(&format!("    \"{}\"{}\n", escape_json(cat), comma));
        }
        out.push_str("  ],\n");

        out.push_str(&format!("  \"min_os_version\": \"{}\",\n", escape_json(&self.min_os_version)));
        out.push_str(&format!("  \"description\": \"{}\",\n", escape_json(&self.description)));
        out.push_str(&format!("  \"author\": \"{}\",\n", escape_json(&self.author)));

        // Sandbox
        out.push_str("  \"sandbox\": {\n");
        out.push_str(&format!("    \"network\": {},\n", self.sandbox.network));
        out.push_str(&format!("    \"audio\": {},\n", self.sandbox.audio));
        out.push_str(&format!("    \"camera\": {},\n", self.sandbox.camera));
        out.push_str(&format!("    \"gpu\": {},\n", self.sandbox.gpu));
        out.push_str(&format!("    \"background_daemon\": {},\n", self.sandbox.background_daemon));

        // filesystem_read
        out.push_str("    \"filesystem_read\": [\n");
        for (i, p) in self.sandbox.filesystem_read.iter().enumerate() {
            let comma = if i + 1 < self.sandbox.filesystem_read.len() { "," } else { "" };
            out.push_str(&format!("      \"{}\"{}\n", escape_json(p), comma));
        }
        out.push_str("    ],\n");

        // filesystem_write
        out.push_str("    \"filesystem_write\": [\n");
        for (i, p) in self.sandbox.filesystem_write.iter().enumerate() {
            let comma = if i + 1 < self.sandbox.filesystem_write.len() { "," } else { "" };
            out.push_str(&format!("      \"{}\"{}\n", escape_json(p), comma));
        }
        out.push_str("    ],\n");

        // ipc_services
        out.push_str("    \"ipc_services\": [\n");
        for (i, s) in self.sandbox.ipc_services.iter().enumerate() {
            let comma = if i + 1 < self.sandbox.ipc_services.len() { "," } else { "" };
            out.push_str(&format!("      \"{}\"{}\n", escape_json(s), comma));
        }
        out.push_str("    ]\n");
        out.push_str("  },\n");

        // env_vars
        out.push_str("  \"env_vars\": {\n");
        for (i, (k, v)) in self.env_vars.iter().enumerate() {
            let comma = if i + 1 < self.env_vars.len() { "," } else { "" };
            out.push_str(&format!("    \"{}\": \"{}\"{}\n", escape_json(k), escape_json(v), comma));
        }
        out.push_str("  },\n");

        // mime_types
        out.push_str("  \"mime_types\": [\n");
        for (i, m) in self.mime_types.iter().enumerate() {
            let comma = if i + 1 < self.mime_types.len() { "," } else { "" };
            out.push_str(&format!("    \"{}\"{}\n", escape_json(m), comma));
        }
        out.push_str("  ]\n");

        out.push_str("}\n");
        out
    }

    /// Parses a Manifest from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, ManifestError> {
        let val = JsonValue::parse(json)?;
        let obj = match val {
            JsonValue::Object(map) => map,
            _ => return Err(ManifestError::SemanticError("Root JSON must be an object".to_string())),
        };

        let bundle_id = obj
            .get("bundle_id")
            .and_then(|v| v.as_str())
            .ok_or(ManifestError::MissingField("bundle_id"))?
            .to_string();

        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or(ManifestError::MissingField("name"))?
            .to_string();

        let version = obj
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("1.0.0")
            .to_string();

        let executable = obj
            .get("executable")
            .and_then(|v| v.as_str())
            .ok_or(ManifestError::MissingField("executable"))?
            .to_string();

        let icon = obj
            .get("icon")
            .and_then(|v| v.as_str())
            .unwrap_or("assets/icon_256.png")
            .to_string();

        let categories = if let Some(JsonValue::Array(arr)) = obj.get("categories") {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        } else {
            vec!["Utilities".to_string()]
        };

        let min_os_version = obj
            .get("min_os_version")
            .and_then(|v| v.as_str())
            .unwrap_or("2.0.0")
            .to_string();

        let description = obj
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let author = obj
            .get("author")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        // Parse sandbox
        let mut sandbox = SandboxPermissions::default();
        if let Some(JsonValue::Object(sb_obj)) = obj.get("sandbox") {
            if let Some(v) = sb_obj.get("network").and_then(|v| v.as_bool()) {
                sandbox.network = v;
            }
            if let Some(v) = sb_obj.get("audio").and_then(|v| v.as_bool()) {
                sandbox.audio = v;
            }
            if let Some(v) = sb_obj.get("camera").and_then(|v| v.as_bool()) {
                sandbox.camera = v;
            }
            if let Some(v) = sb_obj.get("gpu").and_then(|v| v.as_bool()) {
                sandbox.gpu = v;
            }
            if let Some(v) = sb_obj.get("background_daemon").and_then(|v| v.as_bool()) {
                sandbox.background_daemon = v;
            }
            if let Some(JsonValue::Array(arr)) = sb_obj.get("filesystem_read") {
                sandbox.filesystem_read = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
            }
            if let Some(JsonValue::Array(arr)) = sb_obj.get("filesystem_write") {
                sandbox.filesystem_write = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
            }
            if let Some(JsonValue::Array(arr)) = sb_obj.get("ipc_services") {
                sandbox.ipc_services = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
            }
        }

        // Parse env_vars
        let mut env_vars = Vec::new();
        if let Some(JsonValue::Object(env_obj)) = obj.get("env_vars") {
            for (k, v) in env_obj.iter() {
                if let Some(val_str) = v.as_str() {
                    env_vars.push((k.clone(), val_str.to_string()));
                }
            }
        }

        // Parse mime_types
        let mime_types = if let Some(JsonValue::Array(arr)) = obj.get("mime_types") {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        } else {
            Vec::new()
        };

        let manifest = Self {
            bundle_id,
            name,
            version,
            executable,
            icon,
            categories,
            min_os_version,
            description,
            author,
            sandbox,
            env_vars,
            mime_types,
        };

        manifest.validate()?;
        Ok(manifest)
    }
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Lightweight JSON DOM value representation for pure no_std environments.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, JsonValue>> {
        match self {
            JsonValue::Object(m) => Some(m),
            _ => None,
        }
    }

    pub fn parse(input: &str) -> Result<Self, ManifestError> {
        let chars: Vec<(usize, char)> = input.char_indices().collect();
        let mut idx = 0;
        skip_ws(&chars, &mut idx);
        if idx >= chars.len() {
            return Err(ManifestError::EmptyInput);
        }
        let val = parse_val(&chars, &mut idx)?;
        skip_ws(&chars, &mut idx);
        Ok(val)
    }
}

fn skip_ws(chars: &[(usize, char)], idx: &mut usize) {
    while *idx < chars.len() && chars[*idx].1.is_whitespace() {
        *idx += 1;
    }
}

fn parse_val(chars: &[(usize, char)], idx: &mut usize) -> Result<JsonValue, ManifestError> {
    skip_ws(chars, idx);
    if *idx >= chars.len() {
        return Err(ManifestError::UnexpectedEof);
    }
    match chars[*idx].1 {
        '{' => parse_obj(chars, idx),
        '[' => parse_arr(chars, idx),
        '"' => parse_str_val(chars, idx),
        't' | 'f' => parse_bool(chars, idx),
        'n' => parse_null(chars, idx),
        '-' | '0'..='9' => parse_num(chars, idx),
        c => Err(ManifestError::UnexpectedToken(chars[*idx].0, c)),
    }
}

fn parse_obj(chars: &[(usize, char)], idx: &mut usize) -> Result<JsonValue, ManifestError> {
    *idx += 1; // skip '{'
    let mut map = BTreeMap::new();
    loop {
        skip_ws(chars, idx);
        if *idx >= chars.len() {
            return Err(ManifestError::UnexpectedEof);
        }
        if chars[*idx].1 == '}' {
            *idx += 1;
            break;
        }

        // Parse key
        let key = if chars[*idx].1 == '"' {
            parse_str(chars, idx)?
        } else {
            return Err(ManifestError::ExpectedKey(chars[*idx].0));
        };

        skip_ws(chars, idx);
        if *idx >= chars.len() || chars[*idx].1 != ':' {
            let pos = if *idx < chars.len() { chars[*idx].0 } else { 0 };
            return Err(ManifestError::ExpectedColon(pos));
        }
        *idx += 1; // skip ':'

        let val = parse_val(chars, idx)?;
        map.insert(key, val);

        skip_ws(chars, idx);
        if *idx >= chars.len() {
            return Err(ManifestError::UnexpectedEof);
        }
        if chars[*idx].1 == ',' {
            *idx += 1;
        } else if chars[*idx].1 == '}' {
            *idx += 1;
            break;
        } else {
            return Err(ManifestError::ExpectedCommaOrEnd(chars[*idx].0));
        }
    }
    Ok(JsonValue::Object(map))
}

fn parse_arr(chars: &[(usize, char)], idx: &mut usize) -> Result<JsonValue, ManifestError> {
    *idx += 1; // skip '['
    let mut list = Vec::new();
    loop {
        skip_ws(chars, idx);
        if *idx >= chars.len() {
            return Err(ManifestError::UnexpectedEof);
        }
        if chars[*idx].1 == ']' {
            *idx += 1;
            break;
        }

        let val = parse_val(chars, idx)?;
        list.push(val);

        skip_ws(chars, idx);
        if *idx >= chars.len() {
            return Err(ManifestError::UnexpectedEof);
        }
        if chars[*idx].1 == ',' {
            *idx += 1;
        } else if chars[*idx].1 == ']' {
            *idx += 1;
            break;
        } else {
            return Err(ManifestError::ExpectedCommaOrEnd(chars[*idx].0));
        }
    }
    Ok(JsonValue::Array(list))
}

fn parse_str_val(chars: &[(usize, char)], idx: &mut usize) -> Result<JsonValue, ManifestError> {
    let s = parse_str(chars, idx)?;
    Ok(JsonValue::Str(s))
}

fn parse_str(chars: &[(usize, char)], idx: &mut usize) -> Result<String, ManifestError> {
    *idx += 1; // skip initial '"'
    let mut s = String::new();
    while *idx < chars.len() {
        let c = chars[*idx].1;
        if c == '"' {
            *idx += 1;
            return Ok(s);
        } else if c == '\\' {
            *idx += 1;
            if *idx >= chars.len() {
                return Err(ManifestError::UnexpectedEof);
            }
            match chars[*idx].1 {
                '"' => s.push('"'),
                '\\' => s.push('\\'),
                '/' => s.push('/'),
                'n' => s.push('\n'),
                'r' => s.push('\r'),
                't' => s.push('\t'),
                esc => s.push(esc),
            }
            *idx += 1;
        } else {
            s.push(c);
            *idx += 1;
        }
    }
    Err(ManifestError::UnexpectedEof)
}

fn parse_bool(chars: &[(usize, char)], idx: &mut usize) -> Result<JsonValue, ManifestError> {
    if chars[*idx..].len() >= 4 && chars[*idx].1 == 't' && chars[*idx + 1].1 == 'r' && chars[*idx + 2].1 == 'u' && chars[*idx + 3].1 == 'e' {
        *idx += 4;
        Ok(JsonValue::Bool(true))
    } else if chars[*idx..].len() >= 5 && chars[*idx].1 == 'f' && chars[*idx + 1].1 == 'a' && chars[*idx + 2].1 == 'l' && chars[*idx + 3].1 == 's' && chars[*idx + 4].1 == 'e' {
        *idx += 5;
        Ok(JsonValue::Bool(false))
    } else {
        Err(ManifestError::UnexpectedToken(chars[*idx].0, chars[*idx].1))
    }
}

fn parse_null(chars: &[(usize, char)], idx: &mut usize) -> Result<JsonValue, ManifestError> {
    if chars[*idx..].len() >= 4 && chars[*idx].1 == 'n' && chars[*idx + 1].1 == 'u' && chars[*idx + 2].1 == 'l' && chars[*idx + 3].1 == 'l' {
        *idx += 4;
        Ok(JsonValue::Null)
    } else {
        Err(ManifestError::UnexpectedToken(chars[*idx].0, chars[*idx].1))
    }
}

fn parse_num(chars: &[(usize, char)], idx: &mut usize) -> Result<JsonValue, ManifestError> {
    let mut num_str = String::new();
    while *idx < chars.len() && (chars[*idx].1.is_ascii_digit() || chars[*idx].1 == '.' || chars[*idx].1 == '-') {
        num_str.push(chars[*idx].1);
        *idx += 1;
    }
    // Simple integer/float fallback
    let val = if let Ok(n) = num_str.parse::<f64>() {
        n
    } else {
        0.0
    };
    Ok(JsonValue::Number(val))
}
