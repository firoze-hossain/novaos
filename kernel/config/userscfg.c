/*
 * userscfg.c - see userscfg.h for the full design and rationale.
 */
#include "userscfg.h"
#include "../fs/vfs.h"
#include "../lib/string.h"
#include "../include/kernel.h"

/* kernel/rust/users.rs's exported (de)serialization functions - see
 * that file's own doc comments for the full contract of each. */
extern uint32_t rust_users_serialized_size(void);
extern bool rust_users_load(const uint8_t* data, uint32_t data_len);
extern int32_t rust_users_save(uint8_t* out, uint32_t out_len);

typedef struct __attribute__((packed)) {
    char magic[4]; /* "NVUS" */
    uint8_t records[USERSCFG_RECORDS_SIZE];
} userscfg_t;

/* Both userscfg_load() and userscfg_save() call this first - a
 * mismatch here means this file's own USERSCFG_RECORDS_SIZE and
 * users.rs's own RECORD_SIZE*MAX_USERS have drifted apart (a build-
 * time programming error, not a runtime hardware condition), so this
 * panics rather than silently truncating or overrunning a buffer -
 * the same reasoning virtio_net_init()'s own RX_BUFFER_COUNT/SIZE
 * check already established in this codebase. */
static void check_size_agreement(void) {
    if (rust_users_serialized_size() != USERSCFG_RECORDS_SIZE) {
        kernel_panic("userscfg: USERSCFG_RECORDS_SIZE disagrees with "
                     "kernel/rust/users.rs's own rust_users_serialized_size()");
    }
}

bool userscfg_load(void) {
    check_size_agreement();

    userscfg_t cfg;
    int n = vfs_read_file(USERSCFG_FILENAME, &cfg, sizeof(cfg));
    if (n != (int)sizeof(cfg)) {
        return false;
    }
    if (memcmp(cfg.magic, "NVUS", 4) != 0) {
        return false;
    }
    return rust_users_load(cfg.records, sizeof(cfg.records));
}

bool userscfg_save(void) {
    check_size_agreement();

    userscfg_t cfg;
    memcpy(cfg.magic, "NVUS", 4);
    int32_t written = rust_users_save(cfg.records, sizeof(cfg.records));
    if (written != (int32_t)sizeof(cfg.records)) {
        return false;
    }

    vfs_delete_file(USERSCFG_FILENAME); /* ignore result: may not exist yet */
    return vfs_write_file(USERSCFG_FILENAME, &cfg, sizeof(cfg));
}
