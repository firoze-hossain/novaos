#ifndef DRIVERS_ATA_H
#define DRIVERS_ATA_H

#include "../../include/types.h"

#define ATA_SECTOR_SIZE 512

/* Polling PIO driver for the primary ATA bus's master device only
 * (no IRQ, no secondary bus, no ATAPI/CD-ROM, no DMA). That's a real
 * limitation for anything performance-sensitive, but it's the
 * simplest correct way to read a data disk and is enough to back a
 * read-only FAT32 filesystem (see kernel/fs/fat32.c). Tracked in
 * PROGRESS.md as a Phase 3 follow-up (IRQ-driven + secondary bus +
 * write support).
 *
 * Runs IDENTIFY on init and logs whether a usable ATA (not ATAPI)
 * drive was found; every other function silently no-ops (returns
 * false/0) if it wasn't. */
void ata_init(void);
bool ata_is_present(void);

/* Reads `sector_count` consecutive 512-byte sectors starting at LBA
 * `lba` into `buffer` (must be at least sector_count*512 bytes).
 * Returns true on success. */
bool ata_read_sectors(uint32_t lba, uint8_t sector_count, void* buffer);

#endif
