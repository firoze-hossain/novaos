ENTRY(_start)

SECTIONS
{
    . = 1M;
    kernel_start = .;

    .multiboot : {
        *(.multiboot)
    }

    /* Every section below matches both the plain name (what every C
     * object file in this kernel produces) and a `name.*` wildcard
     * (what rustc produces instead, one section per function/static -
     * e.g. `.text.rust_pipe_create`, `.bss._ZN3lib4pipe5PIPES...`,
     * confirmed directly via `objdump -h` on this project's own
     * kernel/rust/*.o files, not assumed). Before this fix, only the
     * exact `.bss` name was matched - every Rust bss-like section
     * (each one of this kernel's few, small Rust statics) was an
     * "orphan" input section as far as this script was concerned,
     * left for the linker's own default orphan-placement heuristics
     * to position somewhere. That happened to still work by accident
     * for every Rust symbol before Phase 36 (small enough, or
     * fortunate enough in the ordering, to still land inside the
     * kernel_start/kernel_end range computed below) - until
     * kernel/rust/pipe.rs's ~8KB PIPES array (Phase 36), whose own
     * orphaned .bss section happened to be placed by the linker
     * starting at *exactly* the same address as kernel_end itself,
     * outside the range pmm_init() reserves as "this is the kernel's
     * own loaded image, not available memory" - meaning pmm_alloc_
     * frame() was free to (and eventually did) hand that exact
     * physical frame out to an ordinary process's page directory,
     * which then silently aliased the same physical memory as this
     * kernel's own live PIPES array. The very first byte written
     * through any pipe overwrote that process's page directory
     * contents with the message being written - confirmed directly,
     * not inferred: a real process's corrupted page-directory entries
     * were read back out of memory as the exact ASCII bytes of this
     * project's own pipe self-test message ("Hello through a NovaOS
     * pipe!"), and traced through a full three-fault cascade (page
     * fault while fetching the very next kernel instruction after
     * switching into that corrupted address space, escalating to a
     * double fault handling that, escalating again to a triple fault)
     * using QEMU's own `-d int` exception tracing. This had been
     * showing up as what looked like an unrelated, mysterious
     * scheduler/process_wait() timing bug throughout Phase 36's own
     * testing - it was never the scheduler at all. */
    .text : {
        *(.text) *(.text.*)
    }

    .rodata : {
        *(.rodata) *(.rodata.*)
    }

    /* Phase 39: driver self-registration (kernel/drivers/driver.h) -
     * each DRIVER_REGISTER call places one `driver_t` here, and
     * driver_init_all() walks every entry between __drivers_start and
     * __drivers_end. KEEP() is required, not just tidy: unlike a
     * normal section, nothing anywhere directly calls or references
     * any individual entry here by name - the *only* thing that
     * refers to this data at all is the start/end symbol pair below,
     * which a linker doing dead/unreferenced-section elimination
     * (`--gc-sections`) could otherwise conclude means none of these
     * entries are actually used, and discard them. This project's own
     * current build doesn't pass --gc-sections, so this isn't
     * currently live-fired - but KEEP() is what every other project
     * using this same self-registration pattern relies on for exactly
     * this reason, and is what protects this section if that ever
     * changes, intentionally or not. */
    .drivers : {
        __drivers_start = .;
        KEEP(*(.drivers))
        __drivers_end = .;
    }

    .data : {
        *(.data) *(.data.*)
    }

    .bss : {
        *(.bss) *(.bss.*)
    }

    . = ALIGN(4096);
    kernel_end = .;
}