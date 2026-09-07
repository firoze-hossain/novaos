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

/* Real bug found through interactive testing (screendumps taken
 * specifically *after* returning to text mode, which nothing
 * exercised this rigorously before): Mode 13h's Chain-4 addressing
 * means a linear framebuffer write at 0xA0000 touches all 4 planes
 * at once, including Plane 2 - where the text-mode character
 * generator (font bitmap data) lives. Any drawing in graphics mode
 * silently destroys the font table text mode needs afterward,
 * producing garbled, stripe-like glyphs on return - functionally
 * harmless (the shell keeps working correctly underneath, confirmed
 * via serial log) but visually broken. Fixed by saving Plane 2's
 * contents before entering graphics mode and restoring them after
 * leaving, using the standard VGA "unchain" technique (temporarily
 * disabling Chain-4/Odd-Even addressing so Read Map Select / Map
 * Mask can access one plane at a time, linearly) rather than trying
 * to reload the actual font bitmap from scratch, which would need
 * embedding and trusting a hand-transcribed 4KB reference table -
 * this way, whatever the BIOS/VGA BIOS already loaded at boot is
 * preserved byte-for-byte, regardless of its exact contents. 8192
 * bytes (256 chars * 32-byte slots) is the standard, safe upper
 * bound for a VGA character generator, even though an 8x16 font only
 * uses the first 16 bytes of each slot. */
#define VGA_FONT_SAVE_SIZE 8192
static uint8_t saved_font_plane2[VGA_FONT_SAVE_SIZE];

static void unchain_for_plane_access(void) {
    outb(VGA_SEQ_INDEX, 0x04);
    outb(VGA_SEQ_DATA, 0x06); /* ext mem=1, odd/even=1(disabled),
                                  chain-4=0(disabled) - linear planar
                                  addressing */
    outb(VGA_GC_INDEX, 0x05);
    outb(VGA_GC_DATA, 0x00); /* read mode 0, host odd/even disabled */
    outb(VGA_GC_INDEX, 0x06);
    outb(VGA_GC_DATA, 0x05); /* same value MODE_13H's own gc[6] uses -
                                  graphics-style addressing covering
                                  0xA0000, already proven to work for
                                  this exact framebuffer window */
}

static void save_font_plane(void) {
    unchain_for_plane_access();
    outb(VGA_GC_INDEX, 0x04);
    outb(VGA_GC_DATA, 0x02); /* Read Map Select = plane 2 */
    for (int i = 0; i < VGA_FONT_SAVE_SIZE; i++) {
        saved_font_plane2[i] = VGA_FRAMEBUFFER[i];
    }
}

static void restore_font_plane(void) {
    unchain_for_plane_access();
    outb(VGA_SEQ_INDEX, 0x02);
    outb(VGA_SEQ_DATA, 0x04); /* Map Mask = plane 2 only */
    for (int i = 0; i < VGA_FONT_SAVE_SIZE; i++) {
        VGA_FRAMEBUFFER[i] = saved_font_plane2[i];
    }
}

static void write_vga_regs(const vga_regs_t* regs) {
    /* Put the sequencer into synchronous reset (SEQ index 0, bit 1)
     * before touching the Miscellaneous Output register or the Clock
     * Mode register (SEQ index 1) - both affect the dot clock the
     * sequencer is actively using. Reprogramming the clock while the
     * sequencer keeps running from the old one leaves its internal
     * counters out of sync with the new timing, which is exactly the
     * kind of thing that produces stretched/striped, unreadable text
     * on the *next* mode entered - a real bug found this way: this
     * kernel's mode-13h screen rendered correctly on entry (compared
     * against MODE_TEXT_80X25's screendump on *exit*, which is what
     * actually exercises this path, since MODE_13H's own seq[1] and
     * MODE_TEXT_80X25's differ - 0x01 vs 0x00, precisely the clock-
     * mode bit this reset protects). This is the standard VGA
     * programming sequence documented across VGA hardware references
     * (assert reset, reprogram, release reset) - not previously
     * followed here. */
    outb(VGA_SEQ_INDEX, 0x00);
    outb(VGA_SEQ_DATA, 0x01); /* synchronous reset asserted */

    outb(VGA_MISC_WRITE, regs->misc);

    for (uint8_t i = 1; i < 5; i++) {
        outb(VGA_SEQ_INDEX, i);
        outb(VGA_SEQ_DATA, regs->seq[i]);
    }

    outb(VGA_SEQ_INDEX, 0x00);
    outb(VGA_SEQ_DATA, regs->seq[0]); /* release reset - both mode
                                          tables' seq[0] is 0x03,
                                          normal operation */

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
    save_font_plane();
    write_vga_regs(&MODE_13H);
}

void vga_graphics_exit(void) {
    write_vga_regs(&MODE_TEXT_80X25);
    restore_font_plane();
    /* restore_font_plane()'s unchain step leaves SEQ4/GC5/GC6 in
     * their linear-planar-access configuration, not text mode's -
     * reapply text mode's full register set so the final state is
     * actually correct, not just "was correct until the plane
     * restore overwrote three of its registers." */
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
