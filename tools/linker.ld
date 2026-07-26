ENTRY(_start)

SECTIONS
{
    . = 1M;
    kernel_start = .;

    .multiboot : {
        *(.multiboot)
    }

    .text : {
        *(.text)
    }

    .rodata : {
        *(.rodata)
    }

    .data : {
        *(.data)
    }

    .bss : {
        *(.bss)
    }

    . = ALIGN(4096);
    kernel_end = .;
}