#ifndef DRIVERS_VIRTIO_BLK_H
#define DRIVERS_VIRTIO_BLK_H

#include "../../include/types.h"

/* virtio_blk.c - Phase 42: this kernel's first virtio driver.
 *
 * Scope, deliberately: proves the driver itself works, correctly,
 * against a real (QEMU-emulated) virtio-blk device - PCI detection,
 * the legacy status/feature handshake, virtqueue setup, and real
 * sector read/write requests, verified via a self-test that writes a
 * known pattern to a spare sector and reads it back byte-for-byte
 * (see PROGRESS.md's Phase 42 entry). NOT wired into the VFS as a
 * boot/mount device - this project's FAT32/ext2 mounting stays on the
 * existing ATA driver. Making virtio-blk an actual, selectable boot
 * device would mean touching VFS/mount code this phase deliberately
 * leaves alone, a separate, larger piece of follow-up work with its
 * own real design questions (which device wins if both ATA and
 * virtio-blk are present? does the FAT32/ext2 driver code need to
 * change to go through a generic "block device" interface instead of
 * calling the ATA driver directly?) - not attempted here.
 *
 * Also deliberately scoped: exactly one request in flight at a time
 * (submit, then poll to completion before the next call) - no queued/
 * concurrent request support. This matches how this project's other
 * synchronous drivers already behave (the ATA driver itself is
 * blocking/polling, not queued) and keeps the virtqueue's descriptor-
 * reuse logic simple and obviously correct (see kernel/rust/
 * virtio_blk.rs's own comment on why reusing fixed descriptor slots
 * 0/1/2 is safe specifically because of this scope limit) rather than
 * needing a free-descriptor-tracking scheme a genuinely concurrent
 * driver would require.
 */

void virtio_blk_init(void);
bool virtio_blk_is_present(void);

/* Reads/writes exactly one 512-byte sector. `buf` must point to at
 * least 512 bytes. Returns true on success, false on any failure
 * (device not present, request timed out waiting for completion, or
 * the device itself reported an error status). Blocks (polls) until
 * the request completes or a bounded retry count is exhausted - see
 * virtio_blk.c's own comment on why a bounded poll, not an unbounded
 * one, matching this kernel's established non-blocking-first
 * convention even in a polling driver. */
bool virtio_blk_read_sector(uint64_t sector, void* buf);
bool virtio_blk_write_sector(uint64_t sector, const void* buf);

#endif
