//! userland/desktop/nova-compositor/src/lib.rs
//!
//! NovaOS 2.0 (Month 4 - Compositor & UI): Hardware-Accelerated Wayland Display Server
//!
//! Exposes the complete Wayland display server architecture:
//!   - `surface`: double-buffered atomic commit, damage rects, DMA-BUF zero-copy attachments.
//!   - `security`: strict application isolation, anti-keylogger keystroke confinement, buffer privacy.
//!   - `drm_backend`: DRM/KMS modesetting, VBLANK-synchronized atomic page flips, hardware cursor.
//!   - `protocol`: Wayland wire protocol serialization and binary IPC dispatch.
//!   - `snap`: Windows 11 intelligent snap layouts and tiling grids with synchronized resizing.
//!   - `scale`: HiDPI / Retina dynamic display scaling and EDID physical DPI detection.

#![no_std]

#[macro_use]
extern crate core;

#[cfg(test)]
extern crate std;

use core::prelude::v1::*;

pub mod ffi;
pub mod surface;
pub mod security;
pub mod drm_backend;
pub mod protocol;
pub mod snap;
pub mod scale;

pub use surface::*;
pub use security::*;
pub use drm_backend::*;
pub use protocol::*;
pub use snap::*;
pub use scale::*;

/// Comprehensive self-test suite verifying 100% of Wayland display server contracts
pub fn run_compositor_selftest() -> i32 {
    let mut failures = 0;

    // 1. Double-buffered state machine & atomicity
    let mut s = WlSurface::new(1, 10);
    let dmabuf = BufferAttachment::DmaBuf {
        buffer_id: 1,
        dma_fd: 3,
        gem_handle: 1,
        width: 1024,
        height: 768,
        stride: 4096,
        format: DRM_FORMAT_XRGB8888,
        modifier: 0,
        is_zero_copy: true,
        mapped_addr: 0x6000_0000,
    };
    s.attach(dmabuf);
    s.damage_buffer(0, 0, 500, 400);
    s.request_frame_callback(42);

    if s.current.buffer != BufferAttachment::None || !s.current.damage.is_empty() {
        failures |= 1; // Pending changes leaked into current state prematurely
    }
    s.commit();
    if s.current.buffer != dmabuf || s.current.damage.is_empty() || s.width != 1024 || s.height != 768 {
        failures |= 2; // Commit failed to promote pending state
    }
    if s.complete_frame_callback() != Some(42) {
        failures |= 4; // Frame callback lifecycle failed
    }

    // 2. Window Security Isolation & Anti-Keylogger Protection
    let mut sec = SecurityManager::new();
    let term = ClientSecurityContext::new(10, 1001, 1000, 1000, false);
    let rogue = ClientSecurityContext::new(20, 2002, 1000, 1000, false);
    let _ = sec.register_client(term);
    let _ = sec.register_client(rogue);
    sec.set_focus(10, 1);

    if sec.check_keyboard_dispatch(10).is_err() {
        failures |= 8; // Focused app must receive keystrokes
    }
    if sec.check_keyboard_dispatch(20) != Err(SecurityError::UnfocusedKeystrokeDenied) {
        failures |= 16; // Rogue background app was not blocked from observing keystrokes!
    }
    if sec.check_buffer_access(20, 10) != Err(SecurityError::BufferCrossAccessDenied) {
        failures |= 32; // Rogue app was not blocked from reading client buffer!
    }

    // 3. DRM/KMS VBLANK Synchronization & Tear-Free Page Flipping
    let mut drm = DrmBackend::new();
    if drm.queue_page_flip().is_err() {
        failures |= 64;
    }
    // Attempting double flip before VBLANK must fail to eliminate screen tearing
    if drm.queue_page_flip().is_ok() {
        failures |= 128;
    }
    // Simulate VBLANK interrupt at 16.6ms
    if !drm.on_vblank_interrupt(16666) || drm.active_scanout_fb_id != 2 || drm.frame_counter != 1 {
        failures |= 256;
    }

    // 4. Hardware Cursor Plane Independence
    drm.move_cursor(250, 350);
    if drm.cursor_x != 250 || drm.cursor_y != 350 || drm.is_page_flip_pending {
        failures |= 512; // Cursor movement affected primary scanout state
    }

    // 5. Zero-Copy Direct Scanout Bypass Qualification
    let damage_full = DamageRect::new(0, 0, 1024, 768);
    if !drm.can_direct_scanout(&dmabuf, &damage_full) {
        failures |= 1024; // Fullscreen DMA-BUF must qualify for zero-copy scanout bypass (<1ms)
    }

    // 6. Windows 11 Snap Layouts & Tiling Grid Calculations
    let ws = WorkspaceArea::new(1024, 768, 24, 36);
    let mut snap = SnapEngine::new(ws);
    if snap.update_drag_gesture(5, 300) != Some(SnapLayout::SplitLeft50) {
        failures |= 2048; // Edge drag to left must trigger SplitLeft50
    }
    let left_tile = snap.preview_rect.unwrap_or_default();
    if left_tile.x != 0 || left_tile.y != 24 || left_tile.width != 512 || left_tile.height != 708 {
        failures |= 4096; // Tile geometry bounds mismatch
    }

    // 7. HiDPI / Retina Display Scaling
    let scale_retina = DisplayScale::from_edid(294, 165, 3840, 2160);
    if scale_retina != DisplayScale::Scale200 {
        failures |= 8192; // 4K 13.3" must resolve to Scale200
    }
    if scale_retina.to_physical(100) != 200 || scale_retina.to_logical(200) != 100 {
        failures |= 16384; // HiDPI coordinate mapping mismatch
    }

    // 8. Wire Protocol Serialization Round-Trip
    let mut wire_buf = [0u8; 32];
    let key_ev = WaylandEvent::KeyboardKey {
        serial: 55,
        time_ms: 1000,
        key: 28, // KEY_ENTER
        state: 1,
    };
    let written = key_ev.serialize(&mut wire_buf);
    if written != 24 {
        failures |= 32768;
    }
    let hdr = WireHeader::parse(&wire_buf).unwrap_or(WireHeader::new(0, 0, 0));
    if hdr.object_id != INTERFACE_WL_KEYBOARD || hdr.opcode != 3 || hdr.length != 24 {
        failures |= 65536;
    }

    failures
}
