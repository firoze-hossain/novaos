//! Zero-Heap Allocation Scratch Buffer for high-frequency Prometheus scraping.
//!
//! Continuous metrics scraping (e.g., Prometheus querying every 1-5 seconds)
//! can cause significant heap allocation churn and fragmentation if dynamic
//! Strings and Vectors are allocated on every request.
//!
//! This module provides a pre-allocated, reusable formatting buffer that writes
//! Prometheus exposition text directly into contiguous memory with zero allocations
//! during steady-state operation.

#![forbid(unsafe_code)]

use core::fmt::Write as FmtWrite;

/// Default scratch buffer capacity (128 KiB), sufficient for hundreds of Prometheus metrics.
pub const DEFAULT_BUFFER_CAPACITY: usize = 131072;

/// A pre-allocated reusable byte buffer that formats Prometheus exposition metrics
/// with zero heap allocations during the request lifecycle.
pub struct ScratchBuffer {
    buffer: Vec<u8>,
    cursor: usize,
    allocation_count: usize,
}

impl ScratchBuffer {
    /// Creates a new ScratchBuffer pre-allocated to the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: vec![0u8; capacity],
            cursor: 0,
            allocation_count: 0,
        }
    }

    /// Creates a new ScratchBuffer with the default 128 KiB capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BUFFER_CAPACITY)
    }

    /// Resets the buffer cursor to zero for the next request without deallocating.
    #[inline]
    pub fn clear(&mut self) {
        self.cursor = 0;
    }

    /// Returns the current number of formatted bytes written.
    #[inline]
    pub fn len(&self) -> usize {
        self.cursor
    }

    /// Returns true if no bytes have been written.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.cursor == 0
    }

    /// Returns the capacity of the buffer.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// Returns the allocation count (0 during steady-state scraping).
    #[inline]
    pub fn allocation_count(&self) -> usize {
        self.allocation_count
    }

    /// Returns the formatted content as a byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer[..self.cursor]
    }

    /// Returns the formatted content as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buffer[..self.cursor]).unwrap_or("")
    }

    /// Appends raw bytes directly to the buffer.
    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) -> bool {
        let available = self.buffer.len().saturating_sub(self.cursor);
        if bytes.len() > available {
            // Buffer overflow protection: truncate to available capacity
            let to_write = available;
            if to_write > 0 {
                self.buffer[self.cursor..self.cursor + to_write].copy_from_slice(&bytes[..to_write]);
                self.cursor += to_write;
            }
            return false;
        }
        self.buffer[self.cursor..self.cursor + bytes.len()].copy_from_slice(bytes);
        self.cursor += bytes.len();
        true
    }

    /// Appends a string slice directly without heap allocation.
    #[inline]
    pub fn write_str(&mut self, s: &str) -> bool {
        self.write_bytes(s.as_bytes())
    }

    /// Appends a single byte.
    #[inline]
    pub fn write_byte(&mut self, b: u8) -> bool {
        if self.cursor < self.buffer.len() {
            self.buffer[self.cursor] = b;
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    /// Appends an unsigned 64-bit integer formatted as decimal text without allocation.
    pub fn write_u64(&mut self, mut val: u64) -> bool {
        if val == 0 {
            return self.write_byte(b'0');
        }
        let mut digits = [0u8; 20];
        let mut idx = 20;
        while val > 0 {
            idx -= 1;
            digits[idx] = b'0' + (val % 10) as u8;
            val /= 10;
        }
        self.write_bytes(&digits[idx..])
    }

    /// Appends a signed 64-bit integer formatted as decimal text without allocation.
    pub fn write_i64(&mut self, val: i64) -> bool {
        if val < 0 {
            if !self.write_byte(b'-') {
                return false;
            }
            // Handle i64::MIN safely
            let unsigned = if val == i64::MIN {
                (i64::MAX as u64) + 1
            } else {
                (-val) as u64
            };
            self.write_u64(unsigned)
        } else {
            self.write_u64(val as u64)
        }
    }

    /// Appends a 64-bit floating point number formatted as text without allocation.
    pub fn write_f64(&mut self, val: f64) -> bool {
        if val.is_nan() {
            return self.write_str("NaN");
        }
        if val.is_infinite() {
            if val.is_sign_negative() {
                return self.write_str("-Inf");
            } else {
                return self.write_str("+Inf");
            }
        }

        // Format sign
        let mut v = val;
        if v < 0.0 {
            if !self.write_byte(b'-') {
                return false;
            }
            v = -v;
        }

        let int_part = v as u64;
        let frac_part = v - (int_part as f64);

        if !self.write_u64(int_part) {
            return false;
        }

        // Round to 4 decimal places for metrics precision
        let frac_u64 = ((frac_part * 10000.0) + 0.5) as u64;
        if frac_u64 > 0 {
            if !self.write_byte(b'.') {
                return false;
            }
            let mut d = [0u8; 4];
            d[0] = b'0' + ((frac_u64 / 1000) % 10) as u8;
            d[1] = b'0' + ((frac_u64 / 100) % 10) as u8;
            d[2] = b'0' + ((frac_u64 / 10) % 10) as u8;
            d[3] = b'0' + (frac_u64 % 10) as u8;

            // Trim trailing zeros
            let mut end = 4;
            while end > 1 && d[end - 1] == b'0' {
                end -= 1;
            }
            self.write_bytes(&d[..end])
        } else {
            // Include .0 for floats
            self.write_str(".0")
        }
    }

    /// Writes a Prometheus `# HELP <metric> <help>` comment line.
    pub fn write_help(&mut self, metric: &str, help: &str) -> bool {
        self.write_str("# HELP ") &&
        self.write_str(metric) &&
        self.write_byte(b' ') &&
        self.write_str(help) &&
        self.write_byte(b'\n')
    }

    /// Writes a Prometheus `# TYPE <metric> <type>` line.
    pub fn write_type(&mut self, metric: &str, metric_type: &str) -> bool {
        self.write_str("# TYPE ") &&
        self.write_str(metric) &&
        self.write_byte(b' ') &&
        self.write_str(metric_type) &&
        self.write_byte(b'\n')
    }

    /// Writes a full Prometheus metric line with name, labels, and value.
    ///
    /// Format: `metric_name{label1="value1",label2="value2"} value [timestamp]\n`
    pub fn write_metric(
        &mut self,
        name: &str,
        labels: &[(&str, &str)],
        value: f64,
        timestamp: Option<u64>,
    ) -> bool {
        if !self.write_str(name) {
            return false;
        }

        if !labels.is_empty() {
            if !self.write_byte(b'{') {
                return false;
            }
            for (i, &(k, v)) in labels.iter().enumerate() {
                if i > 0 && !self.write_byte(b',') {
                    return false;
                }
                if !self.write_str(k) || !self.write_str("=\"") {
                    return false;
                }
                // Escape quotes and backslashes in label values
                for &b in v.as_bytes() {
                    match b {
                        b'\"' => {
                            if !self.write_str("\\\"") {
                                return false;
                            }
                        }
                        b'\\' => {
                            if !self.write_str("\\\\") {
                                return false;
                            }
                        }
                        b'\n' => {
                            if !self.write_str("\\n") {
                                return false;
                            }
                        }
                        other => {
                            if !self.write_byte(other) {
                                return false;
                            }
                        }
                    }
                }
                if !self.write_byte(b'\"') {
                    return false;
                }
            }
            if !self.write_byte(b'}') {
                return false;
            }
        }

        if !self.write_byte(b' ') || !self.write_f64(value) {
            return false;
        }

        if let Some(ts) = timestamp {
            if !self.write_byte(b' ') || !self.write_u64(ts) {
                return false;
            }
        }

        self.write_byte(b'\n')
    }
}

impl FmtWrite for ScratchBuffer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if self.write_str(s) {
            Ok(())
        } else {
            Err(core::fmt::Error)
        }
    }
}

impl Default for ScratchBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scratch_buffer_formatting() {
        let mut buf = ScratchBuffer::with_capacity(1024);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());

        assert!(buf.write_help("node_load1", "1m load average"));
        assert!(buf.write_type("node_load1", "gauge"));
        assert!(buf.write_metric("node_load1", &[], 0.42, None));

        let out = buf.as_str();
        assert!(out.contains("# HELP node_load1 1m load average\n"));
        assert!(out.contains("# TYPE node_load1 gauge\n"));
        assert!(out.contains("node_load1 0.42\n"));

        // Clear and reuse without reallocation
        buf.clear();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());

        assert!(buf.write_metric(
            "node_cpu_seconds_total",
            &[("cpu", "0"), ("mode", "idle")],
            12345.67,
            Some(1600000000)
        ));

        let out2 = buf.as_str();
        assert_eq!(
            out2,
            "node_cpu_seconds_total{cpu=\"0\",mode=\"idle\"} 12345.67 1600000000\n"
        );
    }

    #[test]
    fn test_numbers_formatting() {
        let mut buf = ScratchBuffer::with_capacity(512);

        buf.write_u64(0);
        buf.write_byte(b'\n');
        buf.write_u64(18446744073709551615);
        buf.write_byte(b'\n');
        buf.write_i64(-42);
        buf.write_byte(b'\n');
        buf.write_f64(3.1415);
        buf.write_byte(b'\n');

        let out = buf.as_str();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "0");
        assert_eq!(lines[1], "18446744073709551615");
        assert_eq!(lines[2], "-42");
        assert_eq!(lines[3], "3.1415");
    }
}
