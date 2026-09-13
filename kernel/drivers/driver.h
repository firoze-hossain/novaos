#ifndef DRIVERS_DRIVER_H
#define DRIVERS_DRIVER_H

/*
 * driver.h - Phase 39: driver self-registration
 *
 * Gap this fills: before this phase, every driver's init function was
 * called directly, by name, from kernel/init/main.c - adding a new
 * driver meant editing that file's own boot sequence, not just adding
 * a new driver source file. This is the single biggest concrete
 * blocker to the kernel independence this project has been working
 * toward (see NovaOS-Release-Readiness-Kernel-and-Userland.md's own
 * "no driver registration model" entry): a kernel a different
 * userland/distro could genuinely build hardware support on top of
 * needs a driver contract that doesn't require editing the kernel's
 * own boot code for every new device.
 *
 * How it works: DRIVER_REGISTER places a small, constant driver_t
 * into a dedicated linker section (`.drivers`), one entry per driver,
 * entirely at compile/link time - no runtime registration call, no
 * C++-style static constructors (which need runtime support this
 * early in boot isn't guaranteed to have), nothing for a new driver's
 * source file to coordinate with any other file over. main.c calls
 * driver_init_all() to run every driver's init() in whichever phase
 * it asked for; a new driver's own .c file is the only thing that
 * ever needs to change to add hardware support - main.c's own boot
 * sequence doesn't.
 *
 * Phases exist, rather than one flat list run all at once, specifically
 * to avoid reordering anything relative to today's known-working boot
 * sequence: PS/2 keyboard/mouse currently initialize before the
 * filesystem/network self-tests kernel_late_init() interleaves with
 * driver setup, and the PCI-based drivers (UHCI, AC97) genuinely do
 * need pci_enumerate() to have already run. Moving everything to a
 * single point risked exactly the class of subtle, hard-to-diagnose
 * ordering bug this project has already spent real effort tracking
 * down once (see PROGRESS.md's Phase 38) - not a risk worth taking to
 * make the registration mechanism itself slightly simpler.
 *
 * Deliberately scoped for this phase to drivers with no interleaved,
 * order-sensitive self-test logic of their own between them (PS/2
 * keyboard/mouse, UHCI, AC97) - kernel/init/main.c's own comments
 * explain, driver by driver, why timer/VFS/net stay as explicit calls
 * for now rather than being migrated in this same pass.
 */

typedef enum {
    /* Registered before pci_enumerate() has run, and before the
     * filesystem/network self-tests kernel_late_init() interleaves
     * with driver setup - for drivers, like PS/2 keyboard/mouse, with
     * no dependency on either. */
    DRIVER_PHASE_EARLY = 0,

    /* Registered after pci_enumerate() has already run - for PCI-
     * based drivers (UHCI, AC97) that need PCI config space access
     * already set up to find and configure their own device. */
    DRIVER_PHASE_AFTER_PCI = 1,

    DRIVER_PHASE_COUNT
} driver_phase_t;

typedef struct {
    const char* name;
    void (*init)(void);
    driver_phase_t phase;
} driver_t;

/* Placed by each driver's own .c file - see kernel/drivers/keyboard/
 * keyboard.c for a worked example. `symbol` must be a valid, unique C
 * identifier (used as this driver_t's own variable name) - conventionally
 * the same as the driver's init function. */
#define DRIVER_REGISTER(driver_name, init_fn, driver_phase)               \
    static const driver_t __attribute__((used, section(".drivers")))      \
        __driver_##init_fn = {                                            \
            .name = driver_name,                                         \
            .init = init_fn,                                             \
            .phase = driver_phase,                                       \
        }

/* Runs init() for every driver registered for `phase`, in whatever
 * order the linker happened to place them (undefined, and not
 * meaningful to rely on - every driver registered for the same phase
 * is expected to have no ordering dependency on any other driver in
 * that same phase, only on the phase itself having been reached).
 * Logs each driver's name as it starts, the same "show your work"
 * convention every other subsystem in this kernel's boot log already
 * follows. */
void driver_init_all(driver_phase_t phase);

#endif
