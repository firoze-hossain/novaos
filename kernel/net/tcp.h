#ifndef NET_TCP_H
#define NET_TCP_H

/* Phase 58: superseded by kernel/rust/tcp.rs's real RFC 793 state
 * machine (real retransmission/sliding-window data transfer,
 * LISTEN/passive-open/accept, multiple concurrent connections) and the
 * new Berkeley-sockets-style syscall API (SYS_SOCKET/SYS_BIND/
 * SYS_LISTEN/SYS_ACCEPT/SYS_CONNECT - see kernel/arch/x86/cpu/
 * syscall.h). This header's old single-connection, stop-and-wait,
 * active-open-only client (tcp_connect()/tcp_send()/tcp_receive()/
 * tcp_close()/tcp_handle_packet()) has been removed outright rather
 * than left as unreachable dead code - kernel/net/ip.c now calls
 * straight into rust_tcp_handle_packet() (declared locally at that
 * call site, this project's established no-shared-header FFI
 * convention - see e.g. kernel/include/smp.h's own comment) instead of
 * this header's old tcp_handle_packet(), and kernel/init/main.c's own
 * HTTP self-test now goes through rust_tcp_socket()/rust_tcp_connect()/
 * rust_tcp_send()/rust_tcp_recv()/rust_tcp_close() instead.
 *
 * This file (and kernel/net/tcp.c) is kept only as an intentionally
 * empty placeholder rather than deleted: this session's own file
 * tools can write files on the user's machine but not delete them -
 * see PROGRESS.md's Phase 58 entry, which names both files explicitly
 * so a future contributor doesn't mistake them for accidental cruft.
 * Safe to `git rm` both by hand. */

#endif
