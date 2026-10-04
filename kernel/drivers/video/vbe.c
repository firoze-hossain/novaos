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
#include "../pci/pci.h"
#include "../../arch/x86/io.h"
#include "../../arch/x86/mm/paging.h"
#include "../../include/kernel.h"

#define VBE_DISPI_IOPORT_INDEX 0x01CE
#define VBE_DISPI_IOPORT_DATA  0x01CF

#define VBE_DISPI_INDEX_ID     0x0
#define VBE_DISPI_INDEX_XRES   0x1
#define VBE_DISPI_INDEX_YRES   0x2
#define VBE_DISPI_INDEX_BPP    0x3
#define VBE_DISPI_INDEX_ENABLE 0x4

#define VBE_DISPI_DISABLED    0x00
#define VBE_DISPI_ENABLED     0x01
#define VBE_DISPI_LFB_ENABLED 0x40

/* The six real BGA (Bochs Graphics Adapter) interface versions that
 * have ever existed, 0xB0C0 through 0xB0C5 - reading anything outside
 * this exact range back from VBE_DISPI_INDEX_ID means there is no
 * real Bochs DISPI interface behind these I/O ports at all (most
 * likely: this isn't a Bochs/QEMU-compatible video device), not a
 * newer or older version this driver merely doesn't recognize yet.
 * Verified against OSDev Wiki's own account of the version history,
 * not assumed. */
#define VBE_DISPI_ID_MIN 0xB0C0
#define VBE_DISPI_ID_MAX 0xB0C5

/* QEMU/Bochs's own standard-VGA PCI identity, written in the
 * conventional "vendor:device" order real PCI ID databases and
 * QEMU's own docs both use - PCI ID 1234:1111 means vendor 0x1234,
 * device 0x1111 (confirmed directly against a real PCI scan of this
 * project's own QEMU `-vga std` target during this phase's own
 * testing: `vendor=0x1234 device=0x1111 class=0x3` - an initial
 * transposition of these two constants was caught exactly this way,
 * by checking real hardware output rather than trusting the
 * memorized order). BAR0 on this specific device is the
 * framebuffer's own real physical base address (verified against
 * QEMU's own official specs documentation, qemu.org/docs/master/
 * specs/standard-vga.html, and independently corroborated by OSDev
 * Wiki's own account and real, posted forum results) - never a
 * fixed, hardcoded address (OSDev Wiki's own explicit warning: "It
 * is highly inadvisable to make assumptions about the address of the
 * linear framebuffer. It should always be read from the BGA's PCI
 * BAR0," which is exactly what this driver does, never falling back
 * to the old, legacy-only 0xE0000000 some much older Bochs/QEMU
 * versions used). */
#define BOCHS_VGA_VENDOR_ID 0x1234
#define BOCHS_VGA_DEVICE_ID 0x1111

static bool g_available = false;
static uint32_t g_phys_addr;
static uint32_t g_pitch;
static uint32_t g_width;
static uint32_t g_height;
static uint32_t g_bpp;
static void save_text_font_once(void); /* defined with enter/exit below */
static uint8_t g_red_pos, g_red_size;
static uint8_t g_green_pos, g_green_size;
static uint8_t g_blue_pos, g_blue_size;

static void dispi_write(uint16_t index, uint16_t data) {
    outw(VBE_DISPI_IOPORT_INDEX, index);
    outw(VBE_DISPI_IOPORT_DATA, data);
}

static uint16_t dispi_read(uint16_t index) {
    outw(VBE_DISPI_IOPORT_INDEX, index);
    return inw(VBE_DISPI_IOPORT_DATA);
}

/* Shared by both ways this driver can end up with a real framebuffer
 * (GRUB already negotiated one, or this driver negotiates one itself
 * directly - see try_bochs_direct_probe() below): maps `total_bytes`
 * worth of physical pages starting at `phys_addr` into the kernel's
 * own page directory. See the original call site's own comment
 * (still accurate, just no longer the only caller) for the full
 * reasoning on why the kernel directory specifically, and why no
 * PAGE_USER. */
static bool map_framebuffer_pages(uint32_t phys_addr, uint32_t total_bytes) {
    uint32_t* kernel_pd = (uint32_t*)paging_kernel_directory_phys();
    for (uint32_t off = 0; off < total_bytes; off += 4096) {
        uint32_t page_addr = phys_addr + off;
        if (!paging_map_page(kernel_pd, page_addr, page_addr,
                              PAGE_PRESENT | PAGE_WRITE)) {
            kernel_log("[FAULT] VBE: failed to map framebuffer page at "
                       "0x%x\n", (int)page_addr);
            return false;
        }
    }
    return true;
}

/* Rounds up to whole bytes - the one depth this kernel supports where
 * bpp isn't already byte-aligned is 15 (RGB555), which still occupies
 * 2 real bytes per pixel in memory (1 bit padding), the same as 16bpp
 * - plain `bpp / 8` would silently truncate that to 1 and corrupt
 * every pixel write at 15bpp specifically. */
static uint32_t bytes_per_pixel(void) {
    return (g_bpp + 7) / 8;
}

/* The mode this driver asks for when negotiating directly - 1024x768
 * at 32bpp, the exact same preference this kernel's own Multiboot
 * header already expresses as a *request* to GRUB (see multiboot.asm's
 * own comment on why that request alone isn't enough in this
 * project's own real test environment). XRGB8888 (8 bits per channel,
 * red at bit 16, green at bit 8, blue at bit 0, matching every
 * standard VESA/VBE 32bpp mode's own well-documented layout - not
 * read from anywhere here, because at 32bpp on real VBE/Bochs
 * hardware it is always this layout, not something that varies by
 * device the way it would be dishonest to hardcode for an unknown
 * negotiated mode) is simple, real color depth, far beyond the old
 * 320x200x8 this whole driver exists to replace. */
#define BOCHS_DIRECT_WIDTH  1024
#define BOCHS_DIRECT_HEIGHT 768
#define BOCHS_DIRECT_BPP    32

typedef struct {
    bool found;
    uint8_t bus, device, function;
    uint32_t bar0;
} bochs_vga_location_t;

static bochs_vga_location_t g_bochs_location;

/* pci_enumerate()'s own callback shape - see kernel/drivers/virtio/
 * virtio_blk.c's own find_virtio_blk() for the identical, already-
 * established "static result struct, stop scanning once found"
 * pattern this mirrors exactly, rather than inventing a new one. */
static void find_bochs_vga(const pci_device_t* dev) {
    if (g_bochs_location.found) {
        return;
    }
    if (dev->vendor_id == BOCHS_VGA_VENDOR_ID &&
        dev->device_id == BOCHS_VGA_DEVICE_ID) {
        g_bochs_location.found = true;
        g_bochs_location.bus = dev->bus;
        g_bochs_location.device = dev->device;
        g_bochs_location.function = dev->function;
        g_bochs_location.bar0 =
            pci_config_read32(dev->bus, dev->device, dev->function, 0x10);
    }
}

/* Phase 79 (continued): GRUB's own Multiboot1 video-mode negotiation
 * turned out to fail outright in this project's own real test
 * environment (see multiboot.asm's own comment for the full,
 * extensively-investigated account) - this is the real fix, not a
 * workaround: this driver negotiates a real linear framebuffer
 * directly with the hardware itself, over the Bochs VBE "DISPI"
 * interface (the same one vbe_enter_graphics()/vbe_exit_graphics()
 * already use to toggle text/graphics - this extends that to actual
 * mode-SETTING), needing neither GRUB's cooperation nor any BIOS
 * real-mode call this 32-bit protected-mode kernel could never make
 * directly anyway. Two real, independent checks before touching
 * anything else - this is a genuine "is this hardware actually
 * present and exactly what I think it is" verification, not an
 * assumption that `-vga std` implies it: (1) the DISPI ID register
 * must read back a real BGA version (0xB0C0-0xB0C5) - confirms a real
 * Bochs DISPI interface exists behind these I/O ports at all; (2) a
 * real PCI scan (this kernel's own existing kernel/drivers/pci/pci.c,
 * already proven by virtio_blk.c/ac97.c/uhci.c) must find the exact
 * device (PCI ID 1234:1111) whose BAR0 is where the real spec says
 * the framebuffer's own physical address actually lives - never
 * assumed, and never the old, legacy-only 0xE0000000 some much older
 * Bochs/QEMU versions used instead (see this file's own macro
 * comments for the sources this was verified against). Returns false,
 * gracefully, the moment either check doesn't pan out - correct on
 * real, non-Bochs-compatible hardware too, not just in this project's
 * own QEMU environment. */
static bool try_bochs_direct_probe(void) {
    uint16_t id = dispi_read(VBE_DISPI_INDEX_ID);
    if (id < VBE_DISPI_ID_MIN || id > VBE_DISPI_ID_MAX) {
        kernel_log("[ OK ] VBE: no Bochs DISPI interface present "
                   "(id=0x%x) - falling back to VGA Mode 13h\n", (int)id);
        return false;
    }

    g_bochs_location.found = false;
    pci_enumerate(find_bochs_vga);
    if (!g_bochs_location.found) {
        kernel_log("[ OK ] VBE: Bochs DISPI id present but no matching "
                   "PCI device (1234:1111) found - falling back to VGA "
                   "Mode 13h\n");
        return false;
    }

    /* A 32-bit memory BAR's low 4 bits are type/prefetch flags, not
     * part of the address (standard PCI spec, the same masking every
     * PCI memory BAR anywhere needs) - bit 0 = 0 already confirms
     * "memory space, not I/O space" for a real framebuffer BAR, kept
     * as an explicit check rather than assumed, matching virtio_blk.c's
     * own equally explicit check of its own BAR's low bit (there,
     * confirming I/O space instead - the identical discipline, applied
     * to whichever space this particular BAR actually claims). */
    if (g_bochs_location.bar0 & 0x1) {
        kernel_log("[FAULT] VBE: PCI BAR0 claims I/O space, not memory - "
                   "not the framebuffer BAR this driver expects, falling "
                   "back to VGA Mode 13h\n");
        return false;
    }
    uint32_t phys_addr = g_bochs_location.bar0 & 0xFFFFFFF0;

    /* SAVE THE TEXT FONT BEFORE THE FIRST ENABLE - not after. The Bochs
     * VBE interface clears video memory every time the mode is enabled
     * unless the NOCLEARMEM flag (0x80) is given, and the VGA text font
     * lives in that memory (plane 2 = the framebuffer's first 8 rows).
     * An earlier version of this file saved the font only after this
     * function had enabled the mode and switched back to text - i.e.
     * after the device had already zeroed it - so it saved zeros, and
     * the font "restore" then faithfully restored zeros: the text
     * console went blank at the very first call and every self-test
     * said everything was fine, because each one compared the font to
     * that already-wrecked copy. The blank console was only found by
     * looking at the screen. */
    save_text_font_once();

    dispi_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
    dispi_write(VBE_DISPI_INDEX_XRES, BOCHS_DIRECT_WIDTH);
    dispi_write(VBE_DISPI_INDEX_YRES, BOCHS_DIRECT_HEIGHT);
    dispi_write(VBE_DISPI_INDEX_BPP, BOCHS_DIRECT_BPP);
    dispi_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED);

    g_phys_addr = phys_addr;
    g_width = BOCHS_DIRECT_WIDTH;
    g_height = BOCHS_DIRECT_HEIGHT;
    g_bpp = BOCHS_DIRECT_BPP;
    /* Bochs's own real LFB layout: a plain, unpadded linear stride -
     * verified against OSDev Wiki's own account of the interface
     * (confirmed, not assumed, as part of this same investigation) -
     * unlike real hardware in general, which kernel/arch/x86/boot/
     * multiboot.h's own framebuffer_pitch field comment already warns
     * can pad each row; this device specifically never does. */
    g_pitch = BOCHS_DIRECT_WIDTH * (BOCHS_DIRECT_BPP / 8);
    /* Standard XRGB8888 - see this function's own top comment on why
     * this is correct to hardcode specifically for a 32bpp mode this
     * driver itself requested, not read from anywhere. */
    g_red_pos = 16;   g_red_size = 8;
    g_green_pos = 8;  g_green_size = 8;
    g_blue_pos = 0;   g_blue_size = 8;

    if (!map_framebuffer_pages(g_phys_addr, g_pitch * g_height)) {
        dispi_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
        kernel_log("[FAULT] VBE: falling back to VGA Mode 13h\n");
        return false;
    }

    g_available = true;
    vbe_exit_graphics();
    save_text_font_once(); /* font still pristine: nothing has written
                              the framebuffer yet */
    /* %x, not %02x: kernel/lib/stdio.c's own minimal vsnprintf() only
     * recognises %s/%d/%x/%c, no width/padding specifiers - the exact
     * same class of bug this project already caught once before (see
     * PROGRESS.md's Phase 76 entry) and should have remembered to
     * avoid here the first time. */
    kernel_log("[ OK ] VBE: real linear framebuffer %dx%dx%d at phys "
               "0x%x (pitch %d), negotiated directly via Bochs DISPI "
               "(PCI bus %d dev %d fn %d) - no GRUB/BIOS cooperation "
               "needed - switched to VGA text mode for the console, "
               "real graphics available on request\n",
               (int)g_width, (int)g_height, (int)g_bpp, (int)g_phys_addr,
               (int)g_pitch, (int)g_bochs_location.bus,
               (int)g_bochs_location.device, (int)g_bochs_location.function);
    return true;
}

/* The original mechanism: use whatever real framebuffer GRUB already
 * negotiated and reported via multiboot_info_t, if any - the
 * lowest-risk path when it's actually available (no PCI scan, no
 * mode-setting this driver has to get right itself), but not
 * something this kernel's own header currently requests (see
 * multiboot.asm's own comment on why) - so in practice, today, this
 * always returns false here and vbe_init() falls through to try_
 * bochs_direct_probe() below. Kept exactly as it was, not removed:
 * genuinely still the right mechanism for an environment where GRUB's
 * own negotiation does work (a real machine, or a different GRUB
 * build), which this driver should keep preferring over negotiating
 * the mode itself the moment that header flag is re-enabled. */
static bool try_grub_framebuffer_info(const multiboot_info_t* mbi) {
    if (!(mbi->flags & MULTIBOOT_INFO_FRAMEBUFFER_INFO)) {
        kernel_log("[ OK ] VBE: no framebuffer info from GRUB\n");
        return false;
    }
    if (mbi->framebuffer_type != MULTIBOOT_FRAMEBUFFER_TYPE_RGB) {
        /* INDEXED (palette) or EGA_TEXT - neither is what this driver
         * knows how to pack real colors into (see vbe_put_pixel()'s
         * own field-position-based packing, which only makes sense
         * for direct RGB) - the same honest fallback as no
         * framebuffer at all, not an attempt to half-support them. */
        kernel_log("[ OK ] VBE: framebuffer type %d is not direct RGB "
                   "- trying direct Bochs DISPI negotiation next\n",
                   (int)mbi->framebuffer_type);
        return false;
    }
    if (mbi->framebuffer_width == 0 || mbi->framebuffer_height == 0 ||
        mbi->framebuffer_bpp < 15 || mbi->framebuffer_bpp > 32) {
        kernel_log("[ OK ] VBE: framebuffer dimensions/depth look "
                   "invalid (%dx%d, %d bpp) - trying direct Bochs DISPI "
                   "negotiation next\n",
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
    if (!map_framebuffer_pages(g_phys_addr, g_pitch * g_height)) {
        kernel_log("[FAULT] VBE: failed to use GRUB's own framebuffer "
                   "info - trying direct Bochs DISPI negotiation next\n");
        return false;
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
    save_text_font_once(); /* font still pristine: nothing has written
                              the framebuffer yet */

    kernel_log("[ OK ] VBE: real linear framebuffer %dx%dx%d at phys "
               "0x%x (pitch %d), negotiated by GRUB - switched to VGA "
               "text mode for the console, real graphics available on "
               "request\n",
               (int)g_width, (int)g_height, (int)g_bpp,
               (int)g_phys_addr, (int)g_pitch);
    return true;
}

/* The real orchestrator, and the only one of these three functions
 * kernel/init/main.c ever calls directly: prefers GRUB's own
 * negotiated framebuffer when one is genuinely available (see try_
 * grub_framebuffer_info()'s own comment on why that stays the
 * preferred path rather than being replaced outright), falls through
 * to negotiating directly with the hardware itself when it isn't (see
 * try_bochs_direct_probe()'s own comment for the full account of why
 * that's a real, necessary second path in this project's own actual
 * test environment, not merely a nice-to-have), and only falls all
 * the way back to the existing, completely unmodified VGA Mode 13h
 * path (kernel/drivers/video/vga_graphics.c) when neither one found
 * anything real to work with. */
bool vbe_init(const multiboot_info_t* mbi) {
    if (try_grub_framebuffer_info(mbi)) {
        return true;
    }
    if (try_bochs_direct_probe()) {
        return true;
    }
    kernel_log("[ OK ] VBE: no usable framebuffer found by either path "
               "- falling back to VGA Mode 13h\n");
    return false;
}

bool vbe_get_xrgb8888_surface(volatile uint8_t** out_base, uint32_t* out_width,
                              uint32_t* out_height, uint32_t* out_pitch) {
    if (!g_available || g_bpp != 32 ||
        g_red_pos != 16 || g_red_size != 8 ||
        g_green_pos != 8 || g_green_size != 8 ||
        g_blue_pos != 0 || g_blue_size != 8) {
        return false;
    }
    /* A pitch smaller than a row of pixels would make every row copy
     * overlap its neighbour - cannot happen with the layouts this
     * driver produces, but this is the one gate between a device-
     * reported number and kernel memory writes, so it is checked. */
    if (g_pitch < g_width * 4u) {
        return false;
    }
    *out_base = (volatile uint8_t*)g_phys_addr;
    *out_width = g_width;
    *out_height = g_height;
    *out_pitch = g_pitch;
    return true;
}

bool vbe_available(void) {
    return g_available;
}

uint32_t vbe_get_width(void) { return g_width; }
uint32_t vbe_get_height(void) { return g_height; }
uint32_t vbe_get_bpp(void) { return g_bpp; }

/* Saved once, at init, while the text font is still intact (nothing has
 * been able to write the framebuffer yet) - see vga_graphics.c's
 * vga_graphics_save_text_font() for why the font needs saving at all. */
static bool g_font_saved;

static void save_text_font_once(void) {
    if (!g_font_saved) {
        vga_graphics_save_text_font();
        g_font_saved = true;
    }
}

void vbe_enter_graphics(void) {
    if (!g_available) {
        return;
    }
    /* Normally already done by vbe_init(); repeated here (it is a no-op
     * the second time) so that no path can reach the first framebuffer
     * write with the font unsaved. */
    save_text_font_once();
    dispi_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
}

void vbe_exit_graphics(void) {
    if (!g_available) {
        return;
    }
    dispi_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
    /* Disabling VBE alone reveals whatever ordinary VGA register
     * state already exists underneath - it does not, by itself,
     * guarantee that state is valid 80x25 text mode, and it certainly
     * does not bring back the font: the linear framebuffer aliases the
     * VGA RAM the font lives in, so any pixel written has overwritten
     * it. Restore text mode AND the font explicitly, every time. (The
     * very first call, from vbe_init() before any graphics has
     * happened, has nothing saved yet and nothing to restore - the
     * font is still the BIOS's - so it only sets text mode.) */
    if (g_font_saved) {
        vga_graphics_restore_text_font();
    } else {
        vga_graphics_force_text_mode();
    }
}

bool vbe_font_selftest(void) {
    if (!g_available || !g_font_saved) {
        return false;
    }
    /* 1. The saved copy must BE a font. This is the check whose absence
     *    let a blank console ship: every other check here compares the
     *    live font to the saved one, which passes trivially if the saved
     *    one is garbage. */
    if (!vga_graphics_saved_font_is_plausible()) {
        kernel_log("[FAULT] VBE font self-test: the saved text font is blank - "
                   "it was saved after something had already wiped it\n");
        return false;
    }
    /* 2. A real session, the way callers run one: enter graphics (the
     *    device clears video memory), scribble over exactly the region
     *    the font aliases (the framebuffer's first 8 rows, 4096 bytes
     *    each), exit. The font must be byte-identical afterwards. */
    volatile uint32_t* lfb = (volatile uint32_t*)g_phys_addr;
    vbe_enter_graphics();
    for (uint32_t i = 0; i < 8192; i++) {
        lfb[i] = 0xDEADBEEFu;
    }
    /* Not vacuous: the scribble must really have destroyed it first,
     * or this would pass on a device where there is nothing to restore. */
    bool destroyed = !vga_graphics_text_font_intact();
    vbe_exit_graphics();
    bool restored = vga_graphics_text_font_intact();
    if (!destroyed) {
        kernel_log("[ .. ] VBE font self-test: scribbling the framebuffer did "
                   "not disturb the font on this device (nothing to restore)\n");
    }
    return restored;
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

/* A real, specific, hard-checkable proof that "real color depth"
 * actually means what it claims, not just that negotiation reported
 * success: writes several real, named colors (pure red/green/blue,
 * white, and a deliberately non-trivial mixed color unlikely to
 * survive a packing bug by accident) via the same vbe_put_pixel() any
 * real caller uses, reads each one back through the real, live
 * framebuffer memory via vbe_read_pixel_raw(), and checks the exact
 * expected packed bits for this field layout - not merely "got
 * something back." At this driver's own negotiated depth (32bpp, 8
 * bits per channel - see try_bochs_direct_probe()'s own comment),
 * every channel's own 8 input bits already exactly fit their own 8-
 * bit field with zero rounding, so an exact bitwise match is the
 * correct, achievable bar here - not an approximation tolerance a
 * narrower real depth (16bpp RGB565, say) would genuinely need
 * instead. Returns true only if every single check passes. */
bool vbe_selftest(void) {
    if (!g_available) {
        return false;
    }
    struct {
        uint8_t r, g, b;
        const char* name;
    } colors[] = {
        {255, 0, 0, "red"},
        {0, 255, 0, "green"},
        {0, 0, 255, "blue"},
        {255, 255, 255, "white"},
        {123, 45, 200, "mixed"},
    };
    int x = 10;
    int y = 10;
    bool all_ok = true;
    for (uint32_t i = 0; i < sizeof(colors) / sizeof(colors[0]); i++) {
        vbe_put_pixel(x + (int)i, y, colors[i].r, colors[i].g, colors[i].b);
        uint32_t raw;
        if (!vbe_read_pixel_raw(x + (int)i, y, &raw)) {
            kernel_log("[FAULT] VBE selftest: could not read back pixel "
                       "for '%s'\n", colors[i].name);
            all_ok = false;
            continue;
        }
        uint32_t expected = ((uint32_t)colors[i].r << g_red_pos) |
                             ((uint32_t)colors[i].g << g_green_pos) |
                             ((uint32_t)colors[i].b << g_blue_pos);
        if (raw != expected) {
            kernel_log("[FAULT] VBE selftest: '%s' wrote back 0x%x, "
                       "expected 0x%x\n", colors[i].name, (int)raw,
                       (int)expected);
            all_ok = false;
        }
    }
    return all_ok;
}
