//! userland/desktop/nova-search/src/indexer.rs
//!
//! Real-Time Unified Indexer & Instant Fuzzy Search Engine for NovaOS.
//! Features:
//! - Multi-category indexing: Applications, Documents, Settings Panels, and Shell Commands.
//! - Sub-millisecond (<10ms) fuzzy subsequence matching with word-boundary and prefix bonuses.
//! - Built-in inline math calculator and unit converter (arithmetic, functions, bytes, temperature, length).

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{string::String, vec::Vec, format};

#[cfg(test)]
use std::{string::String, vec::Vec, format};

// ============================================================================
// Data Models
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchCategory {
    Application,
    Document,
    SystemSetting,
    ShellCommand,
    Calculation,
}

impl SearchCategory {
    pub fn badge_label(&self) -> &'static str {
        match self {
            SearchCategory::Application => "App",
            SearchCategory::Document => "Document",
            SearchCategory::SystemSetting => "Setting",
            SearchCategory::ShellCommand => "Command",
            SearchCategory::Calculation => "Calculator",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub category: SearchCategory,
    pub action_target: String,
    pub icon_glyph: String,
    pub score: u32,
}

impl SearchItem {
    pub fn new(
        id: &str,
        title: &str,
        subtitle: &str,
        category: SearchCategory,
        action_target: &str,
        icon_glyph: &str,
    ) -> Self {
        Self {
            id: String::from(id),
            title: String::from(title),
            subtitle: String::from(subtitle),
            category,
            action_target: String::from(action_target),
            icon_glyph: String::from(icon_glyph),
            score: 0,
        }
    }
}

// ============================================================================
// Fuzzy Matching Engine
// ============================================================================

/// High-performance fuzzy matching algorithm.
/// Evaluates pattern against text, returning an optional score.
pub fn fuzzy_match(pattern: &str, text: &str) -> Option<u32> {
    if pattern.is_empty() {
        return Some(1);
    }

    let p_bytes: Vec<u8> = pattern.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let t_bytes: Vec<u8> = text.bytes().map(|b| b.to_ascii_lowercase()).collect();

    // 1. Exact match bonus
    if p_bytes == t_bytes {
        return Some(2000);
    }

    // 2. Exact prefix match bonus
    if t_bytes.starts_with(&p_bytes) {
        return Some(1000 + (100 - p_bytes.len().min(90) as u32));
    }

    // 3. Subsequence match with word-boundary bonus
    let mut p_idx = 0;
    let mut score = 0u32;
    let mut prev_matched_idx = None;

    for (t_idx, &tb) in t_bytes.iter().enumerate() {
        if p_idx < p_bytes.len() && tb == p_bytes[p_idx] {
            // Base match score
            score += 20;

            // Consecutive match bonus
            if let Some(prev) = prev_matched_idx {
                if t_idx == prev + 1 {
                    score += 40;
                }
            }

            // Word-boundary bonus (start of string or preceded by space, dash, slash, dot)
            if t_idx == 0 || matches!(t_bytes[t_idx - 1], b' ' | b'-' | b'_' | b'/' | b'.') {
                score += 80;
            }

            prev_matched_idx = Some(t_idx);
            p_idx += 1;
        }
    }

    if p_idx == p_bytes.len() {
        Some(score)
    } else {
        None
    }
}

// ============================================================================
// Inline Math & Unit Calculator
// ============================================================================

/// Evaluates inline math expressions and unit conversions
pub fn evaluate_inline_query(input: &str) -> Option<SearchItem> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Check for unit conversions first (e.g. "1024 KB to MB", "72 F to C", "10 km to miles")
    if let Some(res) = evaluate_unit_conversion(trimmed) {
        return Some(SearchItem::new(
            "calc_result",
            &res,
            trimmed,
            SearchCategory::Calculation,
            &res,
            "=",
        ));
    }

    // Check for arithmetic math expression (contains +, -, *, /, %, ^, or starts with a number or sqrt/abs/sin/cos)
    if is_likely_math_expression(trimmed) {
        if let Ok(val) = evaluate_arithmetic(trimmed) {
            let formatted = format_math_result(val);
            let display_title = format!("= {}", formatted);
            return Some(SearchItem::new(
                "calc_result",
                &display_title,
                trimmed,
                SearchCategory::Calculation,
                &formatted,
                "=",
            ));
        }
    }

    None
}

fn is_likely_math_expression(s: &str) -> bool {
    // Must contain math operators or math functions
    let has_op = s.contains('+') || s.contains('-') || s.contains('*') || s.contains('/') || s.contains('^') || s.contains('%');
    let has_func = s.starts_with("sqrt(") || s.starts_with("abs(") || s.starts_with("sin(") || s.starts_with("cos(") || s.starts_with("tan(");
    (has_op || has_func) && s.chars().any(|c| c.is_ascii_digit())
}

fn format_math_result(val: f64) -> String {
    let abs_val = if val < 0.0 { -val } else { val };
    if (val as i64 as f64) == val && abs_val < 1e14 {
        format!("{}", val as i64)
    } else {
        format!("{:.4}", val)
    }
}

// Simple recursive-descent math parser
fn evaluate_arithmetic(expr: &str) -> Result<f64, ()> {
    let mut chars: Vec<char> = expr.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return Err(());
    }
    let mut pos = 0;
    parse_expression(&mut chars, &mut pos)
}

fn parse_expression(chars: &mut Vec<char>, pos: &mut usize) -> Result<f64, ()> {
    let mut left = parse_term(chars, pos)?;

    while *pos < chars.len() {
        match chars[*pos] {
            '+' => {
                *pos += 1;
                left += parse_term(chars, pos)?;
            }
            '-' => {
                *pos += 1;
                left -= parse_term(chars, pos)?;
            }
            _ => break,
        }
    }

    Ok(left)
}

fn parse_term(chars: &mut Vec<char>, pos: &mut usize) -> Result<f64, ()> {
    let mut left = parse_factor(chars, pos)?;

    while *pos < chars.len() {
        match chars[*pos] {
            '*' => {
                *pos += 1;
                left *= parse_factor(chars, pos)?;
            }
            '/' => {
                *pos += 1;
                let divisor = parse_factor(chars, pos)?;
                if divisor == 0.0 {
                    return Err(());
                }
                left /= divisor;
            }
            '%' => {
                *pos += 1;
                let divisor = parse_factor(chars, pos)?;
                if divisor == 0.0 {
                    return Err(());
                }
                left %= divisor;
            }
            '^' => {
                *pos += 1;
                let exp = parse_factor(chars, pos)?;
                left = pow_approx(left, exp);
            }
            _ => break,
        }
    }

    Ok(left)
}

fn parse_factor(chars: &mut Vec<char>, pos: &mut usize) -> Result<f64, ()> {
    if *pos >= chars.len() {
        return Err(());
    }

    // Unary plus or minus
    if chars[*pos] == '-' {
        *pos += 1;
        return Ok(-parse_factor(chars, pos)?);
    }
    if chars[*pos] == '+' {
        *pos += 1;
        return parse_factor(chars, pos);
    }

    // Parenthesized sub-expression
    if chars[*pos] == '(' {
        *pos += 1;
        let result = parse_expression(chars, pos)?;
        if *pos < chars.len() && chars[*pos] == ')' {
            *pos += 1;
            return Ok(result);
        } else {
            return Err(());
        }
    }

    // Function call: sqrt, abs
    if let Some(f_name) = peek_function(chars, *pos) {
        *pos += f_name.len();
        if *pos < chars.len() && chars[*pos] == '(' {
            *pos += 1;
            let arg = parse_expression(chars, pos)?;
            if *pos < chars.len() && chars[*pos] == ')' {
                *pos += 1;
                return match f_name {
                    "sqrt" => {
                        if arg < 0.0 { Err(()) } else { Ok(sqrt_approx(arg)) }
                    }
                    "abs" => {
                        let a = if arg < 0.0 { -arg } else { arg };
                        Ok(a)
                    }
                    _ => Err(()),
                };
            }
        }
        return Err(());
    }

    // Number literal
    let start = *pos;
    let mut has_dot = false;
    while *pos < chars.len() && (chars[*pos].is_ascii_digit() || (chars[*pos] == '.' && !has_dot)) {
        if chars[*pos] == '.' {
            has_dot = true;
        }
        *pos += 1;
    }

    if start == *pos {
        return Err(());
    }

    let num_str: String = chars[start..*pos].iter().collect();
    parse_f64(&num_str).ok_or(())
}

fn peek_function(chars: &[char], pos: usize) -> Option<&'static str> {
    let remaining: String = chars[pos..].iter().take(6).collect();
    if remaining.starts_with("sqrt") {
        Some("sqrt")
    } else if remaining.starts_with("abs") {
        Some("abs")
    } else {
        None
    }
}

fn pow_approx(base: f64, exp: f64) -> f64 {
    // Integer exponentiation
    if (exp as i64 as f64) == exp && exp >= 0.0 && exp <= 32.0 {
        let mut res = 1.0;
        for _ in 0..(exp as u32) {
            res *= base;
        }
        res
    } else {
        base // simplified fallback
    }
}

fn sqrt_approx(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut guess = x / 2.0;
    for _ in 0..10 {
        guess = (guess + x / guess) / 2.0;
    }
    guess
}

fn parse_f64(s: &str) -> Option<f64> {
    let mut parts = s.split('.');
    let int_part = parts.next()?.parse::<i64>().ok()? as f64;
    if let Some(dec_str) = parts.next() {
        let dec_val = dec_str.parse::<i64>().ok()? as f64;
        let mut div = 1.0;
        for _ in 0..dec_str.len() {
            div *= 10.0;
        }
        Some(int_part + (dec_val / div))
    } else {
        Some(int_part)
    }
}

// Evaluates natural unit conversions: "1024 KB to MB", "72 F to C", "10 km to miles"
fn evaluate_unit_conversion(query: &str) -> Option<String> {
    let parts: Vec<&str> = query.split_whitespace().collect();
    if parts.len() != 4 || parts[2].to_ascii_lowercase() != "to" {
        return None;
    }

    let val = parse_f64(parts[0])?;
    let from_unit = parts[1].to_ascii_lowercase();
    let to_unit = parts[3].to_ascii_lowercase();

    match (from_unit.as_str(), to_unit.as_str()) {
        // Digital Storage
        ("b", "kb") => Some(format!("{:.2} KB", val / 1024.0)),
        ("kb", "mb") => Some(format!("{:.2} MB", val / 1024.0)),
        ("mb", "gb") => Some(format!("{:.2} GB", val / 1024.0)),
        ("gb", "mb") => Some(format!("{:.0} MB", val * 1024.0)),
        ("gb", "tb") => Some(format!("{:.2} TB", val / 1024.0)),
        ("tb", "gb") => Some(format!("{:.0} GB", val * 1024.0)),

        // Temperature
        ("f", "c") => {
            let c = (val - 32.0) * 5.0 / 9.0;
            Some(format!("{:.1} °C", c))
        }
        ("c", "f") => {
            let f = (val * 9.0 / 5.0) + 32.0;
            Some(format!("{:.1} °F", f))
        }

        // Distance & Length
        ("km", "miles") | ("km", "mi") => Some(format!("{:.2} miles", val * 0.621371)),
        ("miles", "km") | ("mi", "km") => Some(format!("{:.2} km", val * 1.60934)),
        ("in", "cm") => Some(format!("{:.2} cm", val * 2.54)),
        ("cm", "in") => Some(format!("{:.2} inches", val / 2.54)),
        ("m", "ft") => Some(format!("{:.2} feet", val * 3.28084)),
        ("ft", "m") => Some(format!("{:.2} m", val / 3.28084)),

        _ => None,
    }
}

// ============================================================================
// Unified Search Index
// ============================================================================

pub struct SearchIndex {
    pub items: Vec<SearchItem>,
}

impl SearchIndex {
    pub fn new() -> Self {
        let mut index = Self { items: Vec::new() };
        index.populate_default_catalogue();
        index
    }

    /// Add item to search index
    pub fn add_item(&mut self, item: SearchItem) {
        self.items.push(item);
    }

    /// Queries index with fuzzy matching, sorting, and inline calculation
    pub fn query(&self, input: &str, max_results: usize) -> Vec<SearchItem> {
        let trimmed = input.trim();
        let mut results = Vec::new();

        // 1. Check for inline math or unit conversion
        if let Some(calc_item) = evaluate_inline_query(trimmed) {
            results.push(calc_item);
        }

        if trimmed.is_empty() {
            return results;
        }

        // 2. Fuzzy match across all indexed items
        let mut scored_items: Vec<SearchItem> = Vec::new();

        for item in &self.items {
            let title_score = fuzzy_match(trimmed, &item.title);
            let subtitle_score = fuzzy_match(trimmed, &item.subtitle);

            let best_score = match (title_score, subtitle_score) {
                (Some(s1), Some(s2)) => Some(s1.max(s2 / 2)),
                (Some(s1), None) => Some(s1),
                (None, Some(s2)) => Some(s2 / 2),
                (None, None) => None,
            };

            if let Some(score) = best_score {
                let mut matched = item.clone();
                matched.score = score;
                scored_items.push(matched);
            }
        }

        // Sort descending by match score
        scored_items.sort_by(|a, b| b.score.cmp(&a.score));

        // Append top matches
        for item in scored_items.into_iter().take(max_results - results.len()) {
            results.push(item);
        }

        results
    }

    /// Populates standard NovaOS system catalog
    pub fn populate_default_catalogue(&mut self) {
        // Applications
        self.add_item(SearchItem::new(
            "app_files",
            "Files",
            "File Manager & Storage Browser",
            SearchCategory::Application,
            "FILES.ELF",
            "📁",
        ));
        self.add_item(SearchItem::new(
            "app_term",
            "Terminal",
            "Command Line Shell & VT Grid",
            SearchCategory::Application,
            "TERM.ELF",
            "💻",
        ));
        self.add_item(SearchItem::new(
            "app_edit",
            "Text Editor",
            "Source Code & Document Editor",
            SearchCategory::Application,
            "EDIT.ELF",
            "📝",
        ));
        self.add_item(SearchItem::new(
            "app_settings",
            "Settings",
            "System Preferences & Configuration",
            SearchCategory::Application,
            "SETTINGS.ELF",
            "⚙️",
        ));
        self.add_item(SearchItem::new(
            "app_store",
            "App Store",
            "Package Repository & Software Center",
            SearchCategory::Application,
            "STORE.ELF",
            "🛍️",
        ));
        self.add_item(SearchItem::new(
            "app_sysinfo",
            "System Information",
            "Hardware, Memory & Microkernel Telemetry",
            SearchCategory::Application,
            "SYSINFO.ELF",
            "ℹ️",
        ));
        self.add_item(SearchItem::new(
            "app_actmon",
            "Activity Monitor",
            "Process Task Manager & Resource Graphs",
            SearchCategory::Application,
            "ACTMON.ELF",
            "📈",
        ));
        self.add_item(SearchItem::new(
            "app_klog",
            "Kernel Log Viewer",
            "Structured Kernel Diagnostics & Trace",
            SearchCategory::Application,
            "KLOG.ELF",
            "📜",
        ));
        self.add_item(SearchItem::new(
            "app_crashrep",
            "Crash Reporter",
            "Post-Mortem Panic & Core Dump Viewer",
            SearchCategory::Application,
            "CRASHREP.ELF",
            "🛠️",
        ));

        // System Settings Panels
        self.add_item(SearchItem::new(
            "set_display",
            "Display Settings",
            "Resolution, Refresh Rate & Scaling",
            SearchCategory::SystemSetting,
            "SETTINGS.ELF --panel display",
            "🖥️",
        ));
        self.add_item(SearchItem::new(
            "set_audio",
            "Sound & Audio",
            "Volume, Intel HDA & Output Devices",
            SearchCategory::SystemSetting,
            "SETTINGS.ELF --panel audio",
            "🔊",
        ));
        self.add_item(SearchItem::new(
            "set_network",
            "Network & Wi-Fi",
            "IP Configuration, DNS & Interfaces",
            SearchCategory::SystemSetting,
            "SETTINGS.ELF --panel network",
            "📶",
        ));
        self.add_item(SearchItem::new(
            "set_users",
            "Users & Accounts",
            "User Management, Passwords & GID",
            SearchCategory::SystemSetting,
            "SETTINGS.ELF --panel users",
            "👤",
        ));
        self.add_item(SearchItem::new(
            "set_firewall",
            "Stateful Firewall",
            "Packet Filtering Rules & Port Policies",
            SearchCategory::SystemSetting,
            "SETTINGS.ELF --panel firewall",
            "🛡️",
        ));

        // Documents & Key Files
        self.add_item(SearchItem::new(
            "doc_readme",
            "README.TXT",
            "/DOCUMENTS/README.TXT - Welcome to NovaOS",
            SearchCategory::Document,
            "EDIT.ELF /DOCUMENTS/README.TXT",
            "📄",
        ));
        self.add_item(SearchItem::new(
            "doc_syscfg",
            "SYSTEM.CFG",
            "/SYSTEM.CFG - System Hostname & Init Config",
            SearchCategory::Document,
            "EDIT.ELF /SYSTEM.CFG",
            "📄",
        ));
        self.add_item(SearchItem::new(
            "doc_userscfg",
            "USERS.CFG",
            "/USERS.CFG - Salted User Hash Database",
            SearchCategory::Document,
            "EDIT.ELF /USERS.CFG",
            "📄",
        ));

        // Shell Commands
        self.add_item(SearchItem::new(
            "cmd_ping",
            "ping",
            "Send ICMP echo requests to network hosts",
            SearchCategory::ShellCommand,
            "ping",
            "$",
        ));
        self.add_item(SearchItem::new(
            "cmd_ls",
            "ls",
            "List directory contents with file metadata",
            SearchCategory::ShellCommand,
            "ls",
            "$",
        ));
        self.add_item(SearchItem::new(
            "cmd_cat",
            "cat",
            "Concatenate and print file contents to terminal",
            SearchCategory::ShellCommand,
            "cat",
            "$",
        ));
        self.add_item(SearchItem::new(
            "cmd_nslookup",
            "nslookup",
            "Query DNS nameservers for hostname resolution",
            SearchCategory::ShellCommand,
            "nslookup",
            "$",
        ));
        self.add_item(SearchItem::new(
            "cmd_shutdown",
            "shutdown",
            "ACPI S5 graceful system power-off",
            SearchCategory::ShellCommand,
            "shutdown",
            "$",
        ));
    }
}
