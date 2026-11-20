//! userland/desktop/apps/nova-calc/src/model.rs
//!
//! # NovaOS Calculator Domain Model & Mathematical Engine
//!
//! Synthesizes the core capabilities of:
//! 1. macOS Calculator: Precision arithmetic, iconic orange operators, scientific functions.
//! 2. Windows 11 Calculator: Dual-line expression tracking, Fluent memory row, Programmer multi-base view.
//! 3. Ubuntu GNOME Calculator: Clean expression history journal, instant preview, bitwise tools.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcMode {
    Standard,
    Scientific,
    Programmer,
    History,
}

impl CalcMode {
    pub fn name(&self) -> &'static str {
        match self {
            CalcMode::Standard => "Standard",
            CalcMode::Scientific => "Scientific",
            CalcMode::Programmer => "Programmer",
            CalcMode::History => "History",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgBase {
    Hex,
    Dec,
    Oct,
    Bin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordSize {
    Qword = 64,
    Dword = 32,
    Word = 16,
    Byte = 8,
}

impl WordSize {
    pub fn mask(&self) -> u64 {
        match self {
            WordSize::Qword => 0xFFFF_FFFF_FFFF_FFFF,
            WordSize::Dword => 0x0000_0000_FFFF_FFFF,
            WordSize::Word => 0x0000_0000_0000_FFFF,
            WordSize::Byte => 0x0000_0000_0000_00FF,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            WordSize::Qword => "64-bit",
            WordSize::Dword => "32-bit",
            WordSize::Word => "16-bit",
            WordSize::Byte => "8-bit",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryItem {
    pub expr: String,
    pub result: String,
    pub timestamp: String,
}

pub const MAX_HISTORY: usize = 16;

#[derive(Debug, Clone)]
pub struct CalculatorState {
    pub display: String,
    pub expr_line: String,
    pub accumulator: f64,
    pub pending_op: Option<char>,
    pub clear_on_next_digit: bool,
    pub mode: CalcMode,
    pub memory: f64,
    pub has_memory: bool,
    pub prog_base: ProgBase,
    pub word_size: WordSize,
    pub history: Vec<HistoryItem>,
    pub is_error: bool,
}

impl CalculatorState {
    pub fn new() -> Self {
        Self {
            display: "0".to_string(),
            expr_line: String::new(),
            accumulator: 0.0,
            pending_op: None,
            clear_on_next_digit: false,
            mode: CalcMode::Standard,
            memory: 0.0,
            has_memory: false,
            prog_base: ProgBase::Dec,
            word_size: WordSize::Qword,
            history: Vec::new(),
            is_error: false,
        }
    }

    pub fn reset(&mut self) {
        self.display = "0".to_string();
        self.expr_line.clear();
        self.accumulator = 0.0;
        self.pending_op = None;
        self.clear_on_next_digit = false;
        self.is_error = false;
    }

    pub fn clear_entry(&mut self) {
        self.display = "0".to_string();
        self.clear_on_next_digit = false;
        self.is_error = false;
    }

    pub fn input_digit(&mut self, digit: char) {
        if self.is_error {
            self.reset();
        }

        // Validate digit for current programmer base if in programmer mode
        if self.mode == CalcMode::Programmer {
            let valid = match self.prog_base {
                ProgBase::Hex => digit.is_ascii_hexdigit(),
                ProgBase::Dec => digit.is_ascii_digit(),
                ProgBase::Oct => digit >= '0' && digit <= '7',
                ProgBase::Bin => digit == '0' || digit == '1',
            };
            if !valid {
                return;
            }
        } else if !digit.is_ascii_digit() {
            return;
        }

        if self.clear_on_next_digit || self.display == "0" {
            self.display = digit.to_string();
            self.clear_on_next_digit = false;
        } else if self.display.len() < 24 {
            self.display.push(digit);
        }
    }

    pub fn input_decimal(&mut self) {
        if self.mode == CalcMode::Programmer {
            return; // Programmer mode is strictly integer based
        }
        if self.is_error {
            self.reset();
        }
        if self.clear_on_next_digit {
            self.display = "0.".to_string();
            self.clear_on_next_digit = false;
        } else if !self.display.contains('.') {
            self.display.push('.');
        }
    }

    pub fn backspace(&mut self) {
        if self.is_error || self.clear_on_next_digit {
            self.display = "0".to_string();
            self.clear_on_next_digit = false;
            self.is_error = false;
            return;
        }
        if self.display.len() > 1 {
            self.display.pop();
            if self.display == "-" {
                self.display = "0".to_string();
            }
        } else {
            self.display = "0".to_string();
        }
    }

    pub fn toggle_sign(&mut self) {
        if self.is_error || self.display == "0" {
            return;
        }
        if self.mode == CalcMode::Programmer {
            let val = self.current_int();
            let neg = -val;
            self.set_current_int(neg);
        } else {
            if self.display.starts_with('-') {
                self.display.remove(0);
            } else {
                self.display.insert(0, '-');
            }
        }
    }

    pub fn apply_binary_op(&mut self, op: char) {
        if self.is_error {
            return;
        }

        let current_val = self.current_val();

        if let Some(pending) = self.pending_op {
            if !self.clear_on_next_digit {
                let res = self.compute_binary(self.accumulator, current_val, pending);
                match res {
                    Ok(val) => {
                        self.accumulator = val;
                        if self.mode == CalcMode::Programmer {
                            self.set_current_int(val as i64);
                        } else {
                            self.display = format_num(val);
                        }
                    }
                    Err(err) => {
                        self.display = err;
                        self.is_error = true;
                        self.pending_op = None;
                        return;
                    }
                }
            }
        } else {
            self.accumulator = current_val;
        }

        self.pending_op = Some(op);
        let op_symbol = match op {
            '*' => '×',
            '/' => '÷',
            _ => op,
        };
        self.expr_line = format!("{} {}", format_num(self.accumulator), op_symbol);
        self.clear_on_next_digit = true;
    }

    pub fn equals(&mut self) {
        if self.is_error {
            return;
        }

        if let Some(op) = self.pending_op {
            let current_val = self.current_val();
            let op_symbol = match op {
                '*' => '×',
                '/' => '÷',
                _ => op,
            };
            let expr = format!("{} {} {}", format_num(self.accumulator), op_symbol, format_num(current_val));
            let res = self.compute_binary(self.accumulator, current_val, op);
            match res {
                Ok(val) => {
                    self.expr_line = format!("{} =", expr);
                    if self.mode == CalcMode::Programmer {
                        self.set_current_int(val as i64);
                        self.record_history(&expr, &self.display.clone());
                    } else {
                        let result_str = format_num(val);
                        self.display = result_str.clone();
                        self.record_history(&expr, &result_str);
                    }
                    self.accumulator = val;
                    self.pending_op = None;
                    self.clear_on_next_digit = true;
                }
                Err(err) => {
                    self.display = err;
                    self.is_error = true;
                    self.pending_op = None;
                }
            }
        }
    }

    fn compute_binary(&self, a: f64, b: f64, op: char) -> Result<f64, String> {
        match op {
            '+' => Ok(a + b),
            '-' => Ok(a - b),
            '*' | '×' => Ok(a * b),
            '/' | '÷' => {
                if b.abs() < 1e-12 {
                    Err("Error: Divide by 0".to_string())
                } else {
                    Ok(a / b)
                }
            }
            '%' => {
                if b.abs() < 1e-12 {
                    Err("Error: Divide by 0".to_string())
                } else {
                    Ok(a % b)
                }
            }
            '^' => Ok(a.powf(b)),
            // Bitwise operations (Programmer mode)
            '&' => {
                let ia = a as i64;
                let ib = b as i64;
                Ok((ia & ib) as f64)
            }
            '|' => {
                let ia = a as i64;
                let ib = b as i64;
                Ok((ia | ib) as f64)
            }
            'x' | 'X' => {
                let ia = a as i64;
                let ib = b as i64;
                Ok((ia ^ ib) as f64)
            }
            '<' => {
                let ia = a as i64;
                let ib = (b as i64).clamp(0, 63);
                Ok((ia << ib) as f64)
            }
            '>' => {
                let ia = a as i64;
                let ib = (b as i64).clamp(0, 63);
                Ok((ia >> ib) as f64)
            }
            _ => Ok(b),
        }
    }

    pub fn apply_unary_op(&mut self, op: &str) {
        if self.is_error {
            return;
        }

        let val = self.current_val();
        let (expr_desc, res) = match op {
            "sqr" | "x²" => (format!("sqr({})", format_num(val)), Ok(val * val)),
            "sqrt" | "√" => {
                if val < 0.0 {
                    (format!("√({})", format_num(val)), Err("Error: Negative root".to_string()))
                } else {
                    (format!("√({})", format_num(val)), Ok(val.sqrt()))
                }
            }
            "1/x" => {
                if val.abs() < 1e-12 {
                    (format!("1/({})", format_num(val)), Err("Error: Divide by 0".to_string()))
                } else {
                    (format!("1/({})", format_num(val)), Ok(1.0 / val))
                }
            }
            "%" => (format!("{}/100", format_num(val)), Ok(val / 100.0)),
            "sin" => (format!("sin({})", format_num(val)), Ok(val.sin())),
            "cos" => (format!("cos({})", format_num(val)), Ok(val.cos())),
            "tan" => (format!("tan({})", format_num(val)), Ok(val.tan())),
            "ln" => {
                if val <= 0.0 {
                    (format!("ln({})", format_num(val)), Err("Error: Invalid ln".to_string()))
                } else {
                    (format!("ln({})", format_num(val)), Ok(val.ln()))
                }
            }
            "log" | "log10" => {
                if val <= 0.0 {
                    (format!("log({})", format_num(val)), Err("Error: Invalid log".to_string()))
                } else {
                    (format!("log({})", format_num(val)), Ok(val.log10()))
                }
            }
            "fact" | "n!" => {
                if val < 0.0 || val > 20.0 || (val.fract()).abs() > 1e-6 {
                    (format!("fact({})", format_num(val)), Err("Error: Invalid n!".to_string()))
                } else {
                    let n = val as u64;
                    let mut f: u64 = 1;
                    for i in 2..=n {
                        f *= i;
                    }
                    (format!("fact({})", n), Ok(f as f64))
                }
            }
            "abs" | "|x|" => (format!("abs({})", format_num(val)), Ok(val.abs())),
            "not" | "~" => {
                let iv = val as i64;
                let masked = (!iv as u64) & self.word_size.mask();
                (format!("NOT({})", iv), Ok(masked as i64 as f64))
            }
            _ => (String::new(), Ok(val)),
        };

        match res {
            Ok(new_val) => {
                self.expr_line = expr_desc.clone();
                if self.mode == CalcMode::Programmer {
                    self.set_current_int(new_val as i64);
                    self.record_history(&expr_desc, &self.display.clone());
                } else {
                    let res_str = format_num(new_val);
                    self.display = res_str.clone();
                    self.record_history(&expr_desc, &res_str);
                }
                self.clear_on_next_digit = true;
            }
            Err(err) => {
                self.display = err;
                self.is_error = true;
            }
        }
    }

    pub fn insert_constant(&mut self, const_name: &str) {
        if self.is_error {
            self.reset();
        }
        let val = match const_name {
            "pi" | "π" => core::f64::consts::PI,
            "e" => core::f64::consts::E,
            _ => return,
        };
        self.display = format_num(val);
        self.clear_on_next_digit = true;
    }

    // --- Memory Operations ---

    pub fn memory_clear(&mut self) {
        self.memory = 0.0;
        self.has_memory = false;
    }

    pub fn memory_recall(&mut self) {
        if self.has_memory {
            self.display = format_num(self.memory);
            self.clear_on_next_digit = true;
        }
    }

    pub fn memory_store(&mut self) {
        if !self.is_error {
            self.memory = self.current_val();
            self.has_memory = true;
            self.clear_on_next_digit = true;
        }
    }

    pub fn memory_add(&mut self) {
        if !self.is_error {
            self.memory += self.current_val();
            self.has_memory = true;
            self.clear_on_next_digit = true;
        }
    }

    pub fn memory_sub(&mut self) {
        if !self.is_error {
            self.memory -= self.current_val();
            self.has_memory = true;
            self.clear_on_next_digit = true;
        }
    }

    // --- Programmer Base Support ---

    pub fn current_int(&self) -> i64 {
        if self.is_error {
            return 0;
        }
        match self.prog_base {
            ProgBase::Hex => i64::from_str_radix(&self.display, 16).unwrap_or(0),
            ProgBase::Dec => self.display.parse::<i64>().unwrap_or(0),
            ProgBase::Oct => i64::from_str_radix(&self.display, 8).unwrap_or(0),
            ProgBase::Bin => i64::from_str_radix(&self.display, 2).unwrap_or(0),
        }
    }

    pub fn set_current_int(&mut self, mut val: i64) {
        val = (val as u64 & self.word_size.mask()) as i64;
        self.display = match self.prog_base {
            ProgBase::Hex => format!("{:X}", val as u64 & self.word_size.mask()),
            ProgBase::Dec => format!("{}", val),
            ProgBase::Oct => format!("{:o}", val as u64 & self.word_size.mask()),
            ProgBase::Bin => format!("{:b}", val as u64 & self.word_size.mask()),
        };
        self.clear_on_next_digit = true;
    }

    pub fn set_prog_base(&mut self, base: ProgBase) {
        let val = self.current_int();
        self.prog_base = base;
        self.set_current_int(val);
    }

    pub fn set_word_size(&mut self, size: WordSize) {
        self.word_size = size;
        let val = self.current_int();
        self.set_current_int(val);
    }

    pub fn current_val(&self) -> f64 {
        if self.is_error {
            0.0
        } else if self.mode == CalcMode::Programmer {
            self.current_int() as f64
        } else {
            self.display.parse::<f64>().unwrap_or(0.0)
        }
    }

    pub fn formatted_display(&self) -> String {
        if self.is_error {
            return self.display.clone();
        }
        if self.mode == CalcMode::Programmer {
            return self.display.clone();
        }
        format_with_commas(&self.display)
    }

    pub fn record_history(&mut self, expr: &str, result: &str) {
        if self.history.len() >= MAX_HISTORY {
            self.history.remove(0);
        }
        self.history.push(HistoryItem {
            expr: expr.to_string(),
            result: result.to_string(),
            timestamp: "12:45 PM".to_string(),
        });
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }
}

impl Default for CalculatorState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn format_num(val: f64) -> String {
    if val.is_nan() {
        return "NaN".to_string();
    }
    if val.is_infinite() {
        return if val > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() };
    }
    // If it's an integer value, show without decimal
    if (val - val.round()).abs() < 1e-10 && val.abs() < 1e14 {
        format!("{}", val.round() as i64)
    } else {
        let s = format!("{:.8}", val);
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        trimmed.to_string()
    }
}

pub fn format_with_commas(s: &str) -> String {
    if s.starts_with("Error") || s == "NaN" || s == "Infinity" || s == "-Infinity" {
        return s.to_string();
    }
    let is_neg = s.starts_with('-');
    let num_str = if is_neg { &s[1..] } else { s };

    let parts: Vec<&str> = num_str.split('.').collect();
    let int_part = parts[0];
    let frac_part = if parts.len() > 1 { Some(parts[1]) } else { None };

    let mut formatted_int = String::new();
    let len = int_part.len();
    for (i, c) in int_part.chars().enumerate() {
        formatted_int.push(c);
        let remaining = len - 1 - i;
        if remaining > 0 && remaining % 3 == 0 {
            formatted_int.push(',');
        }
    }

    let mut res = if is_neg { "-".to_string() } else { String::new() };
    res.push_str(&formatted_int);
    if let Some(frac) = frac_part {
        res.push('.');
        res.push_str(frac);
    }
    res
}

// --- Recursive Descent Math Expression Parser ---

pub fn evaluate_expression(expr: &str) -> Result<f64, String> {
    let mut tokens = tokenize(expr)?;
    let mut pos = 0;
    let res = parse_additive(&mut tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(format!("Unexpected token '{}'", tokens[pos]));
    }
    Ok(res)
}

fn tokenize(expr: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        if c.is_ascii_digit() || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) {
            let mut num_str = String::new();
            if c == '0' && i + 1 < chars.len() && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
                num_str.push('0');
                num_str.push('x');
                i += 2;
                while i < chars.len() && chars[i].is_ascii_hexdigit() {
                    num_str.push(chars[i]);
                    i += 1;
                }
                let int_val = i64::from_str_radix(&num_str[2..], 16)
                    .map_err(|_| "Invalid hex literal".to_string())?;
                tokens.push(format!("{}", int_val));
                continue;
            }
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                num_str.push(chars[i]);
                i += 1;
            }
            tokens.push(num_str);
            continue;
        }

        if c.is_alphabetic() {
            let mut id = String::new();
            while i < chars.len() && chars[i].is_alphanumeric() {
                id.push(chars[i]);
                i += 1;
            }
            tokens.push(id);
            continue;
        }

        // Symbols
        match c {
            '+' | '-' | '*' | '/' | '%' | '^' | '(' | ')' => {
                tokens.push(c.to_string());
                i += 1;
            }
            '×' => {
                tokens.push("*".to_string());
                i += 1;
            }
            '÷' => {
                tokens.push("/".to_string());
                i += 1;
            }
            _ => {
                return Err(format!("Unknown character '{}'", c));
            }
        }
    }

    Ok(tokens)
}

fn parse_additive(tokens: &[String], pos: &mut usize) -> Result<f64, String> {
    let mut left = parse_multiplicative(tokens, pos)?;
    while *pos < tokens.len() {
        let op = &tokens[*pos];
        if op == "+" {
            *pos += 1;
            let right = parse_multiplicative(tokens, pos)?;
            left += right;
        } else if op == "-" {
            *pos += 1;
            let right = parse_multiplicative(tokens, pos)?;
            left -= right;
        } else {
            break;
        }
    }
    Ok(left)
}

fn parse_multiplicative(tokens: &[String], pos: &mut usize) -> Result<f64, String> {
    let mut left = parse_power(tokens, pos)?;
    while *pos < tokens.len() {
        let op = &tokens[*pos];
        if op == "*" {
            *pos += 1;
            let right = parse_power(tokens, pos)?;
            left *= right;
        } else if op == "/" {
            *pos += 1;
            let right = parse_power(tokens, pos)?;
            if right.abs() < 1e-12 {
                return Err("Error: Divide by 0".to_string());
            }
            left /= right;
        } else if op == "%" {
            *pos += 1;
            let right = parse_power(tokens, pos)?;
            if right.abs() < 1e-12 {
                return Err("Error: Divide by 0".to_string());
            }
            left %= right;
        } else {
            break;
        }
    }
    Ok(left)
}

fn parse_power(tokens: &[String], pos: &mut usize) -> Result<f64, String> {
    let left = parse_unary(tokens, pos)?;
    if *pos < tokens.len() && tokens[*pos] == "^" {
        *pos += 1;
        let right = parse_power(tokens, pos)?;
        return Ok(left.powf(right));
    }
    Ok(left)
}

fn parse_unary(tokens: &[String], pos: &mut usize) -> Result<f64, String> {
    if *pos >= tokens.len() {
        return Err("Unexpected end of expression".to_string());
    }

    if tokens[*pos] == "-" {
        *pos += 1;
        let v = parse_primary(tokens, pos)?;
        return Ok(-v);
    }
    if tokens[*pos] == "+" {
        *pos += 1;
        return parse_primary(tokens, pos);
    }

    parse_primary(tokens, pos)
}

fn parse_primary(tokens: &[String], pos: &mut usize) -> Result<f64, String> {
    if *pos >= tokens.len() {
        return Err("Unexpected end of expression".to_string());
    }

    let tok = &tokens[*pos];

    if tok == "(" {
        *pos += 1;
        let val = parse_additive(tokens, pos)?;
        if *pos >= tokens.len() || tokens[*pos] != ")" {
            return Err("Mismatched parentheses: missing ')'".to_string());
        }
        *pos += 1; // consume ')'
        return Ok(val);
    }

    // Function calls or constants
    match tok.to_lowercase().as_str() {
        "pi" => {
            *pos += 1;
            return Ok(core::f64::consts::PI);
        }
        "e" => {
            *pos += 1;
            return Ok(core::f64::consts::E);
        }
        "sqrt" => {
            *pos += 1;
            let arg = parse_primary(tokens, pos)?;
            if arg < 0.0 {
                return Err("Error: Negative root".to_string());
            }
            return Ok(arg.sqrt());
        }
        "sin" => {
            *pos += 1;
            let arg = parse_primary(tokens, pos)?;
            return Ok(arg.sin());
        }
        "cos" => {
            *pos += 1;
            let arg = parse_primary(tokens, pos)?;
            return Ok(arg.cos());
        }
        "tan" => {
            *pos += 1;
            let arg = parse_primary(tokens, pos)?;
            return Ok(arg.tan());
        }
        "ln" => {
            *pos += 1;
            let arg = parse_primary(tokens, pos)?;
            if arg <= 0.0 {
                return Err("Error: Invalid ln argument".to_string());
            }
            return Ok(arg.ln());
        }
        "log" => {
            *pos += 1;
            let arg = parse_primary(tokens, pos)?;
            if arg <= 0.0 {
                return Err("Error: Invalid log argument".to_string());
            }
            return Ok(arg.log10());
        }
        "abs" => {
            *pos += 1;
            let arg = parse_primary(tokens, pos)?;
            return Ok(arg.abs());
        }
        _ => {}
    }

    // Numeric literal
    if let Ok(num) = tok.parse::<f64>() {
        *pos += 1;
        Ok(num)
    } else {
        Err(format!("Unexpected token '{}'", tok))
    }
}
