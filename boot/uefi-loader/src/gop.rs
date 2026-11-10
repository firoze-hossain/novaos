//! boot/uefi-loader/src/gop.rs - UEFI Graphics Output Protocol (GOP) Engine
//!
//! Provides pure `#![no_std]` UEFI 2.9 GOP discovery and display mode configuration:
//!   - Queries GOP modes for native high-definition display resolutions (1080p / 4K / QHD).
//!   - Extracts linear framebuffer physical base address, size, width, height, and stride.
//!   - Handles standard pixel color formats (PixelBlueGreenRedReserved8BitPerColor / RGB).
//!   - Prepares seamless linear framebuffer handover to NovaOS 64-bit kernel without mode shifts.

// -----------------------------------------------------------------------------
// GOP Pixel Formats & Constants
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum GopPixelFormat {
    PixelRedGreenBlueReserved8BitPerColor = 0,
    PixelBlueGreenRedReserved8BitPerColor = 1,
    PixelBitMask = 2,
    PixelBltOnly = 3,
}

impl Default for GopPixelFormat {
    fn default() -> Self {
        Self::PixelBlueGreenRedReserved8BitPerColor
    }
}

/// Standard GOP mode information descriptor (UEFI 2.9).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct GopModeInfo {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: GopPixelFormat,
    pub pixels_per_scan_line: u32,
}

/// Active Linear Framebuffer descriptor passed to the kernel BootInfo.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct GopFramebuffer {
    pub base_phys: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: GopPixelFormat,
    pub bits_per_pixel: u8,
}

impl GopFramebuffer {
    pub const fn new(
        base_phys: u64,
        size: u64,
        width: u32,
        height: u32,
        stride: u32,
        format: GopPixelFormat,
    ) -> Self {
        Self {
            base_phys,
            size,
            width,
            height,
            stride,
            format,
            bits_per_pixel: 32,
        }
    }

    /// Returns byte offset for pixel (x, y).
    pub fn pixel_byte_offset(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            let offset = (y as u64 * self.stride as u64 + x as u64) * 4;
            if offset + 4 <= self.size {
                Some(offset as usize)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Returns true if this framebuffer meets modern HD display requirements (>= 1080p).
    pub fn is_high_definition(&self) -> bool {
        self.width >= 1920 && self.height >= 1080
    }

    /// Returns true if this framebuffer is 4K Ultra-HD (>= 3840x2160).
    pub fn is_ultra_hd(&self) -> bool {
        self.width >= 3840 && self.height >= 2160
    }
}

// -----------------------------------------------------------------------------
// Mode Selection Engine
// -----------------------------------------------------------------------------

/// Scores a GOP video mode based on resolution and pixel format preference.
pub fn score_gop_mode(info: &GopModeInfo) -> u32 {
    let mut score: u32 = 0;

    // Prefer 32-bit direct color formats
    match info.pixel_format {
        GopPixelFormat::PixelBlueGreenRedReserved8BitPerColor => score += 1000,
        GopPixelFormat::PixelRedGreenBlueReserved8BitPerColor => score += 950,
        GopPixelFormat::PixelBitMask => score += 500,
        GopPixelFormat::PixelBltOnly => return 0, // Unusable for linear framebuffer
    }

    // Resolution scoring
    let pixels = (info.width as u64) * (info.height as u64);
    if info.width == 3840 && info.height == 2160 {
        score += 50_000; // Native 4K UHD
    } else if info.width == 2560 && info.height == 1440 {
        score += 40_000; // 1440p QHD
    } else if info.width == 1920 && info.height == 1080 {
        score += 35_000; // Native 1080p FHD
    } else if info.width == 1024 && info.height == 768 {
        score += 10_000; // Safe XGA fallback
    } else {
        // Linear scale based on total pixels
        score += (pixels / 50_000) as u32;
    }

    score
}

/// Selects the best GOP mode from a list of available candidate modes.
pub fn select_best_mode(modes: &[GopModeInfo]) -> Option<(usize, GopModeInfo)> {
    let mut best_idx = None;
    let mut best_score = 0;
    let mut best_info = GopModeInfo::default();

    for (idx, mode) in modes.iter().enumerate() {
        let score = score_gop_mode(mode);
        if score > best_score {
            best_score = score;
            best_idx = Some(idx);
            best_info = *mode;
        }
    }

    best_idx.map(|idx| (idx, best_info))
}
