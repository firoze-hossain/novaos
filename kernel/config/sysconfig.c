/*
 * sysconfig.c - persists basic system identity (hostname, username)
 * across boots in a single fixed-size file
 */
#include "sysconfig.h"
#include "../fs/vfs.h"
#include "../lib/string.h"

bool sysconfig_load(sysconfig_t* out) {
    int n = vfs_read_file(SYSCONFIG_FILENAME, out, sizeof(*out));
    if (n != (int)sizeof(*out)) {
        return false;
    }
    if (memcmp(out->magic, "NVCF", 4) != 0) {
        return false;
    }
    /* Defensively NUL-terminate even if the stored strings somehow
     * filled their whole field with no terminator - every reader of
     * these fields (vga_printf, string comparisons) assumes one. */
    out->hostname[SYSCONFIG_HOSTNAME_MAX - 1] = '\0';
    out->username[SYSCONFIG_USERNAME_MAX - 1] = '\0';
    return true;
}

bool sysconfig_save(const sysconfig_t* cfg) {
    sysconfig_t to_write = *cfg;
    memcpy(to_write.magic, "NVCF", 4);

    vfs_delete_file(SYSCONFIG_FILENAME); /* ignore result: may not exist yet */
    return vfs_write_file(SYSCONFIG_FILENAME, &to_write, sizeof(to_write));
}
