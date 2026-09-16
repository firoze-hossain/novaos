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

/* Phase 54: kernel/rust/crashdump.rs's own configure/check-and-report
 * entry points - see that module's own header comment for the full
 * design. `crash_report_t` is this file's own C-side mirror of that
 * module's `CrashReport`, laid out field-for-field in the exact same
 * order - a cross-FFI layout contract, not something either language
 * enforces against the other, the same honest caveat crashdump.rs's
 * own `FaultRegs` doc comment states; REASON_BYTES (64) and
 * MAX_STACK_FRAMES (8) below are that module's own private constants,
 * duplicated here as literals rather than shared via a header, matching
 * this project's established "no shared bridge header" FFI convention
 * (see e.g. kernel/fs/fat32.c's own rust_journal_* declarations). */
typedef struct {
    uint32_t ticks;
    uint8_t reason[64];
    uint32_t has_regs;
    uint32_t regs_ds, regs_edi, regs_esi, regs_ebp, regs_esp_dummy,
        regs_ebx, regs_edx, regs_ecx, regs_eax, regs_int_no,
        regs_err_code, regs_eip, regs_cs, regs_eflags, regs_useresp,
        regs_ss;
    uint32_t has_fault_addr;
    uint32_t fault_addr;
    uint32_t stack[8];
    uint32_t stack_count;
} crash_report_t;

extern void rust_crashdump_configure(uint32_t crash_start_lba,
                                      uint32_t crash_len_sectors);
extern int rust_crashdump_check_and_report(crash_report_t* out);

/* Formats and logs a crash_report_t found by rust_crashdump_check_and_
 * report() - split across several kernel_log() calls rather than one,
 * the same fix this project already established for a message this
 * size (see kernel/init/main.c's own comment on kernel_log()'s fixed
 * 256-byte internal buffer, next to the Phase 47/49 user-database
 * self-test's identical split). */
static void report_crash_dump(const crash_report_t* r) {
    kernel_log("[ .. ] Crash dump found from a previous boot (tick %d): "
               "%s\n", (int)r->ticks, r->reason);
    if (r->has_regs) {
        kernel_log("[ .. ]   eax=0x%x ebx=0x%x ecx=0x%x edx=0x%x "
                   "esi=0x%x edi=0x%x ebp=0x%x\n",
                   (unsigned int)r->regs_eax, (unsigned int)r->regs_ebx,
                   (unsigned int)r->regs_ecx, (unsigned int)r->regs_edx,
                   (unsigned int)r->regs_esi, (unsigned int)r->regs_edi,
                   (unsigned int)r->regs_ebp);
        kernel_log("[ .. ]   eip=0x%x cs=0x%x eflags=0x%x esp=0x%x "
                   "ss=0x%x ds=0x%x\n",
                   (unsigned int)r->regs_eip, (unsigned int)r->regs_cs,
                   (unsigned int)r->regs_eflags,
                   (unsigned int)r->regs_useresp,
                   (unsigned int)r->regs_ss, (unsigned int)r->regs_ds);
        kernel_log("[ .. ]   fault vector=%d error_code=0x%x\n",
                   (int)r->regs_int_no, (unsigned int)r->regs_err_code);
    } else {
        kernel_log("[ .. ]   (no CPU register snapshot - this was a "
                   "software-detected panic, not a hardware exception)\n");
    }
    if (r->has_fault_addr) {
        kernel_log("[ .. ]   faulting address (CR2) = 0x%x\n",
                   (unsigned int)r->fault_addr);
    }
    if (r->stack_count > 0) {
        char line[128];
        int pos = snprintf(line, sizeof(line), "[ .. ]   stack:");
        for (uint32_t i = 0; i < r->stack_count && pos < (int)sizeof(line) - 16;
             i++) {
            pos += snprintf(line + pos, sizeof(line) - (size_t)pos,
                             " 0x%x", (unsigned int)r->stack[i]);
        }
        kernel_log("%s\n", line);
    }
}

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

        /* Phase 54: a fourth partition, when present (this project's
         * own tools/build-disk-image.sh now creates one), is this
         * kernel's dedicated crash-dump region - see kernel/rust/
         * crashdump.rs's own header comment for the full design,
         * including why MAX_PARTITIONS (4) means this is the last such
         * region this scheme can ever add. Configured and checked
         * right here, next to Phase 53's own journal configure/
         * recover, for the same reason: as early in boot as this
         * disk's own partition table allows, so a crash from a
         * previous run is reported before anything else a developer
         * might be watching for scrolls past it. Absolute LBAs, so -
         * unlike the journal and FAT32 above - this needs no
         * blockdev_set_partition_offset() call of its own. A disk with
         * no fourth partition (an older image, or one built before
         * this phase) simply never configures crash-dump capture -
         * every panic on such a disk logs to serial/VGA only, exactly
         * as every panic did before this phase (see kernel/rust/
         * crashdump.rs's own rust_crashdump_write_panic() doc
         * comment). */
        if (table.count > 3) {
            rust_crashdump_configure(table.partitions[3].start_lba,
                                      table.partitions[3].sector_count);
            crash_report_t report;
            int found = rust_crashdump_check_and_report(&report);
            if (found > 0) {
                report_crash_dump(&report);
            } else if (found == 0) {
                kernel_log("[ OK ] Crash dump: none pending (clean "
                           "shutdown, or already reported)\n");
            } else {
                kernel_log("[FAULT] Crash dump: check failed (I/O error "
                           "reading the crash-dump region)\n");
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
         * behavior. No journal or crash-dump capture either: this path
         * has no partition table to find a third or fourth partition
         * in, so writes and panics on such a disk continue exactly as
         * before those phases (see
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
