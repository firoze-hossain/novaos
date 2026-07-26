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

/* Reads a root-directory file by 8.3 name. Returns bytes read, or -1
 * if not mounted / not found. */
int vfs_read_file(const char* filename, void* buf, uint32_t buf_size);

#endif
