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
