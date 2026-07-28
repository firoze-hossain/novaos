#ifndef NET_UDP_H
#define NET_UDP_H

#include "../include/types.h"

#define UDP_HEADER_LEN 8

/* Minimal UDP: no checksum validation on receive, and 0 (disabled) is
 * always sent on outgoing packets - allowed by the IPv4 spec, and a
 * reasonable simplification on a trusted local virtual network (the
 * same "don't bother, the link layer already protects this enough for
 * what we need" call NovaOS's ip.c already makes about its own
 * header). See PROGRESS.md. */
bool udp_send(uint32_t dest_ip, uint16_t src_port, uint16_t dest_port,
              const void* payload, uint16_t payload_len);

/* Called by ip_handle_packet() for incoming UDP datagrams; dispatches
 * to whichever single listener udp_listen() most recently registered,
 * if its port matches. There's only ever one active listener - see
 * the header comment in udp.c - which is enough for the one thing
 * that currently uses UDP (the TFTP client, kernel/net/tftp.c), the
 * same one-outstanding-operation pattern arp.c and icmp.c already
 * use. */
void udp_handle_packet(uint32_t src_ip, const uint8_t* payload,
                        uint16_t length);

/* Starts listening on `port` - any previously buffered, unreceived
 * datagram is discarded. */
void udp_listen(uint16_t port);

/* Non-blocking: copies the most recently received datagram (if any,
 * and if one has arrived since the last successful udp_receive() call)
 * into `buf`. Returns true and fills `out_src_ip`/`out_src_port`/
 * `out_len` on success. */
bool udp_receive(uint32_t* out_src_ip, uint16_t* out_src_port, void* buf,
                  uint16_t buf_size, uint16_t* out_len);

#endif
