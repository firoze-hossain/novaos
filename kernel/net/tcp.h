#ifndef NET_TCP_H
#define NET_TCP_H

#include "../include/types.h"

/* Minimal TCP client (RFC 793): active open (client-side) only - no
 * LISTEN/passive open, matching how this project has no server-side
 * functionality anywhere else in its network stack either. One
 * connection at a time, the same "single outstanding operation"
 * pattern arp_resolve()/icmp_ping()/tftp_get()/dns_resolve() already
 * use, rather than a real connection table.
 *
 * Deliberately simplified relative to a full RFC 793 state machine:
 * stop-and-wait data transfer (one segment in flight at a time, no
 * sliding window with multiple unacknowledged segments, no
 * retransmission queue - a lost segment just times out the whole
 * operation, the caller's problem to retry), no congestion control,
 * no TIME_WAIT (closes straight to CLOSED after the final ACK rather
 * than waiting out 2*MSL for delayed duplicate segments - a real
 * simplification that could misbehave against a genuinely lossy or
 * adversarial network, fine for the controlled/local-ish networks
 * this has been tested against). See PROGRESS.md for the full scope
 * note. */

/* Opens a connection to remote_ip:remote_port (a full 3-way
 * handshake). Returns true once ESTABLISHED, false on timeout, RST,
 * or if a connection is already open (only one at a time). */
bool tcp_connect(uint32_t remote_ip, uint16_t remote_port);

/* Sends `len` bytes as one segment and blocks for its ACK. Returns
 * true once acknowledged, false on timeout or if not connected.
 * `len` is not chunked automatically - keep segments under the local
 * MSS (roughly 1400 bytes is safe) or call this multiple times. */
bool tcp_send(const void* data, uint16_t len);

/* Blocks until data arrives (or the peer closes, or `timeout_ticks`
 * elapses) and copies up to `buf_size` bytes into `buf`. Returns the
 * number of bytes received, 0 if the peer closed the connection
 * cleanly with no more data, or -1 on timeout/error. */
int tcp_receive(void* buf, uint16_t buf_size, uint32_t timeout_ticks);

/* Closes the connection (a real 4-way FIN/ACK exchange, not just
 * locally forgetting about it) and returns to a state where
 * tcp_connect() can be called again. Safe to call even if not
 * currently connected. */
void tcp_close(void);

/* Called by ip_handle_packet() for incoming TCP segments - checks
 * whether a segment matches the current connection (if any) and
 * buffers it for whichever of the functions above is waiting. */
void tcp_handle_packet(uint32_t src_ip, const uint8_t* payload,
                        uint16_t length);

#endif
