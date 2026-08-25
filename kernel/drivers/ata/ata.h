#ifndef DRIVERS_ATA_H
#define DRIVERS_ATA_H

#include "../../include/types.h"

#define ATA_SECTOR_SIZE 512

/* Polling PIO driver for the primary ATA bus's master device only
 * (no IRQ, no secondary bus, no ATAPI/CD-ROM, no DMA). That's a real
 * limitation for anything performance-sensitive, but it's simple and
 * correct enough to back a FAT32 filesystem with both read (Phase 3)
 * and write (Phase 8) support - see kernel/fs/fat32.c. Tracked in
 * PROGRESS.md as a follow-up (IRQ-driven + secondary bus).
 *
 * Runs IDENTIFY on init and logs whether a usable ATA (not ATAPI)
 * drive was found; every other function silently no-ops (returns
 * false/0) if it wasn't. */
void ata_init(void);
bool ata_is_present(void);

/* Phase 25: every subsequent ata_read_sectors()/ata_write_sectors()
 * call adds `offset_lba` to whatever LBA it's given, until this is
 * called again - see the fuller comment on the matching static
 * variable in ata.c for the exact scope/limitations of this
 * mechanism. Defaults to 0 (no offset, i.e. whole-disk access,
 * unchanged from every phase before this one). */
void ata_set_partition_offset(uint32_t offset_lba);

/* Reads `sector_count` consecutive 512-byte sectors starting at LBA
 * `lba` into `buffer` (must be at least sector_count*512 bytes).
 * Returns true on success. */
bool ata_read_sectors(uint32_t lba, uint8_t sector_count, void* buffer);

/* Writes `sector_count` consecutive 512-byte sectors starting at LBA
 * `lba` from `buffer`, then flushes the drive's write cache before
 * returning - added in Phase 8 to back FAT32 write support (see
 * kernel/fs/fat32.c). Returns true on success. */
bool ata_write_sectors(uint32_t lba, uint8_t sector_count,
                        const void* buffer);

#endif
