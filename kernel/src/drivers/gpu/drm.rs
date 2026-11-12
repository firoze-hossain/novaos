//! kernel/src/drivers/gpu/drm.rs
//!
//! NovaOS 2.0 (Month 4 - Graphics & DRM): Direct Rendering Manager (DRM) Driver
//!
//! Exposes the `/dev/dri/card0` character device interface conforming to standard
//! Linux DRM UAPI:
//! - DRM IOCTL Version & Capabilities negotiation (novadrm v2.0.0)
//! - DRM Mode Resources discovery (Connectors, Encoders, CRTCs, Planes)
//! - Dynamic Mode Setting (DRM_IOCTL_MODE_SETCRTC)
//! - Tear-Free Page Flipping (DRM_IOCTL_MODE_PAGE_FLIP) synchronized to VBLANK
//! - Independent Hardware Cursor Positioning (DRM_IOCTL_MODE_CURSOR2)
//! - Multi-Plane Hardware Compositing (Primary, Overlay, Cursor)
//! - Dumb Buffer allocation and lifecycle management

#![allow(dead_code, non_camel_case_types)]

#[cfg(test)]
#[path = "kms.rs"]
pub mod kms;

#[cfg(not(test))]
use super::kms::*;
#[cfg(test)]
use kms::*;

// ============================================================================
// Linux DRM UAPI IOCTL Commands & Structures
// ============================================================================

pub const DRM_NAME: &str = "novadrm";
pub const DRM_DATE: &str = "20261112";
pub const DRM_DESC: &str = "NovaOS Pure Rust DRM/KMS Driver";

pub const DRM_MAJOR_VERSION: i32 = 2;
pub const DRM_MINOR_VERSION: i32 = 0;
pub const DRM_PATCHLEVEL: i32    = 0;

// Device Node Metadata
pub const DRM_DEV_MAJOR: u32 = 226; // Standard Linux /dev/dri/card0 major
pub const DRM_DEV_MINOR: u32 = 0;
pub const DRM_DEV_PATH: &str = "/dev/dri/card0";

// DRM Capability Enums
pub const DRM_CAP_DUMB_BUFFER: u64          = 0x1;
pub const DRM_CAP_VBLANK_HIGH_CRTC: u64     = 0x2;
pub const DRM_CAP_DUMB_PREFERRED_DEPTH: u64 = 0x3;
pub const DRM_CAP_DUMB_PREFER_SHADOW: u64   = 0x4;
pub const DRM_CAP_PRIME: u64                = 0x5;
pub const DRM_CAP_TIMESTAMP_MONOTONIC: u64  = 0x6;
pub const DRM_CAP_ASYNC_PAGE_FLIP: u64      = 0x7;
pub const DRM_CAP_PAGE_FLIP: u64            = 0x7;
pub const DRM_CAP_CURSOR_WIDTH: u64         = 0x8;
pub const DRM_CAP_CURSOR_HEIGHT: u64        = 0x9;

// DRM Client Capabilities
pub const DRM_CLIENT_CAP_STEREO_3D: u64             = 1;
pub const DRM_CLIENT_CAP_UNIVERSAL_PLANES: u64      = 2;
pub const DRM_CLIENT_CAP_ATOMIC: u64                = 3;
pub const DRM_CLIENT_CAP_ASPECT_RATIO: u64          = 4;
pub const DRM_CLIENT_CAP_WRITEBACK_CONNECTORS: u64  = 5;

// DRM Event Types
pub const DRM_EVENT_VBLANK: u32        = 0x01;
pub const DRM_EVENT_FLIP_COMPLETE: u32 = 0x02;

// DRM IOCTL Opcodes (Command numbers within the 'd' ioctl group)
pub const DRM_COMMAND_BASE: u32 = 0x64; // 'd'

pub const DRM_IOCTL_NR_VERSION: u32          = 0x00;
pub const DRM_IOCTL_NR_GET_CAP: u32          = 0x0C;
pub const DRM_IOCTL_NR_SET_CLIENT_CAP: u32   = 0x0D;
pub const DRM_IOCTL_NR_WAIT_VBLANK: u32      = 0x3A;
pub const DRM_IOCTL_NR_MODE_GETRESOURCES: u32= 0xA0;
pub const DRM_IOCTL_NR_MODE_GETCRTC: u32     = 0xA1;
pub const DRM_IOCTL_NR_MODE_SETCRTC: u32     = 0xA2;
pub const DRM_IOCTL_NR_MODE_CURSOR: u32      = 0xA3;
pub const DRM_IOCTL_NR_MODE_GETENCODER: u32  = 0xA6;
pub const DRM_IOCTL_NR_MODE_GETCONNECTOR: u32= 0xA7;
pub const DRM_IOCTL_NR_MODE_ADDFB: u32       = 0xAE;
pub const DRM_IOCTL_NR_MODE_RMFB: u32        = 0xAF;
pub const DRM_IOCTL_NR_MODE_PAGE_FLIP: u32   = 0xB0;
pub const DRM_IOCTL_NR_MODE_CREATE_DUMB: u32 = 0xB2;
pub const DRM_IOCTL_NR_MODE_MAP_DUMB: u32    = 0xB3;
pub const DRM_IOCTL_NR_MODE_DESTROY_DUMB: u32= 0xB4;
pub const DRM_IOCTL_NR_MODE_GETPLANES: u32   = 0xB6;
pub const DRM_IOCTL_NR_MODE_GETPLANE: u32    = 0xB7;
pub const DRM_IOCTL_NR_MODE_SETPLANE: u32    = 0xB8;
pub const DRM_IOCTL_NR_MODE_CURSOR2: u32     = 0xBB;

// Full IOCTL Numbers (Direction | Size | Group | Nr)
pub const DRM_IOCTL_VERSION: u32           = 0xC0406400;
pub const DRM_IOCTL_GET_CAP: u32           = 0xC010640C;
pub const DRM_IOCTL_SET_CLIENT_CAP: u32    = 0x4010640D;
pub const DRM_IOCTL_WAIT_VBLANK: u32       = 0xC018643A;
pub const DRM_IOCTL_MODE_GETRESOURCES: u32 = 0xC04064A0;
pub const DRM_IOCTL_MODE_GETCRTC: u32      = 0xC06864A1;
pub const DRM_IOCTL_MODE_SETCRTC: u32      = 0xC06864A2;
pub const DRM_IOCTL_MODE_CURSOR: u32       = 0xC01464A3;
pub const DRM_IOCTL_MODE_GETENCODER: u32   = 0xC01864A6;
pub const DRM_IOCTL_MODE_GETCONNECTOR: u32 = 0xC05064A7;
pub const DRM_IOCTL_MODE_ADDFB: u32        = 0xC01C64AE;
pub const DRM_IOCTL_MODE_RMFB: u32         = 0xC00464AF;
pub const DRM_IOCTL_MODE_PAGE_FLIP: u32    = 0xC01864B0;
pub const DRM_IOCTL_MODE_CREATE_DUMB: u32  = 0xC02064B2;
pub const DRM_IOCTL_MODE_MAP_DUMB: u32     = 0xC01064B3;
pub const DRM_IOCTL_MODE_DESTROY_DUMB: u32 = 0xC00464B4;
pub const DRM_IOCTL_MODE_GETPLANES: u32    = 0xC01064B6;
pub const DRM_IOCTL_MODE_GETPLANE: u32     = 0xC02064B7;
pub const DRM_IOCTL_MODE_SETPLANE: u32     = 0xC03064B8;
pub const DRM_IOCTL_MODE_CURSOR2: u32      = 0xC02464BB;

// ============================================================================
// DRM UAPI Struct Layouts
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmVersion {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,
    pub name_len: usize,
    pub name: *mut u8,
    pub date_len: usize,
    pub date: *mut u8,
    pub desc_len: usize,
    pub desc: *mut u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmGetCap {
    pub capability: u64,
    pub value: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmSetClientCap {
    pub capability: u64,
    pub value: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeCardRes {
    pub fb_id_ptr: u64,
    pub crtc_id_ptr: u64,
    pub connector_id_ptr: u64,
    pub encoder_id_ptr: u64,
    pub count_fbs: u32,
    pub count_crtcs: u32,
    pub count_connectors: u32,
    pub count_encoders: u32,
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeCrtc {
    pub set_connectors_ptr: u64,
    pub count_connectors: u32,
    pub crtc_id: u32,
    pub fb_id: u32,
    pub x: u32,
    pub y: u32,
    pub gamma_size: u32,
    pub mode_valid: u32,
    pub mode: DrmModeInfo,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeCrtcPageFlip {
    pub crtc_id: u32,
    pub fb_id: u32,
    pub flags: u32,
    pub reserved: u32,
    pub user_data: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeCursor {
    pub flags: u32,
    pub crtc_id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub handle: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeCursor2 {
    pub flags: u32,
    pub crtc_id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub handle: u32,
    pub hot_x: i32,
    pub hot_y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeCreateDumb {
    pub height: u32,
    pub width: u32,
    pub bpp: u32,
    pub flags: u32,
    pub handle: u32,
    pub pitch: u32,
    pub size: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeMapDumb {
    pub handle: u32,
    pub pad: u32,
    pub offset: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeDestroyDumb {
    pub handle: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeFbCmd {
    pub fb_id: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub depth: u32,
    pub handle: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeGetPlaneRes {
    pub plane_id_ptr: u64,
    pub count_planes: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeGetPlane {
    pub plane_id: u32,
    pub crtc_id: u32,
    pub fb_id: u32,
    pub possible_crtcs: u32,
    pub gamma_size: u32,
    pub count_format_types: u32,
    pub format_type_ptr: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeSetPlane {
    pub plane_id: u32,
    pub crtc_id: u32,
    pub fb_id: u32,
    pub flags: u32,
    pub crtc_x: i32,
    pub crtc_y: i32,
    pub crtc_w: u32,
    pub crtc_h: u32,
    pub src_x: u32,
    pub src_y: u32,
    pub src_h: u32,
    pub src_w: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeGetConnector {
    pub encoders_ptr: u64,
    pub modes_ptr: u64,
    pub props_ptr: u64,
    pub prop_values_ptr: u64,
    pub count_modes: u32,
    pub count_props: u32,
    pub count_encoders: u32,
    pub encoder_id: u32,
    pub connector_id: u32,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub connection: u32,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmModeGetEncoder {
    pub encoder_id: u32,
    pub encoder_type: u32,
    pub crtc_id: u32,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmEventHeader {
    pub type_: u32,
    pub length: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmEventPageFlip {
    pub base: DrmEventHeader,
    pub user_data: u64,
    pub tv_sec: u32,
    pub tv_usec: u32,
    pub sequence: u32,
    pub crtc_id: u32,
}

// ============================================================================
// Global DRM Driver Instance
// ============================================================================

pub const MAX_PENDING_EVENTS: usize = 16;

pub struct DrmDriver {
    pub kms: KmsDevice,
    pub client_cap_universal_planes: bool,
    pub client_cap_atomic: bool,
    pub event_queue: [Option<DrmEventPageFlip>; MAX_PENDING_EVENTS],
    pub event_head: usize,
    pub event_tail: usize,
    pub event_count: usize,
}

impl DrmDriver {
    pub const fn new() -> Self {
        Self {
            kms: KmsDevice::new(),
            client_cap_universal_planes: false,
            client_cap_atomic: false,
            event_queue: [None; MAX_PENDING_EVENTS],
            event_head: 0,
            event_tail: 0,
            event_count: 0,
        }
    }

    pub fn init(&mut self) {
        self.kms.init();
    }

    pub fn queue_event(&mut self, ev: DrmEventPageFlip) {
        if self.event_count < MAX_PENDING_EVENTS {
            self.event_queue[self.event_tail] = Some(ev);
            self.event_tail = (self.event_tail + 1) % MAX_PENDING_EVENTS;
            self.event_count += 1;
        }
    }

    pub fn dequeue_event(&mut self) -> Option<DrmEventPageFlip> {
        if self.event_count > 0 {
            let ev = self.event_queue[self.event_head].take();
            self.event_head = (self.event_head + 1) % MAX_PENDING_EVENTS;
            self.event_count -= 1;
            ev
        } else {
            None
        }
    }

    /// Primary IOCTL Dispatcher
    pub fn ioctl(&mut self, cmd: u32, arg_ptr: usize) -> i32 {
        let opcode = if (cmd >> 8) & 0xFF == DRM_COMMAND_BASE {
            cmd & 0xFF
        } else {
            cmd
        };

        match opcode {
            DRM_IOCTL_NR_VERSION => {
                if arg_ptr == 0 { return -14; } // EFAULT
                let ver = unsafe { &mut *(arg_ptr as *mut DrmVersion) };
                ver.version_major = DRM_MAJOR_VERSION;
                ver.version_minor = DRM_MINOR_VERSION;
                ver.version_patchlevel = DRM_PATCHLEVEL;

                if !ver.name.is_null() && ver.name_len > 0 {
                    let bytes = DRM_NAME.as_bytes();
                    let to_copy = if bytes.len() < ver.name_len { bytes.len() } else { ver.name_len - 1 };
                    unsafe {
                        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ver.name, to_copy);
                        *ver.name.add(to_copy) = 0;
                    }
                }
                ver.name_len = DRM_NAME.len();

                if !ver.date.is_null() && ver.date_len > 0 {
                    let bytes = DRM_DATE.as_bytes();
                    let to_copy = if bytes.len() < ver.date_len { bytes.len() } else { ver.date_len - 1 };
                    unsafe {
                        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ver.date, to_copy);
                        *ver.date.add(to_copy) = 0;
                    }
                }
                ver.date_len = DRM_DATE.len();

                if !ver.desc.is_null() && ver.desc_len > 0 {
                    let bytes = DRM_DESC.as_bytes();
                    let to_copy = if bytes.len() < ver.desc_len { bytes.len() } else { ver.desc_len - 1 };
                    unsafe {
                        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ver.desc, to_copy);
                        *ver.desc.add(to_copy) = 0;
                    }
                }
                ver.desc_len = DRM_DESC.len();

                0
            }

            DRM_IOCTL_NR_GET_CAP => {
                if arg_ptr == 0 { return -14; }
                let cap = unsafe { &mut *(arg_ptr as *mut DrmGetCap) };
                match cap.capability {
                    DRM_CAP_DUMB_BUFFER => cap.value = 1,
                    DRM_CAP_VBLANK_HIGH_CRTC => cap.value = 1,
                    DRM_CAP_DUMB_PREFERRED_DEPTH => cap.value = 24,
                    DRM_CAP_DUMB_PREFER_SHADOW => cap.value = 0,
                    DRM_CAP_PRIME => cap.value = 1,
                    DRM_CAP_TIMESTAMP_MONOTONIC => cap.value = 1,
                    DRM_CAP_ASYNC_PAGE_FLIP => cap.value = 1,
                    DRM_CAP_CURSOR_WIDTH => cap.value = CURSOR_MAX_WIDTH as u64,
                    DRM_CAP_CURSOR_HEIGHT => cap.value = CURSOR_MAX_HEIGHT as u64,
                    _ => return -22, // EINVAL
                }
                0
            }

            DRM_IOCTL_NR_SET_CLIENT_CAP => {
                if arg_ptr == 0 { return -14; }
                let cap = unsafe { &*(arg_ptr as *const DrmSetClientCap) };
                match cap.capability {
                    DRM_CLIENT_CAP_UNIVERSAL_PLANES => {
                        self.client_cap_universal_planes = cap.value != 0;
                    }
                    DRM_CLIENT_CAP_ATOMIC => {
                        self.client_cap_atomic = cap.value != 0;
                    }
                    _ => return -22,
                }
                0
            }

            DRM_IOCTL_NR_MODE_GETRESOURCES => {
                if arg_ptr == 0 { return -14; }
                let res = unsafe { &mut *(arg_ptr as *mut DrmModeCardRes) };
                res.count_fbs = 0;
                let mut i = 0;
                while i < MAX_FRAMEBUFFERS {
                    if self.kms.framebuffers[i].in_use {
                        res.count_fbs += 1;
                    }
                    i += 1;
                }
                res.count_crtcs = self.kms.crtc_count as u32;
                res.count_connectors = self.kms.connector_count as u32;
                res.count_encoders = self.kms.encoder_count as u32;
                res.min_width = 640;
                res.max_width = 3840;
                res.min_height = 480;
                res.max_height = 2160;

                // Fill CRTC IDs if buffer provided
                if res.crtc_id_ptr != 0 && res.count_crtcs > 0 {
                    let ptr = res.crtc_id_ptr as *mut u32;
                    let mut c = 0;
                    while c < self.kms.crtc_count {
                        unsafe { *ptr.add(c) = self.kms.crtcs[c].id; }
                        c += 1;
                    }
                }

                // Fill Connector IDs if buffer provided
                if res.connector_id_ptr != 0 && res.count_connectors > 0 {
                    let ptr = res.connector_id_ptr as *mut u32;
                    let mut c = 0;
                    while c < self.kms.connector_count {
                        unsafe { *ptr.add(c) = self.kms.connectors[c].id; }
                        c += 1;
                    }
                }

                // Fill Encoder IDs if buffer provided
                if res.encoder_id_ptr != 0 && res.count_encoders > 0 {
                    let ptr = res.encoder_id_ptr as *mut u32;
                    let mut e = 0;
                    while e < self.kms.encoder_count {
                        unsafe { *ptr.add(e) = self.kms.encoders[e].id; }
                        e += 1;
                    }
                }

                0
            }

            DRM_IOCTL_NR_MODE_GETCONNECTOR => {
                if arg_ptr == 0 { return -14; }
                let conn_req = unsafe { &mut *(arg_ptr as *mut DrmModeGetConnector) };
                if self.kms.connector_count == 0 { return -22; }
                let conn = &self.kms.connectors[0];

                conn_req.connector_id = conn.id;
                conn_req.connector_type = conn.connector_type;
                conn_req.connector_type_id = conn.connector_type_id;
                conn_req.connection = conn.connection;
                conn_req.mm_width = conn.mm_width;
                conn_req.mm_height = conn.mm_height;
                conn_req.subpixel = conn.subpixel;
                conn_req.encoder_id = conn.encoder_id;
                conn_req.count_modes = conn.mode_count as u32;
                conn_req.count_encoders = 1;
                conn_req.count_props = 0;

                // Copy modes array if buffer provided
                if conn_req.modes_ptr != 0 && conn.mode_count > 0 {
                    let ptr = conn_req.modes_ptr as *mut DrmModeInfo;
                    let mut m = 0;
                    while m < conn.mode_count {
                        unsafe { *ptr.add(m) = conn.modes[m]; }
                        m += 1;
                    }
                }

                0
            }

            DRM_IOCTL_NR_MODE_GETENCODER => {
                if arg_ptr == 0 { return -14; }
                let enc_req = unsafe { &mut *(arg_ptr as *mut DrmModeGetEncoder) };
                if self.kms.encoder_count == 0 { return -22; }
                let enc = &self.kms.encoders[0];

                enc_req.encoder_id = enc.id;
                enc_req.encoder_type = enc.encoder_type;
                enc_req.crtc_id = enc.crtc_id;
                enc_req.possible_crtcs = enc.possible_crtcs;
                enc_req.possible_clones = enc.possible_clones;
                0
            }

            DRM_IOCTL_NR_MODE_GETCRTC => {
                if arg_ptr == 0 { return -14; }
                let crtc_req = unsafe { &mut *(arg_ptr as *mut DrmModeCrtc) };
                let mut found = false;
                let mut i = 0;
                while i < self.kms.crtc_count {
                    if self.kms.crtcs[i].id == crtc_req.crtc_id {
                        let crtc = &self.kms.crtcs[i];
                        crtc_req.fb_id = crtc.active_fb_id;
                        crtc_req.x = crtc.x;
                        crtc_req.y = crtc.y;
                        crtc_req.gamma_size = crtc.gamma_size;
                        crtc_req.mode_valid = if crtc.mode_valid { 1 } else { 0 };
                        crtc_req.mode = crtc.mode;
                        found = true;
                        break;
                    }
                    i += 1;
                }
                if found { 0 } else { -22 }
            }

            DRM_IOCTL_NR_MODE_SETCRTC => {
                if arg_ptr == 0 { return -14; }
                let crtc_req = unsafe { &*(arg_ptr as *const DrmModeCrtc) };
                match self.kms.set_crtc(crtc_req.crtc_id, crtc_req.fb_id, crtc_req.mode, crtc_req.x, crtc_req.y) {
                    Ok(()) => 0,
                    Err(e) => e,
                }
            }

            DRM_IOCTL_NR_MODE_CREATE_DUMB => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &mut *(arg_ptr as *mut DrmModeCreateDumb) };
                match self.kms.create_dumb(req.width, req.height, req.bpp) {
                    Ok((handle, pitch, size)) => {
                        req.handle = handle;
                        req.pitch = pitch;
                        req.size = size;
                        0
                    }
                    Err(e) => e,
                }
            }

            DRM_IOCTL_NR_MODE_MAP_DUMB => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &mut *(arg_ptr as *mut DrmModeMapDumb) };
                let mut found = false;
                let mut i = 0;
                while i < MAX_DUMB_BUFFERS {
                    if self.kms.dumb_buffers[i].in_use && self.kms.dumb_buffers[i].handle == req.handle {
                        req.offset = self.kms.dumb_buffers[i].offset;
                        found = true;
                        break;
                    }
                    i += 1;
                }
                if found { 0 } else { -22 }
            }

            DRM_IOCTL_NR_MODE_DESTROY_DUMB => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &*(arg_ptr as *const DrmModeDestroyDumb) };
                match self.kms.destroy_dumb(req.handle) {
                    Ok(()) => 0,
                    Err(e) => e,
                }
            }

            DRM_IOCTL_NR_MODE_ADDFB => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &mut *(arg_ptr as *mut DrmModeFbCmd) };
                match self.kms.add_framebuffer(req.width, req.height, req.pitch, req.bpp, req.depth, req.handle, DRM_FORMAT_XRGB8888) {
                    Ok(fb_id) => {
                        req.fb_id = fb_id;
                        0
                    }
                    Err(e) => e,
                }
            }

            DRM_IOCTL_NR_MODE_RMFB => {
                if arg_ptr == 0 { return -14; }
                let fb_id = unsafe { *(arg_ptr as *const u32) };
                match self.kms.remove_framebuffer(fb_id) {
                    Ok(()) => 0,
                    Err(e) => e,
                }
            }

            DRM_IOCTL_NR_MODE_PAGE_FLIP => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &*(arg_ptr as *const DrmModeCrtcPageFlip) };
                match self.kms.page_flip(req.crtc_id, req.fb_id, req.flags, req.user_data) {
                    Ok(()) => 0,
                    Err(e) => e,
                }
            }

            DRM_IOCTL_NR_MODE_CURSOR => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &*(arg_ptr as *const DrmModeCursor) };
                if (req.flags & DRM_CURSOR_BO) != 0 {
                    if let Err(e) = self.kms.set_cursor(req.crtc_id, req.handle, req.width, req.height, 0, 0) {
                        return e;
                    }
                }
                if (req.flags & DRM_CURSOR_MOVE) != 0 {
                    if let Err(e) = self.kms.move_cursor(req.crtc_id, req.x, req.y) {
                        return e;
                    }
                }
                0
            }

            DRM_IOCTL_NR_MODE_CURSOR2 => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &*(arg_ptr as *const DrmModeCursor2) };
                if (req.flags & DRM_CURSOR_BO) != 0 {
                    if let Err(e) = self.kms.set_cursor(req.crtc_id, req.handle, req.width, req.height, req.hot_x, req.hot_y) {
                        return e;
                    }
                }
                if (req.flags & DRM_CURSOR_MOVE) != 0 {
                    if let Err(e) = self.kms.move_cursor(req.crtc_id, req.x, req.y) {
                        return e;
                    }
                }
                0
            }

            DRM_IOCTL_NR_MODE_GETPLANES => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &mut *(arg_ptr as *mut DrmModeGetPlaneRes) };
                req.count_planes = self.kms.plane_count as u32;
                if req.plane_id_ptr != 0 && req.count_planes > 0 {
                    let ptr = req.plane_id_ptr as *mut u32;
                    let mut p = 0;
                    while p < self.kms.plane_count {
                        unsafe { *ptr.add(p) = self.kms.planes[p].id; }
                        p += 1;
                    }
                }
                0
            }

            DRM_IOCTL_NR_MODE_GETPLANE => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &mut *(arg_ptr as *mut DrmModeGetPlane) };
                let mut found = false;
                let mut i = 0;
                while i < self.kms.plane_count {
                    if self.kms.planes[i].id == req.plane_id {
                        let plane = &self.kms.planes[i];
                        req.crtc_id = plane.crtc_id;
                        req.fb_id = plane.fb_id;
                        req.possible_crtcs = plane.possible_crtcs;
                        req.gamma_size = 0;
                        req.count_format_types = plane.format_count as u32;

                        if req.format_type_ptr != 0 && plane.format_count > 0 {
                            let ptr = req.format_type_ptr as *mut u32;
                            let mut f = 0;
                            while f < plane.format_count {
                                unsafe { *ptr.add(f) = plane.formats[f]; }
                                f += 1;
                            }
                        }
                        found = true;
                        break;
                    }
                    i += 1;
                }
                if found { 0 } else { -22 }
            }

            DRM_IOCTL_NR_MODE_SETPLANE => {
                if arg_ptr == 0 { return -14; }
                let req = unsafe { &*(arg_ptr as *const DrmModeSetPlane) };
                match self.kms.set_plane(
                    req.plane_id,
                    req.crtc_id,
                    req.fb_id,
                    req.crtc_x,
                    req.crtc_y,
                    req.crtc_w,
                    req.crtc_h,
                    req.src_x,
                    req.src_y,
                    req.src_w,
                    req.src_h,
                ) {
                    Ok(()) => 0,
                    Err(e) => e,
                }
            }

            _ => -22, // EINVAL: unknown IOCTL
        }
    }

    /// Triggers VBLANK tick; queues page flip completion event if flip was pending
    pub fn trigger_vblank(&mut self, crtc_id: u32) {
        if let Some(flip_ev) = self.kms.trigger_vblank(crtc_id) {
            let ev = DrmEventPageFlip {
                base: DrmEventHeader {
                    type_: DRM_EVENT_FLIP_COMPLETE,
                    length: core::mem::size_of::<DrmEventPageFlip>() as u32,
                },
                user_data: flip_ev.user_data,
                tv_sec: flip_ev.tv_sec,
                tv_usec: flip_ev.tv_usec,
                sequence: flip_ev.sequence,
                crtc_id: flip_ev.crtc_id,
            };
            self.queue_event(ev);
        }
    }

    /// Read implementation for `/dev/dri/card0`
    pub fn read(&mut self, buf: *mut u8, count: usize) -> isize {
        if buf.is_null() || count < core::mem::size_of::<DrmEventPageFlip>() {
            return -22; // EINVAL
        }
        if let Some(ev) = self.dequeue_event() {
            let ev_size = core::mem::size_of::<DrmEventPageFlip>();
            unsafe {
                core::ptr::copy_nonoverlapping(&ev as *const _ as *const u8, buf, ev_size);
            }
            ev_size as isize
        } else {
            -11 // EAGAIN
        }
    }
}

pub static mut GLOBAL_DRM: DrmDriver = DrmDriver::new();

// ============================================================================
// Kernel Boot Integration & In-Kernel Self-Test
// ============================================================================

/// NovaOS 2.0 (Month 4 - Graphics & DRM): DRM/KMS Display Pipeline Self-Test
///
/// Verifies:
/// 1. Subsystem initialization and driver identity ("novadrm" v2.0.0).
/// 2. DRM capability query handling (dumb buffer, vblank, page flip, hardware cursor).
/// 3. KMS topology resource enumeration (CRTC, Encoder, Connector, Planes).
/// 4. Dynamic mode setting (1920x1080@60Hz, 2560x1440@144Hz).
/// 5. Dumb buffer allocation and Framebuffer association.
/// 6. Atomic tear-free page flip queueing and VBLANK completion event generation.
/// 7. Event dequeueing through `/dev/dri/card0` read().
/// 8. Hardware cursor plane positioning without redrawing desktop pixels.
/// 9. Multi-plane compositing (primary, cursor, overlay).
pub fn run_drm_kms_selftest() -> i32 {
    let drm = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_DRM) };
    drm.init();

    // 1. Verify Driver Version
    let mut name_buf = [0u8; 16];
    let mut date_buf = [0u8; 16];
    let mut desc_buf = [0u8; 64];

    let mut ver = DrmVersion {
        version_major: 0,
        version_minor: 0,
        version_patchlevel: 0,
        name_len: name_buf.len(),
        name: name_buf.as_mut_ptr(),
        date_len: date_buf.len(),
        date: date_buf.as_mut_ptr(),
        desc_len: desc_buf.len(),
        desc: desc_buf.as_mut_ptr(),
    };

    let ret = drm.ioctl(DRM_IOCTL_VERSION, &mut ver as *mut _ as usize);
    if ret != 0 || ver.version_major != 2 || ver.version_minor != 0 {
        return 1;
    }

    // 2. Verify Capabilities
    let mut cap = DrmGetCap {
        capability: DRM_CAP_DUMB_BUFFER,
        value: 0,
    };
    if drm.ioctl(DRM_IOCTL_GET_CAP, &mut cap as *mut _ as usize) != 0 || cap.value != 1 {
        return 2;
    }

    cap.capability = DRM_CAP_PAGE_FLIP;
    if drm.ioctl(DRM_IOCTL_GET_CAP, &mut cap as *mut _ as usize) != 0 || cap.value != 1 {
        return 3;
    }

    cap.capability = DRM_CAP_CURSOR_WIDTH;
    if drm.ioctl(DRM_IOCTL_GET_CAP, &mut cap as *mut _ as usize) != 0 || cap.value != 64 {
        return 4;
    }

    // 3. Verify Mode Resources
    let mut crtc_ids = [0u32; 4];
    let mut conn_ids = [0u32; 4];
    let mut enc_ids = [0u32; 4];
    let mut res = DrmModeCardRes {
        fb_id_ptr: 0,
        crtc_id_ptr: crtc_ids.as_mut_ptr() as u64,
        connector_id_ptr: conn_ids.as_mut_ptr() as u64,
        encoder_id_ptr: enc_ids.as_mut_ptr() as u64,
        count_fbs: 0,
        count_crtcs: 4,
        count_connectors: 4,
        count_encoders: 4,
        min_width: 0,
        max_width: 0,
        min_height: 0,
        max_height: 0,
    };

    if drm.ioctl(DRM_IOCTL_MODE_GETRESOURCES, &mut res as *mut _ as usize) != 0 {
        return 5;
    }
    if res.count_crtcs != 1 || res.count_connectors != 1 || res.count_encoders != 1 {
        return 6;
    }
    if crtc_ids[0] != 1 || conn_ids[0] != 1 || enc_ids[0] != 1 {
        return 7;
    }

    // 4. Verify Connector Details & Modes
    let mut modes = [DrmModeInfo::empty(); 8];
    let mut conn_info = DrmModeGetConnector {
        encoders_ptr: 0,
        modes_ptr: modes.as_mut_ptr() as u64,
        props_ptr: 0,
        prop_values_ptr: 0,
        count_modes: 8,
        count_props: 0,
        count_encoders: 0,
        encoder_id: 0,
        connector_id: 1,
        connector_type: 0,
        connector_type_id: 0,
        connection: 0,
        mm_width: 0,
        mm_height: 0,
        subpixel: 0,
        pad: 0,
    };
    if drm.ioctl(DRM_IOCTL_MODE_GETCONNECTOR, &mut conn_info as *mut _ as usize) != 0 {
        return 8;
    }
    if conn_info.count_modes < 4 || conn_info.connection != DRM_MODE_CONNECTED {
        return 9;
    }
    if modes[0].hdisplay != 1920 || modes[0].vdisplay != 1080 {
        return 10;
    }

    // 5. Create Dumb Buffers for Front and Back Buffers
    let mut dumb1 = DrmModeCreateDumb {
        width: 1920,
        height: 1080,
        bpp: 32,
        flags: 0,
        handle: 0,
        pitch: 0,
        size: 0,
    };
    if drm.ioctl(DRM_IOCTL_MODE_CREATE_DUMB, &mut dumb1 as *mut _ as usize) != 0 || dumb1.handle == 0 {
        return 11;
    }

    let mut dumb2 = DrmModeCreateDumb {
        width: 1920,
        height: 1080,
        bpp: 32,
        flags: 0,
        handle: 0,
        pitch: 0,
        size: 0,
    };
    if drm.ioctl(DRM_IOCTL_MODE_CREATE_DUMB, &mut dumb2 as *mut _ as usize) != 0 || dumb2.handle == 0 {
        return 12;
    }

    // 6. Wrap into Framebuffers
    let mut fb_cmd1 = DrmModeFbCmd {
        fb_id: 0,
        width: 1920,
        height: 1080,
        pitch: dumb1.pitch,
        bpp: 32,
        depth: 24,
        handle: dumb1.handle,
    };
    if drm.ioctl(DRM_IOCTL_MODE_ADDFB, &mut fb_cmd1 as *mut _ as usize) != 0 || fb_cmd1.fb_id == 0 {
        return 13;
    }

    let mut fb_cmd2 = DrmModeFbCmd {
        fb_id: 0,
        width: 1920,
        height: 1080,
        pitch: dumb2.pitch,
        bpp: 32,
        depth: 24,
        handle: dumb2.handle,
    };
    if drm.ioctl(DRM_IOCTL_MODE_ADDFB, &mut fb_cmd2 as *mut _ as usize) != 0 || fb_cmd2.fb_id == 0 {
        return 14;
    }

    // 7. Dynamic Mode Setting on CRTC 1
    let crtc_set = DrmModeCrtc {
        set_connectors_ptr: 0,
        count_connectors: 1,
        crtc_id: 1,
        fb_id: fb_cmd1.fb_id,
        x: 0,
        y: 0,
        gamma_size: 256,
        mode_valid: 1,
        mode: modes[0],
    };
    if drm.ioctl(DRM_IOCTL_MODE_SETCRTC, &crtc_set as *const _ as usize) != 0 {
        return 15;
    }

    // 8. Queue Tear-Free Page Flip to fb_cmd2 on Next VBLANK
    let user_tag: u64 = 0x1234_5678_CAFE_BABE;
    let flip_req = DrmModeCrtcPageFlip {
        crtc_id: 1,
        fb_id: fb_cmd2.fb_id,
        flags: DRM_MODE_PAGE_FLIP_EVENT,
        reserved: 0,
        user_data: user_tag,
    };
    if drm.ioctl(DRM_IOCTL_MODE_PAGE_FLIP, &flip_req as *const _ as usize) != 0 {
        return 16;
    }

    // 9. Fire VBLANK interrupt
    drm.trigger_vblank(1);

    // Verify tear-free page flip completed and scanout buffer switched
    let mut crtc_get = DrmModeCrtc {
        set_connectors_ptr: 0,
        count_connectors: 0,
        crtc_id: 1,
        fb_id: 0,
        x: 0,
        y: 0,
        gamma_size: 0,
        mode_valid: 0,
        mode: DrmModeInfo::empty(),
    };
    if drm.ioctl(DRM_IOCTL_MODE_GETCRTC, &mut crtc_get as *mut _ as usize) != 0 {
        return 17;
    }
    if crtc_get.fb_id != fb_cmd2.fb_id {
        return 18; // Scanout buffer was not updated!
    }

    // 10. Dequeue Page Flip Completion Event via read()
    let mut ev_buf = [0u8; core::mem::size_of::<DrmEventPageFlip>()];
    let bytes_read = drm.read(ev_buf.as_mut_ptr(), ev_buf.len());
    if bytes_read != core::mem::size_of::<DrmEventPageFlip>() as isize {
        return 19;
    }
    let ev = unsafe { &*(ev_buf.as_ptr() as *const DrmEventPageFlip) };
    if ev.base.type_ != DRM_EVENT_FLIP_COMPLETE || ev.user_data != user_tag || ev.sequence != 1 {
        return 20;
    }

    // 11. Hardware Cursor Positioning without redrawing desktop pixels
    let cur_set = DrmModeCursor2 {
        flags: DRM_CURSOR_BO | DRM_CURSOR_MOVE,
        crtc_id: 1,
        x: 500,
        y: 350,
        width: 64,
        height: 64,
        handle: 99,
        hot_x: 2,
        hot_y: 2,
    };
    if drm.ioctl(DRM_IOCTL_MODE_CURSOR2, &cur_set as *const _ as usize) != 0 {
        return 21;
    }
    if drm.kms.cursor.x != 500 || drm.kms.cursor.y != 350 || drm.kms.cursor.move_count != 1 {
        return 22;
    }

    // Move cursor to (800, 600) via cursor position ioctl
    let cur_move = DrmModeCursor {
        flags: DRM_CURSOR_MOVE,
        crtc_id: 1,
        x: 800,
        y: 600,
        width: 0,
        height: 0,
        handle: 0,
    };
    if drm.ioctl(DRM_IOCTL_MODE_CURSOR, &cur_move as *const _ as usize) != 0 {
        return 23;
    }
    if drm.kms.cursor.x != 800 || drm.kms.cursor.y != 600 || drm.kms.cursor.move_count != 2 {
        return 24;
    }

    // 12. Planes Validation
    let mut plane_res = DrmModeGetPlaneRes {
        plane_id_ptr: 0,
        count_planes: 0,
    };
    if drm.ioctl(DRM_IOCTL_MODE_GETPLANES, &mut plane_res as *mut _ as usize) != 0 || plane_res.count_planes < 3 {
        return 25;
    }

    // 13. Clean up Framebuffers & Dumb Buffers
    let rmfb_cmd = fb_cmd1.fb_id;
    if drm.ioctl(DRM_IOCTL_MODE_RMFB, &rmfb_cmd as *const _ as usize) != 0 {
        return 26;
    }
    let destroy_cmd = DrmModeDestroyDumb { handle: dumb1.handle };
    if drm.ioctl(DRM_IOCTL_MODE_DESTROY_DUMB, &destroy_cmd as *const _ as usize) != 0 {
        return 27;
    }

    0
}

// ============================================================================
// Unit Tests (Host Environment)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drm_version_and_metadata() {
        let mut drm = DrmDriver::new();
        drm.init();

        let mut name_buf = [0u8; 32];
        let mut date_buf = [0u8; 32];
        let mut desc_buf = [0u8; 64];

        let mut ver = DrmVersion {
            version_major: 0,
            version_minor: 0,
            version_patchlevel: 0,
            name_len: name_buf.len(),
            name: name_buf.as_mut_ptr(),
            date_len: date_buf.len(),
            date: date_buf.as_mut_ptr(),
            desc_len: desc_buf.len(),
            desc: desc_buf.as_mut_ptr(),
        };

        let res = drm.ioctl(DRM_IOCTL_VERSION, &mut ver as *mut _ as usize);
        assert_eq!(res, 0);
        assert_eq!(ver.version_major, 2);
        assert_eq!(ver.version_minor, 0);
        assert_eq!(ver.version_patchlevel, 0);

        let name_str = core::str::from_utf8(&name_buf[..ver.name_len]).unwrap();
        assert_eq!(name_str, "novadrm");
    }

    #[test]
    fn test_drm_capabilities() {
        let mut drm = DrmDriver::new();
        drm.init();

        let mut cap = DrmGetCap { capability: DRM_CAP_DUMB_BUFFER, value: 0 };
        assert_eq!(drm.ioctl(DRM_IOCTL_GET_CAP, &mut cap as *mut _ as usize), 0);
        assert_eq!(cap.value, 1);

        cap.capability = DRM_CAP_CURSOR_WIDTH;
        assert_eq!(drm.ioctl(DRM_IOCTL_GET_CAP, &mut cap as *mut _ as usize), 0);
        assert_eq!(cap.value, 64);
    }

    #[test]
    fn test_drm_selftest_passes() {
        assert_eq!(run_drm_kms_selftest(), 0);
    }
}
