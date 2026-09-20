/*
 * tcp.c - intentionally empty as of Phase 58. See tcp.h's own comment:
 * this file's old stop-and-wait, active-open-only TCP client was
 * removed outright, superseded by kernel/rust/tcp.rs's real RFC 793
 * implementation. Left as an empty translation unit (rather than
 * deleted) only because this session's file tools can write files on
 * the user's machine but not delete them - see PROGRESS.md.
 */
#include "tcp.h"
