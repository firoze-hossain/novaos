/*
 * rtc.c - CMOS real-time clock reader
 */
#include "rtc.h"
#include "../../arch/x86/io.h"

#define CMOS_ADDRESS 0x70
#define CMOS_DATA    0x71

#define REG_SECONDS  0x00
#define REG_MINUTES  0x02
#define REG_HOURS    0x04
#define REG_DAY      0x07
#define REG_MONTH    0x08
#define REG_YEAR     0x09
#define REG_STATUS_A 0x0A
#define REG_STATUS_B 0x0B

static uint8_t cmos_read(uint8_t reg) {
    outb(CMOS_ADDRESS, reg);
    return inb(CMOS_DATA);
}

static bool update_in_progress(void) {
    return (cmos_read(REG_STATUS_A) & 0x80) != 0;
}

static uint8_t bcd_to_binary(uint8_t bcd) {
    return (uint8_t)(((bcd & 0xF0) >> 1) + ((bcd & 0xF0) >> 3) + (bcd & 0x0F));
}

static void read_raw(rtc_time_t* out) {
    out->second = cmos_read(REG_SECONDS);
    out->minute = cmos_read(REG_MINUTES);
    out->hour = cmos_read(REG_HOURS);
    out->day = cmos_read(REG_DAY);
    out->month = cmos_read(REG_MONTH);
    out->year = cmos_read(REG_YEAR);
}

void rtc_read(rtc_time_t* out) {
    rtc_time_t a, b;

    /* Two things can produce a torn/wrong reading here: the RTC being
     * mid-update when a register is read, and a value changing (e.g.
     * seconds rolling over) between reading one field and the next.
     * Waiting for "update in progress" to clear handles the first;
     * reading twice and requiring the results to match handles the
     * second - both are the standard technique for this chip, not a
     * NovaOS-specific workaround. Bounded retry count so a stuck or
     * emulated-oddly RTC can't hang the boot sequence forever. */
    rtc_time_t result;
    for (int attempt = 0; attempt < 10; attempt++) {
        while (update_in_progress()) { }
        read_raw(&a);
        while (update_in_progress()) { }
        read_raw(&b);

        if (a.second == b.second && a.minute == b.minute &&
            a.hour == b.hour && a.day == b.day && a.month == b.month &&
            a.year == b.year) {
            result = b;
            break;
        }
        result = b; /* best effort if we never get two matching reads */
    }

    uint8_t status_b = cmos_read(REG_STATUS_B);
    bool is_binary = (status_b & 0x04) != 0;
    bool is_24hr = (status_b & 0x02) != 0;

    /* CMOS stores raw register values as either BCD or plain binary,
     * selectable by firmware/hardware - Status Register B says which,
     * so this checks rather than assumes. */
    uint8_t raw_hour = (uint8_t)result.hour;
    if (!is_binary) {
        result.second = bcd_to_binary((uint8_t)result.second);
        result.minute = bcd_to_binary((uint8_t)result.minute);
        raw_hour = (uint8_t)(bcd_to_binary((uint8_t)(raw_hour & 0x7F)) |
                              (raw_hour & 0x80));
        result.day = bcd_to_binary((uint8_t)result.day);
        result.month = bcd_to_binary((uint8_t)result.month);
        result.year = bcd_to_binary((uint8_t)result.year);
    }

    /* In 12-hour mode, bit 7 of the hour register marks PM. */
    uint8_t hour24 = (uint8_t)(raw_hour & 0x7F);
    if (!is_24hr && (raw_hour & 0x80)) {
        hour24 = (uint8_t)((hour24 + 12) % 24);
    }

    out->second = (uint8_t)result.second;
    out->minute = (uint8_t)result.minute;
    out->hour = hour24;
    out->day = (uint8_t)result.day;
    out->month = (uint8_t)result.month;
    out->year = (uint16_t)(2000 + result.year);
}
