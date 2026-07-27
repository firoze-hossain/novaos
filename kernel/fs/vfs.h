#ifndef FS_VFS_H
#define FS_VFS_H

#include "../include/types.h"

/* NovaOS's "VFS" is currently a thin pass-through to whichever single
 * filesystem driver is mounted (only fat32.c exists so far) rather
 * than a real multi-filesystem dispatch table. Kept as its own module
 * (instead of having the shell call fat32_* directly) so that adding
 * a second filesystem later - or a real mount-point table - only
 * means changing vfs.c, not every caller. See PROJECT_PLAN.md section
 * 3 and PROGRESS.md for the honest scope of what "VFS" means today. */

/* Initializes the ATA driver and mounts FAT32 on top of it if a
 * suitable drive is found. Safe to call even with no disk attached -
 * vfs_ls()/vfs_read_file() just report "not mounted" in that case. */
void vfs_init(void);

bool vfs_is_mounted(void);

/* Prints the root directory to the VGA console (used by the shell's
 * `ls` command). */
void vfs_ls(void);

/* Raw callback-based directory listing (used by the package manager
 * to find *.PKG files) - vfs_ls() above is the print-to-screen
 * convenience wrapper around the same underlying walk. */
typedef void (*vfs_list_callback_t)(const char* name, uint32_t size,
                                     bool is_directory);
void vfs_list_files(vfs_list_callback_t callback);

/* Reads a root-directory file by 8.3 name. Returns bytes read, or -1
 * if not mounted / not found. */
int vfs_read_file(const char* filename, void* buf, uint32_t buf_size);

/* Creates a new root-directory file. Fails if the name already exists
 * or the filesystem isn't mounted - see fat32_write_file(). */
bool vfs_write_file(const char* filename, const void* data, uint32_t size);

/* Deletes a root-directory file. Returns false if not found or the
 * filesystem isn't mounted. */
bool vfs_delete_file(const char* filename);

#endif
