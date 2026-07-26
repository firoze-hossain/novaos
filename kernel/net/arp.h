#ifndef NET_ARP_H
#define NET_ARP_H

#include "../include/types.h"

/* Resolves an IPv4 address to a MAC address, blocking (via net_poll()
 * in a busy-wait loop, since the NIC has no IRQ - see net.c) for up to
 * ~3 seconds. Returns true and fills mac_out on success.
 *
 * Only ever resolves the destination IP directly - there is no
 * routing table, so this only works for hosts on the same subnet as
 * us (which, for NovaOS's one tested/supported network - QEMU user-
 * mode networking's 10.0.2.0/24 - covers the gateway at 10.0.2.2,
 * which is also the only address anything currently pings). See
 * PROGRESS.md. */
bool arp_resolve(uint32_t ip, uint8_t mac_out[6]);

/* Sends a "who has ip? tell my_ip" broadcast request. */
void arp_send_request(uint32_t ip);

/* Called by eth_handle_frame() for incoming ARP packets: answers
 * requests for our own IP, and records replies into the (one-entry)
 * cache arp_resolve() checks. */
void arp_handle_packet(const uint8_t src_mac[6], const uint8_t* payload,
                        uint16_t length);

#endif
