//! userland/desktop/nova-compositor/src/scale.rs
//!
//! NovaOS Wayland Display Server: High-Resolution Display Scaling (HiDPI / Retina)
//!
//! Provides dynamic display scaling (100%, 125%, 150%, 200%) across the compositor pipeline:
//!   - VESA EDID 1.3/1.4 physical monitor DPI detection.
//!   - Automatic scale factor selection (1.0x, 1.25x, 1.5x, 2.0x).
//!   - Multi-output geometry and per-output DPI management.
//!   - Surface-to-output intersection and preferred scale factor negotiation (`wl_surface.enter`).
//!   - Wayland buffer scaling (`wl_surface.set_buffer_scale`) and `wp_viewport` handling.
//!   - Conservative damage rectangle scaling with zero-leakage ceil/floor snapping.
//!   - Fixed-point permille coordinate transforms (logical points <-> physical device pixels).

#![allow(dead_code)]

use crate::surface::DamageRect;

pub const BASELINE_DPI: f32 = 96.0;
pub const MAX_OUTPUTS: usize = 4;
pub const EDID_BLOCK_SIZE: usize = 128;
pub const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

// -----------------------------------------------------------------------------
// Display Scale Factor
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum DisplayScale {
    #[default]
    Scale100, // 1.0x  (100% - 96 DPI)
    Scale125, // 1.25x (125% - 120 DPI)
    Scale150, // 1.5x  (150% - 144 DPI)
    Scale200, // 2.0x  (200% - 192 DPI Retina/4K)
}

impl DisplayScale {
    pub const fn factor(&self) -> f32 {
        match self {
            DisplayScale::Scale100 => 1.0,
            DisplayScale::Scale125 => 1.25,
            DisplayScale::Scale150 => 1.5,
            DisplayScale::Scale200 => 2.0,
        }
    }

    pub const fn integer_scale(&self) -> u32 {
        match self {
            DisplayScale::Scale200 => 2,
            _ => 1,
        }
    }

    /// Fixed-point scale factor (permille: 1000 = 1.00x)
    pub const fn permille(&self) -> u32 {
        match self {
            DisplayScale::Scale100 => 1000,
            DisplayScale::Scale125 => 1250,
            DisplayScale::Scale150 => 1500,
            DisplayScale::Scale200 => 2000,
        }
    }

    pub const fn percent(&self) -> u32 {
        match self {
            DisplayScale::Scale100 => 100,
            DisplayScale::Scale125 => 125,
            DisplayScale::Scale150 => 150,
            DisplayScale::Scale200 => 200,
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            DisplayScale::Scale100 => "100%",
            DisplayScale::Scale125 => "125%",
            DisplayScale::Scale150 => "150%",
            DisplayScale::Scale200 => "200% (Retina)",
        }
    }

    /// Auto-detects optimal scale factor from EDID physical metrics
    pub fn from_edid(width_mm: u32, height_mm: u32, res_w: u32, res_h: u32) -> Self {
        if width_mm == 0 || height_mm == 0 || res_w == 0 || res_h == 0 {
            return DisplayScale::Scale100;
        }

        // DPI = (res_pixels / mm) * 25.4
        let dpi_x = (res_w as f32 / width_mm as f32) * 25.4;
        let dpi_y = (res_h as f32 / height_mm as f32) * 25.4;
        let dpi = (dpi_x + dpi_y) / 2.0;

        Self::from_dpi(dpi)
    }

    /// Selects scale factor from physical DPI
    pub fn from_dpi(dpi: f32) -> Self {
        if dpi >= 180.0 {
            DisplayScale::Scale200
        } else if dpi >= 135.0 {
            DisplayScale::Scale150
        } else if dpi >= 110.0 {
            DisplayScale::Scale125
        } else {
            DisplayScale::Scale100
        }
    }

    /// Maps logical coordinate to physical device pixels
    pub fn to_physical(&self, logical: i32) -> i32 {
        ((logical as i64 * self.permille() as i64) / 1000) as i32
    }

    /// Maps physical device pixels to logical surface coordinate
    pub fn to_logical(&self, physical: i32) -> i32 {
        ((physical as i64 * 1000) / self.permille() as i64) as i32
    }

    /// Scales a damage rectangle from logical to physical device space
    pub fn scale_rect(&self, r: DamageRect) -> DamageRect {
        DamageRect {
            x: self.to_physical(r.x),
            y: self.to_physical(r.y),
            width: ((r.width as u64 * self.permille() as u64) / 1000) as u32,
            height: ((r.height as u64 * self.permille() as u64) / 1000) as u32,
        }
    }

    /// Unscales a damage rectangle from physical device space to logical surface space
    pub fn unscale_rect(&self, r: DamageRect) -> DamageRect {
        DamageRect {
            x: self.to_logical(r.x),
            y: self.to_logical(r.y),
            width: ((r.width as u64 * 1000) / self.permille() as u64) as u32,
            height: ((r.height as u64 * 1000) / self.permille() as u64) as u32,
        }
    }

    /// Conservative damage rectangle scaling: rounds (x,y) down (floor) and
    /// (x+w, y+h) up (ceil) to eliminate boundary seam artifacts
    pub fn scale_damage_conservative(&self, r: DamageRect) -> DamageRect {
        let perm = self.permille() as i64;
        let x0 = (r.x as i64 * perm) / 1000;
        let y0 = (r.y as i64 * perm) / 1000;
        let x1 = ((r.x as i64 + r.width as i64) * perm + 999) / 1000;
        let y1 = ((r.y as i64 + r.height as i64) * perm + 999) / 1000;

        DamageRect {
            x: x0 as i32,
            y: y0 as i32,
            width: (x1 - x0).max(0) as u32,
            height: (y1 - y0).max(0) as u32,
        }
    }

    /// Conservative damage rectangle unscaling: rounds (x,y) down (floor) and
    /// (x+w, y+h) up (ceil) to guarantee full coverage
    pub fn unscale_damage_conservative(&self, r: DamageRect) -> DamageRect {
        let perm = self.permille() as i64;
        let x0 = (r.x as i64 * 1000) / perm;
        let y0 = (r.y as i64 * 1000) / perm;
        let x1 = ((r.x as i64 + r.width as i64) * 1000 + perm - 1) / perm;
        let y1 = ((r.y as i64 + r.height as i64) * 1000 + perm - 1) / perm;

        DamageRect {
            x: x0 as i32,
            y: y0 as i32,
            width: (x1 - x0).max(0) as u32,
            height: (y1 - y0).max(0) as u32,
        }
    }
}

// -----------------------------------------------------------------------------
// EDID Parser & Monitor Profiler
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct EdidProfile {
    pub manufacturer: [u8; 3],
    pub product_code: u16,
    pub width_mm: u32,
    pub height_mm: u32,
    pub native_width: u32,
    pub native_height: u32,
    pub dpi: u32,
    pub recommended_scale: DisplayScale,
}

impl EdidProfile {
    pub fn parse(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() < EDID_BLOCK_SIZE {
            return Err("EDID buffer underflow: 128 bytes required");
        }
        if raw[0..8] != EDID_HEADER {
            return Err("Invalid EDID header magic");
        }

        // Verify checksum
        let mut sum: u8 = 0;
        for i in 0..EDID_BLOCK_SIZE {
            sum = sum.wrapping_add(raw[i]);
        }
        if sum != 0 {
            return Err("EDID checksum error");
        }

        // Manufacturer ID
        let b0 = raw[8];
        let b1 = raw[9];
        let c1 = ((b0 >> 2) & 0x1F) + b'@';
        let c2 = (((b0 & 0x03) << 3) | ((b1 >> 5) & 0x07)) + b'@';
        let c3 = (b1 & 0x1F) + b'@';

        let prod_code = (raw[10] as u16) | ((raw[11] as u16) << 8);

        // Parse detailed timing descriptor (Descriptor 0 at offset 54)
        let desc = &raw[54..72];
        let pclk = (desc[0] as u32) | ((desc[1] as u32) << 8);

        let (native_w, native_h, w_mm, h_mm) = if pclk != 0 {
            let h_active = (desc[2] as u32) | (((desc[4] >> 4) as u32 & 0x0F) << 8);
            let v_active = (desc[5] as u32) | (((desc[7] >> 4) as u32 & 0x0F) << 8);
            let d_w = (desc[12] as u32) | (((desc[14] >> 4) as u32 & 0x0F) << 8);
            let d_h = (desc[13] as u32) | ((desc[14] as u32 & 0x0F) << 8);
            (h_active, v_active, d_w, d_h)
        } else {
            (1920, 1080, raw[21] as u32 * 10, raw[22] as u32 * 10)
        };

        let dpi_x = if w_mm > 0 { ((native_w as u64 * 2540) / (w_mm as u64 * 100)) as u32 } else { 96 };
        let dpi_y = if h_mm > 0 { ((native_h as u64 * 2540) / (h_mm as u64 * 100)) as u32 } else { 96 };
        let dpi = (dpi_x + dpi_y) / 2;

        let rec_scale = DisplayScale::from_dpi(dpi as f32);

        Ok(Self {
            manufacturer: [c1, c2, c3],
            product_code: prod_code,
            width_mm: w_mm,
            height_mm: h_mm,
            native_width: native_w,
            native_height: native_h,
            dpi,
            recommended_scale: rec_scale,
        })
    }
}

// -----------------------------------------------------------------------------
// Multi-Monitor Output Management & Scale Negotiation
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct DisplayOutput {
    pub id: u32,
    pub name: [u8; 16],
    pub name_len: usize,
    /// Physical position in global compositor layout space
    pub global_x: i32,
    pub global_y: i32,
    /// Native hardware pixel dimensions
    pub width: u32,
    pub height: u32,
    /// Physical dimensions in millimeters
    pub width_mm: u32,
    pub height_mm: u32,
    /// Active display scale factor
    pub scale: DisplayScale,
}

impl Default for DisplayOutput {
    fn default() -> Self {
        let mut name = [0u8; 16];
        name[0..4].copy_from_slice(b"eDP1");
        Self {
            id: 1,
            name,
            name_len: 4,
            global_x: 0,
            global_y: 0,
            width: 1920,
            height: 1080,
            width_mm: 531,
            height_mm: 298,
            scale: DisplayScale::Scale100,
        }
    }
}

impl DisplayOutput {
    pub fn new(id: u32, name_str: &str, gx: i32, gy: i32, w: u32, h: u32, w_mm: u32, h_mm: u32) -> Self {
        let mut name = [0u8; 16];
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(16);
        name[..len].copy_from_slice(&bytes[..len]);

        let scale = DisplayScale::from_edid(w_mm, h_mm, w, h);

        Self {
            id,
            name,
            name_len: len,
            global_x: gx,
            global_y: gy,
            width: w,
            height: h,
            width_mm: w_mm,
            height_mm: h_mm,
            scale,
        }
    }

    /// Logical width of this output in compositor coordinate space
    pub fn logical_width(&self) -> u32 {
        self.scale.to_logical(self.width as i32) as u32
    }

    /// Logical height of this output in compositor coordinate space
    pub fn logical_height(&self) -> u32 {
        self.scale.to_logical(self.height as i32) as u32
    }

    /// Checks if a logical surface intersects this output's logical bounds
    pub fn intersects_logical(&self, sx: i32, sy: i32, sw: u32, sh: u32) -> bool {
        let lx0 = self.global_x;
        let ly0 = self.global_y;
        let lx1 = lx0 + self.logical_width() as i32;
        let ly1 = ly0 + self.logical_height() as i32;

        let rx0 = sx;
        let ry0 = sy;
        let rx1 = sx + sw as i32;
        let ry1 = sy + sh as i32;

        !(rx1 <= lx0 || rx0 >= lx1 || ry1 <= ly0 || ry0 >= ly1)
    }
}

/// Manages multiple display outputs in a multi-monitor setup
pub struct OutputManager {
    pub outputs: [Option<DisplayOutput>; MAX_OUTPUTS],
    pub count: usize,
}

impl OutputManager {
    pub const fn new() -> Self {
        Self {
            outputs: [None, None, None, None],
            count: 0,
        }
    }

    pub fn add_output(&mut self, output: DisplayOutput) -> bool {
        for slot in self.outputs.iter_mut() {
            if slot.is_none() {
                *slot = Some(output);
                self.count += 1;
                return true;
            }
        }
        false
    }

    pub fn get_output(&self, id: u32) -> Option<&DisplayOutput> {
        for slot in self.outputs.iter() {
            if let Some(ref o) = slot {
                if o.id == id {
                    return Some(o);
                }
            }
        }
        None
    }

    pub fn get_output_mut(&mut self, id: u32) -> Option<&mut DisplayOutput> {
        for slot in self.outputs.iter_mut() {
            if let Some(ref mut o) = slot {
                if o.id == id {
                    return Some(o);
                }
            }
        }
        None
    }

    /// Calculates which outputs a logical surface intersects
    pub fn find_intersecting_outputs(&self, sx: i32, sy: i32, sw: u32, sh: u32) -> [Option<u32>; MAX_OUTPUTS] {
        let mut res = [None; MAX_OUTPUTS];
        let mut idx = 0;
        for slot in self.outputs.iter() {
            if let Some(ref o) = slot {
                if o.intersects_logical(sx, sy, sw, sh) {
                    res[idx] = Some(o.id);
                    idx += 1;
                }
            }
        }
        res
    }

    /// Negotiates the preferred scale factor for a surface:
    /// In Wayland protocol, a client must receive the maximum scale factor among
    /// all outputs it currently intersects, guaranteeing crisp rendering on high-DPI.
    pub fn negotiate_surface_scale(&self, sx: i32, sy: i32, sw: u32, sh: u32) -> DisplayScale {
        let mut max_scale = DisplayScale::Scale100;
        for slot in self.outputs.iter() {
            if let Some(ref o) = slot {
                if o.intersects_logical(sx, sy, sw, sh) {
                    if o.scale.permille() > max_scale.permille() {
                        max_scale = o.scale;
                    }
                }
            }
        }
        max_scale
    }
}

// -----------------------------------------------------------------------------
// Wayland Buffer Scaling & Viewport Protocol
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct SurfaceScaleNegotiation {
    pub buffer_scale: u32,
    pub has_viewport: bool,
    pub viewport_src_w: f32,
    pub viewport_src_h: f32,
    pub viewport_dst_w: u32,
    pub viewport_dst_h: u32,
}

impl Default for SurfaceScaleNegotiation {
    fn default() -> Self {
        Self {
            buffer_scale: 1,
            has_viewport: false,
            viewport_src_w: 0.0,
            viewport_src_h: 0.0,
            viewport_dst_w: 0,
            viewport_dst_h: 0,
        }
    }
}

impl SurfaceScaleNegotiation {
    /// Sets client buffer scale factor (`wl_surface.set_buffer_scale`)
    pub fn set_buffer_scale(&mut self, scale: u32) {
        self.buffer_scale = scale.max(1);
    }

    /// Sets wp_viewport destination geometry
    pub fn set_viewport_destination(&mut self, dst_w: u32, dst_h: u32) {
        self.has_viewport = true;
        self.viewport_dst_w = dst_w;
        self.viewport_dst_h = dst_h;
    }

    /// Computes logical surface size from attached buffer dimensions
    pub fn logical_size(&self, buffer_w: u32, buffer_h: u32) -> (u32, u32) {
        if self.has_viewport && self.viewport_dst_w > 0 && self.viewport_dst_h > 0 {
            (self.viewport_dst_w, self.viewport_dst_h)
        } else {
            (
                (buffer_w + self.buffer_scale - 1) / self.buffer_scale,
                (buffer_h + self.buffer_scale - 1) / self.buffer_scale,
            )
        }
    }

    /// Computes final hardware scanout framebuffer dimensions on an output
    pub fn scanout_pixel_size(
        &self,
        buffer_w: u32,
        buffer_h: u32,
        output_scale: DisplayScale,
    ) -> (u32, u32) {
        let (lw, lh) = self.logical_size(buffer_w, buffer_h);
        (
            output_scale.to_physical(lw as i32) as u32,
            output_scale.to_physical(lh as i32) as u32,
        )
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edid_dpi_detection() {
        // 27" 4K Monitor: 3840x2160, 597mm x 336mm -> ~163 DPI -> Scale150
        let s_4k = DisplayScale::from_edid(597, 336, 3840, 2160);
        assert_eq!(s_4k, DisplayScale::Scale150);

        // 13.3" 4K Laptop: 3840x2160, 294mm x 165mm -> ~331 DPI -> Scale200
        let s_retina = DisplayScale::from_edid(294, 165, 3840, 2160);
        assert_eq!(s_retina, DisplayScale::Scale200);

        // 24" 1080p Monitor: 1920x1080, 531mm x 298mm -> ~92 DPI -> Scale100
        let s_1080p = DisplayScale::from_edid(531, 298, 1920, 1080);
        assert_eq!(s_1080p, DisplayScale::Scale100);

        // 14" 1080p Laptop: 1920x1080, 309mm x 174mm -> ~157 DPI -> Scale150
        let s_laptop = DisplayScale::from_edid(309, 174, 1920, 1080);
        assert_eq!(s_laptop, DisplayScale::Scale150);
    }

    #[test]
    fn test_coordinate_scaling_round_trip() {
        let s200 = DisplayScale::Scale200;
        assert_eq!(s200.to_physical(100), 200);
        assert_eq!(s200.to_logical(200), 100);

        let s150 = DisplayScale::Scale150;
        assert_eq!(s150.to_physical(100), 150);
        assert_eq!(s150.to_logical(150), 100);

        let r = DamageRect::new(20, 30, 400, 300);
        let phys_r = s200.scale_rect(r);
        assert_eq!(phys_r.x, 40);
        assert_eq!(phys_r.y, 60);
        assert_eq!(phys_r.width, 800);
        assert_eq!(phys_r.height, 600);
    }

    #[test]
    fn test_conservative_damage_scaling_no_leakage() {
        let s125 = DisplayScale::Scale125;
        let r = DamageRect::new(11, 13, 33, 47);
        let scaled = s125.scale_damage_conservative(r);

        // x0 = (11 * 1250) / 1000 = 13
        assert_eq!(scaled.x, 13);
        // y0 = (13 * 1250) / 1000 = 16
        assert_eq!(scaled.y, 16);
        // x1 = ((11 + 33) * 1250 + 999) / 1000 = (55000 + 999) / 1000 = 55
        // width = 55 - 13 = 42
        assert_eq!(scaled.width, 42);
        // y1 = ((13 + 47) * 1250 + 999) / 1000 = (75000 + 999) / 1000 = 75
        // height = 75 - 16 = 59
        assert_eq!(scaled.height, 59);

        // Verify that unscaled rect completely covers original damage
        let unscaled = s125.unscale_damage_conservative(scaled);
        assert!(unscaled.x <= r.x);
        assert!(unscaled.y <= r.y);
        assert!(unscaled.x + unscaled.width as i32 >= r.x + r.width as i32);
        assert!(unscaled.y + unscaled.height as i32 >= r.y + r.height as i32);
    }

    #[test]
    fn test_multi_monitor_output_scale_negotiation() {
        let mut mgr = OutputManager::new();

        // Monitor 1: Primary 4K Laptop Display at 200% (Retina): 3840x2160, 294x165mm -> 1920x1080 logical
        let m1 = DisplayOutput::new(1, "eDP-1", 0, 0, 3840, 2160, 294, 165);
        assert_eq!(m1.scale, DisplayScale::Scale200);
        assert_eq!(m1.logical_width(), 1920);
        assert_eq!(m1.logical_height(), 1080);
        mgr.add_output(m1);

        // Monitor 2: External 1080p Display at 100%: 1920x1080, 531x298mm -> 1920x1080 logical, positioned to right (x=1920)
        let m2 = DisplayOutput::new(2, "HDMI-1", 1920, 0, 1920, 1080, 531, 298);
        assert_eq!(m2.scale, DisplayScale::Scale100);
        assert_eq!(m2.logical_width(), 1920);
        assert_eq!(m2.logical_height(), 1080);
        mgr.add_output(m2);

        // Surface A: Fully inside Monitor 1 (Retina) -> Scale200
        let scale_a = mgr.negotiate_surface_scale(100, 100, 400, 300);
        assert_eq!(scale_a, DisplayScale::Scale200);

        // Surface B: Fully inside Monitor 2 (Standard) -> Scale100
        let scale_b = mgr.negotiate_surface_scale(2100, 100, 400, 300);
        assert_eq!(scale_b, DisplayScale::Scale100);

        // Surface C: Straddling both monitors (x=1800, w=400 crosses 1920 boundary) -> Max Scale200
        let scale_c = mgr.negotiate_surface_scale(1800, 100, 400, 300);
        assert_eq!(scale_c, DisplayScale::Scale200);
    }

    #[test]
    fn test_wayland_buffer_scale_and_viewport() {
        let mut nego = SurfaceScaleNegotiation::default();

        // 1. Unscaled buffer (scale = 1)
        nego.set_buffer_scale(1);
        let (lw, lh) = nego.logical_size(800, 600);
        assert_eq!(lw, 800);
        assert_eq!(lh, 600);

        // 2. Retina 2x buffer (scale = 2) -> logical size 400x300
        nego.set_buffer_scale(2);
        let (lw2, lh2) = nego.logical_size(800, 600);
        assert_eq!(lw2, 400);
        assert_eq!(lh2, 300);

        // When scanned out on 200% output, scanout size is native 800x600
        let (sw, sh) = nego.scanout_pixel_size(800, 600, DisplayScale::Scale200);
        assert_eq!(sw, 800);
        assert_eq!(sh, 600);

        // 3. Viewport explicit destination geometry overrides buffer scale
        nego.set_viewport_destination(500, 350);
        let (vw, vh) = nego.logical_size(800, 600);
        assert_eq!(vw, 500);
        assert_eq!(vh, 350);
    }
}
