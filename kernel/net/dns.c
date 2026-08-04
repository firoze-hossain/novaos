/*
 * dns.c - minimal DNS client (RFC 1035): A-record queries only
 */
#include "dns.h"
#include "udp.h"
#include "ethernet.h"
#include "net.h"
#include "../drivers/timer/timer.h"
#include "../lib/string.h"

#define DNS_PORT 53
#define DNS_QTYPE_A   1
#define DNS_QCLASS_IN 1

typedef struct __attribute__((packed)) {
    uint16_t id;
    uint16_t flags;
    uint16_t qdcount;
    uint16_t ancount;
    uint16_t nscount;
    uint16_t arcount;
} dns_header_t;

/* "example.com" -> [7]example[3]com[0]. Returns the number of bytes
 * written, or -1 if a label is empty or longer than the DNS-mandated
 * 63-byte maximum. */
static int encode_name(const char* hostname, uint8_t* out) {
    int out_pos = 0;
    const char* label_start = hostname;

    for (;;) {
        const char* p = label_start;
        while (*p && *p != '.') {
            p++;
        }
        int label_len = (int)(p - label_start);
        if (label_len == 0 || label_len > 63) {
            return -1;
        }
        out[out_pos++] = (uint8_t)label_len;
        memcpy(out + out_pos, label_start, (size_t)label_len);
        out_pos += label_len;

        if (*p == '\0') {
            break;
        }
        label_start = p + 1;
    }

    out[out_pos++] = 0; /* root label - terminates the name */
    return out_pos;
}

/* Returns the number of bytes a name starting at `offset` occupies in
 * the packet, without decoding it - just enough to skip past it
 * correctly, which is all a response's echoed question and (almost
 * always compressed) answer NAME fields need. A length byte with its
 * top two bits set is a 2-byte compression pointer, which always
 * terminates a name (a pointer is never followed by more labels - it
 * points to where the rest of the name, if any, already lives
 * elsewhere in the packet). */
static int skip_name(const uint8_t* packet, int offset) {
    int pos = offset;
    for (;;) {
        uint8_t len_byte = packet[pos];
        if ((len_byte & 0xC0) == 0xC0) {
            return (pos - offset) + 2;
        }
        if (len_byte == 0) {
            return (pos - offset) + 1;
        }
        pos += 1 + len_byte;
    }
}

bool dns_resolve(const char* hostname, uint32_t dns_server_ip,
                  uint32_t* out_ip) {
    static uint16_t next_client_port = 52000;
    uint16_t client_port = next_client_port++;
    if (next_client_port == 0) {
        next_client_port = 52000;
    }
    udp_listen(client_port);

    uint8_t query[300];
    dns_header_t* hdr = (dns_header_t*)query;
    uint16_t qid = client_port; /* reusing the port as the query ID is a
                                    simple way to get a value that's both
                                    unique-enough for this single-
                                    outstanding-query client and free to
                                    compute - no separate counter needed */
    hdr->id = eth_htons(qid);
    hdr->flags = eth_htons(0x0100); /* recursion desired */
    hdr->qdcount = eth_htons(1);
    hdr->ancount = 0;
    hdr->nscount = 0;
    hdr->arcount = 0;

    int name_len = encode_name(hostname, query + sizeof(dns_header_t));
    if (name_len < 0) {
        return false;
    }

    int pos = (int)sizeof(dns_header_t) + name_len;
    query[pos++] = 0;
    query[pos++] = DNS_QTYPE_A;
    query[pos++] = 0;
    query[pos++] = DNS_QCLASS_IN;

    if (!udp_send(dns_server_ip, client_port, DNS_PORT, query,
                  (uint16_t)pos)) {
        return false;
    }

    uint32_t deadline = timer_get_ticks() + 300; /* ~3s, matching every
                                                     other synchronous
                                                     network operation
                                                     in this tree */
    while (timer_get_ticks() < deadline) {
        net_poll();

        uint32_t src_ip;
        uint16_t src_port;
        uint8_t response[512];
        uint16_t resp_len;
        if (!udp_receive(&src_ip, &src_port, response, sizeof(response),
                          &resp_len)) {
            continue;
        }
        if (resp_len < sizeof(dns_header_t)) {
            continue;
        }

        dns_header_t resp_hdr;
        memcpy(&resp_hdr, response, sizeof(resp_hdr));
        if (eth_ntohs(resp_hdr.id) != qid) {
            continue; /* a reply to a different, stale query - ignore */
        }

        uint16_t ancount = eth_ntohs(resp_hdr.ancount);
        if (ancount == 0) {
            return false; /* NXDOMAIN or similar - no answer at all */
        }

        int offset = (int)sizeof(dns_header_t);
        offset += skip_name(response, offset); /* the echoed question name */
        offset += 4;                           /* QTYPE + QCLASS */

        for (uint16_t i = 0; i < ancount; i++) {
            if (offset >= resp_len) {
                break;
            }
            offset += skip_name(response, offset); /* this answer's NAME */
            if (offset + 10 > resp_len) {
                break;
            }
            uint16_t rtype =
                (uint16_t)((response[offset] << 8) | response[offset + 1]);
            uint16_t rclass = (uint16_t)((response[offset + 2] << 8) |
                                          response[offset + 3]);
            uint16_t rdlength = (uint16_t)((response[offset + 8] << 8) |
                                            response[offset + 9]);
            offset += 10;

            if (rtype == DNS_QTYPE_A && rclass == DNS_QCLASS_IN &&
                rdlength == 4 && offset + 4 <= resp_len) {
                *out_ip = ((uint32_t)response[offset] << 24) |
                          ((uint32_t)response[offset + 1] << 16) |
                          ((uint32_t)response[offset + 2] << 8) |
                          (uint32_t)response[offset + 3];
                return true;
            }
            offset += rdlength;
        }
        return false; /* had answers, but none were a usable A record */
    }
    return false; /* timed out */
}
