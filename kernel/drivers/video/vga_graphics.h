#ifndef DRIVERS_VIDEO_VGA_GRAPHICS_H
#define DRIVERS_VIDEO_VGA_GRAPHICS_H

#include "../../include/types.h"

#define VGA_GFX_WIDTH  320
#define VGA_GFX_HEIGHT 200

/* VGA "Mode 13h" (320x200, 256 colors) via direct register
 * programming - no BIOS call, so no dependence on real/virtual-8086
 * mode being available (this kernel runs in plain 32-bit protected
 * mode with no VM86 monitor).
 *
 * Phase 79: this used to be the ONLY backing store these functions
 * ever had, deliberately NOT done through Multiboot/GRUB's VBE
 * framebuffer negotiation - this comment, before this phase, said
 * exactly why: "that would replace the VGA text-mode console the
 * existing shell depends on," and porting the whole text console to
 * a framebuffer-rendered font "is a much bigger and riskier change
 * than this phase needs." That concern was real, and is now actually
 * solved rather than just avoided: kernel/drivers/video/vbe.c's own
 * Bochs DISPI enable/disable toggle switches between showing real VGA
 * text mode and showing the real framebuffer without ever re-
 * negotiating the mode or touching the text console's own font/
 * character data at all - see that file's own top comment. Every
 * function below keeps its EXACT existing signature (callers -
 * kernel/arch/x86/cpu/syscall.c's SYS_GFX_* handlers, userland/gui/
 * compositor.c - need zero changes) and now delegates to vbe.c
 * whenever `vbe_available()`, falling back to this file's own
 * original Mode 13h register-programming path, completely unmodified,
 * whenever it isn't (a real machine whose BIOS can't provide the
 * requested mode, or any environment without GRUB's own VBE/
 * video_bochs modules available) - see vga_graphics.c's own comment
 * on exactly how `color_index` (still just a uint8_t, for zero caller
 * changes) becomes a real RGB color once there's a real color depth
 * to render it in. */
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

/* Phase 79: programs the standard 80x25 text-mode VGA registers only -
 * no font-plane save/restore (see vga_graphics_exit()'s own, separate
 * logic for that; this function has no mode-13h-induced font
 * corruption to undo, since kernel/drivers/video/vbe.c's own callers
 * never touch VGA's planar memory at all). The one piece of this
 * file's own logic genuinely needed outside it - vbe.c's own vbe_
 * exit_graphics() calls this directly, since disabling the Bochs
 * DISPI interface alone reveals whatever register state GRUB's own
 * mode-set left behind, not necessarily valid text mode, and this is
 * the cheap, certain way to guarantee it actually is. Not meant for
 * any other caller - every normal "go back to text mode" need should
 * still go through vga_graphics_exit() above. */
void vga_graphics_force_text_mode(void);

#endif
