/* execve(2) probe: raw syscalls only, no libc.
 *
 *   write(1, "exec_hello: before execve\n", 26);
 *   execve("/bin/hello_linux", {"from-execve"}, {"PATH=/bin"});
 *   write(1, "exec_hello: execve failed\n", 26);   // only on error
 *   exit(1);
 *
 * Built by ./build.sh into kernel/assets/user/exec_hello.elf.
 */
void _start(void) {
    __asm__ volatile(
        "mov $1, %%rax\n" /* SYS_write */
        "mov $1, %%rdi\n"
        "lea before(%%rip), %%rsi\n"
        "mov $26, %%rdx\n"
        "syscall\n"
        "mov $59, %%rax\n" /* SYS_execve */
        "lea prog(%%rip), %%rdi\n"
        "lea argv(%%rip), %%rsi\n"
        "lea envp(%%rip), %%rdx\n"
        "syscall\n"
        /* execve only returns when it fails. */
        "mov $1, %%rax\n"
        "mov $1, %%rdi\n"
        "lea failed(%%rip), %%rsi\n"
        "mov $26, %%rdx\n"
        "syscall\n"
        "mov $60, %%rax\n" /* SYS_exit */
        "mov $1, %%rdi\n"
        "syscall\n"
        ".section .rodata\n"
        "prog: .asciz \"/bin/hello_linux\"\n"
        "arg1: .asciz \"from-execve\"\n"
        "env1: .asciz \"PATH=/bin\"\n"
        "before: .asciz \"exec_hello: before execve\\n\"\n"
        "failed: .asciz \"exec_hello: execve failed\\n\"\n"
        ".align 8\n"
        "argv: .quad prog, arg1, 0\n"
        "envp: .quad env1, 0\n"
        ".previous\n"
        :
        :
        : "rax", "rdi", "rsi", "rdx");
    __builtin_unreachable();
}
