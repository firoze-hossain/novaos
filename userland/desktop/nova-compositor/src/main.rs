//! userland/desktop/nova-compositor/src/main.rs
//!
//! NovaOS 2.0 (Month 4 - Compositor & UI): Hardware-Accelerated Wayland Display Server
//!
//! Main entry point for `nova-compositor` (Wayland Display Server).
//! Manages client window surfaces, enforces window security isolation, performs
//! double-buffered VBLANK-synchronized compositing, and utilizes zero-copy DMA-BUF
//! buffer sharing for tear-free 60 FPS presentation.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(dead_code, unused_imports)]

#[macro_use]
extern crate core;

#[cfg(test)]
extern crate std;

use core::prelude::v1::*;
use nova_compositor::*;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"nova-compositor: fatal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
        ffi::sys_exit(1);
    }
}

pub const MAX_SURFACES: usize = 32;

pub struct CompositorServer {
    pub drm: DrmBackend,
    pub security: SecurityManager,
    pub snap: SnapEngine,
    pub scale: DisplayScale,
    pub surfaces: [Option<WlSurface>; MAX_SURFACES],
    pub surface_count: usize,
    pub active_surface_idx: Option<usize>,
    pub running: bool,
}

impl CompositorServer {
    pub fn new() -> Self {
        let ws = WorkspaceArea::new(1024, 768, 24, 36);
        Self {
            drm: DrmBackend::new(),
            security: SecurityManager::new(),
            snap: SnapEngine::new(ws),
            scale: DisplayScale::Scale100,
            surfaces: [const { None }; MAX_SURFACES],
            surface_count: 0,
            active_surface_idx: None,
            running: true,
        }
    }

    pub fn create_surface(&mut self, surface_id: SurfaceId, client_id: ClientId) -> Result<(), &'static str> {
        for slot in self.surfaces.iter_mut() {
            if slot.is_none() {
                *slot = Some(WlSurface::new(surface_id, client_id));
                self.surface_count += 1;
                return Ok(());
            }
        }
        Err("Maximum surface capacity reached")
    }

    pub fn destroy_surface(&mut self, surface_id: SurfaceId) {
        for slot in self.surfaces.iter_mut() {
            if let Some(s) = slot {
                if s.id == surface_id {
                    *slot = None;
                    self.surface_count -= 1;
                    break;
                }
            }
        }
    }

    pub fn find_surface_mut(&mut self, surface_id: SurfaceId) -> Option<&mut WlSurface> {
        self.surfaces.iter_mut().flatten().find(|s| s.id == surface_id)
    }

    pub fn composite_frame(&mut self, now_us: u64) -> bool {
        // 1. Process damage and prepare back buffer
        let mut has_damage = false;
        for s in self.surfaces.iter_mut().flatten() {
            if !s.current.damage.is_empty() {
                has_damage = true;
                s.clear_damage();
            }
        }

        // 2. Tear-free VBLANK page flip
        if has_damage {
            if self.drm.queue_page_flip().is_ok() {
                self.drm.on_vblank_interrupt(now_us);
                // 3. Dispatch frame completion callbacks to clients for smooth 60 FPS
                for s in self.surfaces.iter_mut().flatten() {
                    let _ = s.complete_frame_callback();
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}

// -----------------------------------------------------------------------------
// Entry Point & Self-Test Verification Mode
// -----------------------------------------------------------------------------

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut is_selftest = false;

    if argc > 1 && !argv.is_null() {
        unsafe {
            let arg1 = *argv.offset(1);
            if !arg1.is_null() {
                let bytes = core::slice::from_raw_parts(arg1, 10);
                if bytes.starts_with(b"--selftest") {
                    is_selftest = true;
                }
            }
        }
    }

    if is_selftest {
        let res = run_compositor_selftest();
        if res == 0 {
            unsafe {
                let msg = b"[nova-compositor] PASS: Wayland display server, double-buffered surfaces, DMA-BUF zero-copy, and window isolation contracts verified.\n\0";
                ffi::sys_write(msg.as_ptr());
            }
        } else {
            unsafe {
                let msg = b"[nova-compositor] FAIL: One or more Wayland display server contracts failed.\n\0";
                ffi::sys_write(msg.as_ptr());
            }
        }
        return res;
    }

    // Interactive daemon mode
    unsafe {
        let banner = b"NovaOS Wayland Display Server (nova-compositor v2.0.0)\n\0";
        ffi::sys_write(banner.as_ptr());
        ffi::sys_fb_acquire();
    }

    let mut server = CompositorServer::new();
    let mut time_us: u64 = 0;

    // Active frame loop (60 FPS pacing)
    while server.running {
        time_us += 16666; // 16.66ms per frame = 60.0 FPS
        server.composite_frame(time_us);
        unsafe { ffi::sys_yield() };
        if time_us > 1_000_000 {
            break; // Demo run cycle
        }
    }

    unsafe {
        ffi::sys_fb_release();
    }

    0
}

#[cfg(test)]
mod main_tests {
    use super::*;

    #[test]
    fn test_compositor_server_lifecycle() {
        let mut server = CompositorServer::new();
        assert_eq!(server.surface_count, 0);

        assert!(server.create_surface(1, 100).is_ok());
        assert_eq!(server.surface_count, 1);

        {
            let surface = server.find_surface_mut(1).unwrap();
            let dmabuf = BufferAttachment::DmaBuf {
                buffer_id: 1,
                dma_fd: 3,
                gem_handle: 1,
                width: 800,
                height: 600,
                stride: 3200,
                format: DRM_FORMAT_XRGB8888,
                modifier: 0,
                is_zero_copy: true,
                mapped_addr: 0x7000_0000,
            };
            surface.attach(dmabuf);
            surface.damage_buffer(0, 0, 100, 100);
            surface.request_frame_callback(1);
            surface.commit();
        }

        // Composite frame -> VBLANK flip
        let flipped = server.composite_frame(16666);
        assert!(flipped);
        assert_eq!(server.drm.frame_counter, 1);

        server.destroy_surface(1);
        assert_eq!(server.surface_count, 0);
    }

    #[test]
    fn test_compositor_full_selftest_passes() {
        let code = run_compositor_selftest();
        assert_eq!(code, 0, "All Wayland display server self-test checks must pass (code 0)");
    }
}
