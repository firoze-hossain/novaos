#ifndef DRIVERS_VIDEO_VBE_H
#define DRIVERS_VIDEO_VBE_H

#include "../../include/types.h"
#include "../../arch/x86/boot/multiboot.h"

/* Phase 79: a real VESA/VBE linear framebuffer - real resolution
 * (whatever GRUB actually negotiated, requested at 1024x768 - see
 * kernel/arch/x86/boot/multiboot.asm's own header), real color depth
 * (packs real RGB values according to the ACTUAL negotiated bit-field
 * layout GRUB reports, not a hardcoded assumption about which of
 * RGB565/RGB888/XRGB8888 the hardware happened to choose).
 *
 * Reachable the same way vga_graphics.c's old Mode 13h framebuffer
 * always was - SYS_GFX_PUT_PIXEL and friends (kernel/arch/x86/cpu/
 * syscall.c) still call vga_put_pixel()/vga_fill_rect()/vga_draw_
 * rect(), which now delegate here when a real framebuffer is
 * available (see vga_graphics.c's own comment). Nothing calls this
 * module's own functions directly except vga_graphics.c and this
 * kernel's own boot-time self-test.
 *
 * vbe_init() does three real things, not just one: reads and
 * validates the framebuffer fields Multiboot's own info structure
 * provides (see multiboot.h), maps the framebuffer's own physical
 * pages into this kernel's address space (identity-mapped, matching
 * every other physical address this kernel already treats this way -
 * see kernel/arch/x86/mm/paging.c's own top comment), and - this
 * part matters just as much as the other two - immediately switches
 * the hardware BACK to ordinary VGA text mode, so the existing 80x25
 * text console (every kernel_log()/shell command's own visible
 * output) keeps working exactly as it always did. GRUB leaves the
 * negotiated graphics mode ACTIVE when it hands off control - a real,
 * previously-documented concern (see this file's own git history:
 * vga_graphics.h's own comment, written before this phase, explicitly
 * worried that Multiboot video-mode negotiation "would replace the
 * VGA text-mode console the existing shell depends on") this phase
 * had to actually solve, not just note. vbe_enter_graphics()/vbe_
 * exit_graphics() (called from vga_graphics_enter()/_exit(), which
 * every existing `gui`-command/compositor call site already uses
 * unchanged) toggle the display between the two without needing to
 * re-negotiate the mode each time - the Bochs VBE "DISPI" interface's
 * own enable/disable register (see vbe.c's own comment) is designed
 * for exactly this. */

bool vbe_init(const multiboot_info_t* mbi);

bool vbe_available(void);

uint32_t vbe_get_width(void);
uint32_t vbe_get_height(void);
uint32_t vbe_get_bpp(void);

/* Switches the display from text mode to showing the real
 * framebuffer (already mapped and ready since vbe_init()) - no mode
 * renegotiation, just the Bochs DISPI enable toggle. Only meaningful
 * if vbe_available(); vga_graphics_enter() is the real caller. */
void vbe_enter_graphics(void);

/* The reverse: disables the framebuffer display and restores VGA
 * text mode (vga_graphics_exit() is the real caller) - the console
 * becomes visible again exactly as before `gui` was entered. */
void vbe_exit_graphics(void);

/* Writes one pixel at (x, y), packing r/g/b into the REAL negotiated
 * bit layout (see vbe_init()'s own field-position/mask-size reading) -
 * scaling each 8-bit input channel down to however many bits that
 * channel's real field actually has (5 for RGB565's R/B, 6 for its G,
 * 8 for a real 32bpp XRGB8888 mode, etc.), not assuming any one
 * specific depth. Silently does nothing for an out-of-bounds (x, y)
 * or if vbe_available() is false - matching vga_put_pixel()'s own
 * existing, identical bounds-check convention. */
void vbe_put_pixel(int x, int y, uint8_t r, uint8_t g, uint8_t b);

void vbe_fill_rect(int x, int y, int w, int h, uint8_t r, uint8_t g, uint8_t b);

/* Reads back the real packed pixel value at (x, y) - not decoded back
 * into r/g/b, the raw bytes actually sitting in the framebuffer at
 * that pixel, however many bytes this mode's own bpp occupies. Exists
 * for this kernel's own boot-time self-test (kernel/init/main.c) to
 * verify vbe_put_pixel() actually wrote what it claims to, through
 * the real, mapped memory - not available to any other caller, and
 * not part of vga_graphics.c's own delegation (reading a pixel back
 * was never part of the OLD Mode 13h API either). Returns 0 (and
 * leaves *out_raw untouched, i.e. always reads as 0) if vbe_
 * available() is false or (x, y) is out of bounds. */
bool vbe_read_pixel_raw(int x, int y, uint32_t* out_raw);

#endif
