#ifndef FS_EXT2_H
#define FS_EXT2_H

#include "../include/types.h"

/* Minimal read-only ext2 driver: root directory only (no
 * subdirectories - matching the same scope FAT32 started with in
 * Phase 3), direct + singly-indirect block pointers only (no doubly/
 * triply-indirect - covers files up to roughly block_size/4 + 12
 * blocks, comfortably enough for a block_size of 4096 to reach
 * several megabytes), block sizes up to 4096 bytes only (this
 * driver's internal buffers are sized for that - a filesystem built
 * with an 8192-byte block size, valid on some architectures, isn't
 * supported). Filenames are matched case-sensitively, correctly
 * reflecting how ext2 itself actually works - unlike this project's
 * FAT32 driver, which is case-insensitive by 8.3 convention. See
 * PROGRESS.md for the full scope note and how this was verified
 * against filesystem images built by the real mke2fs/debugfs tools,
 * not just round-tripped against this driver's own writes (which
 * don't exist - this is read-only). */

void ext2_set_partition_offset(uint32_t offset_lba);

/* Reads the superblock at byte offset 1024 and confirms the magic
 * number (0xEF53). Returns true if mounted successfully. */
bool ext2_init(void);
bool ext2_is_mounted(void);

/* Finds `filename` (case-sensitive, e.g. "EXT2TEST.TXT") in the root
 * directory and reads up to `buf_size` bytes of its contents into
 * `buf`. Returns the number of bytes read, or -1 if the file wasn't
 * found or the filesystem isn't mounted. */
int ext2_read_file(const char* filename, void* buf, uint32_t buf_size);

#endif
