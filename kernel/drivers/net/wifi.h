/*
 * kernel/drivers/net/wifi.h - Month 5 (Driver breadth & performance): Wi-Fi driver
 *
 * Provides wireless networking (802.11) support for common laptop chipsets
 * (Intel PRO/Wireless 2200BG/2915ABG/3945ABG and Realtek RTL8180/RTL8185/RTL8188).
 *
 * Feeds directly into NovaOS's existing network stack (ARP, IPv4, ICMP, UDP, TCP)
 * at the exact same layer as RTL8139 and virtio-net by bridging 802.11 wireless
 * frames with 802.3 Ethernet frames via 802.2 LLC/SNAP encapsulation/decapsulation.
 */

#ifndef DRIVERS_NET_WIFI_H
#define DRIVERS_NET_WIFI_H

#include "../../include/types.h"

/* 802.11 Frame Control Bitmasks & Fields */
#define WIFI_FC_VERSION_MASK      0x0003
#define WIFI_FC_TYPE_MASK         0x000C
#define WIFI_FC_SUBTYPE_MASK      0x00F0
#define WIFI_FC_TO_DS             0x0100
#define WIFI_FC_FROM_DS           0x0200
#define WIFI_FC_MORE_FRAG         0x0400
#define WIFI_FC_RETRY             0x0800
#define WIFI_FC_PWR_MGMT          0x1000
#define WIFI_FC_MORE_DATA         0x2000
#define WIFI_FC_PROTECTED         0x4000
#define WIFI_FC_ORDER             0x8000

/* 802.11 Frame Types */
#define WIFI_TYPE_MGMT            0x0000
#define WIFI_TYPE_CTRL            0x0004
#define WIFI_TYPE_DATA            0x0008

/* 802.11 Management Subtypes */
#define WIFI_SUBTYPE_ASSOC_REQ    0x0000
#define WIFI_SUBTYPE_ASSOC_RESP   0x0010
#define WIFI_SUBTYPE_REASSOC_REQ  0x0020
#define WIFI_SUBTYPE_REASSOC_RESP 0x0030
#define WIFI_SUBTYPE_PROBE_REQ    0x0040
#define WIFI_SUBTYPE_PROBE_RESP   0x0050
#define WIFI_SUBTYPE_BEACON       0x0080
#define WIFI_SUBTYPE_ATIM         0x0090
#define WIFI_SUBTYPE_DISASSOC     0x00A0
#define WIFI_SUBTYPE_AUTH         0x00B0
#define WIFI_SUBTYPE_DEAUTH       0x00C0
#define WIFI_SUBTYPE_ACTION       0x00D0

/* 802.11 Data Subtypes */
#define WIFI_SUBTYPE_DATA         0x0000
#define WIFI_SUBTYPE_DATA_CF_ACK  0x0010
#define WIFI_SUBTYPE_DATA_CF_POLL 0x0020
#define WIFI_SUBTYPE_NULL         0x0040

/* 802.11 Information Element IDs */
#define WIFI_IE_SSID              0
#define WIFI_IE_SUPPORTED_RATES   1
#define WIFI_IE_DS_CHANNEL        3
#define WIFI_IE_RSN_WPA2          48

/* Maximum sizes */
#define WIFI_MAX_FRAME_SIZE       2346
#define WIFI_ETH_MAX_FRAME        1518
#define WIFI_MAX_SSID_LEN         32
#define WIFI_MAX_SCAN_APS         16

/* 802.11 MAC Header (24 bytes packed) */
typedef struct __attribute__((packed)) {
    uint16_t frame_control;
    uint16_t duration_id;
    uint8_t  addr1[6]; /* Receiver / BSSID / Destination */
    uint8_t  addr2[6]; /* Transmitter / Source */
    uint8_t  addr3[6]; /* Filtering / BSSID / Destination */
    uint16_t seq_ctrl; /* Sequence Number (15:4) + Fragment (3:0) */
} wifi_mac_hdr_t;

/* 802.2 LLC / SNAP Header (8 bytes packed) for 802.3 <-> 802.11 translation */
typedef struct __attribute__((packed)) {
    uint8_t  dsap;       /* 0xAA */
    uint8_t  ssap;       /* 0xAA */
    uint8_t  control;    /* 0x03 */
    uint8_t  oui[3];     /* 0x00, 0x00, 0x00 */
    uint16_t ethertype;  /* Big-endian: e.g. 0x0800 (IPv4), 0x0806 (ARP) */
} wifi_llc_snap_hdr_t;

/* 802.11 Beacon fixed parameters (12 bytes packed) */
typedef struct __attribute__((packed)) {
    uint64_t timestamp;
    uint16_t beacon_interval;
    uint16_t capabilities;
} wifi_beacon_fixed_t;

/* Link State */
typedef enum {
    WIFI_STATE_DISCONNECTED  = 0,
    WIFI_STATE_SCANNING      = 1,
    WIFI_STATE_ASSOCIATING   = 2,
    WIFI_STATE_ASSOCIATED    = 3,
    WIFI_STATE_AUTHENTICATED = 4
} wifi_state_t;

/* Security / Authentication */
typedef enum {
    WIFI_AUTH_OPEN = 0,
    WIFI_AUTH_WEP  = 1,
    WIFI_AUTH_WPA2 = 2
} wifi_auth_t;

/* Detected Access Point Info */
typedef struct {
    char        ssid[WIFI_MAX_SSID_LEN + 1];
    uint8_t     bssid[6];
    uint8_t     channel;
    int8_t      rssi; /* Signal strength in dBm */
    wifi_auth_t auth;
} wifi_ap_info_t;

/* Telemetry counters */
typedef struct {
    uint32_t tx_packets;
    uint32_t rx_packets;
    uint32_t tx_bytes;
    uint32_t rx_bytes;
    uint32_t dropped_packets;
    uint32_t errors;
} wifi_stats_t;

/* Comprehensive Wi-Fi Status */
typedef struct {
    char         chipset_name[48];
    uint8_t      mac_address[6];
    wifi_state_t state;
    char         current_ssid[WIFI_MAX_SSID_LEN + 1];
    uint8_t      current_bssid[6];
    uint8_t      channel;
    int8_t       rssi;
    uint16_t     link_speed_mbps;
    wifi_stats_t stats;
} wifi_info_t;

/* Core Driver API */
void wifi_init(void);
bool wifi_is_present(void);
bool wifi_is_associated(void);
bool wifi_send(const void* frame, uint16_t length);
uint16_t wifi_receive(void* buffer);
const uint8_t* wifi_mac_address(void);
const wifi_info_t* wifi_get_info(void);

/* Management API */
int  wifi_scan(wifi_ap_info_t* out_aps, int max_aps);
bool wifi_connect(const char* ssid, const char* key);
void wifi_disconnect(void);

/* Translation helpers */
uint16_t wifi_encapsulate(const void* eth_frame, uint16_t eth_len,
                          void* out_wifi_frame, uint16_t max_out,
                          const uint8_t* bssid, const uint8_t* sta_mac);
uint16_t wifi_decapsulate(const void* wifi_frame, uint16_t wifi_len,
                          void* out_eth_frame, uint16_t max_out);

/* Self-test hook */
int wifi_selftest(void);

#endif /* DRIVERS_NET_WIFI_H */
