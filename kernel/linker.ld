OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
    . = 1M;
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
