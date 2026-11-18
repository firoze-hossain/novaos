//! kernel/src/drivers/video/edid.rs
//!
//! Re-export and interface for EDID parsing in video driver subsystem.

#[path = "../gpu/edid.rs"]
pub mod edid_impl;

pub use edid_impl::*;
