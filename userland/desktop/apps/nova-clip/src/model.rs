//! userland/desktop/apps/nova-clip/src/model.rs
//!
//! Domain models, deduplication, search ranking, and hub coordinator for NovaOS Clipboard.

pub use mnemos_core::config::{MnemosConfig, PopupPosition, ThemeMode};
pub use mnemos_core::history::ClipHistory;
pub use mnemos_core::hub::MnemosHub;
pub use mnemos_core::item::{ClipItem, ItemKind};
pub use mnemos_core::search::{rank_items, FuzzyMatcher};
pub use mnemos_platform::{create_native_clipboard, ClipboardBackend, MockClipboard, NovaClipboard};
