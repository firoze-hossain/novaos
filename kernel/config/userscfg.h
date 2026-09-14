#ifndef CONFIG_USERSCFG_H
#define CONFIG_USERSCFG_H

#include "../include/types.h"

/*
 * userscfg.h - Phase 48: persists kernel/rust/users.rs's own UID/GID
 * account database across boots, the exact same "kernel reads/writes
 * a plain file, load validates a magic header + exact size, save
 * overwrites" pattern kernel/config/sysconfig.c already established
 * for SYSTEM.CFG - not a new persistence mechanism invented for this.
 *
 * Deliberately thin: the actual record layout (username/uid/gid/
 * password-hash, 45 bytes per account) is owned entirely by
 * kernel/rust/users.rs's own rust_users_save()/rust_users_load() -
 * this file only adds the magic-header framing and the vfs_read_file/
 * vfs_write_file calls around them, exactly the "small C-side glue"
 * scope this was planned as.
 */

#define USERSCFG_FILENAME "USERS.CFG"

/* Must equal kernel/rust/users.rs's own rust_users_serialized_size()
 * - checked at runtime in userscfg.c (not just assumed), the same
 * "two independent copies of one number must actually agree, checked,
 * not trusted" discipline kernel/drivers/virtio/virtio_net.c already
 * uses for its own RX_BUFFER_COUNT/SIZE. 45 bytes/record (1 + 32 + 4
 * + 4 + 4) * 8 accounts = 360 - see users.rs's own RECORD_SIZE/
 * MAX_USERS for where these numbers come from. */
#define USERSCFG_RECORDS_SIZE 360

/* Loads USERS.CFG, replacing whatever kernel/rust/users.rs's own
 * in-memory database currently holds. Returns false if the file
 * doesn't exist, the filesystem isn't mounted, or the file exists but
 * doesn't look like a real USERS.CFG (wrong magic or wrong size) -
 * the caller treats all three the same way, exactly matching
 * sysconfig_load()'s own documented convention. */
bool userscfg_load(void);

/* Serializes the current in-memory database and writes USERS.CFG -
 * overwrite semantics (deletes any existing file first), matching
 * sysconfig_save(). Returns false on failure (not mounted, disk full,
 * or the runtime size-agreement check above failed). */
bool userscfg_save(void);

#endif
