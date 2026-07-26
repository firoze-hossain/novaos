#ifndef DRIVERS_VIDEO_VGA_GRAPHICS_H
#define DRIVERS_VIDEO_VGA_GRAPHICS_H

#include "../../include/types.h"

#define VGA_GFX_WIDTH  320
#define VGA_GFX_HEIGHT 200

/* VGA "Mode 13h" (320x200, 256 colors) via direct register
 * programming - no BIOS call, so no dependence on real/virtual-8086
 * mode being available (this kernel runs in plain 32-bit protected
 * mode with no VM86 monitor). This is deliberately NOT done through
 * Multiboot/GRUB's VBE framebuffer negotiation: that would replace
 * the VGA text-mode console the existing shell depends on, and doing
 * that safely means porting the whole text console (every existing
 * command's output) to a framebuffer-rendered font, which is a much
 * bigger and riskier change than this phase needs. Mode 13h can be
 * entered and exited at runtime with pure port I/O, so the existing
 * shell is completely unaffected outside of the (opt-in, `gui`
 * command-triggered) time NovaOS is actually in graphics mode. */
void vga_graphics_enter(void);

/* Restores standard 80x25 text mode (mode 3). Callers should also
 * call vga_clear() afterward (see kernel/drivers/vga) - screen
 * contents left over from graphics mode aren't valid text-mode
 * character data. */
void vga_graphics_exit(void);

void vga_put_pixel(int x, int y, uint8_t color_index);
void vga_fill_rect(int x, int y, int w, int h, uint8_t color_index);

/* Draws a 1px rectangle outline (border only, doesn't fill). */
void vga_draw_rect(int x, int y, int w, int h, uint8_t color_index);

#endif
