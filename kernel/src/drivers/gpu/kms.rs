//! kernel/src/drivers/gpu/kms.rs
//!
//! NovaOS 2.0 (Month 4 - Graphics & DRM): Kernel Mode-Setting (KMS) Engine
//!
//! Provides the core display pipeline abstractions:
//! - Connectors (HDMI, Virtual, eDP display outputs) with dynamic resolution modes
//! - Encoders (TMDS, DAC, Virtual format converters)
//! - CRTCs (Cathode Ray Tube Controllers / scanout engines) with VBLANK synchronization
//! - Hardware Display Planes (Primary, Hardware Cursor, and Alpha-Blended Overlay)
//! - Atomic Tear-Free Page Flipping synchronized to monitor refresh rate (60 - 144 Hz)
//! - Hardware Cursor Positioning without dirtying or redrawing desktop pixels
//! - Linear Dumb Buffer allocator for software fallback and client staging

#![allow(dead_code)]

// ============================================================================
// DRM / KMS Constants & Protocol Definitions
// ============================================================================

pub const DRM_DISPLAY_MODE_LEN: usize = 32;

// Connector Types
pub const DRM_MODE_CONNECTOR_UNKNOWN: u32 = 0;
pub const DRM_MODE_CONNECTOR_VGA: u32 = 1;
pub const DRM_MODE_CONNECTOR_DVII: u32 = 2;
pub const DRM_MODE_CONNECTOR_DVID: u32 = 3;
pub const DRM_MODE_CONNECTOR_DVIA: u32 = 4;
pub const DRM_MODE_CONNECTOR_COMPOSITE: u32 = 5;
pub const DRM_MODE_CONNECTOR_SVIDEO: u32 = 6;
pub const DRM_MODE_CONNECTOR_LVDS: u32 = 7;
pub const DRM_MODE_CONNECTOR_COMPONENT: u32 = 8;
pub const DRM_MODE_CONNECTOR_9PINDIN: u32 = 9;
pub const DRM_MODE_CONNECTOR_DISPLAYPORT: u32 = 10;
pub const DRM_MODE_CONNECTOR_HDMIA: u32 = 11;
pub const DRM_MODE_CONNECTOR_HDMIB: u32 = 12;
pub const DRM_MODE_CONNECTOR_TV: u32 = 13;
pub const DRM_MODE_CONNECTOR_EDP: u32 = 14;
pub const DRM_MODE_CONNECTOR_VIRTUAL: u32 = 15;
pub const DRM_MODE_CONNECTOR_DSI: u32 = 16;
pub const DRM_MODE_CONNECTOR_DPI: u32 = 17;

// Connection Status
pub const DRM_MODE_CONNECTED: u32 = 1;
pub const DRM_MODE_DISCONNECTED: u32 = 2;
pub const DRM_MODE_UNKNOWNCONNECTION: u32 = 3;

// Subpixel Geometry
pub const DRM_MODE_SUBPIXEL_UNKNOWN: u32 = 1;
pub const DRM_MODE_SUBPIXEL_HORIZONTAL_RGB: u32 = 2;
pub const DRM_MODE_SUBPIXEL_HORIZONTAL_BGR: u32 = 3;
pub const DRM_MODE_SUBPIXEL_VERTICAL_RGB: u32 = 4;
pub const DRM_MODE_SUBPIXEL_VERTICAL_BGR: u32 = 5;
pub const DRM_MODE_SUBPIXEL_NONE: u32 = 6;

// Encoder Types
pub const DRM_MODE_ENCODER_NONE: u32 = 0;
pub const DRM_MODE_ENCODER_DAC: u32 = 1;
pub const DRM_MODE_ENCODER_TMDS: u32 = 2;
pub const DRM_MODE_ENCODER_LVDS: u32 = 3;
pub const DRM_MODE_ENCODER_TVDAC: u32 = 4;
pub const DRM_MODE_ENCODER_VIRTUAL: u32 = 5;
pub const DRM_MODE_ENCODER_DSI: u32 = 6;
pub const DRM_MODE_ENCODER_DPMST: u32 = 7;
pub const DRM_MODE_ENCODER_DPI: u32 = 8;

// Plane Types
pub const DRM_PLANE_TYPE_OVERLAY: u32 = 0;
pub const DRM_PLANE_TYPE_PRIMARY: u32 = 1;
pub const DRM_PLANE_TYPE_CURSOR: u32 = 2;

// FourCC Pixel Formats
pub const fn fourcc_code(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

pub const DRM_FORMAT_XRGB8888: u32 = fourcc_code(b'X', b'R', b'2', b'4');
pub const DRM_FORMAT_ARGB8888: u32 = fourcc_code(b'A', b'R', b'2', b'4');
pub const DRM_FORMAT_RGB565: u32   = fourcc_code(b'R', b'G', b'1', b'6');
pub const DRM_FORMAT_NV12: u32     = fourcc_code(b'N', b'V', b'1', b'2');

// Mode Flags
pub const DRM_MODE_FLAG_PHSYNC: u32 = 1 << 0;
pub const DRM_MODE_FLAG_NHSYNC: u32 = 1 << 1;
pub const DRM_MODE_FLAG_PVSYNC: u32 = 1 << 2;
pub const DRM_MODE_FLAG_NVSYNC: u32 = 1 << 3;
pub const DRM_MODE_FLAG_INTERLACE: u32 = 1 << 4;

// Mode Types
pub const DRM_MODE_TYPE_BUILTIN: u32 = 1 << 0;
pub const DRM_MODE_TYPE_CLOCK_C: u32 = 1 << 1;
pub const DRM_MODE_TYPE_CRTC_C: u32  = 1 << 2;
pub const DRM_MODE_TYPE_PREFERRED: u32 = 1 << 3;
pub const DRM_MODE_TYPE_DEFAULT: u32 = 1 << 4;
pub const DRM_MODE_TYPE_USERDEF: u32 = 1 << 5;
pub const DRM_MODE_TYPE_DRIVER: u32  = 1 << 6;

// Page Flip Flags
pub const DRM_MODE_PAGE_FLIP_EVENT: u32 = 0x01;
pub const DRM_MODE_PAGE_FLIP_ASYNC: u32 = 0x02;

// Cursor Flags
pub const DRM_CURSOR_BO: u32   = 0x01;
pub const DRM_CURSOR_MOVE: u32 = 0x02;
pub const DRM_CURSOR_FLAGS: u32 = DRM_CURSOR_BO | DRM_CURSOR_MOVE;

// ============================================================================
// Core KMS Data Structures
// ============================================================================

/// Standard DRM display timing mode description (matches struct drm_mode_modeinfo, 68 bytes)
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrmModeInfo {
    pub clock: u32,          // Pixel clock in kHz
    pub hdisplay: u16,       // Horizontal display resolution
    pub hsync_start: u16,    // Horizontal sync start
    pub hsync_end: u16,      // Horizontal sync end
    pub htotal: u16,         // Horizontal total
    pub hskew: u16,
    pub vdisplay: u16,       // Vertical display resolution
    pub vsync_start: u16,    // Vertical sync start
    pub vsync_end: u16,      // Vertical sync end
    pub vtotal: u16,         // Vertical total
    pub vscan: u16,
    pub vrefresh: u32,       // Vertical refresh rate in Hz
    pub flags: u32,          // Sync polarity flags
    pub type_: u32,          // Mode type bitmask (PREFERRED, DRIVER)
    pub name: [u8; DRM_DISPLAY_MODE_LEN], // Human readable name (e.g. "1920x1080")
}

impl DrmModeInfo {
    pub const fn empty() -> Self {
        Self {
            clock: 0,
            hdisplay: 0,
            hsync_start: 0,
            hsync_end: 0,
            htotal: 0,
            hskew: 0,
            vdisplay: 0,
            vsync_start: 0,
            vsync_end: 0,
            vtotal: 0,
            vscan: 0,
            vrefresh: 0,
            flags: 0,
            type_: 0,
            name: [0; DRM_DISPLAY_MODE_LEN],
        }
    }

    pub fn new(
        name_str: &str,
        clock_khz: u32,
        hdisp: u16, hsync_s: u16, hsync_e: u16, htot: u16,
        vdisp: u16, vsync_s: u16, vsync_e: u16, vtot: u16,
        vref: u32,
        flags: u32,
        type_: u32,
    ) -> Self {
        let mut name = [0u8; DRM_DISPLAY_MODE_LEN];
        let bytes = name_str.as_bytes();
        let len = if bytes.len() < DRM_DISPLAY_MODE_LEN { bytes.len() } else { DRM_DISPLAY_MODE_LEN - 1 };
        let mut i = 0;
        while i < len {
            name[i] = bytes[i];
            i += 1;
        }

        Self {
            clock: clock_khz,
            hdisplay: hdisp,
            hsync_start: hsync_s,
            hsync_end: hsync_e,
            htotal: htot,
            hskew: 0,
            vdisplay: vdisp,
            vsync_start: vsync_s,
            vsync_end: vsync_e,
            vtotal: vtot,
            vscan: 0,
            vrefresh: vref,
            flags,
            type_,
            name,
        }
    }

    /// Standard VESA 1024x768 @ 60Hz
    pub fn mode_1024x768_60() -> Self {
        Self::new(
            "1024x768",
            65_000,
            1024, 1048, 1184, 1344,
            768, 771, 777, 806,
            60,
            DRM_MODE_FLAG_NHSYNC | DRM_MODE_FLAG_NVSYNC,
            DRM_MODE_TYPE_DRIVER,
        )
    }

    /// 720p HD 1280x720 @ 60Hz
    pub fn mode_1280x720_60() -> Self {
        Self::new(
            "1280x720",
            74_250,
            1280, 1390, 1430, 1650,
            720, 725, 730, 750,
            60,
            DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC,
            DRM_MODE_TYPE_DRIVER,
        )
    }

    /// 1080p Full HD 1920x1080 @ 60Hz (Default Preferred Mode)
    pub fn mode_1920x1080_60() -> Self {
        Self::new(
            "1920x1080",
            148_500,
            1920, 2008, 2052, 2200,
            1080, 1084, 1089, 1125,
            60,
            DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC,
            DRM_MODE_TYPE_PREFERRED | DRM_MODE_TYPE_DRIVER,
        )
    }

    /// 1440p QHD 2560x1440 @ 144Hz (High-Refresh Gaming Mode)
    pub fn mode_2560x1440_144() -> Self {
        Self::new(
            "2560x1440",
            585_600,
            2560, 2608, 2640, 2720,
            1440, 1443, 1448, 1496,
            144,
            DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC,
            DRM_MODE_TYPE_DRIVER,
        )
    }

    pub fn name_as_str(&self) -> &str {
        let mut len = 0;
        while len < self.name.len() && self.name[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.name[..len]).unwrap_or("unknown")
    }
}

// ============================================================================
// Connector, Encoder & Plane Abstractions
// ============================================================================

pub const MAX_CONNECTOR_MODES: usize = 8;

/// Physical or virtual display connector
#[derive(Clone, Copy, Debug)]
pub struct DrmConnector {
    pub id: u32,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub connection: u32,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: u32,
    pub encoder_id: u32,
    pub modes: [DrmModeInfo; MAX_CONNECTOR_MODES],
    pub mode_count: usize,
    pub current_mode_idx: usize,
}

impl DrmConnector {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            connector_type: DRM_MODE_CONNECTOR_UNKNOWN,
            connector_type_id: 0,
            connection: DRM_MODE_DISCONNECTED,
            mm_width: 0,
            mm_height: 0,
            subpixel: DRM_MODE_SUBPIXEL_NONE,
            encoder_id: 0,
            modes: [DrmModeInfo::empty(); MAX_CONNECTOR_MODES],
            mode_count: 0,
            current_mode_idx: 0,
        }
    }

    pub fn add_mode(&mut self, mode: DrmModeInfo) -> bool {
        if self.mode_count < MAX_CONNECTOR_MODES {
            self.modes[self.mode_count] = mode;
            self.mode_count += 1;
            true
        } else {
            false
        }
    }

    pub fn get_mode(&self, idx: usize) -> Option<&DrmModeInfo> {
        if idx < self.mode_count {
            Some(&self.modes[idx])
        } else {
            None
        }
    }
}

/// Display Encoder translating CRTC signals for a connector
#[derive(Clone, Copy, Debug)]
pub struct DrmEncoder {
    pub id: u32,
    pub encoder_type: u32,
    pub crtc_id: u32,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
}

impl DrmEncoder {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            encoder_type: DRM_MODE_ENCODER_NONE,
            crtc_id: 0,
            possible_crtcs: 0,
            possible_clones: 0,
        }
    }
}

/// Hardware Plane (Primary Scanout, Hardware Cursor, Video Overlay)
#[derive(Clone, Copy, Debug)]
pub struct DrmPlane {
    pub id: u32,
    pub crtc_id: u32,
    pub fb_id: u32,
    pub plane_type: u32,
    pub possible_crtcs: u32,
    pub crtc_x: i32,
    pub crtc_y: i32,
    pub crtc_w: u32,
    pub crtc_h: u32,
    pub src_x: u32, // 16.16 fixed point
    pub src_y: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub formats: [u32; 4],
    pub format_count: usize,
    pub visible: bool,
    pub z_order: u8,
    pub alpha: u8,
}

impl DrmPlane {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            crtc_id: 0,
            fb_id: 0,
            plane_type: DRM_PLANE_TYPE_OVERLAY,
            possible_crtcs: 0,
            crtc_x: 0,
            crtc_y: 0,
            crtc_w: 0,
            crtc_h: 0,
            src_x: 0,
            src_y: 0,
            src_w: 0,
            src_h: 0,
            formats: [0; 4],
            format_count: 0,
            visible: false,
            z_order: 0,
            alpha: 255,
        }
    }

    pub fn supports_format(&self, fmt: u32) -> bool {
        let mut i = 0;
        while i < self.format_count {
            if self.formats[i] == fmt {
                return true;
            }
            i += 1;
        }
        false
    }
}

// ============================================================================
// Hardware Cursor State Machine
// ============================================================================

pub const CURSOR_MAX_WIDTH: u32 = 64;
pub const CURSOR_MAX_HEIGHT: u32 = 64;
pub const CURSOR_BUFFER_PIXELS: usize = (CURSOR_MAX_WIDTH * CURSOR_MAX_HEIGHT) as usize;

/// Hardware Cursor plane controller
///
/// Moving the cursor updates only internal coordinates `x` and `y` without
/// touching or dirtying a single byte of desktop primary framebuffer memory.
pub struct HardwareCursor {
    pub enabled: bool,
    pub x: i32,
    pub y: i32,
    pub hot_x: i32,
    pub hot_y: i32,
    pub width: u32,
    pub height: u32,
    pub handle: u32,
    pub move_count: u64,
    pub pixels: [u32; CURSOR_BUFFER_PIXELS],
}

impl HardwareCursor {
    pub const fn new() -> Self {
        Self {
            enabled: false,
            x: 0,
            y: 0,
            hot_x: 0,
            hot_y: 0,
            width: 0,
            height: 0,
            handle: 0,
            move_count: 0,
            pixels: [0; CURSOR_BUFFER_PIXELS],
        }
    }

    /// Sets cursor shape and hotspot
    pub fn set_image(&mut self, handle: u32, width: u32, height: u32, hot_x: i32, hot_y: i32) -> Result<(), i32> {
        if width > CURSOR_MAX_WIDTH || height > CURSOR_MAX_HEIGHT {
            return Err(-22); // EINVAL
        }
        if handle == 0 {
            self.enabled = false;
            self.handle = 0;
            return Ok(());
        }

        self.handle = handle;
        self.width = width;
        self.height = height;
        self.hot_x = hot_x;
        self.hot_y = hot_y;
        self.enabled = true;
        Ok(())
    }

    /// Repositions hardware cursor instantaneously without redrawing desktop pixels
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.move_count += 1;
    }
}

// ============================================================================
// CRTC & Tear-Free Page Flipping Engine
// ============================================================================

/// CRTC scanout engine with VBLANK event queueing
#[derive(Clone, Copy, Debug)]
pub struct DrmCrtc {
    pub id: u32,
    pub enabled: bool,
    pub mode: DrmModeInfo,
    pub mode_valid: bool,
    pub active_fb_id: u32,
    pub x: u32,
    pub y: u32,
    pub gamma_size: u32,

    // VBLANK timing and synchronization
    pub vblank_counter: u64,
    pub vblank_timestamp_ns: u64,
    pub vblank_period_ns: u64,

    // Atomic Page Flip state
    pub flip_pending: bool,
    pub pending_flip_fb: u32,
    pub pending_flip_user_data: u64,
    pub pending_flip_flags: u32,
    pub completed_flip_count: u64,
    pub last_flip_sequence: u32,
    pub last_flip_timestamp_ns: u64,
}

impl DrmCrtc {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            enabled: false,
            mode: DrmModeInfo::empty(),
            mode_valid: false,
            active_fb_id: 0,
            x: 0,
            y: 0,
            gamma_size: 256,
            vblank_counter: 0,
            vblank_timestamp_ns: 0,
            vblank_period_ns: 16_666_667, // 60Hz: 16.66ms
            flip_pending: false,
            pending_flip_fb: 0,
            pending_flip_user_data: 0,
            pending_flip_flags: 0,
            completed_flip_count: 0,
            last_flip_sequence: 0,
            last_flip_timestamp_ns: 0,
        }
    }

    pub fn set_mode(&mut self, mode: DrmModeInfo, fb_id: u32, x: u32, y: u32) {
        self.mode = mode;
        self.mode_valid = true;
        self.active_fb_id = fb_id;
        self.x = x;
        self.y = y;
        self.enabled = true;
        if mode.vrefresh > 0 {
            self.vblank_period_ns = 1_000_000_000 / (mode.vrefresh as u64);
        }
    }

    pub fn queue_flip(&mut self, fb_id: u32, flags: u32, user_data: u64) -> Result<(), i32> {
        if !self.enabled || !self.mode_valid {
            return Err(-22); // EINVAL
        }
        if self.flip_pending {
            return Err(-16); // EBUSY: flip already pending
        }

        self.pending_flip_fb = fb_id;
        self.pending_flip_user_data = user_data;
        self.pending_flip_flags = flags;
        self.flip_pending = true;
        Ok(())
    }

    /// Advances VBLANK clock and commits pending tear-free page flip if queued
    pub fn on_vblank(&mut self) -> Option<PageFlipEvent> {
        self.vblank_counter += 1;
        self.vblank_timestamp_ns += self.vblank_period_ns;

        if self.flip_pending {
            // Atomically switch active scanout buffer
            self.active_fb_id = self.pending_flip_fb;
            self.flip_pending = false;
            self.completed_flip_count += 1;
            self.last_flip_sequence = self.vblank_counter as u32;
            self.last_flip_timestamp_ns = self.vblank_timestamp_ns;

            let tv_sec = (self.vblank_timestamp_ns / 1_000_000_000) as u32;
            let tv_usec = ((self.vblank_timestamp_ns % 1_000_000_000) / 1_000) as u32;

            Some(PageFlipEvent {
                user_data: self.pending_flip_user_data,
                tv_sec,
                tv_usec,
                sequence: self.last_flip_sequence,
                crtc_id: self.id,
            })
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PageFlipEvent {
    pub user_data: u64,
    pub tv_sec: u32,
    pub tv_usec: u32,
    pub sequence: u32,
    pub crtc_id: u32,
}

// ============================================================================
// Framebuffer & Dumb Buffer Subsystems
// ============================================================================

pub const MAX_FRAMEBUFFERS: usize = 16;
pub const MAX_DUMB_BUFFERS: usize = 16;

#[derive(Clone, Copy, Debug)]
pub struct DrmFramebuffer {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub depth: u32,
    pub format: u32,
    pub handle: u32,
    pub in_use: bool,
    pub paddr: u64,
}

impl DrmFramebuffer {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            width: 0,
            height: 0,
            pitch: 0,
            bpp: 0,
            depth: 0,
            format: 0,
            handle: 0,
            in_use: false,
            paddr: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DumbBuffer {
    pub handle: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub pitch: u32,
    pub size: u64,
    pub offset: u64,
    pub in_use: bool,
}

impl DumbBuffer {
    pub const fn empty() -> Self {
        Self {
            handle: 0,
            width: 0,
            height: 0,
            bpp: 0,
            pitch: 0,
            size: 0,
            offset: 0,
            in_use: false,
        }
    }
}

// ============================================================================
// Global KMS Device Manager
// ============================================================================

pub struct KmsDevice {
    pub connectors: [DrmConnector; 2],
    pub connector_count: usize,
    pub encoders: [DrmEncoder; 2],
    pub encoder_count: usize,
    pub crtcs: [DrmCrtc; 2],
    pub crtc_count: usize,
    pub planes: [DrmPlane; 4],
    pub plane_count: usize,
    pub framebuffers: [DrmFramebuffer; MAX_FRAMEBUFFERS],
    pub dumb_buffers: [DumbBuffer; MAX_DUMB_BUFFERS],
    pub cursor: HardwareCursor,

    next_fb_id: u32,
    next_handle: u32,
    pub initialized: bool,
}

impl KmsDevice {
    pub const fn new() -> Self {
        Self {
            connectors: [DrmConnector::empty(); 2],
            connector_count: 0,
            encoders: [DrmEncoder::empty(); 2],
            encoder_count: 0,
            crtcs: [DrmCrtc::empty(); 2],
            crtc_count: 0,
            planes: [DrmPlane::empty(); 4],
            plane_count: 0,
            framebuffers: [DrmFramebuffer::empty(); MAX_FRAMEBUFFERS],
            dumb_buffers: [DumbBuffer::empty(); MAX_DUMB_BUFFERS],
            cursor: HardwareCursor::new(),
            next_fb_id: 100,
            next_handle: 1,
            initialized: false,
        }
    }

    /// Initializes standard KMS topology:
    /// - CRTC 1 (Primary Scanout Controller)
    /// - Encoder 1 (TMDS / Virtual)
    /// - Connector 1 (HDMI-A / Virtual with 1024x768, 1280x720, 1920x1080@60Hz, 2560x1440@144Hz)
    /// - Planes: Primary (ID 1), Cursor (ID 2), Overlay (ID 3)
    pub fn init(&mut self) {
        if self.initialized {
            return;
        }

        // 1. CRTC 1
        let mut crtc1 = DrmCrtc::empty();
        crtc1.id = 1;
        crtc1.vblank_period_ns = 16_666_667;
        self.crtcs[0] = crtc1;
        self.crtc_count = 1;

        // 2. Encoder 1
        let mut enc1 = DrmEncoder::empty();
        enc1.id = 1;
        enc1.encoder_type = DRM_MODE_ENCODER_TMDS;
        enc1.crtc_id = 1;
        enc1.possible_crtcs = 1 << 0;
        self.encoders[0] = enc1;
        self.encoder_count = 1;

        // 3. Connector 1
        let mut conn1 = DrmConnector::empty();
        conn1.id = 1;
        conn1.connector_type = DRM_MODE_CONNECTOR_HDMIA;
        conn1.connector_type_id = 1;
        conn1.connection = DRM_MODE_CONNECTED;
        conn1.mm_width = 527;  // 24" monitor width
        conn1.mm_height = 296; // 24" monitor height
        conn1.subpixel = DRM_MODE_SUBPIXEL_HORIZONTAL_RGB;
        conn1.encoder_id = 1;
        conn1.add_mode(DrmModeInfo::mode_1920x1080_60());
        conn1.add_mode(DrmModeInfo::mode_2560x1440_144());
        conn1.add_mode(DrmModeInfo::mode_1280x720_60());
        conn1.add_mode(DrmModeInfo::mode_1024x768_60());
        conn1.current_mode_idx = 0;
        self.connectors[0] = conn1;
        self.connector_count = 1;

        // 4. Planes
        // Primary Plane
        let mut p_primary = DrmPlane::empty();
        p_primary.id = 1;
        p_primary.crtc_id = 1;
        p_primary.plane_type = DRM_PLANE_TYPE_PRIMARY;
        p_primary.possible_crtcs = 1 << 0;
        p_primary.formats[0] = DRM_FORMAT_XRGB8888;
        p_primary.formats[1] = DRM_FORMAT_ARGB8888;
        p_primary.format_count = 2;
        p_primary.visible = true;
        p_primary.z_order = 0;
        self.planes[0] = p_primary;

        // Cursor Plane
        let mut p_cursor = DrmPlane::empty();
        p_cursor.id = 2;
        p_cursor.crtc_id = 1;
        p_cursor.plane_type = DRM_PLANE_TYPE_CURSOR;
        p_cursor.possible_crtcs = 1 << 0;
        p_cursor.formats[0] = DRM_FORMAT_ARGB8888;
        p_cursor.format_count = 1;
        p_cursor.crtc_w = 64;
        p_cursor.crtc_h = 64;
        p_cursor.visible = false;
        p_cursor.z_order = 255;
        self.planes[1] = p_cursor;

        // Overlay Plane
        let mut p_overlay = DrmPlane::empty();
        p_overlay.id = 3;
        p_overlay.crtc_id = 1;
        p_overlay.plane_type = DRM_PLANE_TYPE_OVERLAY;
        p_overlay.possible_crtcs = 1 << 0;
        p_overlay.formats[0] = DRM_FORMAT_XRGB8888;
        p_overlay.formats[1] = DRM_FORMAT_ARGB8888;
        p_overlay.formats[2] = DRM_FORMAT_NV12;
        p_overlay.format_count = 3;
        p_overlay.visible = false;
        p_overlay.z_order = 1;
        self.planes[2] = p_overlay;

        self.plane_count = 3;
        self.initialized = true;
    }

    /// Allocates a linear dumb buffer
    pub fn create_dumb(&mut self, width: u32, height: u32, bpp: u32) -> Result<(u32, u32, u64), i32> {
        if width == 0 || height == 0 || bpp == 0 {
            return Err(-22); // EINVAL
        }
        let pitch = (width * ((bpp + 7) / 8) + 63) & !63; // 64-byte aligned pitch
        let size = (pitch as u64) * (height as u64);

        let mut slot = None;
        let mut i = 0;
        while i < MAX_DUMB_BUFFERS {
            if !self.dumb_buffers[i].in_use {
                slot = Some(i);
                break;
            }
            i += 1;
        }

        let idx = slot.ok_or(-12)?; // ENOMEM
        let handle = self.next_handle;
        self.next_handle += 1;

        self.dumb_buffers[idx] = DumbBuffer {
            handle,
            width,
            height,
            bpp,
            pitch,
            size,
            offset: (handle as u64) * 0x1000_0000, // Synthetic mmap offset
            in_use: true,
        };

        Ok((handle, pitch, size))
    }

    pub fn destroy_dumb(&mut self, handle: u32) -> Result<(), i32> {
        let mut i = 0;
        while i < MAX_DUMB_BUFFERS {
            if self.dumb_buffers[i].in_use && self.dumb_buffers[i].handle == handle {
                self.dumb_buffers[i].in_use = false;
                return Ok(());
            }
            i += 1;
        }
        Err(-22) // EINVAL
    }

    /// Registers a KMS Framebuffer wrapping a buffer handle
    pub fn add_framebuffer(
        &mut self,
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
        depth: u32,
        handle: u32,
        format: u32,
    ) -> Result<u32, i32> {
        let mut slot = None;
        let mut i = 0;
        while i < MAX_FRAMEBUFFERS {
            if !self.framebuffers[i].in_use {
                slot = Some(i);
                break;
            }
            i += 1;
        }

        let idx = slot.ok_or(-12)?; // ENOMEM
        let fb_id = self.next_fb_id;
        self.next_fb_id += 1;

        self.framebuffers[idx] = DrmFramebuffer {
            id: fb_id,
            width,
            height,
            pitch,
            bpp,
            depth,
            format: if format == 0 { DRM_FORMAT_XRGB8888 } else { format },
            handle,
            in_use: true,
            paddr: 0xFD00_0000 + (idx as u64) * 0x0080_0000,
        };

        Ok(fb_id)
    }

    pub fn remove_framebuffer(&mut self, fb_id: u32) -> Result<(), i32> {
        let mut i = 0;
        while i < MAX_FRAMEBUFFERS {
            if self.framebuffers[i].in_use && self.framebuffers[i].id == fb_id {
                self.framebuffers[i].in_use = false;
                return Ok(());
            }
            i += 1;
        }
        Err(-22) // EINVAL
    }

    /// Sets CRTC mode and activates framebuffer
    pub fn set_crtc(&mut self, crtc_id: u32, fb_id: u32, mode: DrmModeInfo, x: u32, y: u32) -> Result<(), i32> {
        let mut found = false;
        let mut i = 0;
        while i < self.crtc_count {
            if self.crtcs[i].id == crtc_id {
                self.crtcs[i].set_mode(mode, fb_id, x, y);
                self.planes[0].fb_id = fb_id;
                self.planes[0].crtc_w = mode.hdisplay as u32;
                self.planes[0].crtc_h = mode.vdisplay as u32;
                self.planes[0].src_w = (mode.hdisplay as u32) << 16;
                self.planes[0].src_h = (mode.vdisplay as u32) << 16;
                found = true;
                break;
            }
            i += 1;
        }
        if found { Ok(()) } else { Err(-22) }
    }

    /// Queues tear-free page flip on next VBLANK
    pub fn page_flip(&mut self, crtc_id: u32, fb_id: u32, flags: u32, user_data: u64) -> Result<(), i32> {
        let mut i = 0;
        while i < self.crtc_count {
            if self.crtcs[i].id == crtc_id {
                return self.crtcs[i].queue_flip(fb_id, flags, user_data);
            }
            i += 1;
        }
        Err(-22)
    }

    /// Updates hardware cursor shape and hotspot
    pub fn set_cursor(&mut self, crtc_id: u32, handle: u32, width: u32, height: u32, hot_x: i32, hot_y: i32) -> Result<(), i32> {
        if crtc_id != 1 {
            return Err(-22);
        }
        self.cursor.set_image(handle, width, height, hot_x, hot_y)?;
        self.planes[1].visible = self.cursor.enabled;
        self.planes[1].crtc_w = width;
        self.planes[1].crtc_h = height;
        Ok(())
    }

    /// Repositions hardware cursor without touching primary framebuffer pixels
    pub fn move_cursor(&mut self, crtc_id: u32, x: i32, y: i32) -> Result<(), i32> {
        if crtc_id != 1 {
            return Err(-22);
        }
        self.cursor.set_position(x, y);
        self.planes[1].crtc_x = x - self.cursor.hot_x;
        self.planes[1].crtc_y = y - self.cursor.hot_y;
        Ok(())
    }

    /// Updates hardware plane coordinates, source rectangle, and buffer
    pub fn set_plane(
        &mut self,
        plane_id: u32,
        crtc_id: u32,
        fb_id: u32,
        crtc_x: i32,
        crtc_y: i32,
        crtc_w: u32,
        crtc_h: u32,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
    ) -> Result<(), i32> {
        let mut i = 0;
        while i < self.plane_count {
            if self.planes[i].id == plane_id {
                self.planes[i].crtc_id = crtc_id;
                self.planes[i].fb_id = fb_id;
                self.planes[i].crtc_x = crtc_x;
                self.planes[i].crtc_y = crtc_y;
                self.planes[i].crtc_w = crtc_w;
                self.planes[i].crtc_h = crtc_h;
                self.planes[i].src_x = src_x;
                self.planes[i].src_y = src_y;
                self.planes[i].src_w = src_w;
                self.planes[i].src_h = src_h;
                self.planes[i].visible = fb_id != 0;
                return Ok(());
            }
            i += 1;
        }
        Err(-22)
    }

    /// Fires VBLANK interrupt, completing pending page flip if one is queued
    pub fn trigger_vblank(&mut self, crtc_id: u32) -> Option<PageFlipEvent> {
        let mut i = 0;
        while i < self.crtc_count {
            if self.crtcs[i].id == crtc_id {
                let res = self.crtcs[i].on_vblank();
                if let Some(ref _ev) = res {
                    self.planes[0].fb_id = self.crtcs[i].active_fb_id;
                }
                return res;
            }
            i += 1;
        }
        None
    }
}

// ============================================================================
// Unit Tests (Host Environment)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_creation_and_properties() {
        let mode1080p = DrmModeInfo::mode_1920x1080_60();
        assert_eq!(mode1080p.hdisplay, 1920);
        assert_eq!(mode1080p.vdisplay, 1080);
        assert_eq!(mode1080p.vrefresh, 60);
        assert_eq!(mode1080p.clock, 148_500);
        assert_eq!(mode1080p.name_as_str(), "1920x1080");

        let mode1440p = DrmModeInfo::mode_2560x1440_144();
        assert_eq!(mode1440p.hdisplay, 2560);
        assert_eq!(mode1440p.vdisplay, 1440);
        assert_eq!(mode1440p.vrefresh, 144);
        assert_eq!(mode1440p.name_as_str(), "2560x1440");
    }

    #[test]
    fn test_kms_device_initialization() {
        let mut dev = KmsDevice::new();
        dev.init();

        assert_eq!(dev.crtc_count, 1);
        assert_eq!(dev.encoder_count, 1);
        assert_eq!(dev.connector_count, 1);
        assert_eq!(dev.plane_count, 3);

        let conn = &dev.connectors[0];
        assert_eq!(conn.connection, DRM_MODE_CONNECTED);
        assert_eq!(conn.connector_type, DRM_MODE_CONNECTOR_HDMIA);
        assert_eq!(conn.mode_count, 4);
        assert_eq!(conn.modes[0].name_as_str(), "1920x1080");
    }

    #[test]
    fn test_dumb_buffer_and_framebuffer_lifecycle() {
        let mut dev = KmsDevice::new();
        dev.init();

        let (handle1, pitch1, size1) = dev.create_dumb(1920, 1080, 32).expect("create_dumb failed");
        assert_eq!(handle1, 1);
        assert!(pitch1 >= 1920 * 4);
        assert_eq!(size1, pitch1 as u64 * 1080);

        let fb1 = dev.add_framebuffer(1920, 1080, pitch1, 32, 24, handle1, DRM_FORMAT_XRGB8888)
            .expect("add_framebuffer failed");
        assert_eq!(fb1, 100);

        // Delete framebuffer and dumb buffer
        assert!(dev.remove_framebuffer(fb1).is_ok());
        assert!(dev.destroy_dumb(handle1).is_ok());
    }

    #[test]
    fn test_atomic_page_flip_on_vblank() {
        let mut dev = KmsDevice::new();
        dev.init();

        let (h1, p1, _) = dev.create_dumb(1920, 1080, 32).unwrap();
        let (h2, p2, _) = dev.create_dumb(1920, 1080, 32).unwrap();
        let fb1 = dev.add_framebuffer(1920, 1080, p1, 32, 24, h1, DRM_FORMAT_XRGB8888).unwrap();
        let fb2 = dev.add_framebuffer(1920, 1080, p2, 32, 24, h2, DRM_FORMAT_XRGB8888).unwrap();

        // Set mode with fb1
        dev.set_crtc(1, fb1, DrmModeInfo::mode_1920x1080_60(), 0, 0).unwrap();
        assert_eq!(dev.crtcs[0].active_fb_id, fb1);

        // Queue page flip to fb2
        let user_tag = 0xDEAD_BEEF_CAFE;
        dev.page_flip(1, fb2, DRM_MODE_PAGE_FLIP_EVENT, user_tag).unwrap();
        assert_eq!(dev.crtcs[0].flip_pending, true);
        assert_eq!(dev.crtcs[0].active_fb_id, fb1); // Still fb1 before VBLANK

        // Second page flip while first is pending must fail with EBUSY (-16)
        assert_eq!(dev.page_flip(1, fb1, 0, 0), Err(-16));

        // Fire VBLANK
        let flip_ev = dev.trigger_vblank(1).expect("expected flip completion event");
        assert_eq!(flip_ev.user_data, user_tag);
        assert_eq!(flip_ev.sequence, 1);
        assert_eq!(dev.crtcs[0].active_fb_id, fb2); // Now atomically swapped to fb2!
        assert_eq!(dev.crtcs[0].flip_pending, false);
        assert_eq!(dev.planes[0].fb_id, fb2); // Primary plane updated
    }

    #[test]
    fn test_hardware_cursor_positioning_without_desktop_redraw() {
        let mut dev = KmsDevice::new();
        dev.init();

        // Initially disabled
        assert_eq!(dev.cursor.enabled, false);
        assert_eq!(dev.planes[1].visible, false);

        // Set 64x64 cursor image with hotspot at (5, 5)
        dev.set_cursor(1, 42, 64, 64, 5, 5).unwrap();
        assert_eq!(dev.cursor.enabled, true);
        assert_eq!(dev.planes[1].visible, true);
        assert_eq!(dev.cursor.width, 64);
        assert_eq!(dev.cursor.height, 64);

        // Move cursor to (400, 300)
        dev.move_cursor(1, 400, 300).unwrap();
        assert_eq!(dev.cursor.x, 400);
        assert_eq!(dev.cursor.y, 300);
        assert_eq!(dev.planes[1].crtc_x, 395); // 400 - 5
        assert_eq!(dev.planes[1].crtc_y, 295); // 300 - 5
        assert_eq!(dev.cursor.move_count, 1);

        // Move cursor again to (1024, 768)
        dev.move_cursor(1, 1024, 768).unwrap();
        assert_eq!(dev.cursor.x, 1024);
        assert_eq!(dev.cursor.y, 768);
        assert_eq!(dev.cursor.move_count, 2);
    }

    #[test]
    fn test_overlay_plane_configuration() {
        let mut dev = KmsDevice::new();
        dev.init();

        let overlay_fb = 105;
        dev.set_plane(3, 1, overlay_fb, 100, 100, 640, 480, 0, 0, 640 << 16, 480 << 16).unwrap();

        let plane = &dev.planes[2];
        assert_eq!(plane.id, 3);
        assert_eq!(plane.fb_id, overlay_fb);
        assert_eq!(plane.crtc_x, 100);
        assert_eq!(plane.crtc_y, 100);
        assert_eq!(plane.crtc_w, 640);
        assert_eq!(plane.crtc_h, 480);
        assert_eq!(plane.visible, true);
        assert_eq!(plane.supports_format(DRM_FORMAT_NV12), true);
    }
}
