/*
 * AtrodraX Rust — 32-bit multiboot2 entry, long-mode transition,
 * interrupt stubs, and task context switching.
 */

.set BOOT_STACK_SIZE, 65536

/* ── Early page tables: identity map the first 4 GiB with 2 MiB pages ───── */
.section .bss
.align 4096
.global boot_pml4
boot_pml4:  .space 4096
.global boot_pdpt
boot_pdpt:  .space 4096
.global boot_pd0
boot_pd0:   .space 4096
.global boot_pd1
boot_pd1:   .space 4096
.global boot_pd2
boot_pd2:   .space 4096
.global boot_pd3
boot_pd3:   .space 4096

.align 16
boot_stack:
    .space BOOT_STACK_SIZE
.global boot_stack_top
boot_stack_top:

/* ── Temporary GDT used before the Rust GDT is installed ─────────────────── */
.section .data
.align 8
boot_gdt:
    .quad 0x0000000000000000
    .quad 0x00AF9A000000FFFF
    .quad 0x00AF92000000FFFF
boot_gdt_end:
boot_gdtr:
    .word boot_gdt_end - boot_gdt - 1
    .quad boot_gdt

/* ── 32-bit entry ───────────────────────────────────────────────────────── */
.section .text.boot,"ax"
.code32
.global _start
_start:
    cli
    cld
    movl %eax, %ebp              /* multiboot2 magic */
    movl %ebx, %esi              /* multiboot2 info physical address */

    /* The stack lives in .bss; point at it before clearing .bss. */
    leal boot_stack_top, %esp

    /* Clear .bss (page tables live there). */
    movl $__bss_start, %edi
    movl $__bss_end, %ecx
    subl %edi, %ecx
    xorl %eax, %eax
    rep stosb

    /* PML4[0] -> PDPT */
    movl $boot_pdpt, %eax
    orl  $0x3, %eax
    movl %eax, boot_pml4
    movl $0, boot_pml4 + 4

    /* PML4[511] aliases the same PDPT at 0xFFFFFF80_00000000: the kernel's
     * direct map. Kernel code keeps using low identity addresses for its own
     * image, but page tables and DMA buffers are reached through this window,
     * which user mappings can never occupy. */
    movl %eax, boot_pml4 + 4088
    movl $0, boot_pml4 + 4092

    /* PDPT[0..3] -> four page directories (4 GiB total) */
    movl $boot_pd0, %eax
    orl  $0x3, %eax
    movl %eax, boot_pdpt + 0
    movl $0,   boot_pdpt + 4
    movl $boot_pd1, %eax
    orl  $0x3, %eax
    movl %eax, boot_pdpt + 8
    movl $0,   boot_pdpt + 12
    movl $boot_pd2, %eax
    orl  $0x3, %eax
    movl %eax, boot_pdpt + 16
    movl $0,   boot_pdpt + 20
    movl $boot_pd3, %eax
    orl  $0x3, %eax
    movl %eax, boot_pdpt + 24
    movl $0,   boot_pdpt + 28

    /* Fill the four page directories with 2 MiB supervisor RW pages. */
    movl $boot_pd0, %ebx
    xorl %edx, %edx
    movl $4, %edi
.Lpd_outer:
    movl $512, %ecx
.Lpd_inner:
    movl %edx, %eax
    orl  $0x83, %eax
    movl %eax, (%ebx)
    movl $0, 4(%ebx)
    addl $0x200000, %edx
    addl $8, %ebx
    decl %ecx
    jnz .Lpd_inner
    decl %edi
    jnz .Lpd_outer

    /* Enable PAE */
    movl %cr4, %eax
    orl  $0x20, %eax
    movl %eax, %cr4

    /* Load PML4 */
    movl $boot_pml4, %eax
    movl %eax, %cr3

    /* EFER.LME */
    movl $0xC0000080, %ecx
    rdmsr
    orl  $0x100, %eax
    wrmsr

    /* CR0.PG | CR0.PE */
    movl %cr0, %eax
    orl  $0x80000001, %eax
    movl %eax, %cr0

    lgdt boot_gdtr
    ljmp $0x08, $.Llong_mode

.code64
.Llong_mode:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    xorw %ax, %ax
    movw %ax, %fs
    movw %ax, %gs

    leaq boot_stack_top(%rip), %rsp
    movl %ebp, %edi
    movl %esi, %esi
    call kernel_main

.Lhalt:
    cli
    hlt
    jmp .Lhalt

/* ── Interrupt stubs ────────────────────────────────────────────────────── */
.section .text
.code64

.macro GEN_ISR num
.global isr_\num
isr_\num:
    .if (\num == 8) || (\num == 10) || (\num == 11) || (\num == 12) || (\num == 13) || (\num == 14) || (\num == 17) || (\num == 21) || (\num == 29) || (\num == 30)
        pushq $\num
    .else
        pushq $0
        pushq $\num
    .endif
    jmp isr_common
.endm

GEN_ISR 0
GEN_ISR 1
GEN_ISR 2
GEN_ISR 3
GEN_ISR 4
GEN_ISR 5
GEN_ISR 6
GEN_ISR 7
GEN_ISR 8
GEN_ISR 9
GEN_ISR 10
GEN_ISR 11
GEN_ISR 12
GEN_ISR 13
GEN_ISR 14
GEN_ISR 15
GEN_ISR 16
GEN_ISR 17
GEN_ISR 18
GEN_ISR 19
GEN_ISR 20
GEN_ISR 21
GEN_ISR 22
GEN_ISR 23
GEN_ISR 24
GEN_ISR 25
GEN_ISR 26
GEN_ISR 27
GEN_ISR 28
GEN_ISR 29
GEN_ISR 30
GEN_ISR 31
GEN_ISR 32
GEN_ISR 33
GEN_ISR 34
GEN_ISR 35
GEN_ISR 36
GEN_ISR 37
GEN_ISR 38
GEN_ISR 39
GEN_ISR 40
GEN_ISR 41
GEN_ISR 42
GEN_ISR 43
GEN_ISR 44
GEN_ISR 45
GEN_ISR 46
GEN_ISR 47
GEN_ISR 48
GEN_ISR 49
GEN_ISR 50
GEN_ISR 51
GEN_ISR 52
GEN_ISR 53
GEN_ISR 54
GEN_ISR 55
GEN_ISR 56
GEN_ISR 57
GEN_ISR 58
GEN_ISR 59
GEN_ISR 60
GEN_ISR 61
GEN_ISR 62
GEN_ISR 63

GEN_ISR 128

.global isr_common
isr_common:
    pushq %rax
    pushq %rbx
    pushq %rcx
    pushq %rdx
    pushq %rsi
    pushq %rdi
    pushq %rbp
    pushq %r8
    pushq %r9
    pushq %r10
    pushq %r11
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15

    cld
    movq %rsp, %rdi
    call interrupt_dispatch

    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %r11
    popq %r10
    popq %r9
    popq %r8
    popq %rbp
    popq %rdi
    popq %rsi
    popq %rdx
    popq %rcx
    popq %rbx
    popq %rax
    addq $16, %rsp
    iretq

/* ── GDT reload helper ──────────────────────────────────────────────────── */
.global gdt_flush
gdt_flush:
    lgdt (%rdi)
    pushq $0x08
    leaq 1f(%rip), %rax
    pushq %rax
    lretq
1:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    xorw %ax, %ax
    movw %ax, %fs
    movw %ax, %gs
    ret

/* ── Cooperative task context switch ───────────────────────────────────── */
.global context_switch
context_switch:
    pushq %rbp
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    movq %rsp, (%rdi)
    movq %rsi, %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    popq %rbp
    retq

/* ── Enter ring 3 from a saved TrapFrame ───────────────────────────────── */
.global enter_user
enter_user:
    pushq 168(%rdi)              /* ss */
    pushq 160(%rdi)              /* rsp */
    pushq 152(%rdi)              /* rflags */
    pushq 144(%rdi)              /* cs */
    pushq 136(%rdi)              /* rip */

    movq 0(%rdi), %r15
    movq 8(%rdi), %r14
    movq 16(%rdi), %r13
    movq 24(%rdi), %r12
    movq 32(%rdi), %r11
    movq 40(%rdi), %r10
    movq 48(%rdi), %r9
    movq 56(%rdi), %r8
    movq 64(%rdi), %rbp
    movq 80(%rdi), %rsi
    movq 88(%rdi), %rdx
    movq 96(%rdi), %rcx
    movq 104(%rdi), %rbx
    movq 112(%rdi), %rax
    movq 72(%rdi), %rdi
    iretq

/* ── ISR address table (first 64 vectors) ──────────────────────────────── */
.section .rodata
.align 8
.global isr_stub_table
isr_stub_table:
    .quad isr_0
    .quad isr_1
    .quad isr_2
    .quad isr_3
    .quad isr_4
    .quad isr_5
    .quad isr_6
    .quad isr_7
    .quad isr_8
    .quad isr_9
    .quad isr_10
    .quad isr_11
    .quad isr_12
    .quad isr_13
    .quad isr_14
    .quad isr_15
    .quad isr_16
    .quad isr_17
    .quad isr_18
    .quad isr_19
    .quad isr_20
    .quad isr_21
    .quad isr_22
    .quad isr_23
    .quad isr_24
    .quad isr_25
    .quad isr_26
    .quad isr_27
    .quad isr_28
    .quad isr_29
    .quad isr_30
    .quad isr_31
    .quad isr_32
    .quad isr_33
    .quad isr_34
    .quad isr_35
    .quad isr_36
    .quad isr_37
    .quad isr_38
    .quad isr_39
    .quad isr_40
    .quad isr_41
    .quad isr_42
    .quad isr_43
    .quad isr_44
    .quad isr_45
    .quad isr_46
    .quad isr_47
    .quad isr_48
    .quad isr_49
    .quad isr_50
    .quad isr_51
    .quad isr_52
    .quad isr_53
    .quad isr_54
    .quad isr_55
    .quad isr_56
    .quad isr_57
    .quad isr_58
    .quad isr_59
    .quad isr_60
    .quad isr_61
    .quad isr_62
    .quad isr_63

.section .text
