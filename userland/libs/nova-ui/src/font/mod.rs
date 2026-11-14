//! userland/libs/nova-ui/src/font/mod.rs
//!
//! TrueType / OpenType Vector Typography Engine with Subpixel Antialiasing.

pub mod parser;
pub mod rasterizer;

pub use parser::{create_test_ttf_font, FontMetrics, GlyphOutline, OutlinePoint, ParsedFont};
pub use rasterizer::{
    synthesize_vector_glyph, CachedGlyph, FontEngine, GlyphAtlas, GlyphKey, SubpixelCoverage,
};
