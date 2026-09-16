/*
 * vfs.c - thin pass-through to the mounted filesystem(s)
 *
 * Phase 25 added a second filesystem (ext2) alongside FAT32, found in
 * a separate disk partition rather than assuming the whole disk is
 * one filesystem starting at LBA 0 - see kernel/fs/partition.c for
 * the MBR/GPT parsing this depends on. FAT32 remains "primary" (it's
 * where SYSTEM.CFG, packages, and every other existing fixture live,
 * unchanged since Phase 3/8); ext2 is consulted as a fallback for
 * reads only, deliberately not wired into vfs_write_file()/
 * vfs_delete_file() at all, since this ext2 driver is read-only (see
 * ext2.h).
 */
#include "vfs.h"
#include "fat32.h"
#include "ext2.h"
#include "partition.h"
#include "../drivers/ata/ata.h"
#include "../drivers/blockdev.h"
#include "../drivers/vga/vga.h"
#include "../lib/stdio.h"
#include "../include/kernel.h"

/* Phase 53: kernel/rust/journal.rs's own configure/recover entry
 * points - see that file's header comment for the full design, and
 * this function's own comment below for why recovery must run before
 * fat32_init() mounts anything. */
extern void rust_journal_configure(uint32_t journal_start_lba,
                                    uint32_t journal_len_sectors);
extern int rust_journal_recover(void);

void vfs_init(void) {
    ata_init();
    if (!ata_is_present()) {
        return;
    }

    partition_table_t table;
    if (partition_read_table(&table)) {
        const char* scheme_name =
            (table.scheme == PARTITION_SCHEME_GPT) ? "GPT" : "MBR";
        kernel_log("[ OK ] Partition table found (%s): %d partition(s)\n",
                   scheme_name, table.count);

        /* Phase 53: a third partition, when present (this project's
         * own tools/build-disk-image.sh now creates one - see that
         * script's own comment), is FAT32's dedicated write-ahead
         * journal - see kernel/rust/journal.rs's own header comment
         * for the full design. Configured and recovered *before*
         * fat32_init() mounts anything: recovery's own checkpoint
         * writes must land at the real FAT32 partition offset (set
         * explicitly here, via blockdev_set_partition_offset() -
         * fat32_init() itself hasn't run yet to do its own usual
         * reassertion of that same offset), so any transaction a
         * prior boot's crash left durably committed but not yet
         * checkpointed is fully applied before a single filesystem
         * read happens against this disk. A disk with no third
         * partition (an older image, or one built before this phase)
         * simply never configures a journal - every subsequent
         * fat32.c write then falls back to direct, unjournaled
         * writes, exactly as every disk behaved before this phase
         * (see journal.rs's own rust_journal_configure() doc
         * comment). */
        if (table.count > 2) {
            blockdev_set_partition_offset(table.partitions[0].start_lba);
            rust_journal_configure(table.partitions[2].start_lba,
                                    table.partitions[2].sector_count);
            int recovered = rust_journal_recover();
            if (recovered > 0) {
                kernel_log("[ OK ] Journal: recovered %d pending block(s) "
                           "from an interrupted write (a crash or power "
                           "loss before this boot)\n", recovered);
            } else if (recovered == 0) {
                kernel_log("[ OK ] Journal: clean, no recovery needed\n");
            } else {
                kernel_log("[FAULT] Journal: recovery failed (I/O error "
                           "reading the journal region)\n");
            }
        }

        fat32_set_partition_offset(table.partitions[0].start_lba);
        fat32_init();

        if (table.count > 1) {
            ext2_set_partition_offset(table.partitions[1].start_lba);
            ext2_init();
        }
    } else {
        /* No plausible partition table - fall back to whole-disk FAT32
         * access, exactly like every phase before this one. Both
         * offsets default to 0 already, so no explicit call is even
         * needed here - stated for clarity, not because it changes
         * behavior. No journal either: this path has no partition
         * table to find a third partition in, so writes on such a
         * disk continue exactly as before this phase (see
         * kernel/init/main.c's own virtio-blk raw-mount self-test,
         * which mounts FAT32 this same unpartitioned way and keeps
         * working unchanged). */
        fat32_init();
    }
}

bool vfs_is_mounted(void) {
    return fat32_is_mounted();
}

static void print_entry(const char* name, uint32_t size, bool is_dir) {
    char line[64];
    if (is_dir) {
        snprintf(line, sizeof(line), "  <DIR>  %s\n", name);
    } else {
        snprintf(line, sizeof(line), "  %6d  %s\n", (int)size, name);
    }
    vga_puts(line);
}

void vfs_ls(void) {
    if (!vfs_is_mounted()) {
        vga_puts("No filesystem mounted (no ATA disk found at boot)\n");
        return;
    }
    fat32_list_root(print_entry);
}

void vfs_list_files(vfs_list_callback_t callback) {
    if (!vfs_is_mounted()) {
        return;
    }
    fat32_list_root(callback);
}

int vfs_read_file(const char* filename, void* buf, uint32_t buf_size) {
    if (vfs_is_mounted()) {
        int result = fat32_read_file(filename, buf, buf_size);
        if (result >= 0) {
            return result;
        }
    }
    /* Not found in (or no) FAT32 - try ext2 as a fallback, if it's
     * mounted. FAT32 always wins if a name exists in both, a simple,
     * documented tiebreak rather than surfacing an ambiguity error. */
    if (ext2_is_mounted()) {
        return ext2_read_file(filename, buf, buf_size);
    }
    return -1;
}

bool vfs_write_file(const char* filename, const void* data, uint32_t size) {
    if (!vfs_is_mounted()) {
        return false;
    }
    return fat32_write_file(filename, data, size);
}

bool vfs_delete_file(const char* filename) {
    if (!vfs_is_mounted()) {
        return false;
    }
    return fat32_delete_file(filename);
}
