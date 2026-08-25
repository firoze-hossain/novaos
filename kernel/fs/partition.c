/*
 * partition.c - MBR/GPT partition table parsing (see partition.h)
 */
#include "partition.h"
#include "../drivers/ata/ata.h"
#include "../lib/string.h"
#include "../include/kernel.h"

#define MBR_PARTITION_TABLE_OFFSET 446
#define MBR_ENTRY_SIZE 16
#define MBR_SIGNATURE_OFFSET 510
#define GPT_PROTECTIVE_TYPE 0xEE

typedef struct __attribute__((packed)) {
    char signature[8]; /* "EFI PART" */
    uint32_t revision;
    uint32_t header_size;
    uint32_t header_crc32;
    uint32_t reserved;
    uint64_t my_lba;
    uint64_t alternate_lba;
    uint64_t first_usable_lba;
    uint64_t last_usable_lba;
    uint8_t disk_guid[16];
    uint64_t partition_entry_lba;
    uint32_t num_partition_entries;
    uint32_t size_of_partition_entry;
    uint32_t partition_entry_array_crc32;
} gpt_header_t;

static bool mbr_entry_plausible(const uint8_t* entry, uint8_t* out_type,
                                 uint32_t* out_start, uint32_t* out_count) {
    uint8_t type = entry[4];
    uint32_t start_lba, sector_count;
    memcpy(&start_lba, entry + 8, 4);
    memcpy(&sector_count, entry + 12, 4);

    if (type == 0 || sector_count == 0 || start_lba == 0) {
        return false;
    }
    if (start_lba > 0xFFFFFFFFu - sector_count) {
        return false; /* would overflow - not a real partition */
    }

    *out_type = type;
    *out_start = start_lba;
    *out_count = sector_count;
    return true;
}

static bool try_parse_gpt(partition_table_t* out) {
    uint8_t header_buf[512];
    if (!ata_read_sectors(1, 1, header_buf)) {
        return false;
    }

    gpt_header_t hdr;
    memcpy(&hdr, header_buf, sizeof(hdr));
    if (memcmp(hdr.signature, "EFI PART", 8) != 0) {
        return false;
    }

    /* Only the common case (128-byte entries, evenly dividing a
     * 512-byte sector) is handled - see partition.h's header comment.
     * Anything else bails out gracefully rather than risking an
     * incorrect read. */
    if (hdr.size_of_partition_entry == 0 ||
        512 % hdr.size_of_partition_entry != 0) {
        kernel_log("[ .. ] GPT: unsupported partition entry size (%d "
                   "bytes) - not walking the entry array\n",
                   (int)hdr.size_of_partition_entry);
        return false;
    }

    uint32_t entries_per_sector = 512 / hdr.size_of_partition_entry;
    uint8_t entry_sector[512];
    uint32_t last_loaded_sector = 0xFFFFFFFFu;

    for (uint32_t i = 0;
         i < hdr.num_partition_entries && out->count < MAX_PARTITIONS; i++) {
        uint32_t sector_idx = i / entries_per_sector;
        uint32_t offset_in_sector = (i % entries_per_sector) *
                                     hdr.size_of_partition_entry;

        if (sector_idx != last_loaded_sector) {
            if (!ata_read_sectors(
                    (uint32_t)hdr.partition_entry_lba + sector_idx, 1,
                    entry_sector)) {
                break;
            }
            last_loaded_sector = sector_idx;
        }

        const uint8_t* entry = entry_sector + offset_in_sector;
        const uint8_t* type_guid = entry; /* first 16 bytes */
        bool type_is_zero = true;
        for (int b = 0; b < 16; b++) {
            if (type_guid[b] != 0) {
                type_is_zero = false;
                break;
            }
        }
        if (type_is_zero) {
            continue; /* an unused entry slot */
        }

        uint64_t start_lba64, end_lba64;
        memcpy(&start_lba64, entry + 32, 8);
        memcpy(&end_lba64, entry + 40, 8);

        out->partitions[out->count].start_lba = (uint32_t)start_lba64;
        out->partitions[out->count].sector_count =
            (uint32_t)(end_lba64 - start_lba64 + 1);
        out->partitions[out->count].mbr_type = 0; /* GPT - see partition.h */
        out->count++;
    }

    out->scheme = PARTITION_SCHEME_GPT;
    return out->count > 0;
}

bool partition_read_table(partition_table_t* out) {
    out->scheme = PARTITION_SCHEME_NONE;
    out->count = 0;

    uint8_t sector0[512];
    if (!ata_read_sectors(0, 1, sector0)) {
        return false;
    }
    if (sector0[MBR_SIGNATURE_OFFSET] != 0x55 ||
        sector0[MBR_SIGNATURE_OFFSET + 1] != 0xAA) {
        return false; /* not even a valid boot sector, let alone a
                          partition table */
    }

    /* A GPT disk starts with a "protective MBR" whose single partition
     * entry has type 0xEE covering the whole disk - if that's what we
     * see, the real partition data is in the GPT structures, not this
     * MBR's own (deliberately minimal) entries. */
    if (sector0[MBR_PARTITION_TABLE_OFFSET + 4] == GPT_PROTECTIVE_TYPE) {
        if (try_parse_gpt(out)) {
            return true;
        }
        /* Protective MBR present but GPT parsing failed or found
         * nothing - fall through and see if the plain MBR entries
         * happen to be usable anyway, rather than giving up entirely. */
    }

    for (int i = 0; i < 4; i++) {
        const uint8_t* entry =
            sector0 + MBR_PARTITION_TABLE_OFFSET + i * MBR_ENTRY_SIZE;
        uint8_t type;
        uint32_t start_lba, sector_count;
        if (!mbr_entry_plausible(entry, &type, &start_lba, &sector_count)) {
            continue;
        }
        if (out->count < MAX_PARTITIONS) {
            out->partitions[out->count].start_lba = start_lba;
            out->partitions[out->count].sector_count = sector_count;
            out->partitions[out->count].mbr_type = type;
            out->count++;
        }
    }

    if (out->count > 0) {
        out->scheme = PARTITION_SCHEME_MBR;
        return true;
    }
    return false;
}
