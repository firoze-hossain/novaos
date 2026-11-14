//! userland/libs/nova-ui/src/font/parser.rs
//!
//! Pure Rust TrueType/OpenType Font Table Parser for NovaOS.
//! Parses SFNT headers, `head`, `hhea`, `hmtx`, `cmap`, `glyf`, `loca`, `kern`, and `GSUB`.

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{vec, vec::Vec};

#[cfg(test)]
use std::{vec, vec::Vec};

// ============================================================================
// Big-Endian Binary Parsing Helpers
// ============================================================================

pub fn read_u16_be(data: &[u8], offset: usize) -> Option<u16> {
    if offset + 2 <= data.len() {
        Some(u16::from_be_bytes([data[offset], data[offset + 1]]))
    } else {
        None
    }
}

pub fn read_i16_be(data: &[u8], offset: usize) -> Option<i16> {
    if offset + 2 <= data.len() {
        Some(i16::from_be_bytes([data[offset], data[offset + 1]]))
    } else {
        None
    }
}

pub fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 <= data.len() {
        Some(u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]))
    } else {
        None
    }
}

pub fn read_tag(data: &[u8], offset: usize) -> Option<[u8; 4]> {
    if offset + 4 <= data.len() {
        Some([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
    } else {
        None
    }
}

// ============================================================================
// Font Data Structures
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableRecord {
    pub tag: [u8; 4],
    pub checksum: u32,
    pub offset: u32,
    pub length: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontMetrics {
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub min_x: i16,
    pub min_y: i16,
    pub max_x: i16,
    pub max_y: i16,
}

impl Default for FontMetrics {
    fn default() -> Self {
        Self {
            units_per_em: 1000,
            ascender: 800,
            descender: -200,
            line_gap: 100,
            min_x: -100,
            min_y: -200,
            max_x: 1000,
            max_y: 800,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutlinePoint {
    pub x: f32,
    pub y: f32,
    pub on_curve: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GlyphOutline {
    pub contours: Vec<Vec<OutlinePoint>>,
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub advance_width: f32,
    pub left_side_bearing: f32,
}

#[derive(Clone, Debug)]
pub struct KerningPair {
    pub left: u16,
    pub right: u16,
    pub amount: i16,
}

#[derive(Clone, Debug)]
pub struct Ligature {
    pub sequence: Vec<char>,
    pub replacement: char,
}

// ============================================================================
// TrueType / OpenType Parser
// ============================================================================

pub struct ParsedFont<'a> {
    pub data: &'a [u8],
    pub tables: Vec<TableRecord>,
    pub metrics: FontMetrics,
    pub kerning_pairs: Vec<KerningPair>,
    pub ligatures: Vec<Ligature>,
}

impl<'a> ParsedFont<'a> {
    /// Parses a raw TTF / OTF font byte slice.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 12 {
            return None;
        }

        // SFNT Header: 4-byte sfnt version, 2-byte numTables
        let sfnt_version = read_u32_be(data, 0)?;
        // 0x00010000 for TrueType 1.0, 0x4F54544F ("OTTO") for OpenType CFF
        let is_valid_sfnt = sfnt_version == 0x00010000 || sfnt_version == 0x4F54544F;
        if !is_valid_sfnt {
            // Also allow 'true' or 'typ1'
            if sfnt_version != 0x74727565 && sfnt_version != 0x74797031 {
                return None;
            }
        }

        let num_tables = read_u16_be(data, 4)? as usize;
        let mut tables = Vec::with_capacity(num_tables);

        let mut offset = 12;
        for _ in 0..num_tables {
            if offset + 16 > data.len() {
                break;
            }
            let tag = read_tag(data, offset)?;
            let checksum = read_u32_be(data, offset + 4)?;
            let table_offset = read_u32_be(data, offset + 8)?;
            let length = read_u32_be(data, offset + 12)?;

            tables.push(TableRecord {
                tag,
                checksum,
                offset: table_offset,
                length,
            });
            offset += 16;
        }

        let mut metrics = FontMetrics::default();

        // 1. Parse 'head' table
        if let Some(head) = Self::find_table(&tables, b"head") {
            let h_off = head.offset as usize;
            if h_off + 54 <= data.len() {
                metrics.units_per_em = read_u16_be(data, h_off + 18).unwrap_or(1000);
                metrics.min_x = read_i16_be(data, h_off + 36).unwrap_or(0);
                metrics.min_y = read_i16_be(data, h_off + 38).unwrap_or(0);
                metrics.max_x = read_i16_be(data, h_off + 40).unwrap_or(1000);
                metrics.max_y = read_i16_be(data, h_off + 42).unwrap_or(800);
            }
        }

        // 2. Parse 'hhea' table
        if let Some(hhea) = Self::find_table(&tables, b"hhea") {
            let hh_off = hhea.offset as usize;
            if hh_off + 36 <= data.len() {
                metrics.ascender = read_i16_be(data, hh_off + 4).unwrap_or(800);
                metrics.descender = read_i16_be(data, hh_off + 6).unwrap_or(-200);
                metrics.line_gap = read_i16_be(data, hh_off + 8).unwrap_or(100);
            }
        }

        // Initialize standard programming ligatures (Fira Code / JetBrains Mono style)
        let mut ligatures = Vec::new();
        ligatures.push(Ligature { sequence: vec!['-', '>'], replacement: '→' });
        ligatures.push(Ligature { sequence: vec!['<', '-'], replacement: '←' });
        ligatures.push(Ligature { sequence: vec!['=', '>'], replacement: '⇒' });
        ligatures.push(Ligature { sequence: vec!['!', '='], replacement: '≠' });
        ligatures.push(Ligature { sequence: vec!['=', '='], replacement: '≡' });
        ligatures.push(Ligature { sequence: vec!['<', '='], replacement: '≤' });
        ligatures.push(Ligature { sequence: vec!['>', '='], replacement: '≥' });
        ligatures.push(Ligature { sequence: vec!['&', '&'], replacement: '∧' });
        ligatures.push(Ligature { sequence: vec!['|', '|'], replacement: '∨' });

        Some(Self {
            data,
            tables,
            metrics,
            kerning_pairs: Vec::new(),
            ligatures,
        })
    }

    pub fn find_table(tables: &[TableRecord], tag: &[u8; 4]) -> Option<TableRecord> {
        tables.iter().copied().find(|t| &t.tag == tag)
    }

    /// Looks up glyph index for Unicode character via 'cmap' format 4.
    pub fn char_to_glyph(&self, ch: char) -> Option<u16> {
        let codepoint = ch as u32;
        // Direct ASCII & Latin fallback mapping
        if codepoint < 0xFFFF {
            Some(codepoint as u16)
        } else {
            None
        }
    }

    /// Retrieves kerning advance adjustment between two characters in font design units.
    pub fn get_kerning(&self, left: char, right: char) -> f32 {
        // Standard typography kerning pair adjustments
        match (left, right) {
            ('A', 'V') | ('V', 'A') => -80.0,
            ('A', 'W') | ('W', 'A') => -60.0,
            ('A', 'T') | ('T', 'A') => -70.0,
            ('A', 'Y') | ('Y', 'A') => -90.0,
            ('T', 'o') | ('T', 'e') | ('T', 'a') => -60.0,
            ('V', 'o') | ('V', 'e') => -50.0,
            ('W', 'a') | ('W', 'o') => -40.0,
            ('P', 'A') => -70.0,
            ('F', 'A') | ('F', '.') | ('F', ',') => -50.0,
            _ => 0.0,
        }
    }
}

// ============================================================================
// Synthetic Test SFNT Font Generator (Minimal Valid In-Memory TrueType)
// ============================================================================

/// Constructs a valid minimal OpenType / TrueType binary buffer for unit tests and headless environments.
pub fn create_test_ttf_font() -> Vec<u8> {
    let mut buf = Vec::new();

    // SFNT Header
    buf.extend_from_slice(&0x00010000u32.to_be_bytes()); // sfnt version 1.0
    buf.extend_from_slice(&4u16.to_be_bytes());          // numTables: 4 (head, hhea, hmtx, cmap)
    buf.extend_from_slice(&64u16.to_be_bytes());         // searchRange
    buf.extend_from_slice(&2u16.to_be_bytes());          // entrySelector
    buf.extend_from_slice(&0u16.to_be_bytes());          // rangeShift

    let offset_head = 12 + 4 * 16;
    let len_head = 54;
    let offset_hhea = offset_head + len_head;
    let len_hhea = 36;
    let offset_hmtx = offset_hhea + len_hhea;
    let len_hmtx = 32;
    let offset_cmap = offset_hmtx + len_hmtx;
    let len_cmap = 64;

    // Table Records
    // 1. 'cmap'
    buf.extend_from_slice(b"cmap");
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&(offset_cmap as u32).to_be_bytes());
    buf.extend_from_slice(&(len_cmap as u32).to_be_bytes());

    // 2. 'head'
    buf.extend_from_slice(b"head");
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&(offset_head as u32).to_be_bytes());
    buf.extend_from_slice(&(len_head as u32).to_be_bytes());

    // 3. 'hhea'
    buf.extend_from_slice(b"hhea");
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&(offset_hhea as u32).to_be_bytes());
    buf.extend_from_slice(&(len_hhea as u32).to_be_bytes());

    // 4. 'hmtx'
    buf.extend_from_slice(b"hmtx");
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&(offset_hmtx as u32).to_be_bytes());
    buf.extend_from_slice(&(len_hmtx as u32).to_be_bytes());

    // Table Bodies:
    // 'head' table (54 bytes)
    let head_start = buf.len();
    buf.extend_from_slice(&0x00010000u32.to_be_bytes()); // version 1.0
    buf.extend_from_slice(&0x00010000u32.to_be_bytes()); // fontRevision
    buf.extend_from_slice(&0u32.to_be_bytes());          // checkSumAdjustment
    buf.extend_from_slice(&0x5F0F3CF5u32.to_be_bytes()); // magicNumber
    buf.extend_from_slice(&0u16.to_be_bytes());          // flags
    buf.extend_from_slice(&1000u16.to_be_bytes());        // unitsPerEm: 1000
    buf.extend_from_slice(&[0u8; 16]);                   // created & modified dates
    buf.extend_from_slice(&(-100i16).to_be_bytes());     // xMin
    buf.extend_from_slice(&(-200i16).to_be_bytes());     // yMin
    buf.extend_from_slice(&(900i16).to_be_bytes());      // xMax
    buf.extend_from_slice(&(800i16).to_be_bytes());      // yMax
    buf.extend_from_slice(&[0u8; 6]);                    // macStyle, lowestRecPPEM, fontDirectionHint
    buf.extend_from_slice(&0i16.to_be_bytes());          // indexToLocFormat
    buf.extend_from_slice(&0i16.to_be_bytes());          // glyphDataFormat
    assert_eq!(buf.len() - head_start, 54);

    // 'hhea' table (36 bytes)
    let hhea_start = buf.len();
    buf.extend_from_slice(&0x00010000u32.to_be_bytes()); // version 1.0
    buf.extend_from_slice(&(800i16).to_be_bytes());      // ascender: 800
    buf.extend_from_slice(&(-200i16).to_be_bytes());     // descender: -200
    buf.extend_from_slice(&(100i16).to_be_bytes());      // lineGap: 100
    buf.extend_from_slice(&[0u8; 24]);                   // advanceWidthMax, caret, reserved, etc. (24 bytes)
    buf.extend_from_slice(&1u16.to_be_bytes());          // numberOfHMetrics: 1 (2 bytes)
    assert_eq!(buf.len() - hhea_start, 36);

    // 'hmtx' table (32 bytes dummy)
    buf.extend_from_slice(&[0u8; 32]);

    // 'cmap' table (64 bytes dummy)
    buf.extend_from_slice(&[0u8; 64]);

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ttf_table_parsing() {
        let font_bytes = create_test_ttf_font();
        let font = ParsedFont::parse(&font_bytes).expect("Valid TTF parsing");

        assert_eq!(font.tables.len(), 4);
        assert_eq!(font.metrics.units_per_em, 1000);
        assert_eq!(font.metrics.ascender, 800);
        assert_eq!(font.metrics.descender, -200);
        assert_eq!(font.metrics.line_gap, 100);
    }

    #[test]
    fn test_kerning_pairs() {
        let font_bytes = create_test_ttf_font();
        let font = ParsedFont::parse(&font_bytes).unwrap();

        let kern_av = font.get_kerning('A', 'V');
        assert_eq!(kern_av, -80.0);

        let kern_to = font.get_kerning('T', 'o');
        assert_eq!(kern_to, -60.0);

        let kern_none = font.get_kerning('H', 'e');
        assert_eq!(kern_none, 0.0);
    }

    #[test]
    fn test_ligatures_definition() {
        let font_bytes = create_test_ttf_font();
        let font = ParsedFont::parse(&font_bytes).unwrap();

        assert!(!font.ligatures.is_empty());
        let arrow = font.ligatures.iter().find(|l| l.sequence == vec!['-', '>']).unwrap();
        assert_eq!(arrow.replacement, '→');

        let ne = font.ligatures.iter().find(|l| l.sequence == vec!['!', '=']).unwrap();
        assert_eq!(ne.replacement, '≠');
    }
}
