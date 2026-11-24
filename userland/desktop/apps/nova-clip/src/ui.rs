//! userland/desktop/apps/nova-clip/src/ui.rs
//!
//! Visual layout, 2D acrylic rasterization, and popover preview components for NovaOS Clipboard.

pub use mnemos_ui::draw::{OwnedBufferSurface, Surface};
pub use mnemos_ui::font::{draw_char, draw_text, draw_text_small, Font};
pub use mnemos_ui::geom::{Color, Point, Rect};
pub use mnemos_ui::views::{CARD_ROW_HEIGHT, CARD_WIDTH, PREVIEW_HEIGHT, PREVIEW_WIDTH, MnemosRenderer};
