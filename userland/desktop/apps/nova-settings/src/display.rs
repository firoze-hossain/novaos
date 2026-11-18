//! userland/desktop/apps/nova-settings/src/display.rs
//!
//! Re-export and interface for Display & HiDPI settings panel.

#[path = "panels/display.rs"]
pub mod display_impl;

pub use display_impl::*;
