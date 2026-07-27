/*
 * vfs.c - thin pass-through to the mounted filesystem (FAT32 only, for now)
 */
#include "vfs.h"
#include "fat32.h"
#include "../drivers/ata/ata.h"
#include "../drivers/vga/vga.h"
#include "../lib/stdio.h"

void vfs_init(void) {
    ata_init();
    if (ata_is_present()) {
        fat32_init();
    }
}

bool vfs_is_mounted(void) {
    return fat32_is_mounted();
}

static void print_entry(const char* name, uint32_t size, bool is_dir) {
    char line[64];
    if (is_dir) {
        snprintf(line, sizeof(line), "  <DIR>  %s\n", name);
    } else {
        snprintf(line, sizeof(line), "  %6d  %s\n", (int)size, name);
    }
    vga_puts(line);
}

void vfs_ls(void) {
    if (!vfs_is_mounted()) {
        vga_puts("No filesystem mounted (no ATA disk found at boot)\n");
        return;
    }
    fat32_list_root(print_entry);
}

void vfs_list_files(vfs_list_callback_t callback) {
    if (!vfs_is_mounted()) {
        return;
    }
    fat32_list_root(callback);
}

int vfs_read_file(const char* filename, void* buf, uint32_t buf_size) {
    if (!vfs_is_mounted()) {
        return -1;
    }
    return fat32_read_file(filename, buf, buf_size);
}

bool vfs_write_file(const char* filename, const void* data, uint32_t size) {
    if (!vfs_is_mounted()) {
        return false;
    }
    return fat32_write_file(filename, data, size);
}

bool vfs_delete_file(const char* filename) {
    if (!vfs_is_mounted()) {
        return false;
    }
    return fat32_delete_file(filename);
}
