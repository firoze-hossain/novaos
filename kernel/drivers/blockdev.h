#ifndef DRIVERS_BLOCKDEV_H
#define DRIVERS_BLOCKDEV_H

#include "../include/types.h"

/*
 * blockdev.h - Phase 46: a block-device dispatch abstraction, the
 * same architectural shape kernel/net/net.c's own `active_nic`
 * dispatch already uses for NICs, applied here to storage.
 *
 * Gap this fills: kernel/fs/fat32.c, ext2.c, and partition.c called
 * ata_read_sectors()/ata_write_sectors() directly, at roughly 21 call
 * sites across those three files, with no abstraction at all - the
 * single largest concrete reason virtio-blk (Phase 42) could prove
 * itself against real hardware but couldn't be "wired into the VFS."
 * This phase closes that gap.
 *
 * Deliberately NOT auto-preferring virtio-blk the way net.c prefers
 * virtio-net over RTL8139 when both are present - a real, considered
 * difference, not an inconsistency: this project's own test config
 * attaches ATA (with real FAT32/ext2 filesystems) and virtio-blk (a
 * separate, differently-purposed disk) *simultaneously*, always - for
 * NICs, only one is typically meant to be "the network," so preferring
 * whichever is more capable is the right default; for storage, which
 * device holds *this specific filesystem* is a real, distinct
 * question a boot-time "just prefer the better one" heuristic would
 * get wrong. The active device therefore defaults to `BLOCKDEV_ATA`
 * (preserving every existing call site's exact prior behavior with
 * zero code changes at the call sites beyond the function name) and
 * is only ever switched explicitly - see kernel/init/main.c's own
 * Phase 46 self-test for the one place this project currently does
 * that, and its own comment on why it carefully restores the original
 * device before continuing boot.
 */

typedef enum {
    BLOCKDEV_ATA,
    BLOCKDEV_VIRTIO_BLK,
} blockdev_id_t;

/* 512 bytes, the standard sector size both backends this abstraction
 * currently supports actually use (ata.h's own ATA_SECTOR_SIZE and
 * virtio_blk.c's hardcoded 512 agree) - a device-agnostic name for
 * callers (fat32.c uses this directly for buffer sizing, not just
 * through the read/write functions) that shouldn't need to reach into
 * a specific backend's own header for a constant this abstraction is
 * exactly the place to own. */
#define BLOCKDEV_SECTOR_SIZE 512

void blockdev_select(blockdev_id_t device);
blockdev_id_t blockdev_current(void);

/* Found while replacing the 21 direct ata_* call sites this phase
 * targets, not assumed in advance: ata.c has its own internal
 * partition-offset state (added to every LBA inside
 * ata_read_sectors()/write_sectors() itself), which fat32.c and
 * ext2.c both push their own copy into via ata_set_partition_offset()
 * - repeatedly, before nearly every operation, not just once at
 * mount time, because the two filesystems share that same ATA-level
 * state and would otherwise silently use whichever filesystem's
 * offset was set most recently. Moved here instead, uniformly, for
 * both backends - ata.c's own offset is no longer touched at all
 * (permanently zero from this phase on), and this is the one place
 * that actually adds an offset to an LBA before dispatching, for
 * either device. Every former ata_set_partition_offset() call site
 * becomes a pure rename to blockdev_set_partition_offset() - same
 * call sites, same frequency, preserving the exact "re-assert before
 * each operation" pattern the original code already relied on for
 * fat32/ext2 to coexist correctly. */
void blockdev_set_partition_offset(uint32_t offset_lba);

/* Getter for the value blockdev_set_partition_offset() last stored -
 * needed by anything that temporarily switches the active device
 * (kernel/init/main.c's own Phase 46 self-test is the one place this
 * project currently does that) and must restore the exact prior
 * state afterward, not just "the default" - the current offset is
 * whatever fat32.c's own mount last set it to, which may not be 0. */
uint32_t blockdev_get_partition_offset(void);

/* Dispatches to ata_is_present()/virtio_blk_is_present() - fat32.c's
 * own mount function checks device presence directly (not through a
 * read/write call), so this needed its own entry in the abstraction
 * too, not just the two data-transfer functions. */
bool blockdev_is_present(void);

/* Same signatures as ata_read_sectors()/ata_write_sectors() - every
 * existing call site becomes a pure rename, `ata_` to `blockdev_`,
 * nothing else, when BLOCKDEV_ATA remains selected (the default,
 * unless something explicitly calls blockdev_select() first).
 *
 * When BLOCKDEV_VIRTIO_BLK is selected: internally loops over
 * virtio_blk_read_sector()/write_sector() (which only ever handle one
 * 512-byte sector per call - see virtio_blk.h's own comment on that
 * scope limit), sector_count times - virtio-blk's own request/
 * completion cycle is per-sector; this loop is what lets every
 * existing multi-sector caller (fat32.c's cluster reads in
 * particular, sector_count = sectors_per_cluster) keep working
 * completely unchanged, unaware of the difference. */
bool blockdev_read_sectors(uint32_t lba, uint8_t sector_count, void* buffer);
bool blockdev_write_sectors(uint32_t lba, uint8_t sector_count,
                             const void* buffer);

#endif
