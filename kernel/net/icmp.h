#ifndef NET_ICMP_H
#define NET_ICMP_H

#include "../include/types.h"

/* Sends an ICMP Echo Request to `dest_ip` and busy-waits (via
 * net_poll(), same pattern as arp_resolve()) for up to ~3 seconds for
 * a matching Echo Reply. Returns true if one arrived before the
 * timeout. `out_rtt_ticks`, if non-NULL, receives the round-trip time
 * in timer ticks (100/sec by default - see kernel/drivers/timer). */
bool icmp_ping(uint32_t dest_ip, uint32_t* out_rtt_ticks);

/* Called by ip_handle_packet() for incoming ICMP messages: answers
 * Echo Requests addressed to us, and records Echo Replies for
 * icmp_ping() to notice. */
void icmp_handle_packet(uint32_t src_ip, const uint8_t* payload,
                         uint16_t length);

#endif
