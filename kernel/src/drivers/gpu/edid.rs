//! kernel/src/drivers/gpu/edid.rs
//!
//! NovaOS High-Resolution Display Scaling: VESA EDID 1.3/1.4 Parser & Monitor Profiler
//!
//! Provides comprehensive EDID decoding for the DRM/KMS subsystem:
//!   - 128-byte binary EDID header validation and checksum verification.
//!   - Vendor ID decompression (compressed 5-bit ASCII).
//!   - Physical image dimensions (millimeter accuracy from Detailed Timing Descriptors).
//!   - Native resolution and refresh rate extraction.
//!   - Monitor model name decoding (Descriptor Tag 0xFC).
//!   - Hardware physical DPI calculation (horizontal, vertical, diagonal).
//!   - Automatic display scale recommendation (100%, 125%, 150%, 200%).
//!   - Synthetic EDID generator for hardware emulation and regression testing.

#![allow(dead_code)]

pub const EDID_BLOCK_SIZE: usize = 128;
pub const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

// -----------------------------------------------------------------------------
// Display Scale Recommendation
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum RecommendedScale {
    #[default]
    Scale100, // 1.0x  (100% - standard 96 DPI)
    Scale125, // 1.25x (125% - 120 DPI)
    Scale150, // 1.5x  (150% - 144 DPI)
    Scale200, // 2.0x  (200% - 192 DPI Retina/4K)
}

impl RecommendedScale {
    pub fn factor_permille(&self) -> u32 {
        match self {
            RecommendedScale::Scale100 => 1000,
            RecommendedScale::Scale125 => 1250,
            RecommendedScale::Scale150 => 1500,
            RecommendedScale::Scale200 => 2000,
        }
    }

    pub fn factor_f32(&self) -> f32 {
        match self {
            RecommendedScale::Scale100 => 1.0,
            RecommendedScale::Scale125 => 1.25,
            RecommendedScale::Scale150 => 1.5,
            RecommendedScale::Scale200 => 2.0,
        }
    }

    pub fn percent(&self) -> u32 {
        match self {
            RecommendedScale::Scale100 => 100,
            RecommendedScale::Scale125 => 125,
            RecommendedScale::Scale150 => 150,
            RecommendedScale::Scale200 => 200,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RecommendedScale::Scale100 => "100%",
            RecommendedScale::Scale125 => "125%",
            RecommendedScale::Scale150 => "150%",
            RecommendedScale::Scale200 => "200% (Retina)",
        }
    }
}

// -----------------------------------------------------------------------------
// Parsed EDID Information
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct EdidInfo {
    /// 3-letter manufacturer ID (e.g. "DEL", "NOV", "SAM", "APP")
    pub manufacturer: [u8; 3],
    pub product_code: u16,
    pub serial_number: u32,
    pub manufacture_week: u8,
    pub manufacture_year: u16,
    pub edid_version: u8,
    pub edid_revision: u8,
    pub is_digital: bool,
    /// Physical width in millimeters (from detailed timing descriptor or basic params)
    pub width_mm: u32,
    /// Physical height in millimeters
    pub height_mm: u32,
    /// Native horizontal resolution in pixels
    pub native_width: u32,
    /// Native vertical resolution in pixels
    pub native_height: u32,
    /// Pixel clock in kHz
    pub pixel_clock_khz: u32,
    /// Refresh rate in Hz
    pub refresh_rate_hz: u32,
    /// Monitor model name string (ASCII null-padded)
    pub model_name: [u8; 14],
    pub model_name_len: usize,
    /// Horizontal DPI
    pub dpi_x: u32,
    /// Vertical DPI
    pub dpi_y: u32,
    /// Mean physical DPI
    pub dpi: u32,
    /// Optimal UI scale factor
    pub recommended_scale: RecommendedScale,
}

impl Default for EdidInfo {
    fn default() -> Self {
        Self {
            manufacturer: [b'N', b'O', b'V'],
            product_code: 0x2001,
            serial_number: 1,
            manufacture_week: 1,
            manufacture_year: 2026,
            edid_version: 1,
            edid_revision: 4,
            is_digital: true,
            width_mm: 531,
            height_mm: 298,
            native_width: 1920,
            native_height: 1080,
            pixel_clock_khz: 148500,
            refresh_rate_hz: 60,
            model_name: *b"NovaDisplay   ",
            model_name_len: 11,
            dpi_x: 92,
            dpi_y: 92,
            dpi: 92,
            recommended_scale: RecommendedScale::Scale100,
        }
    }
}

// -----------------------------------------------------------------------------
// EDID Parser Implementation
// -----------------------------------------------------------------------------

pub struct EdidParser;

impl EdidParser {
    /// Validates the 8-byte magic header of an EDID block
    pub fn is_valid_header(raw: &[u8]) -> bool {
        if raw.len() < 8 {
            return false;
        }
        raw[0..8] == EDID_HEADER
    }

    /// Verifies the 8-bit checksum: sum of all 128 bytes % 256 must equal 0
    pub fn is_valid_checksum(raw: &[u8]) -> bool {
        if raw.len() < EDID_BLOCK_SIZE {
            return false;
        }
        let mut sum: u8 = 0;
        for i in 0..EDID_BLOCK_SIZE {
            sum = sum.wrapping_add(raw[i]);
        }
        sum == 0
    }

    /// Calculates the 8-bit checksum byte required to make the sum % 256 == 0
    pub fn compute_checksum(raw: &[u8]) -> u8 {
        let mut sum: u8 = 0;
        for i in 0..(EDID_BLOCK_SIZE - 1) {
            sum = sum.wrapping_add(raw[i]);
        }
        (256u16.wrapping_sub(sum as u16) & 0xFF) as u8
    }

    /// Decodes compressed 5-bit ASCII manufacturer ID from bytes 8..9
    pub fn decode_manufacturer(byte0: u8, byte1: u8) -> [u8; 3] {
        let c1 = ((byte0 >> 2) & 0x1F) + b'@';
        let c2 = (((byte0 & 0x03) << 3) | ((byte1 >> 5) & 0x07)) + b'@';
        let c3 = (byte1 & 0x1F) + b'@';
        [c1, c2, c3]
    }

    /// Encodes 3-letter ASCII manufacturer ID into 2 compressed EDID bytes
    pub fn encode_manufacturer(mfg: &[u8; 3]) -> (u8, u8) {
        let c1 = (mfg[0].to_ascii_uppercase().wrapping_sub(b'@') & 0x1F) as u16;
        let c2 = (mfg[1].to_ascii_uppercase().wrapping_sub(b'@') & 0x1F) as u16;
        let c3 = (mfg[2].to_ascii_uppercase().wrapping_sub(b'@') & 0x1F) as u16;

        let combined = (c1 << 10) | (c2 << 5) | c3;
        let byte0 = ((combined >> 8) & 0xFF) as u8;
        let byte1 = (combined & 0xFF) as u8;
        (byte0, byte1)
    }

    /// Calculates DPI from physical dimension in mm and pixel resolution:
    /// DPI = (pixels / mm) * 25.4 = (pixels * 254) / (mm * 10)
    pub fn calculate_dpi(pixels: u32, mm: u32) -> u32 {
        if mm == 0 || pixels == 0 {
            return 96; // Standard fallback baseline
        }
        ((pixels as u64 * 2540) / (mm as u64 * 100)) as u32
    }

    /// Recommends UI scale factor based on physical DPI
    pub fn recommend_scale(dpi: u32) -> RecommendedScale {
        if dpi >= 180 {
            RecommendedScale::Scale200 // 4K / Retina displays
        } else if dpi >= 135 {
            RecommendedScale::Scale150 // 1440p laptops or high-density 24" 1440p
        } else if dpi >= 110 {
            RecommendedScale::Scale125 // Standard 1440p 27" or 1080p 15" laptops
        } else {
            RecommendedScale::Scale100 // Standard 1080p 24" monitors (92 DPI)
        }
    }

    /// Parses a 128-byte raw EDID block into structured `EdidInfo`
    pub fn parse(raw: &[u8]) -> Result<EdidInfo, &'static str> {
        if raw.len() < EDID_BLOCK_SIZE {
            return Err("EDID buffer underflow: minimum 128 bytes required");
        }
        if !Self::is_valid_header(raw) {
            return Err("Invalid EDID header magic");
        }
        if !Self::is_valid_checksum(raw) {
            return Err("EDID checksum mismatch");
        }

        let mfg = Self::decode_manufacturer(raw[8], raw[9]);
        let prod_code = (raw[10] as u16) | ((raw[11] as u16) << 8);
        let serial = (raw[12] as u32)
            | ((raw[13] as u32) << 8)
            | ((raw[14] as u32) << 16)
            | ((raw[15] as u32) << 24);
        let week = raw[16];
        let year = 1990 + raw[17] as u16;
        let edid_ver = raw[18];
        let edid_rev = raw[19];

        // Basic display parameters
        let is_digital = (raw[20] & 0x80) != 0;
        let basic_w_cm = raw[21] as u32;
        let basic_h_cm = raw[22] as u32;

        let mut width_mm = basic_w_cm * 10;
        let mut height_mm = basic_h_cm * 10;

        let mut native_w = 0u32;
        let mut native_h = 0u32;
        let mut pixel_clock_khz = 0u32;
        let mut refresh_rate = 60u32;

        let mut model_name = [0u8; 14];
        let mut model_name_len = 0usize;

        // Parse 4 Detailed Timing / Monitor Descriptors (Bytes 54..125, each 18 bytes)
        for d in 0..4 {
            let offset = 54 + d * 18;
            let desc = &raw[offset..offset + 18];
            let pclk = (desc[0] as u32) | ((desc[1] as u32) << 8);

            if pclk != 0 {
                // Detailed Timing Descriptor
                if native_w == 0 {
                    let h_active = (desc[2] as u32) | (((desc[4] >> 4) as u32 & 0x0F) << 8);
                    let h_blank = (desc[3] as u32) | ((desc[4] as u32 & 0x0F) << 8);
                    let v_active = (desc[5] as u32) | (((desc[7] >> 4) as u32 & 0x0F) << 8);
                    let v_blank = (desc[6] as u32) | ((desc[7] as u32 & 0x0F) << 8);

                    let d_w_mm = (desc[12] as u32) | (((desc[14] >> 4) as u32 & 0x0F) << 8);
                    let d_h_mm = (desc[13] as u32) | ((desc[14] as u32 & 0x0F) << 8);

                    native_w = h_active;
                    native_h = v_active;
                    pixel_clock_khz = pclk * 10;

                    if d_w_mm > 0 && d_h_mm > 0 {
                        width_mm = d_w_mm;
                        height_mm = d_h_mm;
                    }

                    let h_total = h_active + h_blank;
                    let v_total = v_active + v_blank;
                    if h_total > 0 && v_total > 0 {
                        let total_pixels = h_total as u64 * v_total as u64;
                        if total_pixels > 0 {
                            refresh_rate = ((pixel_clock_khz as u64 * 1000) / total_pixels) as u32;
                        }
                    }
                }
            } else if desc[2] == 0x00 {
                // Monitor Descriptor
                let tag = desc[3];
                if tag == 0xFC {
                    // Monitor Name string (Bytes 5..17)
                    let mut len = 0usize;
                    for k in 0..13 {
                        let ch = desc[5 + k];
                        if ch == 0x0A || ch == 0x00 {
                            break;
                        }
                        if len < 13 {
                            model_name[len] = ch;
                            len += 1;
                        }
                    }
                    model_name_len = len;
                }
            }
        }

        // Fallbacks if detailed descriptor was empty
        if native_w == 0 {
            native_w = 1920;
            native_h = 1080;
        }
        if width_mm == 0 {
            width_mm = 531;
        }
        if height_mm == 0 {
            height_mm = 298;
        }
        if model_name_len == 0 {
            let def = b"NovaDisplay";
            model_name[..def.len()].copy_from_slice(def);
            model_name_len = def.len();
        }

        let dpi_x = Self::calculate_dpi(native_w, width_mm);
        let dpi_y = Self::calculate_dpi(native_h, height_mm);
        let dpi = (dpi_x + dpi_y) / 2;
        let recommended_scale = Self::recommend_scale(dpi);

        Ok(EdidInfo {
            manufacturer: mfg,
            product_code: prod_code,
            serial_number: serial,
            manufacture_week: week,
            manufacture_year: year,
            edid_version: edid_ver,
            edid_revision: edid_rev,
            is_digital,
            width_mm,
            height_mm,
            native_width: native_w,
            native_height: native_h,
            pixel_clock_khz,
            refresh_rate_hz: refresh_rate,
            model_name,
            model_name_len,
            dpi_x,
            dpi_y,
            dpi,
            recommended_scale,
        })
    }
}

// -----------------------------------------------------------------------------
// Synthetic EDID Generator (for testing and virtualization)
// -----------------------------------------------------------------------------

pub struct EdidBuilder {
    raw: [u8; EDID_BLOCK_SIZE],
}

impl EdidBuilder {
    pub fn new(mfg: &[u8; 3], prod_code: u16, model_name: &str) -> Self {
        let mut raw = [0u8; EDID_BLOCK_SIZE];
        // 1. Header
        raw[0..8].copy_from_slice(&EDID_HEADER);

        // 2. Vendor ID
        let (b0, b1) = EdidParser::encode_manufacturer(mfg);
        raw[8] = b0;
        raw[9] = b1;

        // 3. Product code & Serial
        raw[10] = (prod_code & 0xFF) as u8;
        raw[11] = ((prod_code >> 8) & 0xFF) as u8;
        raw[12] = 0x01; // Serial 1
        raw[16] = 20;   // Week 20
        raw[17] = 36;   // Year 2026 (1990 + 36)

        // 4. EDID 1.4 Digital
        raw[18] = 1;
        raw[19] = 4;
        raw[20] = 0xA5; // Digital, 8-bit, DisplayPort

        // 5. Monitor Name Descriptor at Descriptor #1 (offset 54 + 18 = 72)
        raw[72] = 0x00;
        raw[73] = 0x00;
        raw[74] = 0x00;
        raw[75] = 0xFC; // Monitor name tag
        raw[76] = 0x00;

        let name_bytes = model_name.as_bytes();
        let copy_len = name_bytes.len().min(13);
        raw[77..77 + copy_len].copy_from_slice(&name_bytes[..copy_len]);
        if copy_len < 13 {
            raw[77 + copy_len] = 0x0A; // Terminating newline
        }

        Self { raw }
    }

    /// Sets detailed timing and physical millimeter dimensions
    pub fn set_timing_and_dimensions(
        mut self,
        width_px: u32,
        height_px: u32,
        width_mm: u32,
        height_mm: u32,
        refresh_hz: u32,
    ) -> Self {
        // Set basic screen dimensions in cm
        self.raw[21] = ((width_mm + 5) / 10).min(255) as u8;
        self.raw[22] = ((height_mm + 5) / 10).min(255) as u8;

        // Populate Descriptor #0 (offset 54) as Detailed Timing Descriptor
        let h_active = width_px;
        let h_blank = (width_px * 15 / 100).max(80); // ~15% blanking
        let v_active = height_px;
        let v_blank = (height_px * 5 / 100).max(30);   // ~5% blanking

        let h_total = h_active + h_blank;
        let v_total = v_active + v_blank;
        let pclk_khz = ((h_total as u64 * v_total as u64 * refresh_hz as u64) / 1000) as u32;
        let pclk_10k = (pclk_khz / 10) as u16;

        let desc = &mut self.raw[54..72];
        desc[0] = (pclk_10k & 0xFF) as u8;
        desc[1] = ((pclk_10k >> 8) & 0xFF) as u8;

        desc[2] = (h_active & 0xFF) as u8;
        desc[3] = (h_blank & 0xFF) as u8;
        desc[4] = (((h_active >> 8) & 0x0F) << 4) as u8 | (((h_blank >> 8) & 0x0F) as u8);

        desc[5] = (v_active & 0xFF) as u8;
        desc[6] = (v_blank & 0xFF) as u8;
        desc[7] = (((v_active >> 8) & 0x0F) << 4) as u8 | (((v_blank >> 8) & 0x0F) as u8);

        // Sync pulses
        desc[8] = 48; // h_sync_offset
        desc[9] = 32; // h_sync_width
        desc[10] = 0x30;
        desc[11] = 0x00;

        // Image size in mm
        desc[12] = (width_mm & 0xFF) as u8;
        desc[13] = (height_mm & 0xFF) as u8;
        desc[14] = (((width_mm >> 8) & 0x0F) << 4) as u8 | (((height_mm >> 8) & 0x0F) as u8);

        desc[17] = 0x1E; // Progressive, digital separate sync

        self
    }

    /// Finalizes the EDID block by calculating and appending the checksum byte
    pub fn build(mut self) -> [u8; EDID_BLOCK_SIZE] {
        let chk = EdidParser::compute_checksum(&self.raw);
        self.raw[127] = chk;
        self.raw
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edid_manufacturer_encoding() {
        let del = [b'D', b'E', b'L'];
        let (b0, b1) = EdidParser::encode_manufacturer(&del);
        let decoded = EdidParser::decode_manufacturer(b0, b1);
        assert_eq!(decoded, del);

        let nov = [b'N', b'O', b'V'];
        let (nb0, nb1) = EdidParser::encode_manufacturer(&nov);
        let decoded_nov = EdidParser::decode_manufacturer(nb0, nb1);
        assert_eq!(decoded_nov, nov);
    }

    #[test]
    fn test_edid_4k_retina_parsing() {
        // 13.3" 4K Laptop Display: 3840x2160, 294mm x 165mm -> ~331 DPI -> Scale200
        let edid_bytes = EdidBuilder::new(b"NOV", 0x4001, "NovaRetina 4K")
            .set_timing_and_dimensions(3840, 2160, 294, 165, 60)
            .build();

        assert!(EdidParser::is_valid_header(&edid_bytes));
        assert!(EdidParser::is_valid_checksum(&edid_bytes));

        let info = EdidParser::parse(&edid_bytes).expect("Valid EDID parse");
        assert_eq!(&info.manufacturer, b"NOV");
        assert_eq!(info.native_width, 3840);
        assert_eq!(info.native_height, 2160);
        assert_eq!(info.width_mm, 294);
        assert_eq!(info.height_mm, 165);
        assert!(info.dpi >= 300, "DPI must be > 300 for 13.3-inch 4K");
        assert_eq!(info.recommended_scale, RecommendedScale::Scale200);

        let name = core::str::from_utf8(&info.model_name[..info.model_name_len]).unwrap();
        assert_eq!(name, "NovaRetina 4K");
    }

    #[test]
    fn test_edid_1080p_standard_parsing() {
        // 24" 1080p Standard Display: 1920x1080, 531mm x 298mm -> ~92 DPI -> Scale100
        let edid_bytes = EdidBuilder::new(b"DEL", 0x1080, "Dell U2415")
            .set_timing_and_dimensions(1920, 1080, 531, 298, 60)
            .build();

        let info = EdidParser::parse(&edid_bytes).expect("Valid EDID parse");
        assert_eq!(&info.manufacturer, b"DEL");
        assert_eq!(info.native_width, 1920);
        assert_eq!(info.native_height, 1080);
        assert!(info.dpi >= 90 && info.dpi <= 95);
        assert_eq!(info.recommended_scale, RecommendedScale::Scale100);
    }

    #[test]
    fn test_edid_1440p_scaling() {
        // 27" 1440p QHD: 2560x1440, 597mm x 336mm -> ~109 DPI -> Scale100 or 125
        let edid_bytes = EdidBuilder::new(b"SAM", 0x2560, "Samsung S27")
            .set_timing_and_dimensions(2560, 1440, 597, 336, 60)
            .build();

        let info = EdidParser::parse(&edid_bytes).expect("Valid EDID parse");
        assert_eq!(info.native_width, 2560);
        assert_eq!(info.native_height, 1440);

        // 14" 1080p Laptop: 1920x1080, 309mm x 174mm -> ~157 DPI -> Scale150
        let edid_laptop = EdidBuilder::new(b"LEN", 0x1400, "ThinkPad 14")
            .set_timing_and_dimensions(1920, 1080, 309, 174, 60)
            .build();
        let info_laptop = EdidParser::parse(&edid_laptop).expect("Valid EDID parse");
        assert_eq!(info_laptop.recommended_scale, RecommendedScale::Scale150);
    }
}
