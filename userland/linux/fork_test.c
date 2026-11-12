/* fork(2)/wait4(2) probe: raw syscalls only, no libc.
 *
 *   pid = fork();
 *   if (pid == 0) { write("child"); exit(5); }
 *   wait4(pid, &status, 0, 0);
 *   check(status == 5 << 8);
 *
 * Built by ./build.sh into kernel/assets/user/fork_test.elf.
 */

static long syscall0(long n) {
    long result;
    __asm__ volatile("syscall"
                     : "=a"(result)
                     : "a"(n)
                     : "rcx", "r11", "memory");
    return result;
}

static long syscall1(long n, long a) {
    long result;
    __asm__ volatile("syscall"
                     : "=a"(result)
                     : "a"(n), "D"(a)
                     : "rcx", "r11", "memory");
    return result;
}

static long syscall3(long n, long a, long b, long c) {
    long result;
    __asm__ volatile("syscall"
                     : "=a"(result)
                     : "a"(n), "D"(a), "S"(b), "d"(c)
                     : "rcx", "r11", "memory");
    return result;
}

static long syscall4(long n, long a, long b, long c, long d) {
    register long r10 __asm__("r10") = d;
    long result;
    __asm__ volatile("syscall"
                     : "=a"(result)
                     : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10)
                     : "rcx", "r11", "memory");
    return result;
}

static void out(const char *text, unsigned long length) {
    syscall3(1, 1, (long)text, (long)length);
}

#define OUT(text) out(text, sizeof(text) - 1)

void _start(void) {
    long pid = syscall0(57 /* SYS_fork */);

    if (pid == 0) {
        OUT("fork_test: child running\n");
        syscall1(60 /* SYS_exit */, 5);
        for (;;) {
        }
    }
    if (pid < 0) {
        OUT("fork_test: fork failed\n");
        syscall1(60, 1);
        for (;;) {
        }
    }

    int status = 0;
    long reaped = syscall4(61 /* SYS_wait4 */, pid, (long)&status, 0, 0);
    if (reaped != pid) {
        OUT("fork_test: wait4 failed\n");
        syscall1(60, 2);
        for (;;) {
        }
    }
    if (status != (5 << 8)) {
        OUT("fork_test: wrong wait status\n");
        syscall1(60, 3);
        for (;;) {
        }
    }

    OUT("fork_test: child exited with 5, wait4 ok\n");
    syscall1(60, 0);
    for (;;) {
    }
}
