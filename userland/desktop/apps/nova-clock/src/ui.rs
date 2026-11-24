//! userland/desktop/apps/nova-clock/src/ui.rs
//!
//! Visual layout, 2D software rasterization, and interactive components for NovaOS Clock.

pub use chronos_ui::geom::{Color, Point, Rect};
pub use chronos_ui::draw::{BufferSurface32, Surface};
pub use chronos_ui::font::{draw_large_digital, draw_text, draw_text_font, draw_text_small, Font};
pub use chronos_ui::map::WorldMapWidget;
pub use chronos_ui::views::{ChronosView, HEADER_HEIGHT};
pub use chronos_ui::handle_click;
