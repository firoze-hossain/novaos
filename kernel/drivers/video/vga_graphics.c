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
#include "vbe.h"
#include "../../arch/x86/io.h"

/* Phase 79: the standard, widely-documented 16-color EGA/CGA palette -
 * the exact RGB values every DOS-era/VGA-text-mode reference agrees
 * on (the "dim" IRGB combinations at 0xAA/170, "bright" at 0xFF/255,
 * with the one well-known historical exception - index 6, "brown" -
 * deliberately (170,85,0), not the "expected" (170,170,0), an IBM EGA
 * quirk every reference source repeats identically). Used to map the
 * existing `color_index` callers already pass (see vga_put_pixel()'s
 * own comment) into a real RGB color once there's a real color depth
 * (kernel/drivers/video/vbe.c) to render it in - every existing
 * caller this project has (grep-confirmed before writing this table)
 * only ever uses indices in exactly this 0-15 range, so this is a
 * complete, exact answer for them, not an approximation.
 *
 * Indices 16-255 are NOT the real VGA BIOS's own default 256-color
 * DAC table - that table's own exact values are genuinely disputed
 * even among dedicated tools built to reproduce it (its own "9 color
 * cycles" structure past the first 16 has no single, universally-
 * agreed 8-bit upconversion), and nothing in this codebase currently
 * uses any index past 15 to begin with. Rather than risk shipping a
 * subtly-wrong reproduction of contested historical hardware data,
 * indices 16-255 map to a simple, honestly-documented grayscale ramp
 * instead - a real, defined color for every possible uint8_t input,
 * just not a historical VGA DAC reproduction. */
static const uint8_t EGA_PALETTE[16][3] = {
    {0x00, 0x00, 0x00}, {0x00, 0x00, 0xAA}, {0x00, 0xAA, 0x00}, {0x00, 0xAA, 0xAA},
    {0xAA, 0x00, 0x00}, {0xAA, 0x00, 0xAA}, {0xAA, 0x55, 0x00}, {0xAA, 0xAA, 0xAA},
    {0x55, 0x55, 0x55}, {0x55, 0x55, 0xFF}, {0x55, 0xFF, 0x55}, {0x55, 0xFF, 0xFF},
    {0xFF, 0x55, 0x55}, {0xFF, 0x55, 0xFF}, {0xFF, 0xFF, 0x55}, {0xFF, 0xFF, 0xFF},
};

static void color_index_to_rgb(uint8_t color_index, uint8_t* r, uint8_t* g,
                                uint8_t* b) {
    if (color_index < 16) {
        *r = EGA_PALETTE[color_index][0];
        *g = EGA_PALETTE[color_index][1];
        *b = EGA_PALETTE[color_index][2];
        return;
    }
    /* See this table's own comment above - a defined, honest
     * grayscale fallback, not a guess at disputed hardware data
     * nothing here actually needs. */
    *r = *g = *b = color_index;
}

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

void vga_graphics_force_text_mode(void) {
    write_vga_regs(&MODE_TEXT_80X25);
}

void vga_graphics_enter(void) {
    if (vbe_available()) {
        vbe_enter_graphics();
        return;
    }
    save_font_plane();
    write_vga_regs(&MODE_13H);
}

void vga_graphics_exit(void) {
    if (vbe_available()) {
        vbe_exit_graphics();
        return;
    }
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
    if (vbe_available()) {
        /* Phase 79: real resolution/color depth - (x, y) still means
         * exactly what it always did (the old 320x200 logical canvas
         * every existing caller already assumes; see this project's
         * own roadmap - porting the compositor/WM onto the new
         * framebuffer's own, larger real resolution is deliberately a
         * separate, later task, not this one), `color_index` now maps
         * through the real palette above to a real RGB color instead
         * of a raw VGA memory byte. */
        uint8_t r, g, b;
        color_index_to_rgb(color_index, &r, &g, &b);
        vbe_put_pixel(x, y, r, g, b);
        return;
    }
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

/* Phase 81: font save/restore for the VBE path.
 *
 * The same bug the Mode 13h comment above documents, found a second
 * time on a different path - and this time it had been shipping,
 * unnoticed, since the VBE driver first appeared. The VGA text font
 * lives in video RAM (plane 2: the first 32KB, which a linear
 * framebuffer sees as its first 8 rows), and on the Bochs/QEMU "std"
 * VGA device TWO separate things destroy it:
 *
 *   1. Writing pixels to the linear framebuffer - the same RAM.
 *   2. Enabling the VBE mode at all. The Bochs interface clears video
 *      memory on every disabled->enabled transition unless the
 *      NOCLEARMEM flag is passed. So the font is gone the instant
 *      kernel/drivers/video/vbe.c first programs the mode, before any
 *      pixel is drawn.
 *
 * The original VBE driver's comment argued no save/restore was needed
 * because "callers never touch VGA's planar memory at all" - reasoning
 * about which API gets called, not about what the hardware is. The
 * result: from the first boot of that driver onward the text console
 * rendered nothing (blank glyphs; only the cursor, which the CRTC draws
 * itself, survived), while every test passed because they read the
 * serial log. It was found only by taking a screenshot after the first
 * real graphics session, and then fixed wrongly twice before being
 * understood: first by saving the font AFTER the mode was first
 * enabled (so the "saved" font was already zeros, and restoring it
 * restored nothing), and verified by a self-test that compared the
 * font to that same wrecked copy.
 *
 * The rule that makes it right: save while the BIOS's font is still
 * intact - before the first time the VBE mode is enabled (vbe_init()
 * does this, as its first act) - and restore after every return to
 * text mode. vga_graphics_saved_font_is_plausible() exists so a test
 * can notice a blank saved copy, which is how this should have been
 * caught. save_font_plane() leaves SEQ4/GC5/GC6 in their linear-planar
 * configuration, so text mode's register set is reapplied after it,
 * exactly as vga_graphics_exit() does around restore_font_plane(). */
void vga_graphics_save_text_font(void) {
    save_font_plane();
    write_vga_regs(&MODE_TEXT_80X25);
}

void vga_graphics_restore_text_font(void) {
    write_vga_regs(&MODE_TEXT_80X25);
    restore_font_plane();
    write_vga_regs(&MODE_TEXT_80X25);
}

bool vga_graphics_text_font_intact(void) {
    unchain_for_plane_access();
    outb(VGA_GC_INDEX, 0x04);
    outb(VGA_GC_DATA, 0x02); /* Read Map Select = plane 2 */
    bool same = true;
    for (int i = 0; i < VGA_FONT_SAVE_SIZE; i++) {
        if (saved_font_plane2[i] != VGA_FRAMEBUFFER[i]) {
            same = false;
            break;
        }
    }
    write_vga_regs(&MODE_TEXT_80X25);
    return same;
}

bool vga_graphics_saved_font_is_plausible(void) {
    /* A real VGA font is dense: well over a thousand non-zero bytes
     * across its 256 glyphs. An all-zero (or nearly so) copy is not a
     * font - it means the save happened after something already wiped
     * it. */
    int nonzero = 0;
    for (int i = 0; i < VGA_FONT_SAVE_SIZE; i++) {
        if (saved_font_plane2[i] != 0) {
            nonzero++;
        }
    }
    return nonzero > 1000;
}
