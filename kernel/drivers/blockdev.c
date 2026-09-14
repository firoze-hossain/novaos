/*
 * blockdev.c - see blockdev.h for the full design and rationale.
 */
#include "blockdev.h"
#include "ata/ata.h"
#include "virtio/virtio_blk.h"

static blockdev_id_t active_device = BLOCKDEV_ATA;
static uint32_t partition_offset = 0;

void blockdev_select(blockdev_id_t device) {
    active_device = device;
}

blockdev_id_t blockdev_current(void) {
    return active_device;
}

void blockdev_set_partition_offset(uint32_t offset_lba) {
    partition_offset = offset_lba;
}

uint32_t blockdev_get_partition_offset(void) {
    return partition_offset;
}

bool blockdev_is_present(void) {
    switch (active_device) {
        case BLOCKDEV_ATA:
            return ata_is_present();
        case BLOCKDEV_VIRTIO_BLK:
            return virtio_blk_is_present();
        default:
            return false;
    }
}

bool blockdev_read_sectors(uint32_t lba, uint8_t sector_count, void* buffer) {
    lba += partition_offset;
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
        default:
            return false;
    }
}

bool blockdev_write_sectors(uint32_t lba, uint8_t sector_count,
                             const void* buffer) {
    lba += partition_offset;
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
        default:
            return false;
    }
}
