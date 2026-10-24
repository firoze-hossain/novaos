/*
 * kernel/include/crashdump.h - Month 6: Crash Dump Auto-Export Hooks & Control ABI
 *
 * Connects the kernel's persistent crash-dump recorder to userland by:
 * 1. Extending the boot-time pending-crash check to automatically export
 *    detected crash records to well-known files on FAT32 (CRASHDMP.TXT and CRASHDMP.DAT).
 * 2. Providing SYS_CRASHDUMP_CTL (80) for querying crash status, retrieving
 *    structured records, and triggering or clearing exports.
 */

#ifndef NOVA_CRASHDUMP_H
#define NOVA_CRASHDUMP_H

#include <stdint.h>
#include <stdbool.h>

#define SYS_CRASHDUMP_CTL 80

#define CRASHDUMP_OP_STATUS 1
#define CRASHDUMP_OP_GET    2
#define CRASHDUMP_OP_EXPORT 3
#define CRASHDUMP_OP_CLEAR  4

#define CRASHDUMP_EXPORT_TXT "CRASHDMP.TXT"
#define CRASHDUMP_EXPORT_DAT "CRASHDMP.DAT"

/* Field-for-field C mirror of kernel/rust/crashdump.rs CrashReport */
typedef struct {
    uint32_t ticks;
    uint8_t  reason[64];
    uint32_t has_regs;
    uint32_t regs_ds, regs_edi, regs_esi, regs_ebp, regs_esp_dummy,
             regs_ebx, regs_edx, regs_ecx, regs_eax, regs_int_no,
             regs_err_code, regs_eip, regs_cs, regs_eflags, regs_useresp,
             regs_ss;
    uint32_t has_fault_addr;
    uint32_t fault_addr;
    uint32_t stack[8];
    uint32_t stack_count;
} nova_crash_report_t;

#ifndef _CRASH_REPORT_T_DEFINED
#define _CRASH_REPORT_T_DEFINED
typedef nova_crash_report_t crash_report_t;
#endif

typedef struct __attribute__((packed)) {
    uint32_t has_crash;        /* 1 if crash report is currently available */
    uint32_t exported;         /* 1 if successfully exported to CRASHDMP.TXT */
    uint32_t ticks;            /* crash timestamp in ticks */
    uint32_t file_size;        /* byte size of exported CRASHDMP.TXT */
    char     filename[16];     /* e.g. "CRASHDMP.TXT" */
} nova_crashdump_status_t;

/* C prototypes for Rust exports */
int rust_crashdump_auto_export(const void* report_ptr);
int rust_crashdump_get_status(nova_crashdump_status_t* out);
int rust_crashdump_get_report(void* out_report);
int rust_crashdump_clear_exported(void);
int rust_crashdump_export_selftest(void);

#endif /* NOVA_CRASHDUMP_H */
