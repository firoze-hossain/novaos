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
#include "../../kernel/fs/vfs.h"
#include "../../kernel/lib/string.h"
#include "../../kernel/include/kernel.h"

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

/* kernel/rust/pkgsign.rs's own exported verification function - see
 * that file's own module doc comment for the full signing scheme and
 * its honest, symmetric-HMAC trust-model limitations. */
extern bool rust_pkg_verify_signature(const uint8_t* header_ptr,
                                       uint32_t header_len,
                                       uint32_t signature_offset,
                                       const uint8_t* payload_ptr,
                                       uint32_t payload_len);

/* Phase 69: the real point of this whole feature - Gatekeeper/
 * Authenticode's own core idea ("verify a signature before trusting a
 * binary") applied here. Every package, from every source (a local
 * .PKG file or a network fetch), passes through this exact check
 * before install_from_buffer() ever writes a single byte to disk -
 * the same "verify first, act only if it checks out" shape this
 * project already uses for on-disk file I/O and network data
 * elsewhere. `offsetof` isn't available without <stddef.h>'s own
 * broader C library assumptions this freestanding kernel doesn't
 * make, so the signature field's own offset is computed the same way
 * every other fixed-layout offset in this codebase is: from the
 * struct's own field sizes directly, not a magic number. */
#define PKG_SIGNATURE_OFFSET \
    (4 + PKG_NAME_MAX + PKG_VERSION_MAX + PKG_DESC_MAX + 4)

static bool verify_package_signature(const pkg_header_t* header,
                                      const uint8_t* payload) {
    bool ok = rust_pkg_verify_signature(
        (const uint8_t*)header, (uint32_t)sizeof(pkg_header_t),
        (uint32_t)PKG_SIGNATURE_OFFSET, payload, header->payload_size);
    if (!ok) {
        kernel_log("[SECURITY] pkg install refused: '%s' has no valid "
                   "signature - either it was never signed, or it has "
                   "been modified since signing. Nothing was written "
                   "to disk.\n", header->name);
    }
    return ok;
}

/* Phase 64: the actual "install" logic, shared between pkg_install()
 * (payload already sitting in a local .PKG file on disk) and
 * pkg_fetch_and_install() (payload just arrived over the network) -
 * both end up with the exact same thing, a pkg_header_t + payload
 * bytes sitting in memory, and from that point on installing one is
 * identical to installing the other: write the payload out to a new
 * .APP file, record it in INSTALL.DB. Extracted rather than
 * duplicated, matching this file's own established practice
 * elsewhere (derive_app_filename(), load_install_db()/
 * save_install_db()). `filename_hint` is used only to derive the
 * installed .APP's own name (see derive_app_filename()) - for a
 * network fetch there is no real on-disk source filename, so the
 * caller synthesizes one from the package's own manifest name. */
static bool install_from_buffer(const pkg_header_t* header,
                                 const uint8_t* payload,
                                 const char* filename_hint) {
    if (!verify_package_signature(header, payload)) {
        return false;
    }

    char app_filename[13];
    derive_app_filename(filename_hint, app_filename, sizeof(app_filename));

    if (!vfs_write_file(app_filename, payload, header->payload_size)) {
        kernel_log("[FAULT] pkg_install: failed to write '%s'\n",
                   app_filename);
        return false;
    }

    install_record_t records[MAX_INSTALLED];
    int count = load_install_db(records, MAX_INSTALLED);
    if (count >= MAX_INSTALLED) {
        kernel_log("[FAULT] pkg_install: install database full\n");
        vfs_delete_file(app_filename);
        return false;
    }

    bounded_copy(records[count].name, header->name, PKG_NAME_MAX);
    bounded_copy(records[count].version, header->version, PKG_VERSION_MAX);
    bounded_copy(records[count].description, header->description,
                 PKG_DESC_MAX);
    bounded_copy(records[count].app_filename, app_filename, 13);
    count++;

    if (!save_install_db(records, count)) {
        kernel_log("[FAULT] pkg_install: failed to update " INSTALL_DB_FILENAME
                   "\n");
        vfs_delete_file(app_filename);
        return false;
    }

    kernel_log("[ OK ] Installed package '%s' -> %s (signature "
               "verified)\n", header->name, app_filename);
    return true;
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

    const uint8_t* payload = filebuf + sizeof(pkg_header_t);
    return install_from_buffer(&found_header, payload, found_filename);
}

/* kernel/rust/http.rs's own exported HTTP/1.1 GET client - see that
 * file's own doc comment for the full contract (why each negative
 * return value means what it means). */
extern int rust_http_get(const uint8_t* host_ptr, uint32_t host_len,
                          uint16_t port, const uint8_t* path_ptr,
                          uint32_t path_len, uint8_t* out_buf,
                          uint32_t out_buf_cap);

#define MAX_FETCH_SIZE (256 * 1024)

bool pkg_fetch_and_install(const char* repo_host, uint16_t repo_port,
                            const char* name) {
    if (pkg_is_installed(name)) {
        kernel_log("[WARN] pkg_fetch_and_install: '%s' already "
                   "installed\n", name);
        return false;
    }

    /* Repository shape deliberately the simplest one that still
     * genuinely works end to end, matching NovaOS-Release-Readiness-
     * Kernel-and-Userland.md's own explicit guidance ("copy the
     * shape" of a real, proven package manager rather than invent a
     * new one) at the smallest real scale: one fixed path convention,
     * "/packages/<NAME>.PKG" - the exact same on-disk .PKG format
     * (pkg_header_t + payload) this file already parses for local
     * installs, just fetched over HTTP instead of read from FAT32.
     * Dependency resolution and package signing are real, honestly
     * out-of-scope follow-up work, not attempted here - see
     * PROGRESS.md's own entry for this phase. */
    char path[64];
    int pos = 0;
    const char* prefix = "/packages/";
    while (prefix[pos] && pos < (int)sizeof(path) - 1) {
        path[pos] = prefix[pos];
        pos++;
    }
    int i = 0;
    while (name[i] && pos < (int)sizeof(path) - 5) {
        char c = name[i];
        if (c >= 'a' && c <= 'z') {
            c = (char)(c - 32); /* uppercase, matching this FAT32-
                                    rooted project's own 8.3 filename
                                    convention every other .PKG/.APP
                                    file already uses */
        }
        path[pos++] = c;
        i++;
    }
    const char* suffix = ".PKG";
    for (int s = 0; suffix[s] && pos < (int)sizeof(path) - 1; s++) {
        path[pos++] = suffix[s];
    }
    path[pos] = '\0';

    static uint8_t fetch_buf[MAX_FETCH_SIZE];
    int received = rust_http_get(
        (const uint8_t*)repo_host, (uint32_t)strlen(repo_host), repo_port,
        (const uint8_t*)path, (uint32_t)strlen(path), fetch_buf,
        sizeof(fetch_buf));

    if (received < 0) {
        const char* reason;
        switch (received) {
            case -1: reason = "DNS resolution failed"; break;
            case -2: reason = "could not connect to repository"; break;
            case -3: reason = "send failed after connecting"; break;
            case -4: reason = "no data received"; break;
            case -5: reason = "malformed response (no header/body "
                               "boundary)"; break;
            default: reason = "unknown error"; break;
        }
        kernel_log("[WARN] pkg_fetch_and_install: fetching '%s' from "
                   "%s:%d%s failed - %s\n", name, repo_host,
                   (int)repo_port, path, reason);
        return false;
    }

    if (received < (int)sizeof(pkg_header_t)) {
        kernel_log("[FAULT] pkg_fetch_and_install: response too short to "
                   "be a real package (%d bytes)\n", received);
        return false;
    }

    pkg_header_t header;
    memcpy(&header, fetch_buf, sizeof(header));
    if (memcmp(header.magic, "NVPK", 4) != 0) {
        kernel_log("[FAULT] pkg_fetch_and_install: fetched data is not a "
                   "real .PKG (bad magic) - the repository path may be "
                   "wrong, or this isn't really a NovaOS package "
                   "server\n");
        return false;
    }
    if (received < (int)(sizeof(pkg_header_t) + header.payload_size)) {
        kernel_log("[FAULT] pkg_fetch_and_install: '%s' payload "
                   "truncated in transit (got %d bytes, header claims "
                   "%d)\n", name, received,
                   (int)(sizeof(pkg_header_t) + header.payload_size));
        return false;
    }
    if (!str_eq_ci(header.name, name)) {
        kernel_log("[FAULT] pkg_fetch_and_install: fetched package's own "
                   "manifest name ('%s') doesn't match what was "
                   "requested ('%s') - refusing to install a different "
                   "package than the one asked for\n", header.name, name);
        return false;
    }

    /* Synthesize an 8.3 filename hint for derive_app_filename() -
     * there's no real on-disk source filename for a network fetch,
     * so build the same shape a local one would have (<NAME>.PKG). */
    char filename_hint[13];
    bounded_copy(filename_hint, header.name, 9);
    size_t hlen = strlen(filename_hint);
    if (hlen < sizeof(filename_hint) - 4) {
        filename_hint[hlen] = '.';
        filename_hint[hlen + 1] = 'P';
        filename_hint[hlen + 2] = 'K';
        filename_hint[hlen + 3] = 'G';
        filename_hint[hlen + 4] = '\0';
    }

    const uint8_t* payload = fetch_buf + sizeof(pkg_header_t);
    kernel_log("[ OK ] pkg_fetch_and_install: fetched '%s' v%s (%d bytes) "
               "from %s:%d%s\n", header.name, header.version,
               (int)header.payload_size, repo_host, (int)repo_port, path);
    return install_from_buffer(&header, payload, filename_hint);
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
