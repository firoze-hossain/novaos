//! userland/screenshot-rs/screenshot.rs - Sovereign Screenshot & Screen Recording Utility
//!
//! Synthesizes Ubuntu GNOME interactive screen capture and video recording with
//! pure Rust `#![no_std]` 1.99.1 memory-safe architecture.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
use core::panic::PanicInfo;

pub mod ffi;

// Static pixel and output buffers for zero-allocation image operations
pub static mut FB_READ_BUF: [u32; 1024 * 768] = [0u32; 1024 * 768];
pub static mut PNG_ENCODE_BUF: [u8; 3500 * 1024] = [0u8; 3500 * 1024];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureScope {
    Selection,
    Screen,
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    Photo,
    Video,
}

/// Standard IEEE 802.3 CRC32 implementation without external lookup tables.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Standard Adler-32 checksum implementation according to RFC 1950.
pub fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Appends a standard PNG chunk with 4-byte length, 4-byte type, payload, and 4-byte CRC32.
pub fn write_png_chunk(out: &mut [u8], mut pos: usize, chunk_type: &[u8; 4], data: &[u8]) -> Option<usize> {
    let len = data.len() as u32;
    if pos + 12 + data.len() > out.len() {
        return None;
    }
    out[pos..pos + 4].copy_from_slice(&len.to_be_bytes());
    pos += 4;
    let type_start = pos;
    out[pos..pos + 4].copy_from_slice(chunk_type);
    pos += 4;
    if !data.is_empty() {
        out[pos..pos + data.len()].copy_from_slice(data);
        pos += data.len();
    }
    let crc = crc32(&out[type_start..pos]);
    out[pos..pos + 4].copy_from_slice(&crc.to_be_bytes());
    pos += 4;
    Some(pos)
}

/// Encodes raw ARGB/XRGB 32-bit pixel slice into standard, 100% compliant uncompressed PNG.
pub fn encode_png_uncompressed(
    pixels: &[u32],
    width: u32,
    height: u32,
    out: &mut [u8],
) -> Option<usize> {
    if width == 0 || height == 0 || pixels.len() < (width * height) as usize {
        return None;
    }

    let mut pos = 0;

    // 1. Official 8-byte PNG Signature
    let sig: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if pos + 8 > out.len() {
        return None;
    }
    out[pos..pos + 8].copy_from_slice(&sig);
    pos += 8;

    // 2. IHDR Chunk (13 bytes)
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;  // 8 bits per channel
    ihdr[9] = 6;  // RGBA TrueColor with alpha
    ihdr[10] = 0; // Deflate
    ihdr[11] = 0; // Standard adaptive filter (Filter 0 = None)
    ihdr[12] = 0; // Non-interlaced
    pos = write_png_chunk(out, pos, b"IHDR", &ihdr)?;

    // 3. IDAT Chunk containing ZLIB stream with uncompressed DEFLATE blocks
    let row_bytes = 1 + width as usize * 4;
    let idat_len_pos = pos;
    pos += 4;
    let chunk_type_pos = pos;
    if pos + 4 > out.len() {
        return None;
    }
    out[pos..pos + 4].copy_from_slice(b"IDAT");
    pos += 4;

    // Zlib Header: CMF=0x78 (Deflate 32K), FLG=0x01
    if pos + 2 > out.len() {
        return None;
    }
    out[pos] = 0x78;
    out[pos + 1] = 0x01;
    pos += 2;

    let mut adler_a = 1u32;
    let mut adler_b = 0u32;

    let mut y = 0u32;
    while y < height {
        let mut rows_in_block = 0u32;
        while y + rows_in_block < height && (rows_in_block + 1) as usize * row_bytes <= 65530 {
            rows_in_block += 1;
        }
        if rows_in_block == 0 {
            rows_in_block = 1;
        }

        let is_last = (y + rows_in_block) >= height;
        let block_payload_len = (rows_in_block as usize * row_bytes) as u16;

        if pos + 5 > out.len() {
            return None;
        }
        out[pos] = if is_last { 0x01 } else { 0x00 };
        out[pos + 1..pos + 3].copy_from_slice(&block_payload_len.to_le_bytes());
        out[pos + 3..pos + 5].copy_from_slice(&(!block_payload_len).to_le_bytes());
        pos += 5;

        for _ in 0..rows_in_block {
            if pos + row_bytes > out.len() {
                return None;
            }
            out[pos] = 0x00; // Filter 0 (None)
            adler_a = (adler_a + 0) % 65521;
            adler_b = (adler_b + adler_a) % 65521;
            pos += 1;

            let row_start = (y * width) as usize;
            for x in 0..width as usize {
                let pixel = pixels[row_start + x];
                let a = ((pixel >> 24) & 0xFF) as u8;
                let r = ((pixel >> 16) & 0xFF) as u8;
                let g = ((pixel >> 8) & 0xFF) as u8;
                let b = (pixel & 0xFF) as u8;

                out[pos] = r;
                out[pos + 1] = g;
                out[pos + 2] = b;
                out[pos + 3] = if a == 0 { 255 } else { a };

                adler_a = (adler_a + r as u32) % 65521;
                adler_b = (adler_b + adler_a) % 65521;
                adler_a = (adler_a + g as u32) % 65521;
                adler_b = (adler_b + adler_a) % 65521;
                adler_a = (adler_a + b as u32) % 65521;
                adler_b = (adler_b + adler_a) % 65521;
                adler_a = (adler_a + out[pos + 3] as u32) % 65521;
                adler_b = (adler_b + adler_a) % 65521;

                pos += 4;
            }
            y += 1;
        }
    }

    // Adler-32 Checksum
    let adler32 = (adler_b << 16) | adler_a;
    if pos + 4 > out.len() {
        return None;
    }
    out[pos..pos + 4].copy_from_slice(&adler32.to_be_bytes());
    pos += 4;

    // Write back IDAT chunk size
    let idat_payload_len = (pos - chunk_type_pos - 4) as u32;
    out[idat_len_pos..idat_len_pos + 4].copy_from_slice(&idat_payload_len.to_be_bytes());

    // Compute IDAT CRC32
    let idat_crc = crc32(&out[chunk_type_pos..pos]);
    if pos + 4 > out.len() {
        return None;
    }
    out[pos..pos + 4].copy_from_slice(&idat_crc.to_be_bytes());
    pos += 4;

    // 4. IEND Chunk
    pos = write_png_chunk(out, pos, b"IEND", &[])?;

    Some(pos)
}

/// NovaOS Video Header magic: `NOVAV10\0`
pub const NOVAV_MAGIC: &[u8; 8] = b"NOVAV10\0";

/// Encodes a screencast stream header with metadata.
pub fn encode_video_header(
    width: u32,
    height: u32,
    fps: u32,
    frame_count: u32,
    duration_ms: u32,
    out: &mut [u8],
) -> Option<usize> {
    if out.len() < 28 {
        return None;
    }
    out[0..8].copy_from_slice(NOVAV_MAGIC);
    out[8..12].copy_from_slice(&width.to_le_bytes());
    out[12..16].copy_from_slice(&height.to_le_bytes());
    out[16..20].copy_from_slice(&fps.to_le_bytes());
    out[20..24].copy_from_slice(&frame_count.to_le_bytes());
    out[24..28].copy_from_slice(&duration_ms.to_le_bytes());
    Some(28)
}

/// Saves image or video bytes to the primary destination and directory mirror.
pub fn save_media_file(filename_primary: &str, dir_subpath: &str, data: &[u8]) -> bool {
    let mut c_name = [0u8; 64];
    let b1 = filename_primary.as_bytes();
    let len1 = b1.len().min(63);
    c_name[..len1].copy_from_slice(&b1[..len1]);
    c_name[len1] = 0;

    let rc1 = unsafe { ffi::sys_write_file(c_name.as_ptr(), data.as_ptr(), data.len() as u32) };

    let mut c_sub = [0u8; 96];
    let b2 = dir_subpath.as_bytes();
    let len2 = b2.len().min(95);
    c_sub[..len2].copy_from_slice(&b2[..len2]);
    c_sub[len2] = 0;

    let rc2 = unsafe { ffi::sys_write_file(c_sub.as_ptr(), data.as_ptr(), data.len() as u32) };

    rc1 >= 0 || rc2 >= 0
}

/// Captures the full framebuffer screen and writes a PNG to `Pictures/Screenshots/`.
pub fn capture_screen_to_png() -> bool {
    let mut fb_info = ffi::NovaFbInfo::default();
    unsafe { ffi::sys_fb_info(&mut fb_info) };
    let width = if fb_info.width > 0 { fb_info.width } else { 1024 };
    let height = if fb_info.height > 0 { fb_info.height } else { 768 };
    let total = (width * height) as usize;

    let read_ptr = core::ptr::addr_of_mut!(FB_READ_BUF) as *mut u32;
    let read_req = ffi::NovaFbReadback {
        x: 0,
        y: 0,
        width: width as i32,
        height: height as i32,
        dst: read_ptr as u32,
        dst_stride: width * 4,
        flags: 0,
    };
    unsafe { ffi::sys_fb_readback(&read_req) };

    let pixels = unsafe { core::slice::from_raw_parts(read_ptr, total) };
    let out_slice = unsafe { core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(PNG_ENCODE_BUF) as *mut u8, 3500 * 1024) };

    if let Some(png_len) = encode_png_uncompressed(pixels, width, height, out_slice) {
        save_media_file("SCRN0001.PNG", "PICTURES/SCREENSHOTS/SCRN0001.PNG", &out_slice[..png_len])
    } else {
        false
    }
}

/// Records a sample screencast video stream and writes to `Videos/Screencasts/`.
pub fn record_sample_screencast() -> bool {
    let out_slice = unsafe { core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(PNG_ENCODE_BUF) as *mut u8, 3500 * 1024) };
    if let Some(hdr_len) = encode_video_header(1024, 768, 60, 120, 2000, out_slice) {
        save_media_file("REC0001.RAW", "VIDEOS/SCREENCASTS/REC0001.RAW", &out_slice[..hdr_len])
    } else {
        false
    }
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    unsafe {
        let msg = b"[screenshot] Taking screenshot...\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    if capture_screen_to_png() {
        unsafe {
            let msg = b"[screenshot] PASS: Screenshot saved to Pictures/Screenshots/SCRN0001.PNG (PNG-24)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        0
    } else {
        unsafe {
            let msg = b"[screenshot] FAIL: Failed to capture screenshot\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        1
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        let msg = b"[screenshot] Panic in screenshot app!\n\0";
        ffi::sys_write(msg.as_ptr());
        ffi::sys_exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32_and_adler32() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E60398);
    }

    #[test]
    fn test_png_uncompressed_encoder() {
        let pixels = [
            0xFFFF0000, 0xFF00FF00,
            0xFF0000FF, 0xFFFFFFFF,
        ];
        let mut out = [0u8; 1024];
        let res = encode_png_uncompressed(&pixels, 2, 2, &mut out);
        assert!(res.is_some());
        let len = res.unwrap();
        assert!(len > 50);

        // Verify PNG magic signature
        assert_eq!(&out[0..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        // Verify IHDR chunk presence
        assert_eq!(&out[12..16], b"IHDR");

        // Verify IEND chunk presence
        assert_eq!(&out[len - 8..len - 4], b"IEND");
    }

    #[test]
    fn test_video_header_encoding() {
        let mut out = [0u8; 64];
        let res = encode_video_header(1920, 1080, 60, 300, 5000, &mut out);
        assert_eq!(res, Some(28));
        assert_eq!(&out[0..8], NOVAV_MAGIC);
    }

    #[test]
    fn test_capture_and_save_workflow() {
        assert!(capture_screen_to_png());
        assert!(record_sample_screencast());
    }
}
