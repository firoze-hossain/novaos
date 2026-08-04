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
#define NET_DNS_SERVER_IP 0x0A000203u /* 10.0.2.3 - QEMU SLIRP's
                                          conventional built-in DNS
                                          proxy address, one more of
                                          the "self-contained test
                                          target" addresses SLIRP
                                          always answers on, the same
                                          principle the gateway ping
                                          and TFTP self-tests already
                                          rely on (see Phase 19 in
                                          PROGRESS.md) */
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
 * Since neither NIC driver is interrupt-driven, something has to call
 * this regularly for networking to do anything at all - the idle task
 * does, once per trip through its loop (see kernel/init/main.c). */
void net_poll(void);

/* Phase 16: a small seam so kernel/net/ethernet.c doesn't need to know
 * which physical NIC driver is actually active. net_init() picks one
 * (preferring the RTL8139 PCI NIC if Phase 13's enumeration finds one,
 * falling back to the NE2000 ISA driver otherwise - see net.c) and
 * these three functions forward to whichever was chosen. Not a
 * general driver-model abstraction (there's no way to register a
 * third NIC type without editing net.c directly) - just enough to let
 * two concrete drivers coexist in the same tree without ethernet.c
 * hardcoding either one's name. */
bool net_driver_send(const void* frame, uint16_t length);
uint16_t net_driver_receive(void* buffer);
const uint8_t* net_driver_mac_address(void);

/* Standard Internet checksum (RFC 1071): ones-complement sum of every
 * 16-bit word, carries folded back in, then complemented. Shared by
 * ip.c (IP header checksum) and icmp.c (ICMP message checksum, which
 * covers type/code/id/seq/data - a different range but the same
 * algorithm). */
uint16_t net_checksum16(const void* data, uint16_t length);

#endif
