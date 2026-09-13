#ifndef CONFIG_SYSCONFIG_H
#define CONFIG_SYSCONFIG_H

#include "../include/types.h"

#define SYSCONFIG_FILENAME "SYSTEM.CFG"

#define SYSCONFIG_HOSTNAME_MAX 32
#define SYSCONFIG_USERNAME_MAX 32

/* Phase 37: which program the kernel execs as PID 1 at the end of
 * boot - previously the literal string "SHELL.ELF", hardcoded
 * directly into kernel_main() itself (see that function's own
 * comment on why this was a real, named gap: a kernel that hardcodes
 * one specific userland's init program isn't something a *different*
 * userland/distro could actually build on without editing kernel
 * source). 13 bytes, matching every other stored-filename convention
 * already used throughout this kernel (e.g. process.h's
 * allowed_files[][13]) - this project's filesystems are 8.3-only, so
 * an 8-char name + '.' + 3-char extension + NUL is always enough. */
#define SYSCONFIG_INIT_PATH_MAX 13

typedef struct __attribute__((packed)) {
    char magic[4]; /* "NVCF" */
    char hostname[SYSCONFIG_HOSTNAME_MAX];
    char username[SYSCONFIG_USERNAME_MAX];
    char init_path[SYSCONFIG_INIT_PATH_MAX];
} sysconfig_t;

/* Reads SYSTEM.CFG from the mounted filesystem. Returns false if it
 * doesn't exist yet (first boot), the filesystem isn't mounted, or
 * the file exists but doesn't look like a real config (wrong magic) -
 * the caller treats all three the same way: run the first-run wizard.
 *
 * Honest, deliberate limitation, not an oversight: this check is
 * exact-size, not versioned - a SYSTEM.CFG written by a kernel build
 * from before init_path existed is a different size than this
 * struct now expects, so it will also read back as "invalid" and
 * trigger the first-run wizard again once, rather than silently
 * reading garbage into the new field. A real config format version
 * byte (so old configs upgrade cleanly instead of resetting) is
 * follow-up work, not attempted here - see PROGRESS.md.
 */
bool sysconfig_load(sysconfig_t* out);

/* Writes SYSTEM.CFG - overwrite semantics (deletes any existing file
 * first), unlike the underlying fat32_write_file()'s create-only
 * primitive. Returns false on failure (e.g. not mounted, disk full). */
bool sysconfig_save(const sysconfig_t* cfg);

#endif
