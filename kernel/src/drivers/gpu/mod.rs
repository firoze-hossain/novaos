//! kernel/src/drivers/gpu/mod.rs
//!
//! NovaOS 2.0 (Month 4 - Graphics & DRM): GPU and Display Subsystem

pub mod kms;
pub mod drm;
pub mod virgl_cmd;
pub mod virtio_3d;

pub use drm::{run_drm_kms_selftest, GLOBAL_DRM};
pub use virgl_cmd::*;
pub use virtio_3d::{run_virtio_3d_selftest, GLOBAL_VIRTIO_3D};
