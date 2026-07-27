#ifndef DRIVERS_RTC_H
#define DRIVERS_RTC_H

#include "../../include/types.h"

typedef struct {
    uint16_t year;  /* full 4-digit year, e.g. 2026 */
    uint8_t month;  /* 1-12 */
    uint8_t day;    /* 1-31 */
    uint8_t hour;   /* 0-23 */
    uint8_t minute; /* 0-59 */
    uint8_t second; /* 0-59 */
} rtc_time_t;

/* Reads the current date/time from the CMOS real-time clock (ports
 * 0x70/0x71). Handles both BCD and binary storage modes (checked via
 * CMOS Status Register B, rather than assumed) and waits out any
 * in-progress update per the standard "read twice, compare" technique
 * to avoid a torn read - see rtc.c for why that matters. Assumes the
 * 21st century (years 0-99 read from CMOS map to 2000-2099), which
 * covers every real and virtual machine this will plausibly run on
 * for a long time yet. */
void rtc_read(rtc_time_t* out);

#endif
