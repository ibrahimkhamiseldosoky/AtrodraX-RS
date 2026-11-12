//! Linux system call handlers.
//!
//! Step 1 of the Linux ABI: process identity, memory management, terminal and
//! file I/O, time, randomness and the handful of "no-op" calls static binaries
//! make during startup. Unimplemented numbers return `-ENOSYS` and are logged
//! once each.

use crate::apps;
use crate::arch::idt::TrapFrame;
use crate::drivers::rtc;
use crate::fs::vfs;
use crate::mem::vmm;
use crate::sched;
use crate::syscall;
use crate::task::{self, CONSOLE_NODE};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::abi::*;
use super::UserContext;

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

pub fn dispatch(context: &UserContext) -> u64 {
    let UserContext {
        num,
        a0,
        a1,
        a2,
        a3,
        a4,
        a5,
        ..
    } = *context;
    match num {
        SYS_READ => sys_read(a0 as i32, a1, a2 as usize),
        SYS_WRITE => sys_write(a0 as i32, a1, a2 as usize),
        SYS_READV => sys_readv(a0 as i32, a1, a2 as usize),
        SYS_WRITEV => sys_writev(a0 as i32, a1, a2 as usize),
        SYS_MKDIR => sys_mkdir(AT_FDCWD, a0),
        SYS_MKDIRAT => sys_mkdir(a0 as i64, a1),
        SYS_OPEN => sys_openat(AT_FDCWD, a0, a1 as i32),
        SYS_OPENAT => sys_openat(a0 as i64, a1, a2 as i32),
        SYS_CLOSE => sys_close(a0 as i32),
        SYS_DUP => sys_dup(a0 as i32),
        SYS_DUP2 => sys_dup2(a0 as i32, a1 as i32),
        SYS_DUP3 => sys_dup3(a0 as i32, a1 as i32),
        SYS_FCNTL => sys_fcntl(a0 as i32, a1 as i32, a2),
        SYS_PIPE => sys_pipe(a0),
        SYS_PIPE2 => sys_pipe2(a0, a1 as i32),
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
        SYS_EXECVE => sys_execve(a0, a1, a2),
        SYS_CLONE => sys_clone(context),
        SYS_FORK | SYS_VFORK => sys_fork(context),
        SYS_WAIT4 => sys_wait4(a0 as i64, a1, a2, a3),
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
        SYS_GETCWD => sys_getcwd(a0, a1 as usize),
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
    let is_char = node == CONSOLE_NODE
        || vfs::get(node)
            .map(|n| n.kind == vfs::KIND_CHAR)
            .unwrap_or(false);
    let is_dir = !is_char && vfs::is_directory(node);
    let (kind, size) = if is_char {
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

fn sys_getcwd(out: u64, size: usize) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let cwd = task::get(index).cwd.clone();
    let bytes = cwd.as_bytes();
    if size < bytes.len() + 1 {
        return errno(ERANGE);
    }
    let mut data = Vec::with_capacity(bytes.len() + 1);
    data.extend_from_slice(bytes);
    data.push(0);
    if syscall::copy_to_user(&space, out, &data) {
        out
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

/* ── Process control ───────────────────────────────────────────────────── */

/// Read a NULL-terminated array of string pointers from user memory.
fn read_string_array(space: &vmm::AddrSpace, pointer: u64, limit: usize) -> Option<Vec<String>> {
    let mut out = Vec::new();
    if pointer == 0 {
        return Some(out);
    }
    for index in 0..limit {
        let raw = syscall::copy_from_user(space, pointer + index as u64 * 8, 8)?;
        let mut word = [0u8; 8];
        word.copy_from_slice(&raw);
        let address = u64::from_le_bytes(word);
        if address == 0 {
            return Some(out);
        }
        out.push(syscall::read_cstr(space, address, 4096)?);
    }
    Some(out)
}

fn sys_execve(path_ptr: u64, argv_ptr: u64, envp_ptr: u64) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let Some(path) = syscall::read_cstr(&space, path_ptr, 1024) else {
        return errno(EFAULT);
    };
    if path.is_empty() {
        return errno(ENOENT);
    }
    let Some(argv) = read_string_array(&space, argv_ptr, 256) else {
        return errno(EFAULT);
    };
    let Some(envp) = read_string_array(&space, envp_ptr, 256) else {
        return errno(EFAULT);
    };
    // On success `replace` never returns to this program: the SYSCALL stub
    // resumes in the freshly loaded image.
    match crate::exec::replace(&path, &argv, &envp) {
        Ok(()) => 0,
        Err(code) => errno(code),
    }
}

/* ── Process creation ──────────────────────────────────────────────────── */

fn sys_fork(context: &UserContext) -> u64 {
    // fork() is clone(SIGCHLD, 0): the child resumes on the same stack.
    fork_like(context, SIGCHLD, 0, 0)
}

fn sys_clone(context: &UserContext) -> u64 {
    // clone(flags, stack, parent_tid, child_tid, tls)
    fork_like(context, context.a0, context.a1, context.a4)
}

/// Fork-like `clone`: the child receives a private copy of the address space
/// and resumes at the caller's RIP with `rax = 0`, the parent gets the PID.
fn fork_like(context: &UserContext, flags: u64, child_stack: u64, tls: u64) -> u64 {
    // Threads (a shared address space) are not supported yet; neither is
    // vfork's "parent sleeps until exec" contract beyond plain fork.
    if flags & (CLONE_VM | CLONE_THREAD | CLONE_SIGHAND | CLONE_FILES) != 0 {
        return errno(ENOSYS);
    }
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(parent_space) = task::get(index).space else {
        return errno(ESRCH);
    };
    let Some(child_space) = vmm::copy_space(&parent_space) else {
        return errno(ENOMEM);
    };

    let frame = TrapFrame {
        rip: context.rip,
        rsp: if child_stack != 0 { child_stack } else { context.rsp },
        rflags: context.rflags,
        cs: (crate::arch::gdt::USER_CS | 3) as u64,
        ss: (crate::arch::gdt::USER_DS | 3) as u64,
        rax: 0,
        rbx: context.rbx,
        rbp: context.rbp,
        r12: context.r12,
        r13: context.r13,
        r14: context.r14,
        r15: context.r15,
        rdi: context.a0,
        rsi: context.a1,
        rdx: context.a2,
        r10: context.a3,
        r8: context.a4,
        r9: context.a5,
        rcx: context.rip,
        r11: context.rflags,
        ..TrapFrame::default()
    };

    let pid = task::fork_from(index, frame, child_space);
    if pid <= 0 {
        return errno(ENOMEM);
    }
    if let Some(child_index) = task::find(pid) {
        if flags & CLONE_SETTLS != 0 {
            task::get(child_index).fs_base = tls;
        }
    }
    if flags & CLONE_CHILD_SETTID != 0 && context.a3 != 0 {
        if let Some(child_space) = task::find(pid).and_then(|i| task::get(i).space) {
            syscall::copy_to_user(&child_space, context.a3, &(pid as i32).to_le_bytes());
        }
    }
    if flags & CLONE_PARENT_SETTID != 0 && context.a2 != 0 {
        if let Some(space) = syscall::current_space() {
            syscall::copy_to_user(&space, context.a2, &(pid as i32).to_le_bytes());
        }
    }
    pid as u64
}

fn sys_wait4(pid: i64, status_ptr: u64, options: u64, _rusage: u64) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let caller = task::get(index).pid;
    loop {
        let mut found: Option<(i32, i32)> = None;
        let mut has_children = false;
        for child in task::table().iter() {
            if child.ppid != caller || child.pid == caller {
                continue;
            }
            has_children = true;
            if (pid == -1 || pid == 0 || child.pid as i64 == pid)
                && child.state == task::TASK_ZOMBIE
            {
                found = Some((child.pid, child.exit_code));
                break;
            }
        }
        if let Some((child_pid, code)) = found {
            if let Some(child_index) = task::find(child_pid) {
                task::get(child_index).state = task::TASK_DEAD;
            }
            if status_ptr != 0 {
                let Some(space) = syscall::current_space() else {
                    return errno(EFAULT);
                };
                // Normal exit: status = code << 8 (WIFEXITED / WEXITSTATUS).
                let status = ((code & 0xFF) << 8) as i32;
                if !syscall::copy_to_user(&space, status_ptr, &status.to_le_bytes()) {
                    return errno(EFAULT);
                }
            }
            return child_pid as u64;
        }
        if !has_children {
            return errno(ECHILD);
        }
        if options & WNOHANG != 0 {
            return 0;
        }
        sched::block_and_switch(|task| {
            task.state = task::TASK_BLOCKED;
            task.waiting_for = pid as i32;
        });
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
    if let Some((_, readers, _, is_write)) = syscall::pipe_info(entry.node) {
        if !is_write {
            return errno(EBADF);
        }
        if readers <= 0 {
            return errno(EPIPE);
        }
        let mut written = 0usize;
        while written < data.len() {
            let accepted = syscall::pipe_put(entry.node, &data[written..]);
            if accepted == 0 {
                // The pipe is full: let the reader drain it.
                sched::sleep_ms(2);
                continue;
            }
            written += accepted;
        }
        return written as u64;
    }
    // Character devices: writes to /dev/null (and the framebuffer, which is
    // memory-mapped) are discarded but succeed.
    if vfs::is_char_device(entry.node, "null") || vfs::is_char_device(entry.node, "fb0") {
        return data.len() as u64;
    }
    if vfs::is_char_device(entry.node, "kbevents") {
        return errno(EINVAL);
    }
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
    if let Some((_, _, _, is_write)) = syscall::pipe_info(entry.node) {
        if is_write {
            return errno(EBADF);
        }
        let mut temp = vec![0u8; len.min(4096)];
        loop {
            let (buffered, _, writers, _) =
                syscall::pipe_info(entry.node).unwrap_or((0, 0, 0, false));
            if buffered > 0 {
                let taken = syscall::pipe_take(entry.node, &mut temp);
                if taken == 0 {
                    continue;
                }
                return if syscall::copy_to_user(&space, buffer, &temp[..taken]) {
                    taken as u64
                } else {
                    errno(EFAULT)
                };
            }
            if writers <= 0 {
                return 0; // EOF: every write end is closed.
            }
            // Writers are alive but nothing is queued yet: block.
            sched::sleep_ms(5);
        }
    }
    if vfs::is_directory(entry.node) {
        return errno(EISDIR);
    }
    // Character devices: raw keyboard events and /dev/null.
    if vfs::is_char_device(entry.node, "kbevents") {
        if !crate::drivers::keyboard::has_raw_events() {
            return 0;
        }
        let events = (len / 4).min(256);
        let mut buf = Vec::with_capacity(events * 4);
        while buf.len() / 4 < events {
            match crate::drivers::keyboard::next_raw_event() {
                Some((code, ext, pressed)) => buf.extend_from_slice(&[code, ext, pressed, 0]),
                None => break,
            }
        }
        if buf.is_empty() {
            return 0;
        }
        return if syscall::copy_to_user(&space, buffer, &buf) {
            buf.len() as u64
        } else {
            errno(EFAULT)
        };
    }
    if vfs::is_char_device(entry.node, "null") {
        return 0;
    }
    if vfs::is_char_device(entry.node, "fb0") {
        return errno(EINVAL);
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

/// Read an `iovec` array and feed it to the regular read path.  musl uses
/// `readv` for buffered `FILE *` input, which is exactly how DOOM reads the
/// WAD, so a plain `read` is not enough.
fn sys_readv(fd: i32, iov_ptr: u64, count: usize) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let count = count.min(1024);
    let mut total = 0u64;
    for index in 0..count {
        let Some(raw) = syscall::copy_from_user(&space, iov_ptr + index as u64 * 16, 16) else {
            return if total > 0 { total } else { errno(EFAULT) };
        };
        let base = u64::from_le_bytes(raw[0..8].try_into().unwrap());
        let len = u64::from_le_bytes(raw[8..16].try_into().unwrap()) as usize;
        if len == 0 {
            continue;
        }
        let read = sys_read(fd, base, len);
        if (read as i64) < 0 {
            return if total > 0 { total } else { read };
        }
        total += read;
        if (read as usize) < len {
            break;
        }
    }
    total
}

/// Gather-write counterpart of `readv`; musl's `__stdio_write` flushes stdio
/// buffers with it.
fn sys_writev(fd: i32, iov_ptr: u64, count: usize) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let count = count.min(1024);
    let mut total = 0u64;
    for index in 0..count {
        let Some(raw) = syscall::copy_from_user(&space, iov_ptr + index as u64 * 16, 16) else {
            return if total > 0 { total } else { errno(EFAULT) };
        };
        let base = u64::from_le_bytes(raw[0..8].try_into().unwrap());
        let len = u64::from_le_bytes(raw[8..16].try_into().unwrap()) as usize;
        if len == 0 {
            continue;
        }
        let written = sys_write(fd, base, len);
        if (written as i64) < 0 {
            return if total > 0 { total } else { written };
        }
        total += written;
        if (written as usize) < len {
            break;
        }
    }
    total
}

fn sys_mkdir(dirfd: i64, path_ptr: u64) -> u64 {
    let _ = dirfd;
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let Some(path) = syscall::read_cstr(&space, path_ptr, 256) else {
        return errno(EFAULT);
    };
    let cwd = task::get(index).cwd.clone();
    if vfs::resolve(&path, &cwd).is_some() {
        return errno(EEXIST);
    }
    if vfs::mkdir(&path, &cwd) {
        0
    } else {
        errno(ENOENT)
    }
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
    if vfs::is_char_device(node, "kbevents") {
        // Drop shell keystrokes typed before the program opened the device.
        crate::drivers::keyboard::flush_raw();
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

const F_DUPFD: i32 = 0;
const F_GETFD: i32 = 1;
const F_SETFD: i32 = 2;
const F_GETFL: i32 = 3;
const F_SETFL: i32 = 4;

/// Copy the file-description of `fd` into the lowest free slot.
fn sys_dup(fd: i32) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(entry) = task::get_fd(index, fd).copied() else {
        return errno(EBADF);
    };
    let new_fd = task::alloc_fd(index, entry.node, entry.flags);
    if new_fd < 0 {
        return errno(EMFILE);
    }
    if let Some(slot) = task::get_fd(index, new_fd) {
        slot.offset = entry.offset;
    }
    syscall::retain_pipe(entry.node);
    new_fd as u64
}

/// Copy `oldfd` to exactly `newfd`, closing whatever lived there first.
fn sys_dup2(oldfd: i32, newfd: i32) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    if newfd < 0 || newfd as usize >= task::MAX_FDS {
        return errno(EBADF);
    }
    let Some(entry) = task::get_fd(index, oldfd).copied() else {
        return errno(EBADF);
    };
    if oldfd == newfd {
        return newfd as u64;
    }
    if task::get_fd(index, newfd).is_some() {
        syscall::close_fd(index, newfd);
    }
    task::get(index).fds[newfd as usize] = Some(entry);
    syscall::retain_pipe(entry.node);
    newfd as u64
}

/// `dup3(2)`: like `dup2` but rejects equal descriptors. The flags argument
/// only carries `O_CLOEXEC`, which this system does not track.
fn sys_dup3(oldfd: i32, newfd: i32) -> u64 {
    if oldfd == newfd {
        return errno(EINVAL);
    }
    sys_dup2(oldfd, newfd)
}

fn sys_fcntl(fd: i32, command: i32, argument: u64) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(entry) = task::get_fd(index, fd).copied() else {
        return errno(EBADF);
    };
    match command {
        // Duplicate at or above `argument`.
        F_DUPFD => {
            let lowest = argument as usize;
            if lowest >= task::MAX_FDS {
                return errno(EINVAL);
            }
            let task = task::get(index);
            let mut slot = None;
            for candidate in lowest..task::MAX_FDS {
                if task.fds[candidate].is_none() {
                    slot = Some(candidate);
                    break;
                }
            }
            let Some(slot) = slot else {
                return errno(EMFILE);
            };
            task::get(index).fds[slot] = Some(entry);
            syscall::retain_pipe(entry.node);
            slot as u64
        }
        F_GETFD => 0,
        F_SETFD => 0,
        F_GETFL => entry.flags as u64,
        F_SETFL => {
            if let Some(slot) = task::get_fd(index, fd) {
                slot.flags = argument as i32;
            }
            0
        }
        _ => errno(EINVAL),
    }
}

fn sys_pipe(out: u64) -> u64 {
    sys_pipe2(out, 0)
}

fn sys_pipe2(out: u64, _flags: i32) -> u64 {
    let Some(index) = task_index() else {
        return errno(ESRCH);
    };
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let Some((read_fd, write_fd)) = syscall::create_pipe(index) else {
        return errno(ENFILE);
    };
    let mut data = [0u8; 8];
    data[0..4].copy_from_slice(&read_fd.to_le_bytes());
    data[4..8].copy_from_slice(&write_fd.to_le_bytes());
    if !syscall::copy_to_user(&space, out, &data) {
        syscall::close_fd(index, read_fd);
        syscall::close_fd(index, write_fd);
        return errno(EFAULT);
    }
    0
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
    if len == 0 {
        return errno(EINVAL);
    }
    let Some(space) = task::get(index).space else {
        return errno(ENOMEM);
    };
    let len = ((len as u64) + 0xFFF) & !0xFFF;

    // Only the framebuffer is file-backed so far: map its physical pages
    // straight into the caller so a Linux program (DOOM) can draw directly.
    if flags & MAP_ANONYMOUS == 0 {
        if fd < 0 {
            return errno(EINVAL);
        }
        let Some(entry) = task::get_fd(index, fd).copied() else {
            return errno(EBADF);
        };
        if !vfs::is_char_device(entry.node, "fb0") {
            return errno(ENODEV);
        }
        if !crate::drivers::fb::is_ready() {
            return errno(ENODEV);
        }
        if offset < 0 {
            return errno(EINVAL);
        }
        let fb_base = crate::drivers::fb::address();
        let fb_size = crate::drivers::fb::size_bytes() as u64;
        let off = (offset as u64) & !(PAGE_SIZE - 1);
        if off >= fb_size {
            return errno(EINVAL);
        }
        let map_len = len.min(fb_size - off);
        let base = if flags & MAP_FIXED != 0 {
            addr & !0xFFF
        } else {
            task::get(index).mmap_base.max(0x4000_0000)
        };
        if base < 0x4000_0000 {
            return errno(EINVAL);
        }
        if base.checked_add(map_len).map_or(true, |end| end > crate::mem::FOUR_GIB) {
            return errno(ENOMEM);
        }
        let mut page = 0u64;
        while page < map_len {
            let paddr = (fb_base + off + page) & !(PAGE_SIZE - 1);
            if !vmm::map_user_page(&space, base + page, paddr, true) {
                return errno(ENOMEM);
            }
            page += PAGE_SIZE;
        }
        if flags & MAP_FIXED == 0 {
            task::get(index).mmap_base = base + map_len + PAGE_SIZE;
        }
        // Stop the compositor from blitting its backbuffer over the program.
        crate::drivers::fb::claim_direct(task::get(index).pid);
        return base;
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
    if crate::drivers::fb::direct_owner() == pid {
        crate::drivers::fb::release_direct();
    }
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
        FBIOGET_VSCREENINFO => {
            let Some(space) = syscall::current_space() else {
                return errno(EFAULT);
            };
            let mut var = [0u8; 160];
            let w = crate::drivers::fb::width() as u32;
            let h = crate::drivers::fb::height() as u32;
            put_u32(&mut var, 0, w); // xres
            put_u32(&mut var, 4, h); // yres
            put_u32(&mut var, 8, w); // xres_virtual
            put_u32(&mut var, 12, h); // yres_virtual
            put_u32(&mut var, 24, crate::drivers::fb::bpp() as u32);
            // 32-bit BGRX: red 16, green 8, blue 0.
            put_u32(&mut var, 32, 16);
            put_u32(&mut var, 36, 8);
            put_u32(&mut var, 44, 8);
            put_u32(&mut var, 48, 8);
            put_u32(&mut var, 56, 0);
            put_u32(&mut var, 60, 8);
            put_u32(&mut var, 68, 24);
            put_u32(&mut var, 72, 8);
            if syscall::copy_to_user(&space, arg, &var) {
                0
            } else {
                errno(EFAULT)
            }
        }
        // No mode switching: accept and ignore.
        FBIOPUT_VSCREENINFO => 0,
        FBIOGET_FSCREENINFO => {
            let Some(space) = syscall::current_space() else {
                return errno(EFAULT);
            };
            let mut fix = [0u8; 80];
            let id = b"ATRODRAX";
            fix[..id.len()].copy_from_slice(id);
            put_u64(&mut fix, 16, crate::drivers::fb::address());
            put_u32(
                &mut fix,
                24,
                crate::drivers::fb::size_bytes() as u32,
            );
            put_u32(&mut fix, 28, 0); // FB_TYPE_PACKED_PIXELS
            put_u32(&mut fix, 36, 2); // FB_VISUAL_TRUECOLOR
            put_u32(&mut fix, 48, crate::drivers::fb::pitch() as u32);
            if syscall::copy_to_user(&space, arg, &fix) {
                0
            } else {
                errno(EFAULT)
            }
        }
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

/// Wall-clock time with microsecond resolution.  It is anchored once to the
/// CMOS clock and then advanced from the monotonic tick counter, so it never
/// jumps backwards at a second boundary (DOOM's frame timer treats a negative
/// delta as a huge unsigned value and stalls).
fn realtime_us() -> (i64, i64) {
    static mut BASE_SECS: i64 = 0;
    static mut BASE_MS: u64 = 0;
    unsafe {
        let now_ms = crate::sync::uptime_ms();
        if BASE_SECS == 0 {
            BASE_SECS = unix_time();
            BASE_MS = now_ms;
        }
        let elapsed = now_ms.saturating_sub(BASE_MS);
        (
            BASE_SECS + (elapsed / 1000) as i64,
            ((elapsed % 1000) * 1000) as i64,
        )
    }
}

fn sys_gettimeofday(out: u64) -> u64 {
    let Some(space) = syscall::current_space() else {
        return errno(EFAULT);
    };
    let (seconds, micros) = realtime_us();
    let mut buf = [0u8; 16];
    buf[..8].copy_from_slice(&seconds.to_le_bytes());
    buf[8..].copy_from_slice(&micros.to_le_bytes());
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
        let (secs, micros) = realtime_us();
        (secs, micros * 1000)
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
