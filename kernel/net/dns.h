#ifndef NET_DNS_H
#define NET_DNS_H

#include "../include/types.h"

/* Minimal DNS client (RFC 1035): A-record queries only, no caching, no
 * retries beyond the single blocking wait every other synchronous
 * NovaOS network operation uses (arp_resolve(), icmp_ping(),
 * tftp_get()), and only enough name-compression handling to correctly
 * skip over a compressed name in a response (real DNS servers almost
 * always compress the answer's NAME field into a 2-byte pointer back
 * at the question) - not general enough to *build* compressed names,
 * only to parse past them. See PROGRESS.md for the full scope.
 *
 * Resolves `hostname` by querying `dns_server_ip` (see
 * NET_DNS_SERVER_IP in net.h for the default - QEMU SLIRP's built-in
 * DNS proxy) and returns true with `*out_ip` filled in on success. */
bool dns_resolve(const char* hostname, uint32_t dns_server_ip,
                  uint32_t* out_ip);

#endif
