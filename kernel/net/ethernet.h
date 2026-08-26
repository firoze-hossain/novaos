#ifndef NET_ETHERNET_H
#define NET_ETHERNET_H

#include "../include/types.h"

#define ETHERTYPE_IPV4 0x0800
#define ETHERTYPE_ARP  0x0806

#define ETH_HEADER_LEN 14
#define ETH_MTU_PAYLOAD 1500

typedef struct __attribute__((packed)) {
    uint8_t  dest_mac[6];
    uint8_t  src_mac[6];
    uint16_t ethertype; /* big-endian on the wire - see eth_htons() */
} eth_header_t;

/* Byte-swaps a 16-bit value between host (little-endian x86) and
 * network (big-endian) order. Named locally rather than assuming a
 * <arpa/inet.h> (there isn't one - this is a freestanding kernel). */
static inline uint16_t eth_htons(uint16_t host_value) {
    return (uint16_t)((host_value << 8) | (host_value >> 8));
}
#define eth_ntohs eth_htons /* the swap is its own inverse */

/* Same idea, 32-bit - added in Phase 28 for TCP's sequence/ack
 * numbers (ip.c has had an equivalent private helper, be32(), since
 * Phase 6, but it's static to that file; this is the same operation
 * exposed under the naming convention every other byte-swap helper in
 * this tree already uses, rather than reaching into ip.c's internals
 * or duplicating it unnamed). */
static inline uint32_t eth_htonl(uint32_t host_value) {
    return ((host_value & 0x000000FFu) << 24) |
           ((host_value & 0x0000FF00u) << 8) |
           ((host_value & 0x00FF0000u) >> 8) |
           ((host_value & 0xFF000000u) >> 24);
}
#define eth_ntohl eth_htonl /* the swap is its own inverse */

/* Builds an Ethernet header and hands the whole frame to the NIC
 * driver. `payload` is copied immediately after the header. */
bool eth_send(const uint8_t dest_mac[6], uint16_t ethertype,
              const void* payload, uint16_t payload_len);

/* Called by net_poll() for every frame the NIC hands back; dispatches
 * to ARP or IPv4 based on ethertype. */
void eth_handle_frame(const uint8_t* frame, uint16_t length);

#endif
