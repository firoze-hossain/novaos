#ifndef DRIVERS_DRIVER_LOCKS_H
#define DRIVERS_DRIVER_LOCKS_H

#include "../include/types.h"

/*
 * driver_locks.h - Month 5: C declarations for fine-grained per-driver
 * locking implemented in Rust (kernel/rust/driver_locks.rs).
 *
 * Replaces the coarse Phase 57 vfs_lock stopgap with fine-grained,
 * per-driver locks scoped to each driver's own static state.
 */

typedef enum {
    DRIVER_LOCK_FAT32    = 0,
    DRIVER_LOCK_EXT2     = 1,
    DRIVER_LOCK_ATA      = 2,
    DRIVER_LOCK_BLOCKDEV = 3,
    DRIVER_LOCK_SERIAL   = 4,
    DRIVER_LOCK_VGA      = 5,
    DRIVER_LOCK_KEYBOARD = 6,
    DRIVER_LOCK_MOUSE    = 7,
    DRIVER_LOCK_NVME     = 8,
    DRIVER_LOCK_XHCI     = 9,
    DRIVER_LOCK_WIFI     = 10,
    DRIVER_LOCK_RTC      = 11,
    DRIVER_LOCK_COUNT    = 12
} driver_lock_id_t;

/* Core driver lock functions implemented in Rust */
uint32_t    rust_driver_lock_acquire(uint32_t driver_id);
void        rust_driver_lock_release(uint32_t driver_id, uint32_t saved_eflags);
int         rust_driver_lock_try_acquire(uint32_t driver_id, uint32_t* out_saved_eflags);
int         rust_driver_lock_is_locked(uint32_t driver_id);
int         rust_driver_lock_stats(uint32_t driver_id, uint32_t* out_acquires, uint32_t* out_contentions);
const char* rust_driver_lock_name(uint32_t driver_id);

/* Fast dedicated helpers for driver call sites */
uint32_t rust_fat32_lock_acquire(void);
void     rust_fat32_lock_release(uint32_t saved_eflags);

uint32_t rust_ext2_lock_acquire(void);
void     rust_ext2_lock_release(uint32_t saved_eflags);

uint32_t rust_ata_lock_acquire(void);
void     rust_ata_lock_release(uint32_t saved_eflags);

uint32_t rust_blockdev_lock_acquire(void);
void     rust_blockdev_lock_release(uint32_t saved_eflags);

uint32_t rust_serial_lock_acquire(void);
void     rust_serial_lock_release(uint32_t saved_eflags);

uint32_t rust_vga_lock_acquire(void);
void     rust_vga_lock_release(uint32_t saved_eflags);

/* Kernel self-test */
int rust_driver_locks_selftest(void);

#endif
