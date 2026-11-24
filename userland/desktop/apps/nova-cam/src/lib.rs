//! userland/desktop/apps/nova-cam/src/lib.rs
//!
//! # NovaOS Camera Application Library (nova-cam)
//!
//! Sovereign, pure-Rust default Camera & Video Studio for NovaOS, synthesizing
//! Ubuntu Camera (GNOME Snapshot) with dynamic built-in & USB webcam detection,
//! dynamic resolution scaling (480p to 4K), photo snapshots, and video recording.

pub mod model;
pub mod ui;

use model::{CameraHub, CameraMode, DeviceKind, Resolution, TimerDuration, VideoCaptureBackend, NovaCaptureBackend};
use ui::{IrisRenderer, OwnedBufferSurface, Rect, Surface};

/// Executes the comprehensive autonomous qualification suite for NovaOS Camera.
pub fn run_nova_cam_selftest() -> bool {
    let mut hub = CameraHub::new();

    // 1. Device discovery & classification
    if hub.registry.devices.len() < 2 {
        eprintln!("[FAIL] Expected at least 2 camera devices detected");
        return false;
    }
    let has_builtin = hub.registry.devices.iter().any(|d| d.kind == DeviceKind::BuiltIn);
    let has_usb = hub.registry.devices.iter().any(|d| d.kind == DeviceKind::ExternalUsb);
    if !has_builtin || !has_usb {
        eprintln!("[FAIL] Both built-in and external USB cameras must be classified");
        return false;
    }

    // 2. Dynamic resolution scaling
    hub.set_resolution(Resolution::Res480p);
    if hub.config.resolution.width() != 640 || hub.config.resolution.height() != 480 {
        eprintln!("[FAIL] 480p resolution geometry incorrect");
        return false;
    }
    hub.set_resolution(Resolution::Res1080p);
    if hub.config.resolution.width() != 1920 || hub.config.resolution.height() != 1080 {
        eprintln!("[FAIL] 1080p resolution geometry incorrect");
        return false;
    }
    hub.set_resolution(Resolution::Res4K);
    if hub.config.resolution.width() != 3840 || hub.config.resolution.height() != 2160 {
        eprintln!("[FAIL] 4K resolution geometry incorrect");
        return false;
    }

    // 3. Mirror flip & composition grid
    let mut test_frame = iris_core::frame::VideoFrame::new(4, 1, vec![1, 2, 3, 4], 0, 0);
    test_frame.flip_horizontal();
    if test_frame.pixels != vec![4, 3, 2, 1] {
        eprintln!("[FAIL] Horizontal flip mirror failed");
        return false;
    }

    // 4. Photo snapshot and countdown timer
    hub.config.timer = TimerDuration::Delay3s;
    if hub.trigger_shutter(1000).is_some() {
        eprintln!("[FAIL] 3s countdown should not immediately capture");
        return false;
    }
    hub.tick(3000, 1003); // Complete countdown
    if hub.gallery.items.len() < 4 {
        eprintln!("[FAIL] Photo snapshot should be stored in gallery repository");
        return false;
    }
    hub.config.timer = TimerDuration::Off;

    // 5. Video recording session
    hub.set_mode(CameraMode::Video);
    hub.trigger_shutter(2000); // Start
    if !hub.capture.is_recording() {
        eprintln!("[FAIL] Video recording session failed to start");
        return false;
    }
    for f in 1..=10 {
        hub.tick(33, 2000 + f / 10);
    }
    let finished_video = hub.trigger_shutter(2005); // Stop
    if finished_video.is_none() || hub.capture.is_recording() {
        eprintln!("[FAIL] Video recording session failed to stop or store video");
        return false;
    }

    // 6. 2D UI rasterization
    let mut surface = OwnedBufferSurface::new(800, 600);
    IrisRenderer::render(&mut surface, Rect::new(0, 0, 800, 600), &hub, 3000);
    if surface.get_pixel(400, 300).to_u32() == 0 {
        eprintln!("[FAIL] Viewfinder canvas failed to rasterize pixels");
        return false;
    }

    // 7. NovaOS Ring-3 Video IPC
    let mut nova_cam = NovaCaptureBackend::new();
    if nova_cam.probe_devices().is_empty() {
        eprintln!("[FAIL] NovaOS Ring-3 Video IPC probing failed");
        return false;
    }

    true
}
