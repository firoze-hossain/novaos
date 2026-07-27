#ifndef CONFIG_SYSCONFIG_H
#define CONFIG_SYSCONFIG_H

#include "../include/types.h"

#define SYSCONFIG_FILENAME "SYSTEM.CFG"

#define SYSCONFIG_HOSTNAME_MAX 32
#define SYSCONFIG_USERNAME_MAX 32

typedef struct __attribute__((packed)) {
    char magic[4]; /* "NVCF" */
    char hostname[SYSCONFIG_HOSTNAME_MAX];
    char username[SYSCONFIG_USERNAME_MAX];
} sysconfig_t;

/* Reads SYSTEM.CFG from the mounted filesystem. Returns false if it
 * doesn't exist yet (first boot), the filesystem isn't mounted, or
 * the file exists but doesn't look like a real config (wrong magic) -
 * the caller treats all three the same way: run the first-run wizard.
 */
bool sysconfig_load(sysconfig_t* out);

/* Writes SYSTEM.CFG - overwrite semantics (deletes any existing file
 * first), unlike the underlying fat32_write_file()'s create-only
 * primitive. Returns false on failure (e.g. not mounted, disk full). */
bool sysconfig_save(const sysconfig_t* cfg);

#endif
