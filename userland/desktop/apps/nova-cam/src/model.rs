//! userland/desktop/apps/nova-cam/src/model.rs
//!
//! Domain models, device management, and camera hub coordinator for NovaOS Camera.

pub use iris_core::capture::{CaptureEngine, VideoRecordingSession};
pub use iris_core::config::{CameraMode, IrisConfig, TimerDuration};
pub use iris_core::device::{CameraDevice, DeviceKind, DeviceRegistry, PixelFormat, Resolution};
pub use iris_core::frame::{GridMode, VideoFrame};
pub use iris_core::gallery::{GalleryRepo, MediaItem, MediaKind};
pub use iris_core::hub::CameraHub;
pub use iris_platform::{create_platform_camera_engine, LinuxV4L2Backend, NovaCaptureBackend, VideoCaptureBackend};
