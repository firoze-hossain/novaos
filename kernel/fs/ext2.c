/*
 * ext2.c - minimal read-only ext2 driver (see ext2.h for exact scope)
 */
#include "ext2.h"
#include "../drivers/ata/ata.h"
#include "../lib/string.h"
#include "../include/kernel.h"

#define EXT2_MAGIC 0xEF53
#define EXT2_ROOT_INODE 2
#define MAX_BLOCK_SIZE 4096 /* see ext2.h's scope note */
#define MAX_ROOT_DIR_BYTES (4 * MAX_BLOCK_SIZE) /* up to 4 blocks' worth
                                                    of root directory
                                                    entries - generous
                                                    for this project's
                                                    small test images,
                                                    a real limit for
                                                    anything bigger */

typedef struct __attribute__((packed)) {
    uint32_t inodes_count;
    uint32_t blocks_count;
    uint32_t r_blocks_count;
    uint32_t free_blocks_count;
    uint32_t free_inodes_count;
    uint32_t first_data_block;
    uint32_t log_block_size;
    uint32_t log_frag_size;
    uint32_t blocks_per_group;
    uint32_t frags_per_group;
    uint32_t inodes_per_group;
    uint32_t mtime;
    uint32_t wtime;
    uint16_t mnt_count;
    uint16_t max_mnt_count;
    uint16_t magic;
    uint16_t state;
    uint16_t errors;
    uint16_t minor_rev_level;
    uint32_t lastcheck;
    uint32_t checkinterval;
    uint32_t creator_os;
    uint32_t rev_level;
    uint16_t def_resuid;
    uint16_t def_resgid;
    /* EXT2_DYNAMIC_REV fields (rev_level >= 1) - the only real-world
     * case, but read defensively either way since rev_level is
     * checked before these are trusted. */
    uint32_t first_ino;
    uint16_t inode_size;
} ext2_superblock_t;

typedef struct __attribute__((packed)) {
    uint32_t block_bitmap;
    uint32_t inode_bitmap;
    uint32_t inode_table;
    uint16_t free_blocks_count;
    uint16_t free_inodes_count;
    uint16_t used_dirs_count;
    uint16_t pad;
    uint8_t reserved[12];
} ext2_bgd_t;

typedef struct __attribute__((packed)) {
    uint16_t mode;
    uint16_t uid;
    uint32_t size;
    uint32_t atime;
    uint32_t ctime;
    uint32_t mtime;
    uint32_t dtime;
    uint16_t gid;
    uint16_t links_count;
    uint32_t blocks;
    uint32_t flags;
    uint32_t osd1;
    uint32_t block[15]; /* [0..11] direct, [12] singly-indirect,
                            [13] doubly-indirect (unsupported),
                            [14] triply-indirect (unsupported) */
    uint32_t generation;
    uint32_t file_acl;
    uint32_t size_high;
    uint32_t faddr;
    uint8_t osd2[12];
} ext2_inode_t;

typedef struct __attribute__((packed)) {
    uint32_t inode;
    uint16_t rec_len;
    uint8_t name_len;
    uint8_t file_type;
} ext2_dirent_header_t;

static bool mounted = false;
static uint32_t partition_offset = 0;
static ext2_superblock_t sb;
static uint32_t block_size;
static uint32_t inode_size;
static uint32_t bgd_start_block;

static uint8_t block_buf[MAX_BLOCK_SIZE];
static uint32_t indirect_buf[MAX_BLOCK_SIZE / 4];

void ext2_set_partition_offset(uint32_t offset_lba) {
    partition_offset = offset_lba;
}

bool ext2_is_mounted(void) {
    return mounted;
}

static bool read_block(uint32_t block_num, void* buffer) {
    uint32_t sectors_per_block = block_size / 512;
    uint32_t lba = block_num * sectors_per_block;
    return ata_read_sectors(lba, (uint8_t)sectors_per_block, buffer);
}

static bool read_inode(uint32_t inode_num, ext2_inode_t* out) {
    if (inode_num == 0) {
        return false;
    }
    uint32_t group = (inode_num - 1) / sb.inodes_per_group;
    uint32_t index_in_group = (inode_num - 1) % sb.inodes_per_group;

    uint32_t bgd_byte_offset = group * sizeof(ext2_bgd_t);
    uint32_t bgd_block = bgd_start_block + bgd_byte_offset / block_size;
    uint32_t bgd_offset_in_block = bgd_byte_offset % block_size;

    if (!read_block(bgd_block, block_buf)) {
        return false;
    }
    ext2_bgd_t bgd;
    memcpy(&bgd, block_buf + bgd_offset_in_block, sizeof(bgd));

    uint32_t byte_offset_in_table = index_in_group * inode_size;
    uint32_t block_offset = byte_offset_in_table / block_size;
    uint32_t offset_in_block = byte_offset_in_table % block_size;

    if (!read_block(bgd.inode_table + block_offset, block_buf)) {
        return false;
    }
    memcpy(out, block_buf + offset_in_block, sizeof(ext2_inode_t));
    return true;
}

/* --- Write support (Phase 26) - everything below is new. Block/inode
 * allocation is scoped to block group 0 only (see ext2.h); this
 * project's own test images are always small enough to fit entirely
 * in one group (verified empirically against a real mkfs.ext2 image
 * before writing this), but a filesystem large enough to need a
 * second group would silently fail allocation here - a real,
 * documented limitation, not an oversight. */

static bool write_block(uint32_t block_num, const void* buffer) {
    uint32_t sectors_per_block = block_size / 512;
    uint32_t lba = block_num * sectors_per_block;
    return ata_write_sectors(lba, (uint8_t)sectors_per_block, buffer);
}

static bool read_bgd0(ext2_bgd_t* out) {
    if (!read_block(bgd_start_block, block_buf)) {
        return false;
    }
    memcpy(out, block_buf, sizeof(*out));
    return true;
}

static bool write_bgd0(const ext2_bgd_t* bgd) {
    /* The BGD table shares its block with (possibly) other groups'
     * descriptors, so this reads the current block, overwrites just
     * group 0's 32 bytes, and writes the whole block back rather than
     * blindly writing a block containing only group 0's data. */
    if (!read_block(bgd_start_block, block_buf)) {
        return false;
    }
    memcpy(block_buf, bgd, sizeof(*bgd));
    return write_block(bgd_start_block, block_buf);
}

static bool write_superblock(void) {
    uint8_t sb_sector_buf[1024];
    memset(sb_sector_buf, 0, sizeof(sb_sector_buf));
    memcpy(sb_sector_buf, &sb, sizeof(sb));
    return ata_write_sectors(2, 2, sb_sector_buf);
}

/* Finds a free bit in the bitmap block, marks it used, and writes the
 * bitmap block back. Returns the 0-based bit index, or -1 if the
 * bitmap is full (or a disk error occurred). */
static int alloc_from_bitmap(uint32_t bitmap_block, uint32_t total_bits) {
    static uint8_t bitmap_buf[MAX_BLOCK_SIZE];
    if (!read_block(bitmap_block, bitmap_buf)) {
        return -1;
    }

    for (uint32_t byte_idx = 0; byte_idx < (total_bits + 7) / 8;
         byte_idx++) {
        if (bitmap_buf[byte_idx] == 0xFF) {
            continue;
        }
        for (int bit = 0; bit < 8; bit++) {
            uint32_t global_bit = byte_idx * 8 + (uint32_t)bit;
            if (global_bit >= total_bits) {
                break;
            }
            if (!(bitmap_buf[byte_idx] & (1u << bit))) {
                bitmap_buf[byte_idx] |= (uint8_t)(1u << bit);
                if (!write_block(bitmap_block, bitmap_buf)) {
                    return -1;
                }
                return (int)global_bit;
            }
        }
    }
    return -1;
}

/* Allocates one free block from group 0. Returns the real block
 * number, or 0 on failure (0 is never a valid data block number -
 * block 0 is always the boot block). Updates the group's and
 * superblock's free-block counts and writes both back, so the
 * filesystem's own bookkeeping stays consistent for any other tool
 * (a real Linux mount, e2fsck, etc.) that might read this disk later. */
static uint32_t alloc_block(void) {
    ext2_bgd_t bgd;
    if (!read_bgd0(&bgd)) {
        return 0;
    }
    int bit = alloc_from_bitmap(bgd.block_bitmap, sb.blocks_per_group);
    if (bit < 0) {
        return 0;
    }

    uint32_t block_num = sb.first_data_block + (uint32_t)bit;

    bgd.free_blocks_count--;
    if (!write_bgd0(&bgd)) {
        return 0;
    }
    sb.free_blocks_count--;
    if (!write_superblock()) {
        return 0;
    }

    /* Newly allocated blocks should start zeroed - a filesystem
     * consistency expectation (an unwritten tail of the last block a
     * short file uses shouldn't leak whatever was previously on
     * disk), the same reasoning elf.c's segment loading already
     * applies. */
    memset(block_buf, 0, block_size);
    if (!write_block(block_num, block_buf)) {
        return 0;
    }

    return block_num;
}

/* Same idea as alloc_block(), for the inode bitmap. Returns a real,
 * global (1-indexed) inode number, or 0 on failure. */
static uint32_t alloc_inode(void) {
    ext2_bgd_t bgd;
    if (!read_bgd0(&bgd)) {
        return 0;
    }
    int bit = alloc_from_bitmap(bgd.inode_bitmap, sb.inodes_per_group);
    if (bit < 0) {
        return 0;
    }

    uint32_t inode_num = (uint32_t)bit + 1; /* group 0, so no group
                                                offset needed */

    bgd.free_inodes_count--;
    if (!write_bgd0(&bgd)) {
        return 0;
    }
    sb.free_inodes_count--;
    if (!write_superblock()) {
        return 0;
    }

    return inode_num;
}

static bool write_inode(uint32_t inode_num, const ext2_inode_t* inode) {
    uint32_t group = (inode_num - 1) / sb.inodes_per_group;
    uint32_t index_in_group = (inode_num - 1) % sb.inodes_per_group;

    ext2_bgd_t bgd;
    if (group != 0 || !read_bgd0(&bgd)) {
        return false; /* group 0 only - see the header comment above */
    }

    uint32_t byte_offset_in_table = index_in_group * inode_size;
    uint32_t block_offset = byte_offset_in_table / block_size;
    uint32_t offset_in_block = byte_offset_in_table % block_size;

    if (!read_block(bgd.inode_table + block_offset, block_buf)) {
        return false;
    }
    memcpy(block_buf + offset_in_block, inode, sizeof(*inode));
    return write_block(bgd.inode_table + block_offset, block_buf);
}

/* Inserts a new directory entry (inode_num, name) into the root
 * directory by finding trailing slack space in an existing entry and
 * splitting it - the standard ext2 technique, since a directory
 * block's entries always logically fill the entire block (the last
 * real entry's rec_len is stretched to cover whatever's left). Does
 * not support extending the root directory with an additional block
 * if no existing entry has enough slack - a real limitation for a
 * root directory that fills up, not attempted here. */
static bool insert_dirent(const char* name, uint32_t inode_num) {
    ext2_inode_t root;
    if (!read_inode(EXT2_ROOT_INODE, &root)) {
        return false;
    }
    if (root.block[0] == 0) {
        return false; /* an empty root directory shouldn't be possible
                          on any real filesystem, but fail safe */
    }

    /* This driver's root directories are always small enough to fit
     * in the first direct block (see ext2.h's scope note on
     * directories) - only block[0] is searched for slack. */
    if (!read_block(root.block[0], block_buf)) {
        return false;
    }

    size_t name_len = strlen(name);
    /* rec_len must be a multiple of 4 and at least big enough for the
     * 8-byte header plus the name. */
    uint16_t needed_len = (uint16_t)(((8 + name_len) + 3) & ~3u);

    uint32_t pos = 0;
    while (pos + sizeof(ext2_dirent_header_t) <= block_size) {
        ext2_dirent_header_t hdr;
        memcpy(&hdr, block_buf + pos, sizeof(hdr));
        if (hdr.rec_len == 0) {
            break;
        }

        uint16_t used_len =
            (uint16_t)(((8 + hdr.name_len) + 3) & ~3u);
        uint16_t slack = (uint16_t)(hdr.rec_len - used_len);

        if (slack >= needed_len) {
            /* Shrink this entry to only what it actually needs, and
             * place the new entry in the freed slack right after
             * it. */
            ext2_dirent_header_t new_hdr;
            new_hdr.inode = inode_num;
            new_hdr.rec_len = slack;
            new_hdr.name_len = (uint8_t)name_len;
            new_hdr.file_type = 1; /* EXT2_FT_REG_FILE */

            hdr.rec_len = used_len;
            memcpy(block_buf + pos, &hdr, sizeof(hdr));

            uint32_t new_pos = pos + used_len;
            memcpy(block_buf + new_pos, &new_hdr, sizeof(new_hdr));
            memcpy(block_buf + new_pos + sizeof(new_hdr), name, name_len);

            return write_block(root.block[0], block_buf);
        }

        pos += hdr.rec_len;
    }

    return false; /* no slack found anywhere in the block */
}

/* Reads up to buf_size bytes of an inode's file content into buf,
 * following direct then singly-indirect block pointers (see the
 * scope note in ext2.h). Returns the number of bytes actually read. */
static uint32_t read_inode_data(const ext2_inode_t* inode, void* buf,
                                 uint32_t buf_size) {
    uint32_t to_read =
        (inode->size < buf_size) ? inode->size : buf_size;
    uint32_t bytes_read = 0;
    uint8_t* out = (uint8_t*)buf;

    bool indirect_loaded = false;
    uint32_t block_index = 0;
    uint32_t pointers_per_block = block_size / 4;

    while (bytes_read < to_read) {
        uint32_t phys_block;

        if (block_index < 12) {
            phys_block = inode->block[block_index];
        } else if (block_index < 12 + pointers_per_block) {
            if (inode->block[12] == 0) {
                break; /* no indirect block allocated - nothing more
                          to read */
            }
            if (!indirect_loaded) {
                if (!read_block(inode->block[12], indirect_buf)) {
                    break;
                }
                indirect_loaded = true;
            }
            phys_block = indirect_buf[block_index - 12];
        } else {
            kernel_log("[WARN] ext2: file exceeds direct+singly-indirect "
                       "block support (doubly/triply-indirect not "
                       "implemented) - truncating read\n");
            break;
        }

        if (phys_block == 0) {
            break; /* a sparse hole - stop rather than zero-fill, a
                      real but rarely-hit simplification for this
                      driver's scope */
        }
        if (!read_block(phys_block, block_buf)) {
            break;
        }

        uint32_t chunk = block_size;
        if (bytes_read + chunk > to_read) {
            chunk = to_read - bytes_read;
        }
        memcpy(out + bytes_read, block_buf, chunk);
        bytes_read += chunk;
        block_index++;
    }

    return bytes_read;
}

static bool find_in_root(const char* filename, uint32_t* out_inode) {
    ext2_inode_t root;
    if (!read_inode(EXT2_ROOT_INODE, &root)) {
        return false;
    }

    static uint8_t dir_buf[MAX_ROOT_DIR_BYTES];
    uint32_t read_len = read_inode_data(&root, dir_buf, sizeof(dir_buf));
    size_t name_len = strlen(filename);

    uint32_t pos = 0;
    while (pos + sizeof(ext2_dirent_header_t) <= read_len) {
        ext2_dirent_header_t hdr;
        memcpy(&hdr, dir_buf + pos, sizeof(hdr));
        if (hdr.rec_len == 0) {
            break; /* malformed - stop rather than loop forever */
        }

        if (hdr.inode != 0 && hdr.name_len == name_len &&
            pos + sizeof(hdr) + hdr.name_len <= read_len) {
            if (memcmp(dir_buf + pos + sizeof(hdr), filename, name_len) ==
                0) {
                *out_inode = hdr.inode;
                return true;
            }
        }

        pos += hdr.rec_len;
    }

    return false;
}

bool ext2_init(void) {
    ata_set_partition_offset(partition_offset);
    mounted = false;

    /* The superblock always lives at byte offset 1024 regardless of
     * block size - LBA 2, 2 sectors (1024 bytes / 512). */
    uint8_t sb_buf[1024];
    if (!ata_read_sectors(2, 2, sb_buf)) {
        return false;
    }
    memcpy(&sb, sb_buf, sizeof(sb));

    if (sb.magic != EXT2_MAGIC) {
        return false;
    }

    block_size = 1024u << sb.log_block_size;
    if (block_size > MAX_BLOCK_SIZE) {
        kernel_log("[ .. ] ext2: block size %d exceeds this driver's "
                   "%d-byte limit - not mounting\n", (int)block_size,
                   MAX_BLOCK_SIZE);
        return false;
    }

    inode_size = (sb.rev_level >= 1) ? sb.inode_size : 128;
    /* The superblock occupies bytes 1024-2047. If block_size == 1024,
     * that's block 1 (block 0 is the boot block before it); if
     * block_size > 1024, the whole superblock fits inside block 0. The
     * block group descriptor table always starts in the block right
     * after wherever the superblock's block ends. */
    bgd_start_block = (block_size == 1024) ? 2 : 1;

    mounted = true;
    kernel_log("[ OK ] ext2 mounted: block_size=%d, inode_size=%d\n",
               (int)block_size, (int)inode_size);
    return true;
}

int ext2_read_file(const char* filename, void* buf, uint32_t buf_size) {
    ata_set_partition_offset(partition_offset);
    if (!mounted) {
        return -1;
    }

    uint32_t inode_num;
    if (!find_in_root(filename, &inode_num)) {
        return -1;
    }

    ext2_inode_t inode;
    if (!read_inode(inode_num, &inode)) {
        return -1;
    }

    return (int)read_inode_data(&inode, buf, buf_size);
}

bool ext2_write_file(const char* filename, const void* data, uint32_t size) {
    ata_set_partition_offset(partition_offset);
    if (!mounted) {
        return false;
    }

    uint32_t existing_inode;
    if (find_in_root(filename, &existing_inode)) {
        kernel_log("[FAULT] ext2_write_file: '%s' already exists "
                   "(create-only - see ext2.h)\n", filename);
        return false;
    }

    uint32_t blocks_needed = (size + block_size - 1) / block_size;
    if (size == 0) {
        blocks_needed = 0;
    }
    if (blocks_needed > 12) {
        kernel_log("[FAULT] ext2_write_file: '%s' needs %d blocks, "
                   "exceeding this driver's 12-direct-block limit (no "
                   "indirect-block allocation yet)\n", filename,
                   (int)blocks_needed);
        return false;
    }

    ext2_inode_t inode;
    memset(&inode, 0, sizeof(inode));
    inode.mode = 0x81A4; /* S_IFREG (regular file) | 0644 */
    inode.size = size;
    inode.links_count = 1;
    inode.blocks = blocks_needed * (block_size / 512);

    const uint8_t* src = (const uint8_t*)data;
    uint32_t remaining = size;
    for (uint32_t i = 0; i < blocks_needed; i++) {
        uint32_t block_num = alloc_block();
        if (block_num == 0) {
            kernel_log("[FAULT] ext2_write_file: out of free blocks\n");
            return false; /* the blocks already allocated this call are
                              now leaked - see PROGRESS.md */
        }
        inode.block[i] = block_num;

        uint32_t chunk = (remaining < block_size) ? remaining : block_size;
        memcpy(block_buf, src, chunk);
        if (chunk < block_size) {
            memset(block_buf + chunk, 0, block_size - chunk);
        }
        if (!write_block(block_num, block_buf)) {
            kernel_log("[FAULT] ext2_write_file: disk write failed\n");
            return false;
        }
        src += chunk;
        remaining -= chunk;
    }

    uint32_t inode_num = alloc_inode();
    if (inode_num == 0) {
        kernel_log("[FAULT] ext2_write_file: out of free inodes\n");
        return false;
    }
    if (!write_inode(inode_num, &inode)) {
        kernel_log("[FAULT] ext2_write_file: failed to write inode\n");
        return false;
    }

    if (!insert_dirent(filename, inode_num)) {
        kernel_log("[FAULT] ext2_write_file: no room in the root "
                   "directory for '%s'\n", filename);
        return false;
    }

    kernel_log("[ OK ] ext2_write_file: created '%s' (%d bytes, inode "
               "%d, %d block(s))\n", filename, (int)size, (int)inode_num,
               (int)blocks_needed);
    return true;
}
