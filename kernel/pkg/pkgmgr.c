/*
 * pkgmgr.c - nova-pkg: a minimal CLI package manager
 *
 * Reentrancy note that shaped this file's structure: fat32.c's
 * directory-walk and file-read paths share static scratch buffers
 * (see kernel/fs/fat32.c), so calling vfs_read_file() from *inside* a
 * vfs_list_files() callback would corrupt the very directory listing
 * still being iterated. Every function here that needs both "list
 * files" and "read a file's contents" does so in two clearly separate
 * passes - collect names first, read contents after - rather than
 * nesting them.
 */
#include "pkgmgr.h"
#include "../fs/vfs.h"
#include "../lib/string.h"
#include "../include/kernel.h"

#define MAX_PACKAGE_FILE_SIZE 4096
#define MAX_CANDIDATE_PACKAGES 16
#define MAX_INSTALLED 16

#define INSTALL_DB_FILENAME "INSTALL.DB"

typedef struct __attribute__((packed)) {
    char name[PKG_NAME_MAX];
    char version[PKG_VERSION_MAX];
    char description[PKG_DESC_MAX];
    char app_filename[13];
} install_record_t;

static int str_eq_ci(const char* a, const char* b) {
    while (*a && *b) {
        char ca = (*a >= 'A' && *a <= 'Z') ? (char)(*a + 32) : *a;
        char cb = (*b >= 'A' && *b <= 'Z') ? (char)(*b + 32) : *b;
        if (ca != cb) {
            return 0;
        }
        a++;
        b++;
    }
    return *a == '\0' && *b == '\0';
}

/* strncpy isn't in this freestanding libc subset (see PROGRESS.md's
 * libc gaps) - a tiny bounded copy instead, always NUL-terminating
 * within `size` bytes. */
static void bounded_copy(char* dest, const char* src, size_t size) {
    size_t i = 0;
    while (i < size - 1 && src[i]) {
        dest[i] = src[i];
        i++;
    }
    dest[i] = '\0';
}

static bool has_pkg_extension(const char* filename) {
    size_t len = strlen(filename);
    return len > 4 && str_eq_ci(filename + len - 4, ".PKG");
}

/* "EDITOR.PKG" -> "EDITOR.APP" - both 8.3, so this is just replacing
 * whatever follows the last '.' rather than general path handling. */
static void derive_app_filename(const char* pkg_filename, char* out,
                                 size_t out_size) {
    size_t i = 0;
    while (pkg_filename[i] && pkg_filename[i] != '.' && i + 5 < out_size) {
        out[i] = pkg_filename[i];
        i++;
    }
    out[i] = '\0';
    strcpy(out + i, ".APP");
}

static int load_install_db(install_record_t* records, int max_records) {
    static uint8_t buf[MAX_INSTALLED * sizeof(install_record_t)];
    int n = vfs_read_file(INSTALL_DB_FILENAME, buf, sizeof(buf));
    if (n <= 0) {
        return 0; /* no database yet is not an error - nothing installed */
    }

    int count = n / (int)sizeof(install_record_t);
    if (count > max_records) {
        count = max_records;
    }
    memcpy(records, buf, (size_t)count * sizeof(install_record_t));
    return count;
}

static bool save_install_db(const install_record_t* records, int count) {
    vfs_delete_file(INSTALL_DB_FILENAME); /* ignore result: may not exist yet */
    return vfs_write_file(INSTALL_DB_FILENAME, records,
                           (uint32_t)count * sizeof(install_record_t));
}

bool pkg_is_installed(const char* name) {
    install_record_t records[MAX_INSTALLED];
    int count = load_install_db(records, MAX_INSTALLED);
    for (int i = 0; i < count; i++) {
        if (str_eq_ci(records[i].name, name)) {
            return true;
        }
    }
    return false;
}

/* Pass 1: collect *.PKG filenames only, without reading any file's
 * contents - see the reentrancy note at the top of this file. */
static char candidate_names[MAX_CANDIDATE_PACKAGES][13];
static int candidate_count;

static void collect_pkg_filename(const char* name, uint32_t size,
                                  bool is_dir) {
    (void)size;
    if (is_dir || !has_pkg_extension(name)) {
        return;
    }
    if (candidate_count >= MAX_CANDIDATE_PACKAGES) {
        return;
    }
    bounded_copy(candidate_names[candidate_count], name, 13);
    candidate_count++;
}

void pkg_list_available(pkg_list_callback_t callback) {
    if (callback == NULL) {
        return;
    }

    candidate_count = 0;
    vfs_list_files(collect_pkg_filename); /* pass 1: names only */

    install_record_t installed[MAX_INSTALLED];
    int installed_count = load_install_db(installed, MAX_INSTALLED);

    /* Pass 2: now that the directory walk is done and cluster_buf is
     * free again, read each candidate's header. */
    for (int i = 0; i < candidate_count; i++) {
        static uint8_t filebuf[MAX_PACKAGE_FILE_SIZE];
        int n = vfs_read_file(candidate_names[i], filebuf, sizeof(filebuf));
        if (n < (int)sizeof(pkg_header_t)) {
            continue; /* too small to even hold a header - skip */
        }

        pkg_header_t header;
        memcpy(&header, filebuf, sizeof(header));
        if (memcmp(header.magic, "NVPK", 4) != 0) {
            continue; /* not a real package - skip */
        }

        bool is_installed = false;
        for (int j = 0; j < installed_count; j++) {
            if (str_eq_ci(installed[j].name, header.name)) {
                is_installed = true;
                break;
            }
        }

        callback(candidate_names[i], header.name, header.version,
                 header.description, is_installed);
    }
}

void pkg_list_installed(pkg_list_callback_t callback) {
    if (callback == NULL) {
        return;
    }

    install_record_t installed[MAX_INSTALLED];
    int count = load_install_db(installed, MAX_INSTALLED);

    for (int i = 0; i < count; i++) {
        callback(installed[i].app_filename, installed[i].name,
                 installed[i].version, installed[i].description, true);
    }
}

bool pkg_install(const char* name) {
    if (pkg_is_installed(name)) {
        kernel_log("[WARN] pkg_install: '%s' already installed\n", name);
        return false;
    }

    candidate_count = 0;
    vfs_list_files(collect_pkg_filename); /* pass 1: names only */

    /* Pass 2: find the candidate whose manifest name matches. */
    char found_filename[13] = {0};
    pkg_header_t found_header;
    bool found = false;

    for (int i = 0; i < candidate_count; i++) {
        static uint8_t filebuf[MAX_PACKAGE_FILE_SIZE];
        int n = vfs_read_file(candidate_names[i], filebuf, sizeof(filebuf));
        if (n < (int)sizeof(pkg_header_t)) {
            continue;
        }
        pkg_header_t header;
        memcpy(&header, filebuf, sizeof(header));
        if (memcmp(header.magic, "NVPK", 4) != 0) {
            continue;
        }
        if (str_eq_ci(header.name, name)) {
            bounded_copy(found_filename, candidate_names[i], sizeof(found_filename));
            found_header = header;
            found = true;
            break;
        }
    }

    if (!found) {
        kernel_log("[WARN] pkg_install: package '%s' not found\n", name);
        return false;
    }

    /* Re-read the winning package now that the scan is over - simpler
     * and safer than trying to hang on to its payload across the loop
     * above, which reuses the same static scratch buffer for every
     * candidate it checks. */
    static uint8_t filebuf[MAX_PACKAGE_FILE_SIZE];
    int total = vfs_read_file(found_filename, filebuf, sizeof(filebuf));
    if (total < (int)(sizeof(pkg_header_t) + found_header.payload_size)) {
        kernel_log("[FAULT] pkg_install: '%s' payload truncated on disk\n",
                   found_filename);
        return false;
    }

    char app_filename[13];
    derive_app_filename(found_filename, app_filename, sizeof(app_filename));

    const uint8_t* payload = filebuf + sizeof(pkg_header_t);
    if (!vfs_write_file(app_filename, payload, found_header.payload_size)) {
        kernel_log("[FAULT] pkg_install: failed to write '%s'\n", app_filename);
        return false;
    }

    install_record_t records[MAX_INSTALLED];
    int count = load_install_db(records, MAX_INSTALLED);
    if (count >= MAX_INSTALLED) {
        kernel_log("[FAULT] pkg_install: install database full\n");
        vfs_delete_file(app_filename);
        return false;
    }

    bounded_copy(records[count].name, found_header.name, PKG_NAME_MAX);
    bounded_copy(records[count].version, found_header.version, PKG_VERSION_MAX);
    bounded_copy(records[count].description, found_header.description,
                 PKG_DESC_MAX);
    bounded_copy(records[count].app_filename, app_filename, 13);
    count++;

    if (!save_install_db(records, count)) {
        kernel_log("[FAULT] pkg_install: failed to update " INSTALL_DB_FILENAME
                   "\n");
        vfs_delete_file(app_filename);
        return false;
    }

    kernel_log("[ OK ] Installed package '%s' -> %s\n", found_header.name,
               app_filename);
    return true;
}

bool pkg_remove(const char* name) {
    install_record_t records[MAX_INSTALLED];
    int count = load_install_db(records, MAX_INSTALLED);

    int found_index = -1;
    for (int i = 0; i < count; i++) {
        if (str_eq_ci(records[i].name, name)) {
            found_index = i;
            break;
        }
    }

    if (found_index < 0) {
        kernel_log("[WARN] pkg_remove: '%s' is not installed\n", name);
        return false;
    }

    char app_filename[13];
    bounded_copy(app_filename, records[found_index].app_filename,
                 sizeof(app_filename));
    vfs_delete_file(app_filename); /* proceed even if this fails - see below */

    for (int i = found_index; i < count - 1; i++) {
        records[i] = records[i + 1];
    }
    count--;

    if (!save_install_db(records, count)) {
        /* The .APP file is already gone at this point; leaving a
         * stale database record around is the lesser problem - a
         * future pkg_install of the same name would still correctly
         * detect "already installed" and refuse, avoiding a leak, but
         * pkg_list_installed would show a now-broken entry. Logged so
         * it isn't silent. */
        kernel_log("[FAULT] pkg_remove: removed '%s' but failed to update "
                   INSTALL_DB_FILENAME "\n", name);
        return false;
    }

    kernel_log("[ OK ] Removed package '%s'\n", name);
    return true;
}
