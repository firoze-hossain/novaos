//! kernel/rust/virtio_blk.rs - Phase 42: virtio-blk, this kernel's
//! first virtio driver.
//!
//! Gap this fills: every disk driver this kernel has (the ATA/IDE
//! driver FAT32/ext2 both mount through) talks to real, emulated
//! hardware the slow way - programmed I/O, no DMA. virtio is the
//! standard, purpose-built paravirtualized device interface every
//! serious hypervisor (QEMU included, which is what this project
//! develops against) implements specifically so a guest OS doesn't
//! need to emulate real hardware quirks at all - directly relevant to
//! this project's own portability goals (see NovaOS-Release-
//! Readiness-Kernel-and-Userland.md's own virtio-net/virtio-blk
//! entry). Scoped to virtio-blk specifically, not virtio-net, and not
//! wired into the VFS as a boot device - see kernel/drivers/virtio/
//! virtio_blk.c's own header comment for the full scope reasoning.
//!
//! Why the virtqueue specifically belongs in Rust: a virtqueue is a
//! fixed-size ring of descriptors plus two more rings (available,
//! used) layered on top, all cross-referenced by index - structurally
//! the same "ring buffer with index arithmetic that must never be
//! off by one" shape as kernel/rust/pipe.rs's own PIPES buffer, except
//! here a mistake doesn't just corrupt kernel memory silently, it
//! hands a real hardware DMA engine a bad physical address or length
//! and lets it write wherever that points. This is squarely the kind
//! of code this project's own standing rule (new kernel work attempted
//! in Rust first) exists for.
//!
//! Layout note: this module computes the *legacy* virtio virtqueue
//! byte layout (virtio spec, legacy/transitional interface) - three
//! regions (descriptor table, available ring, used ring) with specific
//! per-region byte sizes and a page-alignment requirement on the used
//! ring, computed here from `queue_size` (which is reported by the
//! device at runtime, not fixed by this driver - see virtio_blk.c).
//! No `VIRTIO_F_EVENT_IDX` support (this driver doesn't negotiate that
//! feature), so the avail/used rings are the plain, no-extra-field
//! shape - one honest simplification, not an oversight, since nothing
//! about correctness depends on it and negotiating fewer features is
//! strictly simpler and still spec-compliant.

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;

#[inline(always)]
fn align_4096(x: usize) -> usize {
    (x + 4095) & !4095usize
}

/// Computes the legacy virtqueue byte layout for a given queue size:
/// (avail_ring_offset, used_ring_offset, total_bytes_page_aligned).
/// All three values are computed, never assumed constant, because
/// `queue_size` itself is read from the device at runtime and this
/// driver does not require (or negotiate) any particular value - see
/// this file's own header comment.
fn virtqueue_layout(queue_size: u16) -> (usize, usize, usize) {
    let n = queue_size as usize;
    let desc_bytes = 16 * n; // each virtq_desc: u64 addr, u32 len, u16 flags, u16 next
    let avail_bytes = 4 + 2 * n; // flags(u16) + idx(u16) + ring[n](u16 each)
    let used_bytes = 4 + 8 * n; // flags(u16) + idx(u16) + elem[n](u32 id + u32 len each)

    let avail_offset = desc_bytes;
    let used_offset = align_4096(avail_offset + avail_bytes);
    let total = align_4096(used_offset + used_bytes);
    (avail_offset, used_offset, total)
}

/// Total bytes this queue's memory region needs, rounded up to a
/// whole number of 4096-byte frames - what the C side (virtio_blk.c)
/// uses to decide how many contiguous frames to request from
/// `pmm_alloc_contiguous()` before calling `rust_virtqueue_init`.
#[no_mangle]
pub extern "C" fn rust_virtqueue_total_bytes(queue_size: u16) -> u32 {
    let (_, _, total) = virtqueue_layout(queue_size);
    total as u32
}

#[no_mangle]
pub extern "C" fn rust_virtqueue_pages_needed(queue_size: u16) -> u32 {
    (rust_virtqueue_total_bytes(queue_size) / 4096) as u32
}

/// Zeroes the whole virtqueue memory region - a clean, all-zero
/// starting state is what the virtio spec expects (avail.idx == 0,
/// used.idx == 0, every descriptor initially unused).
///
/// # Safety
/// `mem` must point to at least `rust_virtqueue_total_bytes(queue_size)`
/// bytes of valid, writable memory, held for as long as this queue is
/// in use afterward.
#[no_mangle]
pub unsafe extern "C" fn rust_virtqueue_init(mem: *mut u8, queue_size: u16) {
    let total = rust_virtqueue_total_bytes(queue_size) as usize;
    core::ptr::write_bytes(mem, 0, total);
}

/// Submits a 3-descriptor chain (request header -> data buffer ->
/// 1-byte status) to the avail ring - the standard virtio-blk request
/// shape (virtio spec section 5.2.6). `write` selects the data
/// descriptor's direction from the *device's* perspective: `true`
/// means the driver is handing the device data to write out (a disk
/// write - the device only reads this descriptor), `false` means the
/// driver is asking the device to fill the buffer in (a disk read -
/// the device writes to it, so this descriptor needs
/// `VIRTQ_DESC_F_WRITE`). Always uses descriptor slots 0, 1, 2 for the
/// chain, never rotating through the rest of the table - correct, not
/// just simple, because this driver only ever has one request in
/// flight at a time (see virtio_blk.c's own comment on why that's an
/// honest, current scope limit): reusing the same three slots is safe
/// exactly because the previous request using them is guaranteed
/// complete (its completion polled, see `rust_virtqueue_poll_used`
/// below) before this function is ever called again.
///
/// Returns the head descriptor index used (always `0`, for the reason
/// above) - kept as a return value rather than hardcoded at the call
/// site so a future version of this driver that *does* support
/// multiple in-flight requests can change this function's internals
/// without also needing to change every caller.
///
/// # Safety
/// `mem` must be the same, already-`rust_virtqueue_init`-initialized
/// region `queue_size` was used with. `header_phys`/`data_phys`/
/// `status_phys` must be valid physical addresses for the device to
/// DMA into/out of - this function does not and cannot validate that
/// itself; the caller (virtio_blk.c) is responsible for passing real,
/// suitably-sized buffers it owns.
#[no_mangle]
pub unsafe extern "C" fn rust_virtqueue_submit_request(
    mem: *mut u8,
    queue_size: u16,
    header_phys: u32,
    header_len: u32,
    data_phys: u32,
    data_len: u32,
    status_phys: u32,
    write: bool,
) -> u16 {
    let (avail_offset, _used_offset, _total) = virtqueue_layout(queue_size);
    let n = queue_size as usize;

    let desc = |i: usize| -> *mut u8 { mem.add(i * 16) };
    let write_desc = |i: usize, addr: u32, len: u32, flags: u16, next: u16| {
        let p = desc(i);
        core::ptr::write_unaligned(p as *mut u64, addr as u64);
        core::ptr::write_unaligned(p.add(8) as *mut u32, len);
        core::ptr::write_unaligned(p.add(12) as *mut u16, flags);
        core::ptr::write_unaligned(p.add(14) as *mut u16, next);
    };

    // Descriptor 0: request header - always device-readable only.
    write_desc(0, header_phys, header_len, VIRTQ_DESC_F_NEXT, 1);
    // Descriptor 1: data buffer - direction depends on read vs write.
    let data_flags = if write {
        VIRTQ_DESC_F_NEXT
    } else {
        VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE
    };
    write_desc(1, data_phys, data_len, data_flags, 2);
    // Descriptor 2: 1-byte status - always device-written (the device
    // reports success/failure here), no further chain link.
    write_desc(2, status_phys, 1, VIRTQ_DESC_F_WRITE, 0);

    // Publish the chain via the avail ring, then bump avail.idx - in
    // that order, deliberately: the device must never observe an
    // incremented idx before the ring entry it points to is valid.
    let avail_idx_ptr = mem.add(avail_offset + 2) as *mut u16; // avail.idx is the 2nd u16 field
    let avail_idx = core::ptr::read_volatile(avail_idx_ptr);
    let ring_slot = mem.add(avail_offset + 4 + (avail_idx as usize % n) * 2) as *mut u16;
    core::ptr::write_volatile(ring_slot, 0u16); // head descriptor index, always 0
    core::ptr::write_volatile(avail_idx_ptr, avail_idx.wrapping_add(1));

    0
}

/// Checks the used ring for a newly-completed request. `last_used_idx`
/// is the caller's own record of how far it's already consumed (start
/// at 0, update with the value returned via `*last_used_idx` after
/// each call). Returns the completed request's descriptor id
/// (matching what `rust_virtqueue_submit_request` returned) if a new
/// completion is available, or `-1` if nothing new has completed yet -
/// a non-blocking check, meant to be polled in a loop by the caller,
/// matching this kernel's own established non-blocking-primitive
/// convention elsewhere (kernel/rust/pipe.rs's own `-2` "would block"
/// result is the same underlying idea, applied here to hardware
/// completion instead of a software ring buffer).
///
/// # Safety
/// `mem`/`queue_size` must match the queue this was `_init`'d and
/// `_submit_request`'d with. `last_used_idx` must point to a valid,
/// writable `u16`.
#[no_mangle]
pub unsafe extern "C" fn rust_virtqueue_poll_used(
    mem: *mut u8,
    queue_size: u16,
    last_used_idx: *mut u16,
) -> i32 {
    let (_avail_offset, used_offset, _total) = virtqueue_layout(queue_size);
    let n = queue_size as usize;

    let used_idx_ptr = mem.add(used_offset + 2) as *const u16;
    let used_idx = core::ptr::read_volatile(used_idx_ptr);
    let last = core::ptr::read(last_used_idx);

    if used_idx == last {
        return -1;
    }

    let elem_offset = used_offset + 4 + (last as usize % n) * 8;
    let id = core::ptr::read_volatile(mem.add(elem_offset) as *const u32);
    core::ptr::write(last_used_idx, last.wrapping_add(1));
    id as i32
}

/// Ring-0 self-test, called directly from kernel_main() - verifies
/// virtqueue_layout()'s byte-offset arithmetic against hand-computed
/// values for two queue sizes, before ever trusting it against real
/// hardware. Chosen specifically because this is the part of this
/// module most likely to hide an off-by-one - unlike pipe.rs's own
/// self-test, real hardware DMA is involved once this runs against an
/// actual device, so a layout mistake here is a correctness bug that
/// would otherwise only surface as the device silently misbehaving or
/// this kernel's memory getting corrupted, not a clean Rust panic.
/// Returns a bitmask (0 = every check passed), the same convention
/// kernel/rust/spinlock.rs's own self-test already uses.
#[no_mangle]
pub extern "C" fn rust_virtqueue_selftest() -> i32 {
    let mut code = 0;

    // Hand-computed for queue_size=4: desc=64B, avail_offset=64,
    // avail=4+8=12B -> used_offset rounds 76 up to 4096, used=4+32=36B
    // -> total rounds 4132 up to 8192.
    let (avail4, used4, total4) = virtqueue_layout(4);
    if avail4 != 64 || used4 != 4096 || total4 != 8192 {
        code |= 1;
    }

    // Hand-computed for queue_size=256 (QEMU's common virtio-blk-pci
    // default): desc=4096B (exactly 1 page), avail_offset=4096,
    // avail=4+512=516B -> used_offset rounds 4612 up to 8192,
    // used=4+2048=2052B -> total rounds 10244 up to 12288 (3 pages).
    let (avail256, used256, total256) = virtqueue_layout(256);
    if avail256 != 4096 || used256 != 8192 || total256 != 12288 {
        code |= 2;
    }

    if rust_virtqueue_pages_needed(256) != 3 {
        code |= 4;
    }

    code
}
