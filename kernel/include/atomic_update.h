/*
 * atomic_update.h - Month 6: Atomic OS Update Mechanism (ABI & Kernel C Interface)
 *
 * Provides safe apply and rollback with dual boot slots (Slot A / Slot B),
 * cryptographic package verification (HMAC-SHA256 via pkgsign), and
 * write-ahead journal transactional safety on FAT32.
 */
#ifndef KERNEL_ATOMIC_UPDATE_H
#define KERNEL_ATOMIC_UPDATE_H

#include "types.h"

#define BOOTSLOT_MAGIC "NOVABOOT"
#define BOOTSLOT_FILENAME "BOOTSLOT.DAT"
#define OSUPDATE_FILENAME "OSUPDATE.PKG"

#define NOVA_BOOT_SLOT_A 0
#define NOVA_BOOT_SLOT_B 1

#define NOVA_SLOT_STATE_NORMAL      0
#define NOVA_SLOT_STATE_TESTING     1
#define NOVA_SLOT_STATE_ROLLED_BACK 2

#define NOVA_UPDATE_OP_STATUS           1
#define NOVA_UPDATE_OP_STAGE_AND_APPLY  2
#define NOVA_UPDATE_OP_CONFIRM          3
#define NOVA_UPDATE_OP_ROLLBACK         4

#define NOVA_UPDATE_OK               0
#define NOVA_UPDATE_ERR_INVALID_OP  -1
#define NOVA_UPDATE_ERR_BAD_HEADER  -2
#define NOVA_UPDATE_ERR_BAD_SIG     -3
#define NOVA_UPDATE_ERR_IO          -4
#define NOVA_UPDATE_ERR_JOURNAL     -5
#define NOVA_UPDATE_ERR_PERM        -6
#define NOVA_UPDATE_ERR_FAULT       -7

typedef struct __attribute__((packed)) {
    uint8_t active_slot;
    uint8_t fallback_slot;
    uint8_t slot_state;
    uint8_t boot_attempts;
    char    slot_a_version[16];
    char    slot_b_version[16];
    int32_t last_error;
} nova_update_status_t;

/* C-callable entry points exported by kernel/rust/atomic_update.rs */
void rust_os_update_init(void);
void rust_os_update_boot_hook(void);
int  rust_os_update_confirm_healthy(void);
int  rust_os_update_apply_package(const void* pkg_ptr, uint32_t pkg_len);
int  rust_os_update_rollback(void);
int  rust_os_update_get_status(nova_update_status_t* out_status);
int  rust_os_update_selftest(void);

#endif /* KERNEL_ATOMIC_UPDATE_H */
