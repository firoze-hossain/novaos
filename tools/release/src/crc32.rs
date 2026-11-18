//! Standard IEEE 802.3 CRC32 calculation (used for GPT header and partition table validation)

pub struct Hasher {
    state: u32,
}

impl Hasher {
    pub fn new() -> Self {
        Hasher { state: 0xFFFF_FFFF }
    }

    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let mut c = (self.state ^ (byte as u32)) & 0xFF;
            for _ in 0..8 {
                if c & 1 != 0 {
                    c = 0xEDB8_8320 ^ (c >> 1);
                } else {
                    c >>= 1;
                }
            }
            self.state = c ^ (self.state >> 8);
        }
    }

    pub fn finalize(self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}

pub fn checksum(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32_empty() {
        assert_eq!(checksum(b""), 0);
    }

    #[test]
    fn test_crc32_known_vector() {
        assert_eq!(checksum(b"123456789"), 0xCBF43926);
    }
}
