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

/* Non-blocking: returns true and fills mac_out immediately if `ip`
 * is already in the one-entry cache, false otherwise (does NOT send
 * a request or wait - see arp_send_request() for that). Safe to call
 * from a syscall handler, unlike arp_resolve() itself - see
 * SYS_PING_START/POLL's comment in syscall.h for why that distinction
 * matters (interrupts disabled for a syscall's whole duration means
 * the timer IRQ arp_resolve()'s own ~3s deadline depends on can never
 * fire, turning a bounded wait into an unbounded hang for any address
 * that was never already resolved - the second, deeper layer of the
 * same bug icmp_ping_start()/poll() were split to avoid, found only
 * when actually testing a ping to an address with no cached ARP
 * entry, not the gateway a boot self-test had already resolved). */
bool arp_is_cached(uint32_t ip, uint8_t mac_out[6]);

/* Sends a "who has ip? tell my_ip" broadcast request. */
void arp_send_request(uint32_t ip);

/* Called by eth_handle_frame() for incoming ARP packets: answers
 * requests for our own IP, and records replies into the (one-entry)
 * cache arp_resolve() checks. */
void arp_handle_packet(const uint8_t src_mac[6], const uint8_t* payload,
                        uint16_t length);

#endif
