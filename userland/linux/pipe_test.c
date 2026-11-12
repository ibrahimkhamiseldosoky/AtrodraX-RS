/* pipe(2)/dup2(2)/execve(2) probe: raw syscalls only, no libc.
 *
 *   pipe(fds);
 *   pid = fork();
 *   if (pid == 0) {
 *       dup2(fds[1], 1);            // stdout -> pipe
 *       close(fds[0]); close(fds[1]);
 *       execve("/bin/hello_linux", ...);
 *   }
 *   close(fds[1]);
 *   read(fds[0], buf);              // "hello from linux binary\n"
 *   write(1, buf);
 *   wait4(pid, &status);
 *
 * Built by ./build.sh into kernel/assets/user/pipe_test.elf.
 */

static long syscall0(long n) {
    long result;
    __asm__ volatile("syscall" : "=a"(result) : "a"(n) : "rcx", "r11", "memory");
    return result;
}

static long syscall1(long n, long a) {
    long result;
    __asm__ volatile("syscall" : "=a"(result) : "a"(n), "D"(a) : "rcx", "r11", "memory");
    return result;
}

static long syscall2(long n, long a, long b) {
    long result;
    __asm__ volatile("syscall"
                     : "=a"(result)
                     : "a"(n), "D"(a), "S"(b)
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
    int fds[2];
    if (syscall1(22 /* SYS_pipe */, (long)fds) != 0) {
        OUT("pipe_test: pipe failed\n");
        syscall1(60, 1);
        for (;;) {
        }
    }

    long pid = syscall0(57 /* SYS_fork */);
    if (pid == 0) {
        syscall2(33 /* SYS_dup2 */, fds[1], 1);
        syscall1(3 /* SYS_close */, fds[0]);
        syscall1(3, fds[1]);
        syscall3(59 /* SYS_execve */, (long)"/bin/hello_linux", 0, 0);
        syscall1(60, 2);
        for (;;) {
        }
    }
    if (pid < 0) {
        OUT("pipe_test: fork failed\n");
        syscall1(60, 3);
        for (;;) {
        }
    }

    syscall1(3, fds[1]);

    char buffer[128];
    long total = 0;
    for (;;) {
        long n = syscall3(0 /* SYS_read */, fds[0], (long)(buffer + total), sizeof(buffer) - 1 - total);
        if (n <= 0) {
            break;
        }
        total += n;
        if (total >= (long)sizeof(buffer) - 1) {
            break;
        }
    }
    syscall1(3, fds[0]);
    buffer[total] = 0;

    int status = 0;
    long reaped = syscall4(61 /* SYS_wait4 */, pid, (long)&status, 0, 0);
    if (reaped != pid) {
        OUT("pipe_test: wait4 failed\n");
        syscall1(60, 4);
        for (;;) {
        }
    }

    OUT("pipe_test: child wrote through the pipe: ");
    out(buffer, (unsigned long)total);
    if (total == 24 && buffer[0] == 'h' && status == 0) {
        OUT("pipe_test: ok\n");
        syscall1(60, 0);
    } else {
        OUT("pipe_test: unexpected payload or status\n");
        syscall1(60, 5);
    }
    for (;;) {
    }
}
