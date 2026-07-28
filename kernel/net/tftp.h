#ifndef NET_TFTP_H
#define NET_TFTP_H

#include "../include/types.h"

#define TFTP_SERVER_PORT 69

/* Minimal TFTP client (RFC 1350): read-only ("octet"/binary mode
 * only), no write, no options extension, no retransmission on a lost
 * packet (see PROGRESS.md - fine on a local virtual network with
 * effectively zero loss, a real concern on anything lossier). Fetches
 * `remote_filename` from `server_ip` into `buf`, blocking (via
 * net_poll() in a loop, the same synchronous pattern arp_resolve()
 * and icmp_ping() already use) for up to a few seconds per block.
 * Returns the total number of bytes received, or -1 on failure
 * (timeout, server error, or the file not fitting in buf_size). */
int tftp_get(uint32_t server_ip, const char* remote_filename, void* buf,
             uint32_t buf_size);

#endif
