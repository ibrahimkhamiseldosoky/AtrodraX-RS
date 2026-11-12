/* Minimal Linux-ABI probe: raw syscalls only, no libc, no SSE.
 *
 *   write(1, "hello from linux binary\n", 24);
 *   exit(0);
 *
 * Built by ./build.sh into kernel/assets/user/hello_linux.elf.
 */
void _start(void) {
    __asm__ volatile(
        "mov $1, %%rax\n"   /* SYS_write */
        "mov $1, %%rdi\n"   /* fd = stdout */
        "lea msg(%%rip), %%rsi\n"
        "mov $24, %%rdx\n"
        "syscall\n"
        "mov $60, %%rax\n"  /* SYS_exit */
        "xor %%rdi, %%rdi\n"
        "syscall\n"
        ".section .rodata\n"
        "msg: .asciz \"hello from linux binary\\n\"\n"
        ".previous\n"
        :
        :
        : "rax", "rdi", "rsi", "rdx");
    __builtin_unreachable();
}
