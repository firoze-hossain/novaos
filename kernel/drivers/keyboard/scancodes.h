#ifndef DRIVERS_KEYBOARD_SCANCODES_H
#define DRIVERS_KEYBOARD_SCANCODES_H

/* PS/2 Scan Code Set 1, "make" codes only (bit 7 clear). A "break" code
 * is the same value with bit 7 set (make | 0x80), handled separately in
 * keyboard.c. 0 means "no ASCII equivalent" (arrows, F-keys, etc. -
 * left for a later revision when we add a proper keycode enum). */

#define SC_LSHIFT 0x2A
#define SC_RSHIFT 0x36
#define SC_CAPSLOCK 0x3A
#define SC_ENTER 0x1C
#define SC_BACKSPACE 0x0E
#define SC_TAB 0x0F

static const char scancode_ascii_lower[128] = {
    0,   27,  '1', '2', '3', '4', '5', '6', '7', '8', /* 0x00-0x09 */
    '9', '0', '-', '=', '\b','\t','q', 'w', 'e', 'r', /* 0x0A-0x13 */
    't', 'y', 'u', 'i', 'o', 'p', '[', ']', '\n', 0,   /* 0x14-0x1D (0x1D=LCtrl) */
    'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';', /* 0x1E-0x27 */
    '\'','`', 0,  '\\','z', 'x', 'c', 'v', 'b', 'n',  /* 0x28-0x31 (0x2A=LShift) */
    'm', ',', '.', '/', 0,  '*', 0,  ' ', 0,   0,     /* 0x32-0x3B (0x36=RShift) */
    0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   /* 0x3C-0x45 (F1-F10, NumLock, ScrollLock) */
    0,   0,   0,   0,   0,   0,   0,   '7', '8', '9', /* 0x46-0x49 */
    '-', '4', '5', '6', '+', '1', '2', '3', '0', '.', /* 0x4A-0x53 */
};

static const char scancode_ascii_upper[128] = {
    0,   27,  '!', '@', '#', '$', '%', '^', '&', '*',
    '(', ')', '_', '+', '\b','\t','Q', 'W', 'E', 'R',
    'T', 'Y', 'U', 'I', 'O', 'P', '{', '}', '\n', 0,
    'A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L', ':',
    '"', '~', 0,  '|', 'Z', 'X', 'C', 'V', 'B', 'N',
    'M', '<', '>', '?', 0,  '*', 0,  ' ', 0,   0,
    0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    0,   0,   0,   0,   0,   0,   0,   '7', '8', '9',
    '-', '4', '5', '6', '+', '1', '2', '3', '0', '.',
};

#endif
