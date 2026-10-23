/*
 * blockdev.c - see blockdev.h for the full design and rationale.
 */
#include "blockdev.h"
#include "driver_locks.h"
#include "ata/ata.h"
#include "virtio/virtio_blk.h"
#include "nvme/nvme.h"

static blockdev_id_t active_device = BLOCKDEV_ATA;
static uint32_t partition_offset = 0;

void blockdev_select(blockdev_id_t device) {
    uint32_t flags = rust_blockdev_lock_acquire();
    active_device = device;
    rust_blockdev_lock_release(flags);
}

blockdev_id_t blockdev_current(void) {
    uint32_t flags = rust_blockdev_lock_acquire();
    blockdev_id_t dev = active_device;
    rust_blockdev_lock_release(flags);
    return dev;
}

void blockdev_set_partition_offset(uint32_t offset_lba) {
    uint32_t flags = rust_blockdev_lock_acquire();
    partition_offset = offset_lba;
    rust_blockdev_lock_release(flags);
}

uint32_t blockdev_get_partition_offset(void) {
    uint32_t flags = rust_blockdev_lock_acquire();
    uint32_t off = partition_offset;
    rust_blockdev_lock_release(flags);
    return off;
}

bool blockdev_is_present(void) {
    switch (active_device) {
        case BLOCKDEV_ATA:
            return ata_is_present();
        case BLOCKDEV_VIRTIO_BLK:
            return virtio_blk_is_present();
        case BLOCKDEV_NVME:
            return nvme_is_present();
        default:
            return false;
    }
}

/* Phase 53: factored out of blockdev_read_sectors() so
 * blockdev_read_sectors_absolute() (kernel/rust/journal.rs's own
 * on-disk storage I/O - see blockdev.h's comment on why it needs to
 * skip partition_offset entirely) shares the exact same per-device
 * dispatch logic rather than duplicating it. Takes an already-final
 * LBA - callers decide whether that means "+ partition_offset" or
 * not. */
static bool dispatch_read(uint32_t lba, uint8_t sector_count, void* buffer) {
    switch (active_device) {
        case BLOCKDEV_ATA:
            return ata_read_sectors(lba, sector_count, buffer);
        case BLOCKDEV_VIRTIO_BLK: {
            uint8_t* out = (uint8_t*)buffer;
            for (uint8_t i = 0; i < sector_count; i++) {
                if (!virtio_blk_read_sector((uint64_t)lba + i,
                                             out + (uint32_t)i * 512)) {
                    return false;
                }
            }
            return true;
        }
        case BLOCKDEV_NVME:
            return nvme_read_sectors((uint64_t)lba, sector_count, buffer);
        default:
            return false;
    }
}

static bool dispatch_write(uint32_t lba, uint8_t sector_count,
                            const void* buffer) {
    switch (active_device) {
        case BLOCKDEV_ATA:
            return ata_write_sectors(lba, sector_count, buffer);
        case BLOCKDEV_VIRTIO_BLK: {
            const uint8_t* in = (const uint8_t*)buffer;
            for (uint8_t i = 0; i < sector_count; i++) {
                if (!virtio_blk_write_sector((uint64_t)lba + i,
                                              in + (uint32_t)i * 512)) {
                    return false;
                }
            }
            return true;
        }
        case BLOCKDEV_NVME:
            return nvme_write_sectors((uint64_t)lba, sector_count, buffer);
        default:
            return false;
    }
}

bool blockdev_read_sectors(uint32_t lba, uint8_t sector_count, void* buffer) {
    uint32_t flags = rust_blockdev_lock_acquire();
    uint32_t target_lba = lba + partition_offset;
    rust_blockdev_lock_release(flags);
    return dispatch_read(target_lba, sector_count, buffer);
}

bool blockdev_write_sectors(uint32_t lba, uint8_t sector_count,
                             const void* buffer) {
    uint32_t flags = rust_blockdev_lock_acquire();
    uint32_t target_lba = lba + partition_offset;
    rust_blockdev_lock_release(flags);
    return dispatch_write(target_lba, sector_count, buffer);
}

bool blockdev_read_sectors_absolute(uint32_t lba, uint8_t sector_count,
                                     void* buffer) {
    return dispatch_read(lba, sector_count, buffer);
}

bool blockdev_write_sectors_absolute(uint32_t lba, uint8_t sector_count,
                                      const void* buffer) {
    return dispatch_write(lba, sector_count, buffer);
}
