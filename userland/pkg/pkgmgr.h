#ifndef PKG_PKGMGR_H
#define PKG_PKGMGR_H

#include "../../kernel/include/types.h"

/* nova-pkg: a minimal package manager, CLI only.
 *
 * Scope note: the original Phase 8 plan (PROJECT_PLAN.md) called for
 * both a CLI package manager AND a GUI "Software Center" front-end.
 * Only the CLI half is built here - a GUI front-end needs general
 * text rendering in graphics mode, which doesn't exist yet (Phase 7's
 * VGA Mode 13h mode only has a handful of hand-built digit glyphs,
 * see kernel/gui/font5x7.h). Tracked as follow-up work.
 *
 * Package format: a package is a single file named "<NAME>.PKG" in
 * the FAT32 root directory (there are no subdirectories to organize a
 * real repository into yet - see fs/fat32.h). Each .PKG file is a
 * small fixed header (see pkg_header_t below) followed immediately by
 * the raw payload bytes. "Installing" a package copies its payload
 * out to a new file "<NAME>.APP" and records the install in a simple
 * on-disk database file (INSTALL.DB); "removing" it deletes that file
 * and the database record. There is no network fetch - packages must
 * already exist on the mounted disk (see tools/fixtures/ for the demo
 * packages baked into the test image by `make disk.img`) - NovaOS's
 * network stack (Phase 6) only speaks ICMP, not HTTP/FTP, so there is
 * nothing to fetch a package *from* yet.
 */

#define PKG_NAME_MAX 16
#define PKG_VERSION_MAX 8
#define PKG_DESC_MAX 64

typedef struct __attribute__((packed)) {
    char magic[4]; /* "NVPK" */
    char name[PKG_NAME_MAX];
    char version[PKG_VERSION_MAX];
    char description[PKG_DESC_MAX];
    uint32_t payload_size;
} pkg_header_t;

typedef void (*pkg_list_callback_t)(const char* filename, const char* name,
                                     const char* version,
                                     const char* description,
                                     bool installed);

/* Scans the root directory for "*.PKG" files, parses each manifest
 * header, and calls `callback` once per valid package found -
 * `installed` reflects whether INSTALL.DB currently lists it. */
void pkg_list_available(pkg_list_callback_t callback);

/* Same idea, but only for entries INSTALL.DB lists (regardless of
 * whether the original .PKG file is still present). */
void pkg_list_installed(pkg_list_callback_t callback);

/* Installs package `name` (matched against the manifest's `name`
 * field, case-insensitive) - finds "<X>.PKG" among all root-directory
 * packages whose manifest name matches, extracts its payload to
 * "<X>.APP", and records the install. Fails if not found, already
 * installed, or the underlying FAT32 write fails (e.g. disk full). */
bool pkg_install(const char* name);

/* Removes an installed package: deletes its "<X>.APP" file and the
 * INSTALL.DB record. Fails if not currently installed. */
bool pkg_remove(const char* name);

/* Phase 64: fetches "<name>.PKG" from a real HTTP repository server
 * (repo_host:repo_port, "/packages/<NAME>.PKG") and installs it -
 * the real network-fetch path this header's own earlier comment
 * named as a genuine gap ("There is no network fetch... nothing to
 * fetch a package *from* yet"), now closed. Verifies the fetched
 * data is really a NovaOS package (magic + a payload size that
 * actually fits what arrived) and that its own manifest name matches
 * what was asked for, before ever writing anything to disk - the
 * same "don't trust the network, verify before acting" discipline
 * this project applies to file I/O over ATA/FAT32/ext2 already.
 * Fails (returns false, with a specific, logged reason) on DNS/
 * connect/send failure, a malformed or non-package response, a
 * truncated payload, a name mismatch, or if `name` is already
 * installed. */
bool pkg_fetch_and_install(const char* repo_host, uint16_t repo_port,
                            const char* name);

bool pkg_is_installed(const char* name);

#endif
