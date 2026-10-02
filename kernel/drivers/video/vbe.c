/*
 * vbe.c - a real VESA/VBE linear framebuffer - see vbe.h's own
 * top comment for the full design.
 *
 * The Bochs VBE "DISPI" interface (used here only to toggle between
 * showing VGA text and showing the framebuffer - GRUB already did
 * the real mode-negotiation before this kernel ever ran, via its own
 * real-mode BIOS VBE calls this 32-bit protected-mode kernel could
 * never make directly): two 16-bit I/O ports, an index/data pair,
 * the same shape every other indexed VGA register group in this
 * kernel's own vga_graphics.c already uses. Port numbers and
 * register indices verified against multiple independent sources
 * (OSDev Wiki, QEMU's own standard-vga documentation, and QEMU's own
 * bochs-vbe.h source) while writing this, not assumed from memory -
 * getting a port/index wrong here wouldn't just fail visibly, it
 * could just as easily silently write to the wrong register and
 * leave graphics mode looking fine while something unrelated quietly
 * misbehaves.
 */
#include "vbe.h"
#include "vga_graphics.h"
#include "../../arch/x86/io.h"
#include "../../arch/x86/mm/paging.h"
#include "../../include/kernel.h"

#define VBE_DISPI_IOPORT_INDEX 0x01CE
#define VBE_DISPI_IOPORT_DATA  0x01CF

#define VBE_DISPI_INDEX_ENABLE 0x4

#define VBE_DISPI_DISABLED 0x00
#define VBE_DISPI_ENABLED  0x01

static bool g_available = false;
static uint32_t g_phys_addr;
static uint32_t g_pitch;
static uint32_t g_width;
static uint32_t g_height;
static uint32_t g_bpp;
static uint8_t g_red_pos, g_red_size;
static uint8_t g_green_pos, g_green_size;
static uint8_t g_blue_pos, g_blue_size;

static void dispi_write(uint16_t index, uint16_t data) {
    outw(VBE_DISPI_IOPORT_INDEX, index);
    outw(VBE_DISPI_IOPORT_DATA, data);
}

/* Rounds up to whole bytes - the one depth this kernel supports where
 * bpp isn't already byte-aligned is 15 (RGB555), which still occupies
 * 2 real bytes per pixel in memory (1 bit padding), the same as 16bpp
 * - plain `bpp / 8` would silently truncate that to 1 and corrupt
 * every pixel write at 15bpp specifically. */
static uint32_t bytes_per_pixel(void) {
    return (g_bpp + 7) / 8;
}

bool vbe_init(const multiboot_info_t* mbi) {
    if (!(mbi->flags & MULTIBOOT_INFO_FRAMEBUFFER_INFO)) {
        kernel_log("[ OK ] VBE: no framebuffer info from GRUB - "
                   "falling back to VGA Mode 13h\n");
        return false;
    }
    if (mbi->framebuffer_type != MULTIBOOT_FRAMEBUFFER_TYPE_RGB) {
        /* INDEXED (palette) or EGA_TEXT - neither is what this driver
         * knows how to pack real colors into (see vbe_put_pixel()'s
         * own field-position-based packing, which only makes sense
         * for direct RGB) - the same honest fallback as no
         * framebuffer at all, not an attempt to half-support them. */
        kernel_log("[ OK ] VBE: framebuffer type %d is not direct RGB "
                   "- falling back to VGA Mode 13h\n",
                   (int)mbi->framebuffer_type);
        return false;
    }
    if (mbi->framebuffer_width == 0 || mbi->framebuffer_height == 0 ||
        mbi->framebuffer_bpp < 15 || mbi->framebuffer_bpp > 32) {
        kernel_log("[ OK ] VBE: framebuffer dimensions/depth look "
                   "invalid (%dx%d, %d bpp) - falling back to VGA "
                   "Mode 13h\n",
                   (int)mbi->framebuffer_width, (int)mbi->framebuffer_height,
                   (int)mbi->framebuffer_bpp);
        return false;
    }

    g_phys_addr = (uint32_t)mbi->framebuffer_addr; /* low 32 bits - see
        multiboot.h's own comment on this field's real 64-bit width */
    g_pitch = mbi->framebuffer_pitch;
    g_width = mbi->framebuffer_width;
    g_height = mbi->framebuffer_height;
    g_bpp = mbi->framebuffer_bpp;
    g_red_pos = mbi->framebuffer_red_field_position;
    g_red_size = mbi->framebuffer_red_mask_size;
    g_green_pos = mbi->framebuffer_green_field_position;
    g_green_size = mbi->framebuffer_green_mask_size;
    g_blue_pos = mbi->framebuffer_blue_field_position;
    g_blue_size = mbi->framebuffer_blue_mask_size;

    /* Identity-mapped, matching every other physical address this
     * kernel already treats this way (see kernel/arch/x86/mm/
     * paging.c's own top comment) - simpler and more consistent than
     * picking a separate virtual region, and this kernel's own 32-bit
     * addressing has no higher-half split to need one for. The
     * framebuffer's real physical address (confirmed directly on
     * this project's own QEMU `-vga std` target: 0xFD000000) sits far
     * above the 64MB this kernel's boot-time identity map already
     * covers, so these pages are mapped fresh here, into the KERNEL's
     * own page directory specifically (paging_kernel_directory_phys()
     * - not a per-process one) - added before any process exists
     * (kernel_main() calls vbe_init() immediately after paging_init(),
     * well before process_init()), so every process created from this
     * point on inherits it automatically, the same "add it to the
     * template before anyone clones the template" reasoning kernel/
     * task/process.c's own free_user_address_space() comment already
     * documents for how a process's own page directory ends up
     * sharing kernel entries in the first place. PAGE_USER is
     * deliberately not set - every real write to this memory happens
     * from ring-0 code (a kernel task like the compositor, or a
     * syscall handler, regardless of which ring the calling process
     * itself runs at - see kernel/arch/x86/cpu/syscall.c's own SYS_
     * GFX_PUT_PIXEL handler), matching the old VGA_FRAMEBUFFER's own,
     * identical, never-ring-3-accessible treatment. */
    uint32_t total_bytes = g_pitch * g_height;
    uint32_t* kernel_pd = (uint32_t*)paging_kernel_directory_phys();
    for (uint32_t off = 0; off < total_bytes; off += 4096) {
        uint32_t page_addr = g_phys_addr + off;
        if (!paging_map_page(kernel_pd, page_addr, page_addr,
                              PAGE_PRESENT | PAGE_WRITE)) {
            kernel_log("[FAULT] VBE: failed to map framebuffer page at "
                       "0x%x - falling back to VGA Mode 13h\n",
                       (int)page_addr);
            return false;
        }
    }

    g_available = true;

    /* The one step that actually matters as much as the mapping
     * itself - see vbe.h's own top comment on why. GRUB leaves its
     * own negotiated graphics mode showing; without this, the
     * existing 80x25 text console - every kernel_log()/shell
     * command's own visible output - would be invisible from this
     * point on, even though nothing about how this kernel writes to
     * it actually changed. */
    vbe_exit_graphics();

    kernel_log("[ OK ] VBE: real linear framebuffer %dx%dx%d at phys "
               "0x%x (pitch %d) - switched to VGA text mode for the "
               "console, real graphics available on request\n",
               (int)g_width, (int)g_height, (int)g_bpp,
               (int)g_phys_addr, (int)g_pitch);
    return true;
}

bool vbe_available(void) {
    return g_available;
}

uint32_t vbe_get_width(void) { return g_width; }
uint32_t vbe_get_height(void) { return g_height; }
uint32_t vbe_get_bpp(void) { return g_bpp; }

void vbe_enter_graphics(void) {
    if (!g_available) {
        return;
    }
    dispi_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
}

void vbe_exit_graphics(void) {
    if (!g_available) {
        return;
    }
    dispi_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
    /* Disabling VBE alone reveals whatever ordinary VGA register
     * state already exists underneath - it does not, by itself,
     * guarantee that state is valid 80x25 text mode (GRUB's own
     * mode-set may have left the underlying VGA registers in some
     * GRUB/BIOS-specific state this kernel never chose). Force it
     * explicitly, every time - cheap (plain port I/O, no font-plane
     * save/restore - see vga_graphics_force_text_mode()'s own
     * comment on why it's the right function for exactly this), and
     * removes any doubt about what's actually on screen once this
     * returns. */
    vga_graphics_force_text_mode();
}

void vbe_put_pixel(int x, int y, uint8_t r, uint8_t g, uint8_t b) {
    if (!g_available || x < 0 || y < 0 ||
        (uint32_t)x >= g_width || (uint32_t)y >= g_height) {
        return;
    }
    uint32_t red_bits = g_red_size > 0 ? ((uint32_t)r >> (8 - g_red_size)) : 0;
    uint32_t green_bits = g_green_size > 0 ? ((uint32_t)g >> (8 - g_green_size)) : 0;
    uint32_t blue_bits = g_blue_size > 0 ? ((uint32_t)b >> (8 - g_blue_size)) : 0;
    uint32_t packed = (red_bits << g_red_pos) | (green_bits << g_green_pos) |
                       (blue_bits << g_blue_pos);

    uint32_t bpp_bytes = bytes_per_pixel();
    volatile uint8_t* pixel = (volatile uint8_t*)
        (g_phys_addr + (uint32_t)y * g_pitch + (uint32_t)x * bpp_bytes);
    for (uint32_t i = 0; i < bpp_bytes; i++) {
        pixel[i] = (uint8_t)(packed >> (i * 8));
    }
}

void vbe_fill_rect(int x, int y, int w, int h, uint8_t r, uint8_t g, uint8_t b) {
    for (int row = y; row < y + h; row++) {
        for (int col = x; col < x + w; col++) {
            vbe_put_pixel(col, row, r, g, b);
        }
    }
}

bool vbe_read_pixel_raw(int x, int y, uint32_t* out_raw) {
    if (!g_available || x < 0 || y < 0 ||
        (uint32_t)x >= g_width || (uint32_t)y >= g_height) {
        return false;
    }
    uint32_t bpp_bytes = bytes_per_pixel();
    volatile uint8_t* pixel = (volatile uint8_t*)
        (g_phys_addr + (uint32_t)y * g_pitch + (uint32_t)x * bpp_bytes);
    uint32_t raw = 0;
    for (uint32_t i = 0; i < bpp_bytes; i++) {
        raw |= ((uint32_t)pixel[i]) << (i * 8);
    }
    *out_raw = raw;
    return true;
}
