#ifndef FS_PARTITION_H
#define FS_PARTITION_H

#include "../include/types.h"

#define MAX_PARTITIONS 4

typedef enum {
    PARTITION_SCHEME_NONE, /* no plausible partition table found - the
                               whole disk should be treated as one
                               filesystem starting at LBA 0, exactly
                               like every phase before this one */
    PARTITION_SCHEME_MBR,
    PARTITION_SCHEME_GPT,
} partition_scheme_t;

typedef struct {
    uint32_t start_lba;
    uint32_t sector_count;
    uint8_t mbr_type; /* the raw MBR partition type byte (0 for GPT
                          partitions - see the header comment below for
                          why GPT type GUIDs aren't decoded) */
} partition_entry_t;

typedef struct {
    partition_scheme_t scheme;
    int count;
    partition_entry_t partitions[MAX_PARTITIONS];
} partition_table_t;

/* Reads LBA 0 (and, if it looks like a GPT protective MBR, LBA 1 and
 * the GPT partition entry array too) and fills `out` with whatever
 * partitions were found. Always reads at absolute LBAs regardless of
 * any partition offset ata_set_partition_offset() may currently have
 * set - this function's whole purpose is figuring out what that
 * offset *should* be for each filesystem, so it has to read the raw,
 * unshifted disk layout itself.
 *
 * Auto-detection is deliberately conservative rather than just
 * trusting the 0x55AA boot signature at bytes 510-511: that signature
 * is *also* present at the end of an ordinary FAT32 boot sector (a
 * bare, unpartitioned FAT32 disk, exactly what every disk image
 * before this phase was), so it can't distinguish "this is a
 * partition table" from "this is directly a filesystem." Instead,
 * this also checks whether at least one of the four 16-byte entries
 * at offset 446 looks like a plausible partition (non-zero type,
 * non-zero sector count, start+count not obviously nonsensical) -
 * real boot code occupying that same byte range in a bare filesystem
 * is unlikely to coincidentally satisfy all of that. Not a
 * cryptographic guarantee, just the same practical heuristic real
 * partition-probing code uses. Returns PARTITION_SCHEME_NONE (with
 * count 0) if nothing plausible was found, so the caller can safely
 * fall back to whole-disk access.
 *
 * GPT support reads the header and partition entry array structurally
 * but does not verify either CRC32 checksum (no CRC32 implementation
 * exists in this kernel) and does not decode partition type GUIDs -
 * partitions are exposed by index only (partitions[0], [1], ...),
 * which is sufficient for a disk image this project builds and
 * controls the layout of, but not for correctly identifying a
 * specific partition's role (e.g. "find the ESP") on an arbitrary
 * real-world GPT disk. See PROGRESS.md. */
bool partition_read_table(partition_table_t* out);

#endif
