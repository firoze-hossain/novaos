/*
 * nova_pkgrepo_abi.h - ABI definitions for NovaOS package repository server.
 *
 * Month 4 - Server & security core: Package-repository network protocol
 * support, server side. Exposes control and telemetry for the in-kernel
 * package repository HTTP server.
 */

#ifndef _NOVA_PKGREPO_ABI_H
#define _NOVA_PKGREPO_ABI_H

#include <stdint.h>

#define NOVA_SYS_PKGREPO_CTL 78

#define NOVA_PKGREPO_CTL_START  1
#define NOVA_PKGREPO_CTL_STOP   2
#define NOVA_PKGREPO_CTL_STATUS 3

#define NOVA_PKGREPO_DEFAULT_PORT 8080

typedef struct {
    uint32_t is_running;
    uint16_t port;
    uint16_t _reserved;
    uint32_t requests_total;
    uint32_t packages_served;
    uint32_t bytes_served;
} nova_pkgrepo_status_t;

#endif /* _NOVA_PKGREPO_ABI_H */
