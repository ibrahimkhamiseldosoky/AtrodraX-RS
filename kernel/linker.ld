OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

/*
 * The kernel is loaded at 256 MiB. Everything is identity mapped, so this is
 * simply a physical address inside RAM. Keeping the image (and its 64 MiB heap
 * arena in .bss) out of the low 256 MiB leaves room for statically linked Linux
 * binaries, which link at 0x400000 and must not share memory with the kernel.
 */
SECTIONS
{
    . = 0x10000000;
    __kernel_start = .;

    .text ALIGN(4K) : {
        KEEP(*(.multiboot2))
        KEEP(*(.text.boot))
        *(.text .text.*)
    }
    .rodata ALIGN(4K) : {
        *(.rodata .rodata.*)
    }
    .data ALIGN(4K) : {
        *(.data .data.*)
    }
    .bss ALIGN(4K) : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    __kernel_end = .;
    __kernel_size = __kernel_end - __kernel_start;

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.comment)
        *(.note .note.*)
        *(.gcc_except_table)
    }
}
