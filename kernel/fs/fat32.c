/*
 * fat32.c - read-only FAT32 driver (root directory, 8.3 names only)
 */
#include "fat32.h"
#include "../drivers/ata/ata.h"
#include "../lib/string.h"
#include "../include/kernel.h"

#define FAT_ATTR_VOLUME_ID 0x08
#define FAT_ATTR_DIRECTORY 0x10
#define FAT_ATTR_LFN       0x0F /* READ_ONLY|HIDDEN|SYSTEM|VOLUME_ID */

#define FAT32_END_OF_CHAIN 0x0FFFFFF8u

#define MAX_CLUSTER_SECTORS 64 /* supports cluster sizes up to 32KB */

typedef struct {
    uint8_t  jmp[3];
    uint8_t  oem[8];
    uint16_t bytes_per_sector;
    uint8_t  sectors_per_cluster;
    uint16_t reserved_sector_count;
    uint8_t  num_fats;
    uint16_t root_entry_count;
    uint16_t total_sectors_16;
    uint8_t  media;
    uint16_t fat_size_16;
    uint16_t sectors_per_track;
    uint16_t num_heads;
    uint32_t hidden_sectors;
    uint32_t total_sectors_32;
    /* FAT32 extended BPB */
    uint32_t fat_size_32;
    uint16_t ext_flags;
    uint16_t fs_version;
    uint32_t root_cluster;
    uint16_t fs_info;
    uint16_t backup_boot_sector;
    uint8_t  reserved[12];
    uint8_t  drive_number;
    uint8_t  reserved1;
    uint8_t  boot_signature;
    uint32_t volume_id;
    uint8_t  volume_label[11];
    uint8_t  fs_type[8];
} __attribute__((packed)) fat32_bpb_t;

typedef struct {
    uint8_t  name[11];
    uint8_t  attr;
    uint8_t  nt_reserved;
    uint8_t  create_time_tenth;
    uint16_t create_time;
    uint16_t create_date;
    uint16_t last_access_date;
    uint16_t first_cluster_hi;
    uint16_t write_time;
    uint16_t write_date;
    uint16_t first_cluster_lo;
    uint32_t file_size;
} __attribute__((packed)) fat_dirent_t;

static bool mounted = false;
static uint16_t bytes_per_sector;
static uint8_t sectors_per_cluster;
static uint32_t fat_start_lba;
static uint32_t data_start_lba;
static uint32_t root_cluster;
static uint8_t num_fats;
static uint32_t fat_size_32; /* sectors per FAT copy */

static uint8_t cluster_buf[MAX_CLUSTER_SECTORS * ATA_SECTOR_SIZE];
static uint8_t fat_sector_buf[ATA_SECTOR_SIZE];

static uint32_t cluster_to_lba(uint32_t cluster) {
    return data_start_lba + (cluster - 2) * sectors_per_cluster;
}

static uint32_t fat_next_cluster(uint32_t cluster) {
    uint32_t fat_offset = cluster * 4;
    uint32_t fat_sector = fat_start_lba + (fat_offset / bytes_per_sector);
    uint32_t ent_offset = fat_offset % bytes_per_sector;

    if (!ata_read_sectors(fat_sector, 1, fat_sector_buf)) {
        return FAT32_END_OF_CHAIN;
    }

    uint32_t value;
    memcpy(&value, &fat_sector_buf[ent_offset], sizeof(value));
    return value & 0x0FFFFFFFu;
}

/* Writes a FAT entry to every copy of the FAT (num_fats is usually 2,
 * kept in sync for redundancy the way real FAT32 does) - a
 * read-modify-write so the reserved top 4 bits of the 32-bit entry
 * are preserved rather than clobbered. */
static bool fat_set_next_cluster(uint32_t cluster, uint32_t value) {
    uint32_t fat_offset = cluster * 4;
    uint32_t sector_in_fat = fat_offset / bytes_per_sector;
    uint32_t ent_offset = fat_offset % bytes_per_sector;

    for (uint8_t copy = 0; copy < num_fats; copy++) {
        uint32_t fat_sector = fat_start_lba + copy * fat_size_32 + sector_in_fat;

        if (!ata_read_sectors(fat_sector, 1, fat_sector_buf)) {
            return false;
        }

        uint32_t existing;
        memcpy(&existing, &fat_sector_buf[ent_offset], sizeof(existing));
        uint32_t new_value = (existing & 0xF0000000u) | (value & 0x0FFFFFFFu);
        memcpy(&fat_sector_buf[ent_offset], &new_value, sizeof(new_value));

        if (!ata_write_sectors(fat_sector, 1, fat_sector_buf)) {
            return false;
        }
    }
    return true;
}

/* Scans the FAT for a free (== 0) cluster entry, starting from
 * cluster 2 (the lowest valid data cluster number in FAT32). No
 * free-space cache or FSInfo-sector shortcut - a fine linear-scan
 * tradeoff for a filesystem this small, but would want one on a much
 * larger volume. */
static uint32_t find_free_cluster(void) {
    uint32_t total_fat_bytes = fat_size_32 * bytes_per_sector;
    uint32_t total_entries = total_fat_bytes / 4;

    for (uint32_t cluster = 2; cluster < total_entries; cluster++) {
        if (fat_next_cluster(cluster) == 0) {
            return cluster;
        }
    }
    return 0; /* out of space */
}

/* Allocates `count` clusters, chains them together in the FAT (each
 * pointing to the next, the last marked end-of-chain), and returns
 * the first cluster number, or 0 on failure (partial allocations on
 * failure are not rolled back - see PROGRESS.md). */
static uint32_t alloc_cluster_chain(uint32_t count) {
    if (count == 0) {
        return 0;
    }

    uint32_t first = 0;
    uint32_t prev = 0;

    for (uint32_t i = 0; i < count; i++) {
        uint32_t cluster = find_free_cluster();
        if (cluster == 0) {
            return 0; /* out of space */
        }
        /* Mark it used immediately (even before chaining) so the next
         * find_free_cluster() call in this same loop doesn't pick the
         * same cluster again. */
        if (!fat_set_next_cluster(cluster, FAT32_END_OF_CHAIN)) {
            return 0;
        }

        if (first == 0) {
            first = cluster;
        } else {
            fat_set_next_cluster(prev, cluster);
        }
        prev = cluster;
    }

    return first;
}

static void free_cluster_chain(uint32_t cluster) {
    while (cluster >= 2 && cluster < FAT32_END_OF_CHAIN) {
        uint32_t next = fat_next_cluster(cluster);
        fat_set_next_cluster(cluster, 0);
        cluster = next;
    }
}

static void to_fat_8_3(const char* input, uint8_t out[11]) {
    for (int i = 0; i < 11; i++) {
        out[i] = ' ';
    }

    int i = 0, j = 0;
    while (input[i] && input[i] != '.' && j < 8) {
        char c = input[i++];
        if (c >= 'a' && c <= 'z') {
            c = (char)(c - 32);
        }
        out[j++] = (uint8_t)c;
    }
    while (input[i] && input[i] != '.') {
        i++;
    }
    if (input[i] == '.') {
        i++;
        int k = 8;
        while (input[i] && k < 11) {
            char c = input[i++];
            if (c >= 'a' && c <= 'z') {
                c = (char)(c - 32);
            }
            out[k++] = (uint8_t)c;
        }
    }
}

static void from_fat_8_3(const uint8_t raw[11], char* out) {
    int p = 0;
    for (int i = 0; i < 8; i++) {
        if (raw[i] != ' ') {
            out[p++] = (char)raw[i];
        }
    }
    if (raw[8] != ' ') {
        out[p++] = '.';
        for (int i = 8; i < 11; i++) {
            if (raw[i] != ' ') {
                out[p++] = (char)raw[i];
            }
        }
    }
    out[p] = '\0';
}

bool fat32_init(void) {
    mounted = false;

    if (!ata_is_present()) {
        kernel_log("[ .. ] FAT32: no ATA drive, skipping mount\n");
        return false;
    }

    uint8_t boot_sector[ATA_SECTOR_SIZE];
    if (!ata_read_sectors(0, 1, boot_sector)) {
        kernel_log("[FAULT] FAT32: failed to read boot sector\n");
        return false;
    }

    fat32_bpb_t bpb;
    memcpy(&bpb, boot_sector, sizeof(bpb));

    /* FAT32 is identified by fat_size_16 == 0 (the 16-bit field is
     * reserved and fat_size_32 is used instead) plus root_entry_count
     * == 0 (FAT32 has no fixed-size root directory region - the root
     * is just another cluster chain, pointed to by root_cluster). */
    if (bpb.bytes_per_sector != ATA_SECTOR_SIZE || bpb.fat_size_16 != 0 ||
        bpb.fat_size_32 == 0 || bpb.root_entry_count != 0) {
        kernel_log("[ .. ] FAT32: boot sector doesn't look like FAT32, "
                   "skipping mount\n");
        return false;
    }

    if (bpb.sectors_per_cluster == 0 ||
        (uint32_t)bpb.sectors_per_cluster > MAX_CLUSTER_SECTORS) {
        kernel_log("[FAULT] FAT32: unsupported cluster size (%d sectors)\n",
                   (int)bpb.sectors_per_cluster);
        return false;
    }

    bytes_per_sector = bpb.bytes_per_sector;
    sectors_per_cluster = bpb.sectors_per_cluster;
    fat_start_lba = bpb.reserved_sector_count;
    data_start_lba = fat_start_lba + bpb.num_fats * bpb.fat_size_32;
    root_cluster = bpb.root_cluster;
    num_fats = bpb.num_fats;
    fat_size_32 = bpb.fat_size_32;

    mounted = true;
    kernel_log("[ OK ] FAT32 mounted (cluster=%dB, root_cluster=%d)\n",
               (int)(sectors_per_cluster * bytes_per_sector),
               (int)root_cluster);
    return true;
}

bool fat32_is_mounted(void) {
    return mounted;
}

/* Shared root-directory walk. `want_name` NULL means "list everything
 * via callback"; non-NULL means "find this raw 11-byte name" and
 * return a pointer to a static copy of its directory entry, or NULL. */
static fat_dirent_t found_entry;

static bool walk_root(const uint8_t want_name[11],
                       fat32_list_callback_t callback) {
    uint32_t cluster = root_cluster;

    while (cluster < FAT32_END_OF_CHAIN) {
        uint32_t lba = cluster_to_lba(cluster);
        if (!ata_read_sectors(lba, sectors_per_cluster, cluster_buf)) {
            return false;
        }

        uint32_t entries_per_cluster =
            (uint32_t)sectors_per_cluster * bytes_per_sector /
            sizeof(fat_dirent_t);

        for (uint32_t i = 0; i < entries_per_cluster; i++) {
            fat_dirent_t entry;
            memcpy(&entry, &cluster_buf[i * sizeof(fat_dirent_t)],
                   sizeof(entry));

            if (entry.name[0] == 0x00) {
                return false; /* end of directory */
            }
            if (entry.name[0] == 0xE5) {
                continue; /* deleted */
            }
            if (entry.attr == FAT_ATTR_LFN) {
                continue; /* long-filename continuation entry */
            }
            if (entry.attr & FAT_ATTR_VOLUME_ID) {
                continue; /* volume label */
            }

            if (want_name != NULL) {
                if (memcmp(entry.name, want_name, 11) == 0) {
                    found_entry = entry;
                    return true;
                }
            } else if (callback != NULL) {
                char display_name[13];
                from_fat_8_3(entry.name, display_name);
                callback(display_name, entry.file_size,
                         (entry.attr & FAT_ATTR_DIRECTORY) != 0);
            }
        }

        cluster = fat_next_cluster(cluster);
    }

    return false;
}

void fat32_list_root(fat32_list_callback_t callback) {
    if (!mounted) {
        return;
    }
    walk_root(NULL, callback);
}

int fat32_read_file(const char* filename, void* buf, uint32_t buf_size) {
    if (!mounted) {
        return -1;
    }

    uint8_t want_name[11];
    to_fat_8_3(filename, want_name);

    if (!walk_root(want_name, NULL)) {
        return -1;
    }

    uint32_t cluster =
        ((uint32_t)found_entry.first_cluster_hi << 16) |
        found_entry.first_cluster_lo;
    uint32_t remaining = found_entry.file_size;
    if (remaining > buf_size) {
        remaining = buf_size;
    }

    uint32_t total_copied = 0;
    uint8_t* out = (uint8_t*)buf;
    uint32_t cluster_bytes = (uint32_t)sectors_per_cluster * bytes_per_sector;

    while (remaining > 0 && cluster < FAT32_END_OF_CHAIN) {
        uint32_t lba = cluster_to_lba(cluster);
        if (!ata_read_sectors(lba, sectors_per_cluster, cluster_buf)) {
            break;
        }

        uint32_t chunk = (remaining < cluster_bytes) ? remaining : cluster_bytes;
        memcpy(out + total_copied, cluster_buf, chunk);
        total_copied += chunk;
        remaining -= chunk;

        cluster = fat_next_cluster(cluster);
    }

    return (int)total_copied;
}

/* Scans the whole root directory chain for an existing entry matching
 * `want_name`. On a hit, returns its location (which cluster, and the
 * byte offset of the 32-byte entry within that cluster's data) so the
 * caller can read-modify-write it back - used by delete. */
static bool find_existing_entry_location(const uint8_t want_name[11],
                                          uint32_t* out_cluster,
                                          uint32_t* out_offset) {
    uint32_t cluster = root_cluster;

    while (cluster < FAT32_END_OF_CHAIN) {
        uint32_t lba = cluster_to_lba(cluster);
        if (!ata_read_sectors(lba, sectors_per_cluster, cluster_buf)) {
            return false;
        }

        uint32_t entries_per_cluster =
            (uint32_t)sectors_per_cluster * bytes_per_sector /
            sizeof(fat_dirent_t);

        for (uint32_t i = 0; i < entries_per_cluster; i++) {
            uint32_t offset = i * sizeof(fat_dirent_t);
            fat_dirent_t entry;
            memcpy(&entry, &cluster_buf[offset], sizeof(entry));

            if (entry.name[0] == 0x00) {
                return false; /* end of directory - nothing found */
            }
            if (entry.name[0] == 0xE5 || entry.attr == FAT_ATTR_LFN ||
                (entry.attr & FAT_ATTR_VOLUME_ID)) {
                continue;
            }
            if (memcmp(entry.name, want_name, 11) == 0) {
                *out_cluster = cluster;
                *out_offset = offset;
                return true;
            }
        }

        cluster = fat_next_cluster(cluster);
    }
    return false;
}

/* Scans for the first free (deleted, or "end of directory") slot,
 * extending the directory with a freshly allocated, zeroed cluster if
 * every existing cluster in the chain is completely full. Returns the
 * slot's location the same way find_existing_entry_location() does. */
static bool find_free_slot(uint32_t* out_cluster, uint32_t* out_offset) {
    uint32_t cluster = root_cluster;
    uint32_t last_cluster = root_cluster;

    while (cluster < FAT32_END_OF_CHAIN) {
        uint32_t lba = cluster_to_lba(cluster);
        if (!ata_read_sectors(lba, sectors_per_cluster, cluster_buf)) {
            return false;
        }

        uint32_t entries_per_cluster =
            (uint32_t)sectors_per_cluster * bytes_per_sector /
            sizeof(fat_dirent_t);

        for (uint32_t i = 0; i < entries_per_cluster; i++) {
            uint32_t offset = i * sizeof(fat_dirent_t);
            uint8_t first_byte = cluster_buf[offset];
            if (first_byte == 0x00 || first_byte == 0xE5) {
                *out_cluster = cluster;
                *out_offset = offset;
                return true;
            }
        }

        last_cluster = cluster;
        uint32_t next = fat_next_cluster(cluster);
        if (next >= FAT32_END_OF_CHAIN) {
            break; /* fully scanned, every cluster completely full */
        }
        cluster = next;
    }

    /* Every existing directory cluster is full - extend the chain
     * with one new, zeroed cluster and use its first slot. */
    uint32_t new_cluster = alloc_cluster_chain(1);
    if (new_cluster == 0) {
        return false; /* out of disk space */
    }
    memset(cluster_buf, 0, (size_t)sectors_per_cluster * bytes_per_sector);
    if (!ata_write_sectors(cluster_to_lba(new_cluster), sectors_per_cluster,
                            cluster_buf)) {
        return false;
    }
    if (!fat_set_next_cluster(last_cluster, new_cluster)) {
        return false;
    }

    *out_cluster = new_cluster;
    *out_offset = 0;
    return true;
}

bool fat32_write_file(const char* filename, const void* data, uint32_t size) {
    if (!mounted) {
        return false;
    }

    uint8_t raw_name[11];
    to_fat_8_3(filename, raw_name);

    uint32_t existing_cluster, existing_offset;
    if (find_existing_entry_location(raw_name, &existing_cluster,
                                      &existing_offset)) {
        return false; /* already exists - no overwrite; delete first */
    }

    uint32_t cluster_bytes = (uint32_t)sectors_per_cluster * bytes_per_sector;
    uint32_t clusters_needed = (size == 0)
                                    ? 1
                                    : (size + cluster_bytes - 1) / cluster_bytes;

    uint32_t first_cluster = alloc_cluster_chain(clusters_needed);
    if (first_cluster == 0) {
        kernel_log("[FAULT] fat32_write_file: out of disk space for '%s'\n",
                   filename);
        return false;
    }

    /* Write the data across the allocated chain, one cluster at a
     * time, zero-padding the tail of the final cluster. */
    const uint8_t* src = (const uint8_t*)data;
    uint32_t remaining = size;
    uint32_t cluster = first_cluster;

    while (cluster < FAT32_END_OF_CHAIN) {
        uint32_t chunk = (remaining < cluster_bytes) ? remaining : cluster_bytes;
        if (chunk > 0) {
            memcpy(cluster_buf, src, chunk);
        }
        if (chunk < cluster_bytes) {
            memset(cluster_buf + chunk, 0, cluster_bytes - chunk);
        }

        if (!ata_write_sectors(cluster_to_lba(cluster), sectors_per_cluster,
                                cluster_buf)) {
            free_cluster_chain(first_cluster);
            return false;
        }

        src += chunk;
        remaining -= chunk;
        cluster = fat_next_cluster(cluster);
    }

    /* Now claim a directory entry slot and fill it in. */
    uint32_t dir_cluster, dir_offset;
    if (!find_free_slot(&dir_cluster, &dir_offset)) {
        free_cluster_chain(first_cluster);
        return false;
    }

    if (!ata_read_sectors(cluster_to_lba(dir_cluster), sectors_per_cluster,
                           cluster_buf)) {
        free_cluster_chain(first_cluster);
        return false;
    }

    fat_dirent_t entry;
    memset(&entry, 0, sizeof(entry));
    memcpy(entry.name, raw_name, 11);
    entry.attr = 0; /* plain archive/data file */
    entry.first_cluster_hi = (uint16_t)(first_cluster >> 16);
    entry.first_cluster_lo = (uint16_t)(first_cluster & 0xFFFF);
    entry.file_size = size;

    memcpy(&cluster_buf[dir_offset], &entry, sizeof(entry));

    if (!ata_write_sectors(cluster_to_lba(dir_cluster), sectors_per_cluster,
                            cluster_buf)) {
        free_cluster_chain(first_cluster);
        return false;
    }

    return true;
}

bool fat32_delete_file(const char* filename) {
    if (!mounted) {
        return false;
    }

    uint8_t raw_name[11];
    to_fat_8_3(filename, raw_name);

    uint32_t dir_cluster, dir_offset;
    if (!find_existing_entry_location(raw_name, &dir_cluster, &dir_offset)) {
        return false;
    }

    if (!ata_read_sectors(cluster_to_lba(dir_cluster), sectors_per_cluster,
                           cluster_buf)) {
        return false;
    }

    fat_dirent_t entry;
    memcpy(&entry, &cluster_buf[dir_offset], sizeof(entry));
    uint32_t first_cluster =
        ((uint32_t)entry.first_cluster_hi << 16) | entry.first_cluster_lo;

    cluster_buf[dir_offset] = 0xE5; /* mark deleted */
    if (!ata_write_sectors(cluster_to_lba(dir_cluster), sectors_per_cluster,
                            cluster_buf)) {
        return false;
    }

    free_cluster_chain(first_cluster);
    return true;
}
