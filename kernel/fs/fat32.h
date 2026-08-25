#ifndef FS_FAT32_H
#define FS_FAT32_H

#include "../include/types.h"

/* FAT32 driver: root directory only (no subdirectories), 8.3
 * filenames only (no long filename / VFAT entries). Read support
 * landed in Phase 3; write support (create + delete a file) landed in
 * Phase 8 to back the package manager. Reads/writes through
 * kernel/drivers/ata/ata.c. See PROGRESS.md for what's still
 * deferred (no append/truncate/rename, no subdirectories). */

/* Parses the boot sector at LBA 0 and confirms it's actually FAT32.
 * Returns true if mounted successfully. */
bool fat32_init(void);

/* Phase 25: sets the LBA this driver treats as its own "sector 0" -
 * call once before fat32_init() if FAT32 lives in a disk partition
 * rather than starting at the very first sector of the disk. Every
 * one of this file's public functions re-applies this offset via
 * ata_set_partition_offset() at its own start, so interleaving calls
 * to this driver with a different filesystem (see ext2.h) that also
 * uses the disk stays correct as long as each individual call
 * completes without another filesystem's call landing in the middle -
 * see ata.h's fuller comment on that limitation. Defaults to 0 (whole
 * disk), unchanged from every phase before this one. */
void fat32_set_partition_offset(uint32_t offset_lba);

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

/* Creates a new file in the root directory with the given contents.
 * Fails (returns false) if a file with that name already exists -
 * there's no overwrite/truncate/append here, only create-new and
 * delete-then-recreate (see fat32_delete_file). Allocates a fresh
 * cluster chain, writes the data, extends the root directory with a
 * new cluster if it's completely full of existing entries. */
bool fat32_write_file(const char* filename, const void* data,
                       uint32_t size);

/* Removes a file: marks its directory entry deleted and frees its
 * entire cluster chain back to the free pool. Returns false if the
 * file wasn't found. */
bool fat32_delete_file(const char* filename);

#endif
