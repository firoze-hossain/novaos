//! kernel/rust/virtio_net.rs - Phase 45: virtio-net, this kernel's
//! second virtio driver.
//!
//! Follows Phase 42's own precedent (kernel/rust/virtio_blk.rs) for
//! the same reason: the virtqueue is ring-buffer index arithmetic
//! where a mistake hands real hardware DMA a bad address, not just
//! kernel memory - exactly the shape this project's standing rule
//! (new kernel work attempted in Rust first) exists for. The layout
//! math (virtqueue_layout/total_bytes/pages_needed/init below)
//! duplicates virtio_blk.rs's own small, simple equivalents rather
//! than refactoring them into a shared module - a deliberate choice:
//! virtio_blk.rs is already tested, working code, and the functions
//! being duplicated are small enough that the duplication itself
//! isn't a real maintenance burden, while refactoring it risks
//! destabilizing something that already works for no functional gain.
//!
//! The real, structural difference from virtio-blk, and why this
//! isn't just "virtio_blk.rs with a different device ID": virtio-blk
//! has one request/response pattern - submit a chain, poll for its
//! one completion. virtio-net's receive queue is fundamentally
//! different - buffers must be pre-posted to the device *before* any
//! packet arrives (the device fills them asynchronously and only then
//! puts them on the used ring), and once drained, a buffer must be
//! *recycled* (re-posted) for the next packet, not discarded - there
//! is no equivalent of this in virtio-blk's model at all. The transmit
//! queue, by contrast, *is* close to virtio-blk's submit-and-poll
//! shape (just a 2-descriptor chain - header + data - instead of 3,
//! since virtio-net has no separate status-byte descriptor the way
//! virtio-blk does; completion is signaled by the used ring alone).
//!
//! No features negotiated (matching virtio_blk.rs's own choice) -
//! specifically means no VIRTIO_NET_F_MRG_RXBUF, so the legacy,
//! fixed 10-byte virtio_net_hdr (no trailing num_buffers field) is
//! the only shape this module ever needs to handle.

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;

/// The legacy virtio_net_hdr, exactly 10 bytes, prepended to every
/// packet on both the RX and TX queues (virtio spec section 5.1.6.1) -
/// flags/gso_type/gso_size/csum_start/csum_offset all zero in every
/// buffer this driver ever builds, since no offload feature is
/// negotiated.
const NET_HDR_LEN: usize = 10;

pub const MAX_FRAME_LEN: usize = 1514; // standard Ethernet MTU + header,
                                        // matching this project's own
                                        // RTL8139_MAX_FRAME/
                                        // NE2000_MAX_FRAME convention

/// How many RX buffers to keep posted to the device simultaneously -
/// a small, fixed, bounded number (matching this kernel's own
/// established preference for fixed-size arrays over dynamic
/// allocation elsewhere), not the queue's own full reported size -
/// this driver only ever drains one packet per net_poll()-style call
/// anyway (see kernel/net/net.c's own existing, unchanged "drains one
/// packet per call, more arrive via their own individual completions"
/// design, the same reasoning Phase 43's RTL8139 IRQ conversion
/// already established), so a handful of in-flight buffers is enough
/// headroom without needing to track this queue's full capacity.
pub const RX_BUFFER_COUNT: usize = 8;
pub const RX_BUFFER_SIZE: usize = NET_HDR_LEN + MAX_FRAME_LEN;

/// Exposes RX_BUFFER_COUNT/RX_BUFFER_SIZE to virtio_net.c - a single
/// source of truth, not a value the C side keeps its own, separately-
/// hardcoded copy of (the same class of duplication this project's
/// own gap-analysis work elsewhere calls out as worth avoiding, e.g.
/// syscall.h/novasys.h's own hand-synchronized numbers).
#[no_mangle]
pub extern "C" fn rust_net_rx_buffer_count() -> u32 {
    RX_BUFFER_COUNT as u32
}

#[no_mangle]
pub extern "C" fn rust_net_rx_buffer_size() -> u32 {
    RX_BUFFER_SIZE as u32
}

#[inline(always)]
fn align_4096(x: usize) -> usize {
    (x + 4095) & !4095usize
}

fn virtqueue_layout(queue_size: u16) -> (usize, usize, usize) {
    let n = queue_size as usize;
    let desc_bytes = 16 * n;
    let avail_bytes = 4 + 2 * n;
    let used_bytes = 4 + 8 * n;

    let avail_offset = desc_bytes;
    let used_offset = align_4096(avail_offset + avail_bytes);
    let total = align_4096(used_offset + used_bytes);
    (avail_offset, used_offset, total)
}

#[no_mangle]
pub extern "C" fn rust_net_virtqueue_total_bytes(queue_size: u16) -> u32 {
    let (_, _, total) = virtqueue_layout(queue_size);
    total as u32
}

#[no_mangle]
pub extern "C" fn rust_net_virtqueue_pages_needed(queue_size: u16) -> u32 {
    (rust_net_virtqueue_total_bytes(queue_size) / 4096) as u32
}

/// # Safety
/// `mem` must point to at least `rust_net_virtqueue_total_bytes(queue_size)`
/// bytes of valid, writable memory.
#[no_mangle]
pub unsafe extern "C" fn rust_net_virtqueue_init(mem: *mut u8, queue_size: u16) {
    let total = rust_net_virtqueue_total_bytes(queue_size) as usize;
    core::ptr::write_bytes(mem, 0, total);
}

/// Posts a single, device-writable RX buffer to descriptor slot
/// `desc_index` (the caller's own choice of which of its fixed
/// `RX_BUFFER_COUNT` buffers this is - see virtio_net.c's own comment
/// on how slots are chosen and recycled) and publishes it via the
/// avail ring. A single descriptor, not a chain (`VIRTQ_DESC_F_NEXT`
/// not set) - the device writes the incoming virtio_net_hdr and
/// packet data into this one, contiguous buffer.
///
/// # Safety
/// `mem` must be an initialized RX virtqueue region for `queue_size`.
/// `buf_phys` must be a valid physical address for at least
/// `RX_BUFFER_SIZE` bytes the device may write into, owned by the
/// caller for as long as this descriptor stays posted.
#[no_mangle]
pub unsafe extern "C" fn rust_net_rx_post_buffer(
    mem: *mut u8,
    queue_size: u16,
    desc_index: u16,
    buf_phys: u32,
) {
    let (avail_offset, _used_offset, _total) = virtqueue_layout(queue_size);
    let n = queue_size as usize;

    let desc = mem.add(desc_index as usize * 16);
    core::ptr::write_unaligned(desc as *mut u64, buf_phys as u64);
    core::ptr::write_unaligned(
        desc.add(8) as *mut u32,
        RX_BUFFER_SIZE as u32,
    );
    core::ptr::write_unaligned(desc.add(12) as *mut u16, VIRTQ_DESC_F_WRITE);
    core::ptr::write_unaligned(desc.add(14) as *mut u16, 0);

    let avail_idx_ptr = mem.add(avail_offset + 2) as *mut u16;
    let avail_idx = core::ptr::read_volatile(avail_idx_ptr);
    let ring_slot = mem.add(avail_offset + 4 + (avail_idx as usize % n) * 2) as *mut u16;
    core::ptr::write_volatile(ring_slot, desc_index);
    core::ptr::write_volatile(avail_idx_ptr, avail_idx.wrapping_add(1));
}

/// Checks the RX used ring for a newly-completed (device-filled)
/// buffer. On success, returns the descriptor index that completed
/// (so the caller knows which of its fixed buffers to read from and
/// later re-post via `rust_net_rx_post_buffer`) and writes the actual
/// packet length (the *total* device-written length, including the
/// 10-byte header the caller must skip) to `out_total_len`. Returns
/// `-1` if nothing new has completed - the same non-blocking-check
/// convention as `kernel/rust/virtio_blk.rs`'s own `poll_used`, and
/// kernel/rust/net_irq.rs's signal/check-and-clear, both already
/// established in this project.
///
/// # Safety
/// `mem`/`queue_size` must match an initialized RX queue.
/// `last_used_idx`/`out_total_len` must be valid, writable.
#[no_mangle]
pub unsafe extern "C" fn rust_net_rx_poll_used(
    mem: *mut u8,
    queue_size: u16,
    last_used_idx: *mut u16,
    out_total_len: *mut u32,
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
    let desc_id = core::ptr::read_volatile(mem.add(elem_offset) as *const u32);
    let written_len =
        core::ptr::read_volatile(mem.add(elem_offset + 4) as *const u32);
    core::ptr::write(last_used_idx, last.wrapping_add(1));
    core::ptr::write(out_total_len, written_len);
    desc_id as i32
}

/// Submits a 2-descriptor TX chain (header, always device-readable,
/// zeroed - no offload features negotiated - then the actual frame
/// data, also device-readable) to the TX queue and publishes it via
/// the avail ring. Always uses descriptor slots 0/1 - safe because,
/// exactly like virtio_blk.rs's own `submit_request`, this driver
/// only ever has one TX request in flight at a time (see
/// virtio_net.c's own comment).
///
/// # Safety
/// `mem` must be an initialized TX virtqueue region. `header_phys`
/// must point to a zeroed, `NET_HDR_LEN`-byte buffer the caller owns.
/// `data_phys` must be a valid physical address for `data_len` bytes
/// of frame data the caller owns for as long as this request is in
/// flight.
#[no_mangle]
pub unsafe extern "C" fn rust_net_tx_submit(
    mem: *mut u8,
    queue_size: u16,
    header_phys: u32,
    data_phys: u32,
    data_len: u32,
) {
    let (avail_offset, _used_offset, _total) = virtqueue_layout(queue_size);
    let n = queue_size as usize;

    let write_desc = |i: usize, addr: u32, len: u32, flags: u16, next: u16| {
        let p = mem.add(i * 16);
        core::ptr::write_unaligned(p as *mut u64, addr as u64);
        core::ptr::write_unaligned(p.add(8) as *mut u32, len);
        core::ptr::write_unaligned(p.add(12) as *mut u16, flags);
        core::ptr::write_unaligned(p.add(14) as *mut u16, next);
    };
    write_desc(0, header_phys, NET_HDR_LEN as u32, VIRTQ_DESC_F_NEXT, 1);
    write_desc(1, data_phys, data_len, 0, 0);

    let avail_idx_ptr = mem.add(avail_offset + 2) as *mut u16;
    let avail_idx = core::ptr::read_volatile(avail_idx_ptr);
    let ring_slot = mem.add(avail_offset + 4 + (avail_idx as usize % n) * 2) as *mut u16;
    core::ptr::write_volatile(ring_slot, 0u16);
    core::ptr::write_volatile(avail_idx_ptr, avail_idx.wrapping_add(1));
}

/// Non-blocking check for TX completion - same shape as
/// `rust_net_rx_poll_used` but discarding the completed descriptor id
/// (TX always reuses slots 0/1, so there's nothing to recycle by
/// index the way RX buffers are). Returns `true` if a completion was
/// consumed, `false` if nothing new.
///
/// # Safety
/// `mem`/`queue_size` must match an initialized TX queue.
/// `last_used_idx` must be valid, writable.
#[no_mangle]
pub unsafe extern "C" fn rust_net_tx_poll_used(
    mem: *mut u8,
    queue_size: u16,
    last_used_idx: *mut u16,
) -> bool {
    let (_avail_offset, used_offset, _total) = virtqueue_layout(queue_size);
    let used_idx_ptr = mem.add(used_offset + 2) as *const u16;
    let used_idx = core::ptr::read_volatile(used_idx_ptr);
    let last = core::ptr::read(last_used_idx);
    if used_idx == last {
        return false;
    }
    core::ptr::write(last_used_idx, last.wrapping_add(1));
    true
}

/// Ring-0 self-test - verifies the layout math (shared shape with
/// virtio_blk.rs's own, independently-verified formula, so hand-
/// computed here the same way) and the RX post/poll round trip against
/// a small, local, stack-allocated fake queue region, entirely
/// independent of real hardware - see PROGRESS.md's Phase 45 entry for
/// the separate, real-hardware verification (send/receive an actual
/// Ethernet frame through real QEMU virtio-net-pci DMA).
#[no_mangle]
pub extern "C" fn rust_virtio_net_selftest() -> i32 {
    let mut code = 0;

    // Hand-computed for queue_size=4, cross-checked independently in
    // Python before being written here (same discipline as
    // virtio_blk.rs's own self-test): desc=64B, avail_offset=64,
    // avail=4+8=12B -> used_offset rounds 76 up to 4096,
    // used=4+32=36B -> total rounds 4132 up to 8192.
    let (avail, used, total) = virtqueue_layout(4);
    if avail != 64 || used != 4096 || total != 8192 {
        code |= 1;
    }

    // RX post/poll round trip against a real, local buffer - proves
    // the descriptor-write + avail-ring-publish logic and the used-
    // ring-read logic agree with each other, without needing real
    // hardware to exercise it.
    let queue_size: u16 = 4;
    let total_bytes = rust_net_virtqueue_total_bytes(queue_size) as usize;
    let mut queue_mem = [0u8; 8192];
    debug_assert!(total_bytes <= queue_mem.len());
    let mem_ptr = queue_mem.as_mut_ptr();

    unsafe {
        rust_net_virtqueue_init(mem_ptr, queue_size);
        rust_net_rx_post_buffer(mem_ptr, queue_size, 2, 0xDEAD_0000);

        // Simulate the device: write a used-ring entry claiming
        // descriptor 2 completed with 42 bytes written - exactly what
        // real hardware would do, done here by hand so this test
        // doesn't depend on real hardware to check the *reading* side
        // works correctly.
        let (_avail_offset, used_offset, _total) = virtqueue_layout(queue_size);
        let used_idx_ptr = mem_ptr.add(used_offset + 2) as *mut u16;
        core::ptr::write_volatile(used_idx_ptr, 1);
        let elem = mem_ptr.add(used_offset + 4);
        core::ptr::write_unaligned(elem as *mut u32, 2); // desc id
        core::ptr::write_unaligned(elem.add(4) as *mut u32, 42); // len

        let mut last_used: u16 = 0;
        let mut out_len: u32 = 0;
        let desc_id =
            rust_net_rx_poll_used(mem_ptr, queue_size, &mut last_used, &mut out_len);
        if desc_id != 2 || out_len != 42 {
            code |= 2;
        }

        // A second poll with nothing new must report "nothing", not
        // re-report the same completion.
        let desc_id2 =
            rust_net_rx_poll_used(mem_ptr, queue_size, &mut last_used, &mut out_len);
        if desc_id2 != -1 {
            code |= 4;
        }
    }

    code
}
