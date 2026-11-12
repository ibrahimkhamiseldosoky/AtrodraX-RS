//! Linux system call handlers.
//!
//! Step 1 of the Linux ABI: process identity, memory management, terminal and
//! file I/O, time, randomness and the handful of "no-op" calls static binaries
//! make during startup. Unimplemented numbers return `-ENOSYS` and are logged
//! once each.

use crate::apps;
use crate::drivers::rtc;
use crate::fs::vfs;
use crate::mem::vmm;
use crate::sched;
use crate::syscall;
use crate::task::{self, CONSOLE_NODE};
use alloc::vec;
use alloc::vec::Vec;

use super::abi::*;

const FS_BASE_MSR: u32 = 0xC000_0100;

/// Upper bound for `brk`. The initial user stack lives at 0x0F00_0000 and the
/// kernel image starts at 0x1000_0000 (see `exec.rs` and `linker.ld`).
const BRK_LIMIT: u64 = 0x0E00_0000;

static mut LOGGED: [bool; 512] = [false; 512];

fn errno(code: i64) -> u64 {
    (-code) as u64
}

fn warn_unimplemented(num: u64) {
    unsafe {
        if num < LOGGED.len() as u64 && !LOGGED[num as usize] {
            LOGGED[num as usize] = true;
            crate::kprintln!("[linux] syscall {} not implemented (ENOSYS)", num);
        }
    }
}

fn task_index() -> Option<usize> {
    syscall::current_index()
}

fn task_pid() -> u64 {
    sched::current().map(|t| t.pid as u64).unwrap_or(0)
}

fn fd_is_console(index: usize, fd: i32) -> bool {
    match task::get_fd(index, fd) {
        Some(entry) => entry.node == CONSOLE_NODE,
        // Before a program opens anything, 0/1/2 are the controlling terminal.
        None => (0..=2).contains(&fd),
    }
}

/* ── Dispatch ──────────────────────────────────────────────────────────── */

pub fn dispatch(num: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    match num {
        SYS_READ => sys_read(a0 as i32, a1, a2 as usize),
        SYS_WRITE => sys_write(a0 as i32, a1, a2 as usize),
        SYS_OPEN => sys_openat(AT_FDCWD, a0, a1 as i32),
        SYS_OPENAT => sys_openat(a0 as i64, a1, a2 as i32),
        SYS_CLOSE => sys_close(a0 as i32),
        SYS_LSEEK => sys_lseek(a0 as i32, a1 as i64, a2 as i32),
        SYS_BRK => sys_brk(a0),
        SYS_MMAP => sys_mmap(a0, a1 as usize, a2 as i32, a3 as i32, a4 as i32, a5 as i64),
        SYS_MUNMAP | SYS_MPROTECT => 0,
        SYS_ARCH_PRCTL => sys_arch_prctl(a0, a1),
        SYS_UNAME => sys_uname(a0),
        SYS_GETPID | SYS_GETTID => task_pid(),
        SYS_GETPPID => sched::current().map(|t| t.ppid as u64).unwrap_or(0),
        SYS_GETUID | SYS_GETEUID | SYS_GETGID | SYS_GETEGID => 0,
        // The system has a single group (0), so a supplementary-list query
        // always reports no entries.
        SYS_GETGROUPS => 0,
        SYS_SETGROUPS => 0,
        SYS_EXIT | SYS_EXIT_GROUP => sys_exit(a0 as i32),
        SYS_SET_TID_ADDRESS => task_pid(),
        SYS_RT_SIGACTION => sys_rt_sigaction(a0, a1, a2),
        SYS_RT_SIGPROCMASK => 0,
        SYS_IOCTL => sys_ioctl(a0 as i32, a1, a2),
        SYS_GETTIMEOFDAY => sys_gettimeofday(a0),
        SYS_TIME => sys_time(a0),
        SYS_CLOCK_GETTIME => sys_clock_gettime(a0, a1),
        SYS_SCHED_YIELD => {
            sched::yield_now();
            0
        }
        SYS_GETRANDOM => sys_getrandom(a0, a1 as usize),
        SYS_FUTEX => sys_futex(a0),
        SYS_NANOSLEEP => sys_nanosleep(a0),
        SYS_FSTAT => sys_fstat(a0 as i32, a1),
        SYS_NEWFSTATAT => sys_newfstatat(a0 as i64, a1, a2, a3 as i32),
        SYS_STATFS => sys_statfs(a0, a1),
        SYS_FSTATFS => sys_fstatfs(a0 as i32, a1),
        SYS_READLINKAT => errno(EINVAL),
        SYS_SET_ROBUST_LIST => 0,
        SYS_PRLIMIT64 => sys_prlimit64(a0, a1, a2, a3),
        SYS_GETDENTS64 => sys_getdents64(a0 as i32, a1, a2 as usize),
        // glibc/busybox probe these; they have no effect on a uniprocessor
        // with no restartable sequences.
        SYS_PRCTL => 0,
        SYS_RSEQ => errno(ENOSYS),
        _ => {
            warn_unimplemented(num);
            errno(ENOSYS)
        }
    }
}

/* ── stat(2) ───────────────────────────────────────────────────────────── */

/// `struct stat` as x86-64 Linux lays it out (144 bytes).
fn fill_stat(node: usize, out: &mut [u8; 144]) {
    let is_dir = node != CONSOLE_NODE && vfs::is_directory(node);
    let (kind, size) = if node == CONSOLE_NODE {
        (S_IFCHR | 0o620, 0)
    } else if is_dir {
        (S_IFDIR | 0o755, 0)
    } else {
        (S_IFREG | 0o644, vfs::size_of(node))
    };

    put_u64(out, 0, 0x100); // st_dev
    put_u64(out, 8, node as u64 + 1); // st_ino
    put_u64(out, 16, if is_dir { 2 } else { 1 }); // st_nlink
    put_u32(out, 24, kind); // st_mode
    put_u32(out, 28, 0); // st_uid
    put_u32(out, 32, 0); // st_gid
    put_u64(out, 40, 0); // st_rdev
    put_u64(out, 48, size as u64); // st_size
    put_u64(out, 56, 4096); // st_blksize
    put_u64(out, 64, ((size as u64) + 511) / 512); // st_blocks
    let now = unix_time();
    for offset in [72, 88, 104] {
        put_u64(out, offset, now.max(0) as u64); // st_atim/st_mtim/st_ctim
        put_u64(out, offset + 8, 0);
    }
}

fn put_u64(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut [u8], offset: usize, value: u32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn sys_fstat(fd: i32, out: u64) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let Some(entry) = task::get_fd(index, fd).copied() else {
        return errno(EBADF);
    };
    let mut stat = [0u8; 144];
    fill_stat(entry.node, &mut stat);
    if syscall::copy_to_user(&space, out, &stat) {
        0
    } else {
        errno(EFAULT)
    }
}

fn sys_newfstatat(dirfd: i64, path_ptr: u64, out: u64, flags: i32) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let path = syscall::read_cstr(&space, path_ptr, 256).unwrap_or_default();
    let node = if path.is_empty() {
        if flags & AT_EMPTY_PATH == 0 {
            return errno(ENOENT);
        }
        match task::get_fd(index, dirfd as i32).copied() {
            Some(entry) => entry.node,
            None => return errno(EBADF),
        }
    } else {
        let cwd = task::get(index).cwd.clone();
        match vfs::resolve(&path, &cwd) {
            Some(node) => node,
            None => return errno(ENOENT),
        }
    };
    let mut stat = [0u8; 144];
    fill_stat(node, &mut stat);
    if syscall::copy_to_user(&space, out, &stat) {
        0
    } else {
        errno(EFAULT)
    }
}

/// `struct statfs` as x86-64 Linux lays it out (120 bytes). The RAMFS lives in
/// kernel memory backed by physical frames, so the numbers are derived from the
/// page allocator instead of a disk.
fn fill_statfs(out: &mut [u8; 120]) {
    let free = crate::mem::pmm::free_pages() as u64;
    let total = crate::boot::total_memory_bytes() / 4096;
    put_u64(out, 0, 0x8584_58F6); // f_type: RAMFS_MAGIC
    put_u64(out, 8, 4096); // f_bsize
    put_u64(out, 16, total); // f_blocks
    put_u64(out, 24, free); // f_bfree
    put_u64(out, 32, free); // f_bavail
    put_u64(out, 40, total / 2); // f_files
    put_u64(out, 48, total / 4); // f_ffree
    put_u64(out, 64, 255); // f_namelen
    put_u64(out, 72, 4096); // f_frsize
}

fn sys_statfs(path_ptr: u64, out: u64) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    if syscall::read_cstr(&space, path_ptr, 4096).is_none() {
        return errno(EFAULT);
    }
    let mut buf = [0u8; 120];
    fill_statfs(&mut buf);
    if syscall::copy_to_user(&space, out, &buf) {
        0
    } else {
        errno(EFAULT)
    }
}

fn sys_fstatfs(fd: i32, out: u64) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    if task::get_fd(index, fd).is_none() {
        return errno(EBADF);
    }
    let mut buf = [0u8; 120];
    fill_statfs(&mut buf);
    if syscall::copy_to_user(&space, out, &buf) {
        0
    } else {
        errno(EFAULT)
    }
}

fn sys_prlimit64(_pid: u64, resource: u64, _new_limit: u64, old_limit: u64) -> u64 {
    if old_limit == 0 {
        return 0;
    }
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    // Only the limits libc actually inspects answer meaningfully; everything
    // else reports "unlimited".
    let (cur, max) = match resource {
        3 => (8 * 1024 * 1024u64, 8 * 1024 * 1024u64), // RLIMIT_STACK
        7 => (1024, 1024),                             // RLIMIT_NOFILE
        _ => (u64::MAX, u64::MAX),
    };
    let mut buf = [0u8; 16];
    put_u64(&mut buf, 0, cur);
    put_u64(&mut buf, 8, max);
    if syscall::copy_to_user(&space, old_limit, &buf) {
        0
    } else {
        errno(EFAULT)
    }
}

/* ── I/O ───────────────────────────────────────────────────────────────── */

fn sys_write(fd: i32, buffer: u64, len: usize) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    if len == 0 {
        return 0;
    }
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let Some(data) = syscall::copy_from_user(&space, buffer, len.min(1024 * 1024)) else {
        return errno(EFAULT);
    };

    if fd == 1 || fd == 2 {
        if fd_is_console(index, fd) {
            let pid = task::get(index).pid;
            let written = apps::stdio_write(pid, &data);
            if written >= 0 {
                return written as u64;
            }
            // No terminal window (headless boot): still mirror to serial.
            for byte in &data {
                crate::arch::serial::write_byte(*byte);
            }
            return data.len() as u64;
        }
    }

    let Some(entry) = task::get_fd(index, fd).copied() else {
        return errno(EBADF);
    };
    let written = vfs::write(entry.node, entry.offset, &data);
    if let Some(entry) = task::get_fd(index, fd) {
        entry.offset += written;
    }
    written as u64
}

fn sys_read(fd: i32, buffer: u64, len: usize) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    if len == 0 {
        return 0;
    }
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };

    if fd_is_console(index, fd) {
        let pid = task::get(index).pid;
        let mut temp = vec![0u8; len.min(4096)];
        loop {
            let read = apps::stdio_read(pid, &mut temp);
            if read < 0 {
                return errno(EIO);
            }
            if read > 0 {
                return if syscall::copy_to_user(&space, buffer, &temp[..read as usize]) {
                    read as u64
                } else {
                    errno(EFAULT)
                };
            }
            // Nothing queued yet: block until the terminal has input. The
            // timer wakes us, so this does not spin the CPU.
            sched::sleep_ms(10);
        }
    }

    let Some(entry) = task::get_fd(index, fd).copied() else {
        return errno(EBADF);
    };
    if vfs::is_directory(entry.node) {
        return errno(EISDIR);
    }
    let data = vfs::read(entry.node, entry.offset, len);
    if !syscall::copy_to_user(&space, buffer, &data) {
        return errno(EFAULT);
    }
    if let Some(entry) = task::get_fd(index, fd) {
        entry.offset += data.len();
    }
    data.len() as u64
}

fn sys_openat(dirfd: i64, path_ptr: u64, flags: i32) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let Some(path) = syscall::read_cstr(&space, path_ptr, 256) else {
        return errno(EFAULT);
    };
    // `dirfd` is accepted for AT_FDCWD-style relative paths; directory fds are
    // not modelled yet, which matches the native VFS.
    let _ = dirfd;
    let cwd = task::get(index).cwd.clone();
    let mut node = vfs::resolve(&path, &cwd);
    if node.is_none() && flags & O_CREAT != 0 {
        node = vfs::create_file_path(&path, &cwd);
    }
    let Some(node) = node else {
        return errno(ENOENT);
    };
    if vfs::is_directory(node) && flags & 0o3 != 0 {
        return errno(EISDIR);
    }
    if flags & O_TRUNC != 0 && !vfs::is_directory(node) {
        vfs::truncate(node, 0);
    }
    let fd = task::alloc_fd(index, node, 0);
    if fd < 0 {
        return errno(EMFILE);
    }
    if flags & O_APPEND != 0 {
        if let Some(entry) = task::get_fd(index, fd) {
            entry.offset = vfs::size_of(node);
        }
    }
    fd as u64
}

fn sys_close(fd: i32) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    if syscall::close_fd(index, fd) == 0 {
        0
    } else {
        errno(EBADF)
    }
}

fn sys_lseek(fd: i32, offset: i64, whence: i32) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(entry) = task::get_fd(index, fd).copied() else {
        return errno(EBADF);
    };
    let size = if entry.node == CONSOLE_NODE {
        0
    } else {
        vfs::size_of(entry.node) as i64
    };
    let next = match whence {
        SEEK_SET => offset,
        SEEK_CUR => entry.offset as i64 + offset,
        SEEK_END => size + offset,
        _ => return errno(EINVAL),
    };
    if next < 0 {
        return errno(EINVAL);
    }
    if let Some(entry) = task::get_fd(index, fd) {
        entry.offset = next as usize;
    }
    next as u64
}

/* ── Memory ────────────────────────────────────────────────────────────── */

fn sys_brk(requested: u64) -> u64 {
    let Some(index) = task_index() else {
        return 0;
    };
    let (current, start) = {
        let task = task::get(index);
        (task.brk, task.brk_start)
    };
    let current = if current == 0 { start } else { current };

    if requested == 0 || requested < start {
        return current;
    }
    // The kernel keeps a low identity mapping of its own image and of the
    // initial user stack, so the heap must stay below both of them.
    if requested > BRK_LIMIT {
        return current;
    }
    if requested > current {
        let Some(space) = task::get(index).space else {
            return current;
        };
        let old_page = (current + 0xFFF) & !0xFFF;
        let new_page = (requested + 0xFFF) & !0xFFF;
        if new_page > old_page && !vmm::map_user_range(&space, old_page, new_page, true) {
            return current;
        }
    }
    task::get(index).brk = requested;
    requested
}

fn sys_mmap(addr: u64, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let _ = offset;
    if len == 0 {
        return errno(EINVAL);
    }
    let Some(space) = task::get(index).space else {
        return errno(ENOMEM);
    };
    let len = ((len as u64) + 0xFFF) & !0xFFF;

    // Only anonymous memory is backed for now; file-backed mappings need a
    // page cache and are deferred with the /dev/fb0 work.
    if flags & MAP_ANONYMOUS == 0 {
        if fd >= 0 {
            return errno(ENODEV);
        }
        return errno(EINVAL);
    }

    let base = if flags & MAP_FIXED != 0 {
        addr & !0xFFF
    } else if addr >= 0x4000_0000 && addr & 0xFFF == 0 {
        addr
    } else {
        task::get(index).mmap_base.max(0x4000_0000)
    };
    if base < 0x4000_0000 {
        return errno(EINVAL);
    }
    let Some(end) = base.checked_add(len) else {
        return errno(ENOMEM);
    };
    if end > crate::mem::FOUR_GIB {
        return errno(ENOMEM);
    }
    let writable = prot & PROT_WRITE != 0 || prot == PROT_NONE;
    if !vmm::map_user_range(&space, base, end, writable) {
        return errno(ENOMEM);
    }
    if flags & MAP_FIXED == 0 {
        task::get(index).mmap_base = end + PAGE_SIZE;
    }
    base
}

/* ── Process identity and control ──────────────────────────────────────── */

fn sys_exit(code: i32) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let pid = task::get(index).pid;
    apps::notify_death(pid);
    for fd in 0..task::MAX_FDS as i32 {
        if task::get_fd(index, fd).is_some() {
            syscall::close_fd(index, fd);
        }
    }
    sched::exit_current(code)
}

fn sys_arch_prctl(code: u64, address: u64) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    match code {
        ARCH_SET_FS => {
            task::get(index).fs_base = address;
            // Take effect immediately; the scheduler reloads it on every
            // resume so a context switch cannot silently revert TLS.
            unsafe { crate::arch::cpu::wrmsr(FS_BASE_MSR, address) };
            0
        }
        ARCH_GET_FS => {
            let Some(space) = syscall::current_space() else {
                return errno(EFAULT);
            };
            let value = task::get(index).fs_base;
            if syscall::copy_to_user(&space, address, &value.to_le_bytes()) {
                0
            } else {
                errno(EFAULT)
            }
        }
        _ => errno(EINVAL),
    }
}

const UTS_FIELD: usize = 65;

fn push_uts(buf: &mut Vec<u8>, value: &str) {
    let mut field = [0u8; UTS_FIELD];
    let bytes = value.as_bytes();
    let count = bytes.len().min(UTS_FIELD - 1);
    field[..count].copy_from_slice(&bytes[..count]);
    buf.extend_from_slice(&field);
}

fn sys_uname(out: u64) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let mut buf = Vec::with_capacity(UTS_FIELD * 6);
    push_uts(&mut buf, "Linux");
    push_uts(&mut buf, "atrodrax");
    push_uts(&mut buf, "6.1.0-atrodrax");
    push_uts(&mut buf, "AtrodraX");
    push_uts(&mut buf, "x86_64");
    push_uts(&mut buf, "(none)");
    if syscall::copy_to_user(&space, out, &buf) {
        0
    } else {
        errno(EFAULT)
    }
}

fn sys_rt_sigaction(signum: u64, action: u64, old: u64) -> u64 {
    // Only SIGCHLD/SIG_IGN are meaningful so far. Reporting "no old handler"
    // is enough for libc startup, which installs handlers without reading.
    let _ = (signum, action, old);
    0
}

/* ── Terminal and misc ─────────────────────────────────────────────────── */

fn sys_ioctl(fd: i32, request: u64, arg: u64) -> u64 {
    let _ = fd;
    match request {
        TIOCGWINSZ => {
            let Some(space) = syscall::current_space() else {
                return errno(EFAULT);
            };
            // struct winsize { ws_row, ws_col, ws_xpixel, ws_ypixel }
            let mut buf = [0u8; 8];
            buf[0..2].copy_from_slice(&24u16.to_le_bytes());
            buf[2..4].copy_from_slice(&80u16.to_le_bytes());
            if syscall::copy_to_user(&space, arg, &buf) {
                0
            } else {
                errno(EFAULT)
            }
        }
        FIONREAD => {
            let Some(space) = syscall::current_space() else {
                return errno(EFAULT);
            };
            if syscall::copy_to_user(&space, arg, &0i32.to_le_bytes()) {
                0
            } else {
                errno(EFAULT)
            }
        }
        // Termios translation is not modelled; accepting the write keeps
        // programs that call tcsetattr() happy.
        TCSETS => 0,
        _ => errno(ENOTTY),
    }
}

/* ── Time and randomness ───────────────────────────────────────────────── */

/// Unix epoch seconds from the CMOS clock.
fn unix_time() -> i64 {
    let t = rtc::now();
    let year = t.year as i64;
    let month = t.month as i64;
    let day = t.day as i64;
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + t.hour as i64 * 3_600 + t.minute as i64 * 60 + t.second as i64
}

fn sys_gettimeofday(out: u64) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let seconds = unix_time();
    let mut buf = [0u8; 16];
    buf[..8].copy_from_slice(&seconds.to_le_bytes());
    // Timeval microseconds are always zero at this resolution.
    if syscall::copy_to_user(&space, out, &buf) {
        0
    } else {
        errno(EFAULT)
    }
}

fn sys_time(out: u64) -> u64 {
    let seconds = unix_time();
    if out != 0 {
        let Some(space) = syscall::current_space() else {
            return errno(EFAULT);
        };
        if !syscall::copy_to_user(&space, out, &seconds.to_le_bytes()) {
            return errno(EFAULT);
        }
    }
    seconds as u64
}

fn sys_clock_gettime(clock: u64, out: u64) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let (seconds, nanos) = if clock == CLOCK_REALTIME {
        (unix_time(), 0)
    } else {
        let ms = crate::sync::uptime_ms();
        ((ms / 1000) as i64, ((ms % 1000) * 1_000_000) as i64)
    };
    let mut buf = [0u8; 16];
    buf[..8].copy_from_slice(&seconds.to_le_bytes());
    buf[8..].copy_from_slice(&nanos.to_le_bytes());
    if syscall::copy_to_user(&space, out, &buf) {
        0
    } else {
        errno(EFAULT)
    }
}

fn sys_getrandom(buffer: u64, len: usize) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let mut state = crate::arch::cpu::rdtsc() ^ 0x9E37_79B9_7F4A_7C15;
    let mut data = vec![0u8; len.min(4096)];
    for byte in data.iter_mut() {
        // xorshift64*
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
    let count = data.len();
    if syscall::copy_to_user(&space, buffer, &data) {
        count as u64
    } else {
        errno(EFAULT)
    }
}

fn sys_futex(operation: u64) -> u64 {
    // Single-threaded tasks cannot contend, so a WAKE succeeds and a WAIT is
    // reported as completed. Real futex queues arrive with clone()/threads.
    match operation & 0x7F {
        FUTEX_WAIT | FUTEX_WAKE => 0,
        _ => 0,
    }
}

fn sys_nanosleep(request: u64) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let Some(raw) = syscall::copy_from_user(&space, request, 16) else {
        return errno(EFAULT);
    };
    let seconds = i64::from_le_bytes(raw[0..8].try_into().unwrap()).max(0) as u64;
    let nanos = i64::from_le_bytes(raw[8..16].try_into().unwrap()).max(0) as u64;
    let ms = seconds.saturating_mul(1000) + nanos / 1_000_000;
    if ms > 0 {
        sched::sleep_ms(ms);
    }
    0
}

/* ── Directories ───────────────────────────────────────────────────────── */

/// `getdents64`: emit `struct linux_dirent64` records until the buffer is full.
fn sys_getdents64(fd: i32, buffer: u64, len: usize) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let Some(entry) = task::get_fd(index, fd).copied() else {
        return errno(EBADF);
    };
    if entry.node == CONSOLE_NODE || !vfs::is_directory(entry.node) {
        return errno(ENOTDIR);
    }

    let listing = vfs::list(entry.node);
    let mut out: Vec<u8> = Vec::new();
    let mut next = entry.offset;
    while next < listing.len() {
        let (name, kind, child) = &listing[next];
        let record_len = (19 + name.len() + 1 + 7) & !7;
        if out.len() + record_len > len {
            break;
        }
        out.resize(out.len() + record_len, 0);
        let at = out.len() - record_len;
        out[at..at + 8].copy_from_slice(&(*child as u64 + 1).to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&((next + 1) as u64).to_le_bytes());
        out[at + 16..at + 18].copy_from_slice(&(record_len as u16).to_le_bytes());
        out[at + 18] = match *kind {
            vfs::KIND_DIR | vfs::KIND_FAT_DIR => 4,  // DT_DIR
            vfs::KIND_CHAR => 2,                     // DT_CHR
            _ => 8,                                  // DT_REG
        };
        out[at + 19..at + 19 + name.len()].copy_from_slice(name.as_bytes());
        next += 1;
    }

    if let Some(entry) = task::get_fd(index, fd) {
        entry.offset = next;
    }
    if out.is_empty() {
        return 0;
    }
    if syscall::copy_to_user(&space, buffer, &out) {
        out.len() as u64
    } else {
        errno(EFAULT)
    }
}
