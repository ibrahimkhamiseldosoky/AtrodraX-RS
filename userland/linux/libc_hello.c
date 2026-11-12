/* Linux ABI probe built against a real C library.
 *
 * Unlike hello.c (raw syscalls, no libc), this one exercises what libc needs
 * from the kernel: TLS via arch_prctl, brk/mmap for the allocator, buffered
 * stdio at exit, uname, stat/fstat, file reads and the startup stack
 * (argc/argv/envp/auxv).
 *
 * Build: ./build.sh  (gcc -static, so no PT_INTERP is required)
 */
#include <dirent.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/utsname.h>
#include <unistd.h>

static int counter;

static void show_stat(const char *label, const struct stat *st) {
    printf("%s: mode=%06o size=%ld nlink=%ld\n", label,
           (unsigned)st->st_mode, (long)st->st_size, (long)st->st_nlink);
}

int main(int argc, char **argv) {
    printf("libc hello: argc=%d\n", argc);
    for (int i = 0; i < argc; i++) {
        printf("  argv[%d] = %s\n", i, argv[i]);
    }

    struct utsname u;
    if (uname(&u) == 0) {
        printf("uname: %s %s %s\n", u.sysname, u.release, u.machine);
    } else {
        printf("uname failed\n");
    }

    char *p = malloc(64 * 1024);
    if (p) {
        memset(p, 0x5a, 64 * 1024);
        counter = 42;
        printf("malloc: %zu bytes at %p, counter=%d\n", (size_t)64 * 1024, p, counter);
        free(p);
    } else {
        printf("malloc failed\n");
    }

    struct stat st;
    if (stat("/README.md", &st) == 0) {
        show_stat("stat /README.md", &st);
    } else {
        printf("stat /README.md failed\n");
    }
    if (stat("/etc", &st) == 0) {
        show_stat("stat /etc", &st);
    } else {
        printf("stat /etc failed\n");
    }
    if (fstat(1, &st) == 0) {
        show_stat("fstat 1", &st);
    } else {
        printf("fstat 1 failed\n");
    }

    int fd = open("/README.md", O_RDONLY);
    if (fd >= 0) {
        char buf[48];
        ssize_t n = read(fd, buf, sizeof(buf) - 1);
        if (n > 0) {
            buf[n] = 0;
            char *nl = strchr(buf, '\n');
            if (nl) {
                *nl = 0;
            }
            printf("read: \"%s\"\n", buf);
        }
        close(fd);
    } else {
        printf("open /README.md failed\n");
    }

    printf("pid=%d ppid=%d\n", (int)getpid(), (int)getppid());

    DIR *dir = opendir("/bin");
    if (dir) {
        int shown = 0;
        struct dirent *de;
        while ((de = readdir(dir)) != NULL && shown < 5) {
            printf("  /bin/%-12s dtype=%d\n", de->d_name, (int)de->d_type);
            shown++;
        }
        closedir(dir);
    } else {
        printf("opendir /bin failed\n");
    }

    return 7;
}
