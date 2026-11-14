//! userland/desktop/nova-compositor/src/scale.rs
//!
//! NovaOS Wayland Display Server: High-Resolution Display Scaling (HiDPI / Retina)
//!
//! Provides dynamic display scaling (100%, 125%, 150%, 200%) across the compositor pipeline:
//!   - EDID physical monitor DPI detection (physical width/height in mm vs pixel resolution).
//!   - Automatic scale factor selection (1.0x, 1.25x, 1.5x, 2.0x).
//!   - Bi-directional coordinate mapping: logical surface space <-> physical hardware device pixels.
//!   - Buffer scaling negotiation (`wl_surface.set_buffer_scale`, `wp_viewport`).

#![allow(dead_code)]

use crate::surface::DamageRect;

pub const BASELINE_DPI: f32 = 96.0;

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
    pub fn factor(&self) -> f32 {
        match self {
            DisplayScale::Scale100 => 1.0,
            DisplayScale::Scale125 => 1.25,
            DisplayScale::Scale150 => 1.5,
            DisplayScale::Scale200 => 2.0,
        }
    }

    pub fn integer_scale(&self) -> u32 {
        match self {
            DisplayScale::Scale200 => 2,
            _ => 1,
        }
    }

    /// Fixed-point scale factor (permille: 1000 = 1.00x)
    pub fn permille(&self) -> u32 {
        match self {
            DisplayScale::Scale100 => 1000,
            DisplayScale::Scale125 => 1250,
            DisplayScale::Scale150 => 1500,
            DisplayScale::Scale200 => 2000,
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
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edid_dpi_detection() {
        // 27" 4K Monitor: 3840x2160, 597mm x 336mm -> ~163 DPI -> Scale150 or Scale200
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
}
