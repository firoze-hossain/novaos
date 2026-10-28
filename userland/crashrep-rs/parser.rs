//! userland/crashrep-rs/parser.rs - Crash report parser and diagnostic analyzer.
//! Parses human-readable and structured `CRASHDMP.TXT` reports and `NovaCrashReport` binary records.

#![allow(dead_code)]

use crate::ffi::NovaCrashReport;

pub const MAX_REASON_LEN: usize = 80;
pub const MAX_STACK_FRAMES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedCrashReport {
    pub has_crash: bool,
    pub ticks: u32,
    pub reason: [u8; MAX_REASON_LEN],
    pub reason_len: usize,
    pub has_regs: bool,
    pub int_no: u32,
    pub err_code: u32,
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub ds: u32,
    pub ss: u32,
    pub has_fault_addr: bool,
    pub fault_addr: u32,
    pub stack: [u32; MAX_STACK_FRAMES],
    pub stack_count: usize,
    pub raw_text_len: usize,
}

impl Default for ParsedCrashReport {
    fn default() -> Self {
        ParsedCrashReport {
            has_crash: false,
            ticks: 0,
            reason: [0u8; MAX_REASON_LEN],
            reason_len: 0,
            has_regs: false,
            int_no: 0,
            err_code: 0,
            eip: 0,
            cs: 0,
            eflags: 0,
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
            esi: 0,
            edi: 0,
            ebp: 0,
            esp: 0,
            ds: 0,
            ss: 0,
            has_fault_addr: false,
            fault_addr: 0,
            stack: [0u32; MAX_STACK_FRAMES],
            stack_count: 0,
            raw_text_len: 0,
        }
    }
}

impl ParsedCrashReport {
    pub fn reason_str(&self) -> &str {
        if self.reason_len == 0 {
            return "No reason provided";
        }
        core::str::from_utf8(&self.reason[..self.reason_len]).unwrap_or("Corrupt reason text")
    }

    pub fn vector_title(&self) -> &'static str {
        if !self.has_regs {
            return "Software Panic";
        }
        match self.int_no {
            0 => "Divide Error (#DE)",
            1 => "Debug Exception (#DB)",
            2 => "NMI Interrupt",
            3 => "Breakpoint (#BP)",
            4 => "Overflow (#OF)",
            5 => "BOUND Range (#BR)",
            6 => "Invalid Opcode (#UD)",
            7 => "Device Not Available (#NM)",
            8 => "Double Fault (#DF)",
            10 => "Invalid TSS (#TS)",
            11 => "Segment Not Present (#NP)",
            12 => "Stack-Segment Fault (#SS)",
            13 => "General Protection Fault (#GP)",
            14 => "Page Fault (#PF)",
            16 => "x87 FPU Error (#MF)",
            17 => "Alignment Check (#AC)",
            18 => "Machine Check (#MC)",
            19 => "SIMD Floating-Point (#XM)",
            _ => "Hardware Exception",
        }
    }

    pub fn vector_desc(&self) -> &'static str {
        if !self.has_regs {
            return "Kernel detected an unrecoverable software invariant violation";
        }
        match self.int_no {
            0 => "Division by zero or division result overflow",
            1 => "Hardware debug register trap or single-step execution flag",
            3 => "Software breakpoint instruction (INT 3) encountered",
            6 => "CPU attempted to execute an undefined or illegal machine instruction",
            8 => "Fatal exception occurred while processing a prior interrupt or exception",
            13 => "Privilege level violation, segment limit overrun, or GP protection fault",
            14 => "Access to unmapped virtual memory address or paging rights violation",
            _ => "Low-level CPU trap or hardware exception encountered",
        }
    }

    pub fn fault_analysis(&self) -> &'static str {
        if self.has_regs && self.int_no == 14 {
            let present = (self.err_code & 1) != 0;
            let write = (self.err_code & 2) != 0;
            let user = (self.err_code & 4) != 0;

            if !present && write && !user {
                "Kernel write to unmapped address (null/wild pointer dereference)"
            } else if !present && !write && !user {
                "Kernel read from unmapped memory page (invalid pointer dereference)"
            } else if present && write && !user {
                "Kernel write to read-only page (protection key or COW violation)"
            } else if !present && write && user {
                "Ring-3 userland write to unmapped memory address"
            } else if !present && !write && user {
                "Ring-3 userland read from unmapped memory page"
            } else {
                "Virtual memory paging rights violation"
            }
        } else if self.has_regs && self.int_no == 13 {
            "General Protection Fault: invalid segment, privilege violation, or misaligned operand"
        } else if self.has_regs && self.int_no == 6 {
            "Invalid Opcode: attempted execution of invalid or non-instruction byte sequence"
        } else if self.has_regs && self.int_no == 0 {
            "Divide by Zero: CPU arithmetic division error encountered in execution"
        } else {
            "Unhandled kernel condition resulting in panic"
        }
    }

    pub fn symbol_for_address(addr: u32) -> &'static str {
        if (addr >= 0x08048000 && addr < 0x08100000) || (addr >= 0x80000000 && addr < 0x81000000) {
            "userland application code"
        } else if addr >= 0x00100000 && addr < 0x00110000 {
            "kernel core init & scheduler"
        } else if addr >= 0x00110000 && addr < 0x00130000 {
            "kernel memory & paging subsystem"
        } else if addr >= 0x00130000 && addr < 0x00160000 {
            "kernel VFS & filesystem driver"
        } else if addr >= 0x00160000 && addr < 0x00200000 {
            "kernel device driver / rust module"
        } else if addr < 0x00100000 {
            "low memory / BIOS / IVT stub"
        } else {
            "heap / dynamic allocation"
        }
    }
}

// ===========================================================================
// Parsing Implementation
// ===========================================================================

pub fn parse_crash_text(data: &[u8]) -> ParsedCrashReport {
    let mut rep = ParsedCrashReport::default();
    if data.is_empty() {
        return rep;
    }
    rep.raw_text_len = data.len();
    rep.has_crash = true;

    // Check if [CRASH_METADATA] section is present
    let meta_tag = b"[CRASH_METADATA]";
    let mut meta_start = None;
    for i in 0..data.len().saturating_sub(meta_tag.len()) {
        if &data[i..i + meta_tag.len()] == meta_tag {
            meta_start = Some(i + meta_tag.len());
            break;
        }
    }

    if let Some(start) = meta_start {
        parse_metadata_lines(&data[start..], &mut rep);
    } else {
        // Fallback: parse human-readable report
        parse_human_lines(data, &mut rep);
    }

    rep
}

pub fn from_binary_report(bin: &NovaCrashReport) -> ParsedCrashReport {
    let mut rep = ParsedCrashReport {
        has_crash: true,
        ticks: bin.ticks,
        has_regs: bin.has_regs != 0,
        int_no: bin.regs_int_no,
        err_code: bin.regs_err_code,
        eip: bin.regs_eip,
        cs: bin.regs_cs,
        eflags: bin.regs_eflags,
        eax: bin.regs_eax,
        ebx: bin.regs_ebx,
        ecx: bin.regs_ecx,
        edx: bin.regs_edx,
        esi: bin.regs_esi,
        edi: bin.regs_edi,
        ebp: bin.regs_ebp,
        esp: bin.regs_useresp,
        ds: bin.regs_ds,
        ss: bin.regs_ss,
        has_fault_addr: bin.has_fault_addr != 0,
        fault_addr: bin.fault_addr,
        stack: bin.stack,
        stack_count: (bin.stack_count as usize).min(MAX_STACK_FRAMES),
        raw_text_len: 0,
        ..Default::default()
    };

    let mut rlen = 0;
    while rlen < bin.reason.len() && bin.reason[rlen] != 0 && rlen < MAX_REASON_LEN {
        rep.reason[rlen] = bin.reason[rlen];
        rlen += 1;
    }
    rep.reason_len = rlen;
    rep
}

fn parse_metadata_lines(slice: &[u8], rep: &mut ParsedCrashReport) {
    let mut i = 0;
    while i < slice.len() {
        // Find line start and end
        while i < slice.len() && (slice[i] == b'\n' || slice[i] == b'\r') {
            i += 1;
        }
        if i >= slice.len() {
            break;
        }
        let line_start = i;
        while i < slice.len() && slice[i] != b'\n' && slice[i] != b'\r' {
            i += 1;
        }
        let line = &slice[line_start..i];
        if line == b"END_METADATA=1" {
            break;
        }

        // Split on '='
        if let Some(eq_pos) = find_byte(line, b'=') {
            let key = &line[..eq_pos];
            let val = &line[eq_pos + 1..];

            match key {
                b"TICKS" => rep.ticks = parse_u32_dec(val),
                b"REASON" => {
                    let len = val.len().min(MAX_REASON_LEN);
                    rep.reason[..len].copy_from_slice(&val[..len]);
                    rep.reason_len = len;
                }
                b"HAS_REGS" => rep.has_regs = parse_u32_dec(val) != 0,
                b"INT_NO" => rep.int_no = parse_u32_dec(val),
                b"ERR_CODE" => rep.err_code = parse_u32_hex(val),
                b"EIP" => rep.eip = parse_u32_hex(val),
                b"EAX" => rep.eax = parse_u32_hex(val),
                b"EBX" => rep.ebx = parse_u32_hex(val),
                b"ECX" => rep.ecx = parse_u32_hex(val),
                b"EDX" => rep.edx = parse_u32_hex(val),
                b"ESI" => rep.esi = parse_u32_hex(val),
                b"EDI" => rep.edi = parse_u32_hex(val),
                b"EBP" => rep.ebp = parse_u32_hex(val),
                b"ESP" => rep.esp = parse_u32_hex(val),
                b"CR2" => {
                    rep.fault_addr = parse_u32_hex(val);
                    rep.has_fault_addr = true;
                }
                b"STACK_COUNT" => {
                    rep.stack_count = (parse_u32_dec(val) as usize).min(MAX_STACK_FRAMES);
                }
                b"STACK_0" => rep.stack[0] = parse_u32_hex(val),
                b"STACK_1" => rep.stack[1] = parse_u32_hex(val),
                b"STACK_2" => rep.stack[2] = parse_u32_hex(val),
                b"STACK_3" => rep.stack[3] = parse_u32_hex(val),
                b"STACK_4" => rep.stack[4] = parse_u32_hex(val),
                b"STACK_5" => rep.stack[5] = parse_u32_hex(val),
                b"STACK_6" => rep.stack[6] = parse_u32_hex(val),
                b"STACK_7" => rep.stack[7] = parse_u32_hex(val),
                _ => {}
            }
        }
    }
}

fn parse_human_lines(slice: &[u8], rep: &mut ParsedCrashReport) {
    let mut i = 0;
    while i < slice.len() {
        while i < slice.len() && (slice[i] == b'\n' || slice[i] == b'\r') {
            i += 1;
        }
        if i >= slice.len() {
            break;
        }
        let line_start = i;
        while i < slice.len() && slice[i] != b'\n' && slice[i] != b'\r' {
            i += 1;
        }
        let line = &slice[line_start..i];

        if starts_with(line, b"Timestamp: ") {
            let rest = &line[11..];
            if let Some(space_pos) = find_byte(rest, b' ') {
                rep.ticks = parse_u32_dec(&rest[..space_pos]);
            }
        } else if starts_with(line, b"Panic Reason:") {
            // Next line has the reason
            while i < slice.len() && (slice[i] == b'\n' || slice[i] == b'\r') {
                i += 1;
            }
            let r_start = i;
            while i < slice.len() && slice[i] != b'\n' && slice[i] != b'\r' {
                i += 1;
            }
            let mut r_line = &slice[r_start..i];
            while !r_line.is_empty() && r_line[0] == b' ' {
                r_line = &r_line[1..];
            }
            let len = r_line.len().min(MAX_REASON_LEN);
            rep.reason[..len].copy_from_slice(&r_line[..len]);
            rep.reason_len = len;
        } else if starts_with(line, b"  Faulting EIP:     ") {
            rep.has_regs = true;
            let rest = &line[20..];
            rep.eip = parse_u32_hex(rest);
        } else if starts_with(line, b"Faulting Address (CR2):") {
            while i < slice.len() && (slice[i] == b'\n' || slice[i] == b'\r') {
                i += 1;
            }
            let a_start = i;
            while i < slice.len() && slice[i] != b'\n' && slice[i] != b'\r' {
                i += 1;
            }
            let mut a_line = &slice[a_start..i];
            while !a_line.is_empty() && a_line[0] == b' ' {
                a_line = &a_line[1..];
            }
            rep.fault_addr = parse_u32_hex(a_line);
            rep.has_fault_addr = true;
        }
    }
}

// ===========================================================================
// String / Hex Helper Utilities
// ===========================================================================

pub fn find_byte(slice: &[u8], target: u8) -> Option<usize> {
    for (idx, &b) in slice.iter().enumerate() {
        if b == target {
            return Some(idx);
        }
    }
    None
}

pub fn starts_with(slice: &[u8], prefix: &[u8]) -> bool {
    slice.len() >= prefix.len() && &slice[..prefix.len()] == prefix
}

pub fn parse_u32_dec(slice: &[u8]) -> u32 {
    let mut val = 0u32;
    for &b in slice {
        if b >= b'0' && b <= b'9' {
            val = val.saturating_mul(10).saturating_add((b - b'0') as u32);
        } else if b != b' ' {
            break;
        }
    }
    val
}

pub fn parse_u32_hex(slice: &[u8]) -> u32 {
    let mut s = slice;
    while !s.is_empty() && s[0] == b' ' {
        s = &s[1..];
    }
    if s.len() >= 2 && (s[0] == b'0' && (s[1] == b'x' || s[1] == b'X')) {
        s = &s[2..];
    }
    let mut val = 0u32;
    for &b in s {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'f' => (b - b'a' + 10) as u32,
            b'A'..=b'F' => (b - b'A' + 10) as u32,
            _ => break,
        };
        val = (val << 4) | digit;
    }
    val
}

pub fn format_hex_u32(val: u32, buf: &mut [u8; 10]) -> &str {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        let shift = (7 - i) * 4;
        let nibble = ((val >> shift) & 0x0F) as usize;
        buf[2 + i] = HEX_CHARS[nibble];
    }
    core::str::from_utf8(buf).unwrap_or("0x00000000")
}

pub fn format_u32_dec(mut val: u32, buf: &mut [u8; 12]) -> &str {
    if val == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap_or("0");
    }
    let mut digits = [0u8; 12];
    let mut count = 0;
    while val > 0 && count < 12 {
        digits[count] = b'0' + (val % 10) as u8;
        val /= 10;
        count += 1;
    }
    for i in 0..count {
        buf[i] = digits[count - 1 - i];
    }
    core::str::from_utf8(&buf[..count]).unwrap_or("0")
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::MOCK_SAMPLE_CRASHDMP_TXT;

    #[test]
    fn test_parse_crash_text_metadata() {
        let rep = parse_crash_text(MOCK_SAMPLE_CRASHDMP_TXT);
        assert!(rep.has_crash);
        assert_eq!(rep.ticks, 344865);
        assert_eq!(rep.reason_str(), "selftest: synthetic page fault crash export");
        assert!(rep.has_regs);
        assert_eq!(rep.int_no, 14);
        assert_eq!(rep.err_code, 0x00000002);
        assert_eq!(rep.eip, 0x80481234);
        assert_eq!(rep.eax, 0x11223344);
        assert_eq!(rep.ebx, 0x04000000);
        assert_eq!(rep.ecx, 0x06000000);
        assert_eq!(rep.edx, 0x05000000);
        assert_eq!(rep.esi, 0x02000000);
        assert_eq!(rep.edi, 0x01000000);
        assert_eq!(rep.ebp, 0x03000000);
        assert_eq!(rep.esp, 0x07000000);
        assert!(rep.has_fault_addr);
        assert_eq!(rep.fault_addr, 0x014ab000);
        assert_eq!(rep.stack_count, 3);
        assert_eq!(rep.stack[0], 0x80481000);
        assert_eq!(rep.stack[1], 0x80482000);
        assert_eq!(rep.stack[2], 0x80483000);
    }

    #[test]
    fn test_vector_and_fault_analysis() {
        let rep = parse_crash_text(MOCK_SAMPLE_CRASHDMP_TXT);
        assert_eq!(rep.vector_title(), "Page Fault (#PF)");
        assert!(rep.vector_desc().contains("unmapped virtual memory"));
        assert!(rep.fault_analysis().contains("Kernel write to unmapped address"));
    }

    #[test]
    fn test_symbol_classification() {
        assert_eq!(ParsedCrashReport::symbol_for_address(0x80481234), "userland application code");
        assert_eq!(ParsedCrashReport::symbol_for_address(0x00105000), "kernel core init & scheduler");
        assert_eq!(ParsedCrashReport::symbol_for_address(0x00120000), "kernel memory & paging subsystem");
        assert_eq!(ParsedCrashReport::symbol_for_address(0x00140000), "kernel VFS & filesystem driver");
        assert_eq!(ParsedCrashReport::symbol_for_address(0x00180000), "kernel device driver / rust module");
    }

    #[test]
    fn test_hex_and_dec_formatting() {
        let mut hex_buf = [0u8; 10];
        assert_eq!(format_hex_u32(0x80481234, &mut hex_buf), "0x80481234");
        assert_eq!(format_hex_u32(0x00000002, &mut hex_buf), "0x00000002");

        let mut dec_buf = [0u8; 12];
        assert_eq!(format_u32_dec(344865, &mut dec_buf), "344865");
        assert_eq!(format_u32_dec(0, &mut dec_buf), "0");
    }
}
