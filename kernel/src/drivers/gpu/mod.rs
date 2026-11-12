//! kernel/src/drivers/gpu/mod.rs
//!
//! NovaOS 2.0 (Month 4 - Graphics & DRM): GPU and Display Subsystem

pub mod kms;
pub mod drm;

pub use drm::{run_drm_kms_selftest, GLOBAL_DRM};
