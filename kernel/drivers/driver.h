#ifndef DRIVERS_DRIVER_H
#define DRIVERS_DRIVER_H

/*
 * driver.h - Phase 39 (extended Phase 76): driver self-registration
 *
 * Gap this fills: before this started, every driver's init function
 * was called directly, by name, from kernel/init/main.c - adding a
 * new driver meant editing that file's own boot sequence, not just
 * adding a new driver source file. This was the single biggest
 * concrete blocker to the kernel independence this project has been
 * working toward (see NovaOS-Release-Readiness-Kernel-and-Userland.md's
 * own "no driver registration model" entry): a kernel a different
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
 * sequence. Phase 39 migrated the drivers with no interleaved, order-
 * sensitive self-test logic of their own between them (PS/2 keyboard/
 * mouse, UHCI, AC97); Phase 76 finished the migration this file's own
 * comment used to describe as deliberately deferred - the timer, VFS,
 * and network "drivers" (vfs_init()/net_init() are each really a small
 * orchestrator picking and initializing among several real, lower-level
 * drivers of their own, not a single device driver in the PS/2/UHCI
 * sense, but the same self-registration contract applies to them
 * identically) - each given its OWN phase, positioned to reproduce
 * today's exact sequence rather than merged into DRIVER_PHASE_EARLY or
 * DRIVER_PHASE_AFTER_PCI: verified, not merely assumed, that none of
 * timer/PS2-keyboard/PS2-mouse has any real code-level dependency on
 * the others (no shared state, no timing/delay coupling - see
 * PROGRESS.md's own Phase 76 entry for exactly what was checked), which
 * would have made DRIVER_PHASE_TIMER unnecessary as a phase distinct
 * from DRIVER_PHASE_EARLY - kept separate anyway, since a phase costs
 * nothing extra and this file's own standing philosophy is to avoid
 * reordering risk even where a dependency search comes up empty, not
 * just where one is found. Moving everything to a single point risked
 * exactly the class of subtle, hard-to-diagnose ordering bug this
 * project has already spent real effort tracking down once (see
 * PROGRESS.md's Phase 38) - not a risk worth taking to make the
 * registration mechanism itself slightly simpler.
 */

typedef enum {
    /* The PIT timer (IRQ0) - registers its tick handler and, in
     * kernel/init/main.c, is immediately followed by timer_set_tick_
     * hook(scheduler_on_tick) once this phase returns (scheduler
     * wiring, deliberately kept as an explicit main.c call rather than
     * folded into the timer driver itself - see that driver's own
     * comment on why). Runs first, exactly where it did before Phase
     * 76 migrated it - see this file's own top comment on why it gets
     * its own phase rather than joining DRIVER_PHASE_EARLY below. */
    DRIVER_PHASE_TIMER = 0,

    /* Registered before pci_enumerate() has run, and before the
     * filesystem/network self-tests kernel_late_init() interleaves
     * with driver setup - for drivers, like PS/2 keyboard/mouse, with
     * no dependency on either. */
    DRIVER_PHASE_EARLY,

    /* The VFS: mounts whatever's on the primary ATA disk (FAT32/ext2,
     * partition table, journal, crash-dump region - see kernel/fs/
     * vfs.c's own vfs_init()). Runs after DRIVER_PHASE_EARLY, exactly
     * where it did before Phase 76 - it has no real dependency on
     * PS/2 keyboard/mouse either, but see this file's own top comment
     * on why that alone isn't reason enough to merge phases. */
    DRIVER_PHASE_FILESYSTEM,

    /* Networking: picks and initializes whichever NIC is actually
     * present (virtio-net preferred, then RTL8139, then NE2000 - see
     * kernel/net/net.c's own net_init()). Runs after DRIVER_PHASE_
     * FILESYSTEM, exactly where it did before Phase 76. Notably NOT
     * DRIVER_PHASE_AFTER_PCI, even though it reads PCI configuration
     * space to find RTL8139/virtio-net: PCI config space is plain I/O
     * port reads (0xCF8/0xCFC), available from the moment the kernel
     * is running, never dependent on pci_enumerate() itself having
     * run first (kernel/drivers/virtio/virtio_net.c's own comment
     * - written when this driver was deliberately NOT self-registered,
     * for exactly this reason - explains this in full; Phase 76 acts
     * on that same reasoning to migrate net_init() itself instead). */
    DRIVER_PHASE_NETWORK,

    /* Registered after pci_enumerate() has already run - for PCI-
     * based drivers (UHCI, AC97, virtio-blk) that need PCI config
     * space access already set up to find and configure their own
     * device. */
    DRIVER_PHASE_AFTER_PCI,

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
