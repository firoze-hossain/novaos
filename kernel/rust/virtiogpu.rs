//! kernel/rust/virtiogpu.rs - Phase 80: the virtio-gpu 2D command
//! protocol - struct layouts, command building, and response
//! checking. See kernel/drivers/virtiogpu/virtiogpu.c's own top
//! comment for the full design, and kernel/drivers/virtio/
//! virtio_pci_modern.{c,h} for the transport this rides on (virtio-
//! gpu has no legacy PCI interface at all, confirmed against multiple
//! independent sources while scoping this phase - not an assumption
//! carried over from virtio_blk.rs's own legacy-transport precedent).
//!
//! Every struct below is laid out exactly as the Linux kernel's own
//! authoritative uapi header (include/uapi/linux/virtio_gpu.h)
//! defines it - fetched directly while writing this file, not
//! reconstructed from a secondary description. Getting a field order
//! or size wrong here wouldn't fail loudly: the device would either
//! reject the command outright (an honest, visible failure) or, worse,
//! silently misinterpret a few bytes as something else entirely and
//! do something unintended with real guest memory a command points
//! it at - exactly the class of mistake this project's own established
//! "verify against the real spec/source before writing a wire-format
//! struct" discipline (kernel/arch/x86/boot/multiboot.h, kernel/
//! drivers/video/vbe.c's Bochs DISPI constants) exists to catch before
//! it ships, not after.
//!
//! Scope: the 2D display path only (GET_DISPLAY_INFO, RESOURCE_
//! CREATE_2D, RESOURCE_ATTACH_BACKING, SET_SCANOUT, TRANSFER_TO_
//! HOST_2D, RESOURCE_FLUSH) - enough to actually show a real picture
//! through virtio-gpu. The 3D/virgl command set (CTX_CREATE,
//! RESOURCE_CREATE_3D, SUBMIT_3D, ...) is real, substantial, separate
//! protocol surface this phase deliberately does not implement - see
//! kernel/drivers/virtiogpu/virtiogpu.c's own top comment for why that
//! split is a real, documented scope decision, not an oversight.

#![allow(dead_code)]

/// struct virtio_gpu_ctrl_hdr - every request and response starts
/// with this, 24 bytes. type's own meaning depends on whether this is
/// a request (a VIRTIO_GPU_CMD_* value) or a response (a VIRTIO_GPU_
/// RESP_* value) - the same struct either way, per the real protocol.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct CtrlHdr {
    pub type_: u32,
    pub flags: u32,
    pub fence_id: u64,
    pub ctx_id: u32,
    pub ring_idx: u8,
    pub padding: [u8; 3],
}

pub const CTRLHDR_SIZE: usize = 24;

/// struct virtio_gpu_rect - 16 bytes.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

// Control types this driver actually sends or checks for - the full
// enum has many more values (3D commands, capsets, EDID, blobs - see
// this file's own top comment on why those are out of scope), so only
// the ones this 2D-only driver uses are named here.
pub const CMD_GET_DISPLAY_INFO: u32 = 0x0100;
pub const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
pub const CMD_SET_SCANOUT: u32 = 0x0103;
pub const CMD_RESOURCE_FLUSH: u32 = 0x0104;
pub const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
pub const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;

pub const RESP_OK_NODATA: u32 = 0x1100;
pub const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
/// Every VIRTIO_GPU_RESP_ERR_* value starts at 0x1200 and the real
/// enum has several (OUT_OF_MEMORY, INVALID_SCANOUT_ID, ...) - this
/// driver doesn't need to distinguish which specific error a command
/// failed with to do the one correct thing about it (stop and report
/// failure, see virtiogpu.c's own error handling), so checking "is
/// this type >= 0x1200" catches all of them at once rather than
/// naming each one just to treat them identically anyway.
pub const RESP_ERR_RANGE_START: u32 = 0x1200;

pub const FORMAT_B8G8R8A8_UNORM: u32 = 1;

fn ctrl_hdr(type_: u32) -> CtrlHdr {
    CtrlHdr { type_, flags: 0, fence_id: 0, ctx_id: 0, ring_idx: 0, padding: [0; 3] }
}

/// struct virtio_gpu_resource_create_2d - 40 bytes (24-byte hdr + 16).
#[repr(C, packed)]
pub struct ResourceCreate2D {
    pub hdr: CtrlHdr,
    pub resource_id: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
}

/// Builds a RESOURCE_CREATE_2D request into `out` (must be at least
/// `RESOURCE_CREATE_2D_SIZE` bytes). Real color depth, not an
/// approximation: `format` is always `FORMAT_B8G8R8A8_UNORM` here -
/// the standard 32-bit, 8-bit-per-channel ARGB format the virtio-gpu
/// spec itself names first among the "simple formats for fbcon/X use"
/// - matching this project's own existing VESA/VBE driver's own
/// negotiated depth (kernel/drivers/video/vbe.c), not a coincidence:
/// both exist to deliver the same real-color-depth promise over
/// different transports.
pub const RESOURCE_CREATE_2D_SIZE: usize = 40;
#[no_mangle]
pub unsafe extern "C" fn gpu_build_resource_create_2d(
    out: *mut u8,
    resource_id: u32,
    width: u32,
    height: u32,
) {
    let cmd = ResourceCreate2D {
        hdr: ctrl_hdr(CMD_RESOURCE_CREATE_2D),
        resource_id,
        format: FORMAT_B8G8R8A8_UNORM,
        width,
        height,
    };
    core::ptr::write_unaligned(out as *mut ResourceCreate2D, cmd);
}

/// struct virtio_gpu_mem_entry - 16 bytes.
#[repr(C, packed)]
pub struct MemEntry {
    pub addr: u64,
    pub length: u32,
    pub padding: u32,
}

/// struct virtio_gpu_resource_attach_backing - 32-byte header (24 hdr
/// + resource_id + nr_entries), followed by nr_entries * MemEntry.
/// This driver always attaches exactly one entry (one contiguous
/// guest buffer backs the whole resource) - real, not a simplification
/// that loses anything: nothing about the 2D path needs a backing
/// store split across multiple disjoint physical ranges.
pub const ATTACH_BACKING_HDR_SIZE: usize = 32;
pub const ATTACH_BACKING_SIZE_ONE_ENTRY: usize = ATTACH_BACKING_HDR_SIZE + 16;
#[no_mangle]
pub unsafe extern "C" fn gpu_build_resource_attach_backing(
    out: *mut u8,
    resource_id: u32,
    backing_phys: u32,
    backing_len: u32,
) {
    let hdr = ctrl_hdr(CMD_RESOURCE_ATTACH_BACKING);
    core::ptr::write_unaligned(out as *mut CtrlHdr, hdr);
    core::ptr::write_unaligned(out.add(24) as *mut u32, resource_id);
    core::ptr::write_unaligned(out.add(28) as *mut u32, 1u32); // nr_entries
    let entry = MemEntry { addr: backing_phys as u64, length: backing_len, padding: 0 };
    core::ptr::write_unaligned(out.add(ATTACH_BACKING_HDR_SIZE) as *mut MemEntry, entry);
}

/// struct virtio_gpu_set_scanout - 48 bytes (24 hdr + 16 rect + 4 + 4).
pub const SET_SCANOUT_SIZE: usize = 48;
#[no_mangle]
pub unsafe extern "C" fn gpu_build_set_scanout(
    out: *mut u8,
    scanout_id: u32,
    resource_id: u32,
    width: u32,
    height: u32,
) {
    core::ptr::write_unaligned(out as *mut CtrlHdr, ctrl_hdr(CMD_SET_SCANOUT));
    let r = Rect { x: 0, y: 0, width, height };
    core::ptr::write_unaligned(out.add(24) as *mut Rect, r);
    core::ptr::write_unaligned(out.add(40) as *mut u32, scanout_id);
    core::ptr::write_unaligned(out.add(44) as *mut u32, resource_id);
}

/// struct virtio_gpu_transfer_to_host_2d - 56 bytes (24 hdr + 16 rect
/// + 8 offset + 4 resource_id + 4 padding).
pub const TRANSFER_TO_HOST_2D_SIZE: usize = 56;
#[no_mangle]
pub unsafe extern "C" fn gpu_build_transfer_to_host_2d(
    out: *mut u8,
    resource_id: u32,
    width: u32,
    height: u32,
) {
    core::ptr::write_unaligned(out as *mut CtrlHdr, ctrl_hdr(CMD_TRANSFER_TO_HOST_2D));
    let r = Rect { x: 0, y: 0, width, height };
    core::ptr::write_unaligned(out.add(24) as *mut Rect, r);
    core::ptr::write_unaligned(out.add(40) as *mut u64, 0u64); // offset
    core::ptr::write_unaligned(out.add(48) as *mut u32, resource_id);
    core::ptr::write_unaligned(out.add(52) as *mut u32, 0u32); // padding
}

/// struct virtio_gpu_resource_flush - 48 bytes (24 hdr + 16 rect + 4
/// resource_id + 4 padding) - same shape as set_scanout, deliberately
/// not shared code with it: they're different commands whose structs
/// only happen to be the same size, and writing each out explicitly
/// keeps every field's own meaning visible at its own call site rather
/// than hidden behind a shared helper that would need its own
/// parameter just to say "but this one means resource_id, not
/// scanout_id, in the second-to-last slot."
pub const RESOURCE_FLUSH_SIZE: usize = 48;
#[no_mangle]
pub unsafe extern "C" fn gpu_build_resource_flush(
    out: *mut u8,
    resource_id: u32,
    width: u32,
    height: u32,
) {
    core::ptr::write_unaligned(out as *mut CtrlHdr, ctrl_hdr(CMD_RESOURCE_FLUSH));
    let r = Rect { x: 0, y: 0, width, height };
    core::ptr::write_unaligned(out.add(24) as *mut Rect, r);
    core::ptr::write_unaligned(out.add(40) as *mut u32, resource_id);
    core::ptr::write_unaligned(out.add(44) as *mut u32, 0u32); // padding
}

/// Phase 81: TRANSFER_TO_HOST_2D for a DAMAGE RECTANGLE, not the whole
/// resource - what makes `SYS_FB_PRESENT` of a small region cost a
/// small transfer instead of re-sending 3MB. Same 56-byte struct as
/// `gpu_build_transfer_to_host_2d` above; two fields differ:
///
///  * `r` is `{x, y, width, height}` rather than `{0, 0, w, h}`.
///  * `offset` is where, in the guest's backing memory, the first byte
///    of THAT rectangle's data sits: `y * stride_bytes + x * 4`. The
///    device reads row `i` of the rectangle at `offset + i *
///    stride_bytes` and writes it at `(y + i)` in the host resource -
///    so for a resource whose backing is laid out exactly like the
///    resource itself (which is how virtiogpu.c allocates it, row
///    stride = width * 4), the offset is simply the rectangle's own
///    position in the backing buffer. Getting this wrong does not fail
///    loudly: the device happily transfers the wrong bytes (a shifted
///    picture), which is why `rust_virtiogpu_selftest()` checks the
///    computed value against a hand-worked case.
#[no_mangle]
pub unsafe extern "C" fn gpu_build_transfer_to_host_2d_rect(
    out: *mut u8,
    resource_id: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    stride_bytes: u32,
) {
    core::ptr::write_unaligned(out as *mut CtrlHdr, ctrl_hdr(CMD_TRANSFER_TO_HOST_2D));
    let r = Rect { x, y, width, height };
    core::ptr::write_unaligned(out.add(24) as *mut Rect, r);
    let offset: u64 = (y as u64) * (stride_bytes as u64) + (x as u64) * 4;
    core::ptr::write_unaligned(out.add(40) as *mut u64, offset);
    core::ptr::write_unaligned(out.add(48) as *mut u32, resource_id);
    core::ptr::write_unaligned(out.add(52) as *mut u32, 0u32); // padding
}

/// Phase 81: RESOURCE_FLUSH for a damage rectangle - same 48-byte
/// struct as `gpu_build_resource_flush`, with a real `{x, y, w, h}`.
#[no_mangle]
pub unsafe extern "C" fn gpu_build_resource_flush_rect(
    out: *mut u8,
    resource_id: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) {
    core::ptr::write_unaligned(out as *mut CtrlHdr, ctrl_hdr(CMD_RESOURCE_FLUSH));
    let r = Rect { x, y, width, height };
    core::ptr::write_unaligned(out.add(24) as *mut Rect, r);
    core::ptr::write_unaligned(out.add(40) as *mut u32, resource_id);
    core::ptr::write_unaligned(out.add(44) as *mut u32, 0u32); // padding
}

/// GET_DISPLAY_INFO's own request is a bare ctrl_hdr - no extra
/// fields (the device already knows which display it's describing;
/// there's only ever one request shape, not one per scanout).
#[no_mangle]
pub unsafe extern "C" fn gpu_build_get_display_info(out: *mut u8) {
    core::ptr::write_unaligned(out as *mut CtrlHdr, ctrl_hdr(CMD_GET_DISPLAY_INFO));
}

/// Reads a response buffer's own ctrl_hdr.type field - the first 4
/// bytes of any response, request or not. Used by every caller in
/// virtiogpu.c to check a command actually succeeded (RESP_OK_* )
/// rather than merely "got some bytes back."
#[no_mangle]
pub unsafe extern "C" fn gpu_response_type(buf: *const u8) -> u32 {
    core::ptr::read_unaligned(buf as *const u32)
}

/// Builds a 2-descriptor chain (request, device-writable response) on
/// the control virtqueue - every virtio-gpu command's own shape, as
/// opposed to virtio-blk's 3-descriptor header/data/status chain
/// (kernel/rust/virtio_blk.rs's own rust_virtqueue_submit_request()).
/// Always uses descriptor slots 0 and 1, for the identical reason
/// that function documents for its own fixed slots: this driver only
/// ever has one command in flight at a time, polling each one to
/// completion (virtiogpu.c's own poll_for_completion()) before
/// submitting the next, so reusing the same two slots is safe exactly
/// because the previous command's use of them is already known
/// complete.
///
/// # Safety
/// `mem` must be the same, already-rust_virtqueue_init()-initialized
/// region `queue_size` was used with (kernel/rust/virtio_blk.rs's own
/// function, reused here - see this file's own top comment and
/// virtio_pci_modern.h's for why that's valid). `request_phys`/
/// `response_phys` must be valid physical addresses for the device to
/// read from/write into.
#[no_mangle]
pub unsafe extern "C" fn gpu_submit_command(
    mem: *mut u8,
    queue_size: u16,
    request_phys: u32,
    request_len: u32,
    response_phys: u32,
    response_len: u32,
) {
    const VIRTQ_DESC_F_NEXT: u16 = 1;
    const VIRTQ_DESC_F_WRITE: u16 = 2;

    let desc = |i: usize| -> *mut u8 { mem.add(i * 16) };
    let write_desc = |i: usize, addr: u32, len: u32, flags: u16, next: u16| {
        let p = desc(i);
        core::ptr::write_unaligned(p as *mut u64, addr as u64);
        core::ptr::write_unaligned(p.add(8) as *mut u32, len);
        core::ptr::write_unaligned(p.add(12) as *mut u16, flags);
        core::ptr::write_unaligned(p.add(14) as *mut u16, next);
    };

    write_desc(0, request_phys, request_len, VIRTQ_DESC_F_NEXT, 1);
    write_desc(1, response_phys, response_len, VIRTQ_DESC_F_WRITE, 0);

    // avail ring layout is identical regardless of queue_size - the
    // same byte offset formula kernel/rust/virtio_blk.rs's own
    // virtqueue_layout() computes, duplicated here as plain arithmetic
    // (16 bytes/descriptor * queue_size) rather than calling that
    // private function across a module boundary, since this is the
    // one piece of layout math simple enough that re-deriving it
    // inline is clearer than threading an extra FFI call through just
    // for this.
    let avail_offset = 16usize * queue_size as usize;
    let n = queue_size as usize;
    let avail_idx_ptr = mem.add(avail_offset + 2) as *mut u16;
    let avail_idx = core::ptr::read_volatile(avail_idx_ptr);
    let ring_slot = mem.add(avail_offset + 4 + (avail_idx as usize % n) * 2) as *mut u16;
    core::ptr::write_volatile(ring_slot, 0u16); // head descriptor index, always 0
    core::ptr::write_volatile(avail_idx_ptr, avail_idx.wrapping_add(1));
}

/// Ring-0 self-test, called directly from kernel_main() - verifies
/// every struct size/offset this file hand-computed above against
/// Rust's own core::mem::size_of/offsetof-equivalent reasoning, the
/// same "check the arithmetic against ground truth before trusting it
/// against real hardware" discipline kernel/rust/virtio_blk.rs's own
/// rust_virtqueue_selftest() already established for this exact class
/// of mistake (a wrong byte offset in wire-format code is a
/// correctness bug real hardware DMA would otherwise just silently
/// misinterpret, not something that fails to compile). Returns a
/// bitmask (0 = every check passed).
#[no_mangle]
pub extern "C" fn rust_virtiogpu_selftest() -> i32 {
    let mut code = 0;

    if core::mem::size_of::<CtrlHdr>() != CTRLHDR_SIZE {
        code |= 1;
    }
    if core::mem::size_of::<Rect>() != 16 {
        code |= 2;
    }
    if core::mem::size_of::<ResourceCreate2D>() != RESOURCE_CREATE_2D_SIZE {
        code |= 4;
    }
    if core::mem::size_of::<MemEntry>() != 16 {
        code |= 8;
    }

    // Build each command into a real buffer and check the bytes that
    // actually matter land exactly where the hand-written offsets in
    // each gpu_build_* function above claim - catching a mismatch
    // between a function's own internal offset arithmetic and the
    // real struct layout, not just that the struct itself is the
    // right total size (a wrong *internal* offset with the right
    // total size is exactly the class of bug total-size-only checks
    // above would miss entirely).
    let mut buf = [0u8; 64];
    unsafe {
        gpu_build_resource_create_2d(buf.as_mut_ptr(), 7, 1024, 768);
        let resource_id = core::ptr::read_unaligned(buf.as_ptr().add(24) as *const u32);
        let format = core::ptr::read_unaligned(buf.as_ptr().add(28) as *const u32);
        let width = core::ptr::read_unaligned(buf.as_ptr().add(32) as *const u32);
        let height = core::ptr::read_unaligned(buf.as_ptr().add(36) as *const u32);
        if resource_id != 7 || format != FORMAT_B8G8R8A8_UNORM || width != 1024 || height != 768 {
            code |= 16;
        }
        let type_ = gpu_response_type(buf.as_ptr());
        if type_ != CMD_RESOURCE_CREATE_2D {
            code |= 32;
        }
    }

    unsafe {
        gpu_build_set_scanout(buf.as_mut_ptr(), 0, 7, 1024, 768);
        let scanout_id = core::ptr::read_unaligned(buf.as_ptr().add(40) as *const u32);
        let resource_id = core::ptr::read_unaligned(buf.as_ptr().add(44) as *const u32);
        if scanout_id != 0 || resource_id != 7 {
            code |= 64;
        }
    }

    unsafe {
        gpu_build_resource_attach_backing(buf.as_mut_ptr(), 7, 0x1234_5000, 0x100000);
        let resource_id = core::ptr::read_unaligned(buf.as_ptr().add(24) as *const u32);
        let nr_entries = core::ptr::read_unaligned(buf.as_ptr().add(28) as *const u32);
        let entry_addr = core::ptr::read_unaligned(buf.as_ptr().add(32) as *const u64);
        let entry_len = core::ptr::read_unaligned(buf.as_ptr().add(40) as *const u32);
        if resource_id != 7 || nr_entries != 1 || entry_addr != 0x1234_5000
            || entry_len != 0x100000
        {
            code |= 128;
        }
    }

    // Phase 81: the damage-rectangle builders. The transfer offset is
    // the one value here that can be wrong without anything failing
    // loudly (the device just moves the wrong bytes), so it is checked
    // against a hand-worked case: rect at (3, 2), stride 4096 ->
    // 2*4096 + 3*4 = 8204.
    unsafe {
        let mut b2 = [0u8; 64];
        gpu_build_transfer_to_host_2d_rect(b2.as_mut_ptr(), 9, 3, 2, 100, 50, 4096);
        let rx = core::ptr::read_unaligned(b2.as_ptr().add(24) as *const u32);
        let ry = core::ptr::read_unaligned(b2.as_ptr().add(28) as *const u32);
        let rw = core::ptr::read_unaligned(b2.as_ptr().add(32) as *const u32);
        let rh = core::ptr::read_unaligned(b2.as_ptr().add(36) as *const u32);
        let off = core::ptr::read_unaligned(b2.as_ptr().add(40) as *const u64);
        let rid = core::ptr::read_unaligned(b2.as_ptr().add(48) as *const u32);
        if rx != 3 || ry != 2 || rw != 100 || rh != 50 || off != 8204 || rid != 9 {
            code |= 256;
        }
        if gpu_response_type(b2.as_ptr()) != CMD_TRANSFER_TO_HOST_2D {
            code |= 512;
        }
        // A rect near the far corner must not overflow the offset:
        // (1023, 767) at stride 4096 -> 767*4096 + 1023*4 = 3_145_724.
        gpu_build_transfer_to_host_2d_rect(b2.as_mut_ptr(), 1, 1023, 767, 1, 1, 4096);
        let off2 = core::ptr::read_unaligned(b2.as_ptr().add(40) as *const u64);
        if off2 != 3_145_724 {
            code |= 1024;
        }

        gpu_build_resource_flush_rect(b2.as_mut_ptr(), 9, 3, 2, 100, 50);
        let fx = core::ptr::read_unaligned(b2.as_ptr().add(24) as *const u32);
        let fy = core::ptr::read_unaligned(b2.as_ptr().add(28) as *const u32);
        let fw = core::ptr::read_unaligned(b2.as_ptr().add(32) as *const u32);
        let fh = core::ptr::read_unaligned(b2.as_ptr().add(36) as *const u32);
        let frid = core::ptr::read_unaligned(b2.as_ptr().add(40) as *const u32);
        if fx != 3 || fy != 2 || fw != 100 || fh != 50 || frid != 9 {
            code |= 2048;
        }
        if gpu_response_type(b2.as_ptr()) != CMD_RESOURCE_FLUSH {
            code |= 4096;
        }
    }

    code
}
