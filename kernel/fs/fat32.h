#ifndef FS_FAT32_H
#define FS_FAT32_H

#include "../include/types.h"

/* Read-only FAT32 driver: root directory only (no subdirectories),
 * 8.3 filenames only (no long filename / VFAT entries), no write
 * support. Reads through kernel/drivers/ata/ata.c. See PROGRESS.md
 * for what's deferred. */

/* Parses the boot sector at LBA 0 and confirms it's actually FAT32.
 * Returns true if mounted successfully. */
bool fat32_init(void);

bool fat32_is_mounted(void);

typedef void (*fat32_list_callback_t)(const char* name_8_3, uint32_t size,
                                       bool is_directory);

/* Walks the root directory, calling `callback` once per real entry
 * (skips deleted entries, the volume label, and LFN continuation
 * entries). */
void fat32_list_root(fat32_list_callback_t callback);

/* Finds `filename` (case-insensitive, e.g. "HELLO.TXT") in the root
 * directory and reads up to `buf_size` bytes of its contents into
 * `buf`. Returns the number of bytes read, or -1 if the file wasn't
 * found. */
int fat32_read_file(const char* filename, void* buf, uint32_t buf_size);

#endif
