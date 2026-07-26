#ifndef NET_NET_H
#define NET_NET_H

#include "../include/types.h"

/* NovaOS has no DHCP client, so networking uses a fixed static
 * configuration matching QEMU user-mode ("SLIRP") networking's
 * defaults - the one network this has been built and tested against.
 * A real network (or a different QEMU netdev configuration) would
 * need these changed to match, or a DHCP client written - tracked as
 * future work in PROGRESS.md. */
#define NET_OUR_IP      0x0A00020Fu /* 10.0.2.15  */
#define NET_GATEWAY_IP  0x0A000202u /* 10.0.2.2   */
#define NET_NETMASK     0xFFFFFF00u /* 255.255.255.0 */

/* Builds a uint32_t from four dotted-decimal octets, host byte order
 * (i.e. ip_make(10,0,2,15) == NET_OUR_IP) - used anywhere a specific
 * address needs spelling out (the shell's `ping` command, tests). */
static inline uint32_t ip_make(uint8_t a, uint8_t b, uint8_t c, uint8_t d) {
    return ((uint32_t)a << 24) | ((uint32_t)b << 16) | ((uint32_t)c << 8) | d;
}

/* Initializes the NIC and logs our MAC/IP configuration. Safe to call
 * even with no NIC attached - net_poll()/everything else just becomes
 * a no-op in that case, the same pattern Phase 3's vfs_init() used for
 * "no disk attached." */
void net_init(void);

bool net_is_up(void);

/* Drains at most one received frame from the NIC and dispatches it.
 * Since the NE2000 driver is polled (no IRQ), something has to call
 * this regularly for networking to do anything at all - the idle task
 * does, once per trip through its loop (see kernel/init/main.c). */
void net_poll(void);

/* Standard Internet checksum (RFC 1071): ones-complement sum of every
 * 16-bit word, carries folded back in, then complemented. Shared by
 * ip.c (IP header checksum) and icmp.c (ICMP message checksum, which
 * covers type/code/id/seq/data - a different range but the same
 * algorithm). */
uint16_t net_checksum16(const void* data, uint16_t length);

#endif
