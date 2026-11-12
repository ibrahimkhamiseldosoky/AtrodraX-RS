/*
 * linux/entry.S — x86-64 SYSCALL entry for Linux-ABI tasks.
 *
 * The CPU enters here with:
 *   rax = syscall number
 *   rdi, rsi, rdx, r10, r8, r9 = arguments
 *   rcx = user RIP, r11 = user RFLAGS
 *   rsp = user stack
 *
 * SFMASK clears IF before we get here, so the handler runs with interrupts
 * disabled on the task's kernel stack. That is what makes the transient
 * `linux_user_rsp_tmp` slot safe: nothing can preempt the entry sequence.
 *
 * The frame we build is per task (it lives on the task's kernel stack), so a
 * syscall that blocks and resumes later still returns to the right user stack.
 *
 * Frame layout (low -> high): r9, r8, r10, rdx, rsi, rdi, r11, rcx, user rsp
 */

.code64

.section .data
.align 8
.global linux_kernel_rsp0
linux_kernel_rsp0:
    .quad 0
linux_user_rsp_tmp:
    .quad 0

.section .text
.global linux_syscall_entry
linux_syscall_entry:
    /* Save the user stack, then switch to this task's kernel stack. */
    movq %rsp, linux_user_rsp_tmp(%rip)
    movq linux_kernel_rsp0(%rip), %rsp
    andq $-16, %rsp

    /* Return frame: user rsp first so it is the last value popped. */
    pushq linux_user_rsp_tmp(%rip)
    pushq %rcx
    pushq %r11
    pushq %rdi
    pushq %rsi
    pushq %rdx
    pushq %r10
    pushq %r8
    pushq %r9

    /* Marshal SysV arguments: (num, a0, a1, a2, a3, a4) + a5 on the stack. */
    movq %rax, %rdi
    movq 40(%rsp), %rsi
    movq 32(%rsp), %rdx
    movq 24(%rsp), %rcx
    movq 16(%rsp), %r8
    movq 8(%rsp), %r9
    pushq (%rsp)              /* a5 = saved user r9; also 16-byte aligns */

    call linux_syscall_dispatch

    addq $8, %rsp             /* drop a5 */

    /* execve() replaced the image: resume in the new program instead of
     * returning to the old one. Frame slots (after the drop):
     *   0 r9, 8 r8, 16 r10, 24 rdx, 32 rsi, 40 rdi, 48 r11, 56 rcx, 64 rsp
     */
    cmpq $0, linux_exec_pending(%rip)
    je 1f
    movq $0, linux_exec_pending(%rip)
    movq linux_exec_rsp(%rip), %rax
    movq %rax, 64(%rsp)       /* user RSP, popped into rsp last */
    movq linux_exec_rip(%rip), %rax
    movq %rax, 56(%rsp)       /* RCX -> user RIP for sysret */
    movq linux_exec_flags(%rip), %rax
    movq %rax, 48(%rsp)       /* R11 -> RFLAGS for sysret */
    /* A fresh image starts with undefined general registers, but crt0 relies
     * on the argument slots being clear. */
    movq $0, 40(%rsp)         /* rdi */
    movq $0, 32(%rsp)         /* rsi */
    movq $0, 24(%rsp)         /* rdx */
    movq $0, 16(%rsp)         /* r10 */
    movq $0, 8(%rsp)          /* r8 */
    movq $0, 0(%rsp)          /* r9 */
    xorl %eax, %eax
1:

    popq %r9
    popq %r8
    popq %r10
    popq %rdx
    popq %rsi
    popq %rdi
    popq %r11
    popq %rcx
    popq %rsp                 /* back to the user stack */
    sysretq

/* Faulting here means SYSRET was handed a non-canonical RIP/stack. Surface it
 * instead of triple-faulting. */
.global linux_sysret_fault
linux_sysret_fault:
    cli
    hlt
