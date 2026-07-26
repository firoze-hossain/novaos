/*
 * vga_graphics.c - VGA Mode 13h via direct register programming
 *
 * The four VGA register groups and their index/data port pairs:
 *   Sequencer:           0x3C4 (index) / 0x3C5 (data)
 *   CRT Controller:      0x3D4 (index) / 0x3D5 (data)
 *   Graphics Controller:  0x3CE (index) / 0x3CF (data)
 *   Attribute Controller:  0x3C0 (index AND data, alternating - see
 *                           enter()'s comment on the flip-flop)
 * plus a single Miscellaneous Output register, write-only at 0x3C2.
 *
 * The register value tables below are the standard, widely-published
 * VGA BIOS mode 3 (80x25 text) and mode 13h (320x200x256) register
 * dumps - the same tables that appear throughout VGA hardware
 * references and OS-dev tutorials, since they describe how the actual
 * VGA standard defines these modes, not a NovaOS-specific choice.
 */
#include "vga_graphics.h"
#include "../../arch/x86/io.h"

#define VGA_MISC_WRITE 0x3C2

#define VGA_SEQ_INDEX 0x3C4
#define VGA_SEQ_DATA  0x3C5

#define VGA_CRTC_INDEX 0x3D4
#define VGA_CRTC_DATA  0x3D5

#define VGA_GC_INDEX 0x3CE
#define VGA_GC_DATA  0x3CF

#define VGA_AC_INDEX_DATA 0x3C0
#define VGA_INPUT_STATUS1 0x3DA

#define VGA_FRAMEBUFFER ((volatile uint8_t*)0xA0000)

typedef struct {
    uint8_t misc;
    uint8_t seq[5];
    uint8_t crtc[25];
    uint8_t gc[9];
    uint8_t ac[21];
} vga_regs_t;

static const vga_regs_t MODE_13H = {
    .misc = 0x63,
    .seq  = {0x03, 0x01, 0x0F, 0x00, 0x0E},
    .crtc = {0x5F, 0x4F, 0x50, 0x82, 0x54, 0x80, 0xBF, 0x1F,
             0x00, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x9C, 0x0E, 0x8F, 0x28, 0x40, 0x96, 0xB9, 0xA3,
             0xFF},
    .gc   = {0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x05, 0x0F, 0xFF},
    .ac   = {0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
             0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
             0x41, 0x00, 0x0F, 0x00, 0x00},
};

static const vga_regs_t MODE_TEXT_80X25 = {
    .misc = 0x67,
    .seq  = {0x03, 0x00, 0x03, 0x00, 0x02},
    .crtc = {0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F,
             0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00, 0x50,
             0x9C, 0x0E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3,
             0xFF},
    .gc   = {0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF},
    .ac   = {0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07,
             0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
             0x0C, 0x00, 0x0F, 0x08, 0x00},
};

static void write_vga_regs(const vga_regs_t* regs) {
    outb(VGA_MISC_WRITE, regs->misc);

    for (uint8_t i = 0; i < 5; i++) {
        outb(VGA_SEQ_INDEX, i);
        outb(VGA_SEQ_DATA, regs->seq[i]);
    }

    /* CRTC registers 0-7 are write-protected unless bit 7 of index
     * 0x11 is cleared first - do that as its own read-modify-write
     * before the main loop, which will then set index 0x11 to its
     * final table value along with everything else. */
    outb(VGA_CRTC_INDEX, 0x11);
    outb(VGA_CRTC_DATA, (uint8_t)(inb(VGA_CRTC_DATA) & 0x7F));

    for (uint8_t i = 0; i < 25; i++) {
        outb(VGA_CRTC_INDEX, i);
        outb(VGA_CRTC_DATA, regs->crtc[i]);
    }

    for (uint8_t i = 0; i < 9; i++) {
        outb(VGA_GC_INDEX, i);
        outb(VGA_GC_DATA, regs->gc[i]);
    }

    /* The Attribute Controller shares one port for both index and
     * data, distinguished by an internal flip-flop: reading the input
     * status register resets it to "expect an index byte next", after
     * which alternating writes to 0x3C0 are treated as index, data,
     * index, data, ... A final index-only write (0x20, PAS bit set)
     * re-enables video output after programming is done. */
    (void)inb(VGA_INPUT_STATUS1);
    for (uint8_t i = 0; i < 21; i++) {
        outb(VGA_AC_INDEX_DATA, i);
        outb(VGA_AC_INDEX_DATA, regs->ac[i]);
    }
    outb(VGA_AC_INDEX_DATA, 0x20);
}

void vga_graphics_enter(void) {
    write_vga_regs(&MODE_13H);
}

void vga_graphics_exit(void) {
    write_vga_regs(&MODE_TEXT_80X25);
}

void vga_put_pixel(int x, int y, uint8_t color_index) {
    if (x < 0 || y < 0 || x >= VGA_GFX_WIDTH || y >= VGA_GFX_HEIGHT) {
        return;
    }
    VGA_FRAMEBUFFER[(uint32_t)y * VGA_GFX_WIDTH + (uint32_t)x] = color_index;
}

void vga_fill_rect(int x, int y, int w, int h, uint8_t color_index) {
    for (int row = y; row < y + h; row++) {
        for (int col = x; col < x + w; col++) {
            vga_put_pixel(col, row, color_index);
        }
    }
}

void vga_draw_rect(int x, int y, int w, int h, uint8_t color_index) {
    for (int col = x; col < x + w; col++) {
        vga_put_pixel(col, y, color_index);
        vga_put_pixel(col, y + h - 1, color_index);
    }
    for (int row = y; row < y + h; row++) {
        vga_put_pixel(x, row, color_index);
        vga_put_pixel(x + w - 1, row, color_index);
    }
}
