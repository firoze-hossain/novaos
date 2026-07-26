#ifndef NET_IP_H
#define NET_IP_H

#include "../include/types.h"

#define IP_PROTO_ICMP 1
#define IP_PROTO_UDP  17

#define IP_HEADER_LEN 20

/* Sends `payload` wrapped in an IPv4 header to `dest_ip`. ARP-resolves
 * the destination directly (no routing table - see arp.h) and returns
 * false if that fails or the NIC rejects the send. */
bool ip_send(uint32_t dest_ip, uint8_t protocol, const void* payload,
             uint16_t payload_len);

/* Called by eth_handle_frame() for incoming IPv4 packets; validates
 * the header and dispatches to ICMP/UDP by protocol number. */
void ip_handle_packet(const uint8_t src_mac[6], const uint8_t* payload,
                       uint16_t length);

#endif
