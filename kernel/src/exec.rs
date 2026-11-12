//! Create ring-3 tasks from ELF files in the VFS.

use crate::arch::gdt;
use crate::arch::idt::TrapFrame;
use crate::elf;
use crate::fs::vfs;
use crate::linux::abi;
use crate::mem::vmm;
use crate::task::{self, Pid};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

// The kernel image now starts at 256 MiB (see kernel/linker.ld), so the user
// stack sits just below it instead of at 256 MiB.
pub const USER_STACK_VIRT: u64 = 0x0F00_0000;
pub const USER_STACK_SIZE: u64 = 64 * 1024;
pub const USER_STACK_TOP: u64 = USER_STACK_VIRT + USER_STACK_SIZE;

fn copy_string(space: &vmm::AddrSpace, at: u64, value: &str) -> bool {
    let bytes = value.as_bytes();
    if !vmm::write_user(space, at, bytes) {
        return false;
    }
    vmm::write_user(space, at + bytes.len() as u64, &[0])
}

fn write_u64(space: &vmm::AddrSpace, at: u64, value: u64) -> bool {
    vmm::write_user(space, at, &value.to_le_bytes())
}

/// Build the System V style argument block on the user stack. Native AtrodraX
/// crt0 ignores it, but argv-aware programs (shell, doom) expect rdi/rsi.
fn build_stack(space: &vmm::AddrSpace, argv: &[String]) -> (u64, u64, u64) {
    let mut string_area = USER_STACK_TOP - 16;
    let mut pointers: Vec<u64> = Vec::new();
    for argument in argv.iter().rev() {
        let size = argument.len() as u64 + 1;
        string_area -= size;
        string_area &= !0xF;
        copy_string(space, string_area, argument);
        pointers.push(string_area);
    }
    pointers.reverse();

    let pointer_area = (string_area - (pointers.len() as u64 + 1) * 8) & !0xF;
    for (index, pointer) in pointers.iter().enumerate() {
        write_u64(space, pointer_area + index as u64 * 8, *pointer);
    }
    write_u64(space, pointer_area + pointers.len() as u64 * 8, 0);

    let rsp = pointer_area - 8;
    (rsp, argv.len() as u64, pointer_area)
}

/// Linux process startup block: `argc`, `argv`, `envp` and `auxv` on the
/// stack, with `rsp` pointing at `argc`. This is what libc `_start` expects
/// (the native AtrodraX crt0 instead takes `rdi`/`rsi`).
fn build_linux_stack(
    space: &vmm::AddrSpace,
    argv: &[String],
    env: &[String],
    loaded: &elf::Loaded,
) -> u64 {
    let mut cursor = USER_STACK_TOP - 16;
    let mut pointers: Vec<u64> = Vec::new();
    for argument in argv.iter().rev() {
        let size = argument.len() as u64 + 1;
        cursor -= size;
        cursor &= !0xF;
        copy_string(space, cursor, argument);
        pointers.push(cursor);
    }
    pointers.reverse();

    let mut env_pointers: Vec<u64> = Vec::new();
    for variable in env.iter().rev() {
        let size = variable.len() as u64 + 1;
        cursor -= size;
        cursor &= !0xF;
        copy_string(space, cursor, variable);
        env_pointers.push(cursor);
    }
    env_pointers.reverse();

    // AT_RANDOM points at 16 bytes libc may use to seed its canary.
    cursor -= 16;
    cursor &= !0xF;
    let random_at = cursor;
    let mut seed = crate::arch::cpu::rdtsc() ^ 0xA5A5_5A5A_1234_5678;
    let mut random = [0u8; 16];
    for byte in random.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        *byte = seed as u8;
    }
    vmm::write_user(space, random_at, &random);

    let auxv: [(u64, u64); 12] = [
        (abi::AT_PHDR, loaded.phdr),
        (abi::AT_PHENT, loaded.phent),
        (abi::AT_PHNUM, loaded.phnum),
        (abi::AT_PAGESZ, abi::PAGE_SIZE),
        (abi::AT_BASE, 0),
        (abi::AT_ENTRY, loaded.entry),
        (abi::AT_RANDOM, random_at),
        (abi::AT_UID, 0),
        (abi::AT_EUID, 0),
        (abi::AT_GID, 0),
        (abi::AT_EGID, 0),
        (abi::AT_SECURE, 0),
    ];

    let words = 1
        + pointers.len() as u64
        + 1
        + env_pointers.len() as u64
        + 1
        + auxv.len() as u64 * 2
        + 2;
    let sp = (cursor - words * 8) & !0xF;

    write_u64(space, sp, pointers.len() as u64);
    let mut at = sp + 8;
    for pointer in &pointers {
        write_u64(space, at, *pointer);
        at += 8;
    }
    write_u64(space, at, 0); // argv terminator
    at += 8;
    for pointer in &env_pointers {
        write_u64(space, at, *pointer);
        at += 8;
    }
    write_u64(space, at, 0); // envp terminator
    at += 8;
    for (kind, value) in auxv {
        write_u64(space, at, kind);
        write_u64(space, at + 8, value);
        at += 16;
    }
    write_u64(space, at, abi::AT_NULL);
    write_u64(space, at + 8, 0);
    sp
}

fn resolve_path(path: &str) -> Option<usize> {
    if let Some(index) = vfs::resolve(path, "/") {
        return Some(index);
    }
    if !path.starts_with('/') {
        if let Some(index) = vfs::resolve(&alloc::format!("/{}", path), "/") {
            return Some(index);
        }
        if let Some(index) = vfs::resolve(&alloc::format!("/bin/{}", path), "/") {
            return Some(index);
        }
    } else {
        // The userspace shell builds `/name` for bare commands.  Programs are
        // installed under /bin, so retry there when the path has no directory
        // component of its own.
        let trimmed = path.trim_start_matches('/');
        let bare = trimmed.strip_prefix("./").unwrap_or(trimmed);
        if !bare.is_empty() && !bare.contains('/') {
            if let Some(index) = vfs::resolve(&alloc::format!("/bin/{}", bare), "/") {
                return Some(index);
            }
        }
    }
    None
}

fn base_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .unwrap_or(trimmed)
        .into()
}

/// Environment handed to programs started by the native shell.
pub fn default_env() -> Vec<String> {
    vec![
        String::from("PATH=/bin:/usr/bin:/sbin"),
        String::from("HOME=/home/user"),
        String::from("TERM=linux"),
        String::from("USER=root"),
    ]
}

struct Prepared {
    space: vmm::AddrSpace,
    loaded: elf::Loaded,
    rsp: u64,
    argc: u64,
    argv_ptr: u64,
}

/// Load `path` into a fresh address space and build its initial stack.
fn prepare(path: &str, argv: &[String], env: &[String]) -> Result<Prepared, i64> {
    let node = resolve_path(path).ok_or(abi::ENOENT)?;
    if vfs::is_directory(node) {
        return Err(abi::EISDIR);
    }
    let image = vfs::read_all(node);
    if image.is_empty() || !elf::is_elf(&image) {
        return Err(abi::ENOEXEC);
    }
    let space = vmm::create().ok_or(abi::ENOMEM)?;
    let loaded = match elf::load(&space, &image) {
        Some(loaded) => loaded,
        None => {
            vmm::destroy(space);
            return Err(abi::ENOEXEC);
        }
    };
    if !vmm::map_user_range(&space, USER_STACK_VIRT, USER_STACK_TOP, true) {
        vmm::destroy(space);
        return Err(abi::ENOMEM);
    }
    let arguments: Vec<String> = if argv.is_empty() {
        vec![base_name(path)]
    } else {
        argv.to_vec()
    };
    let (rsp, argc, argv_ptr) = if loaded.linux_compat {
        (build_linux_stack(&space, &arguments, env, &loaded), 0, 0)
    } else {
        build_stack(&space, &arguments)
    };
    Ok(Prepared {
        space,
        loaded,
        rsp,
        argc,
        argv_ptr,
    })
}

/// Load `path` and create a new task. Returns the PID, or -1 on failure.
pub fn spawn(path: &str, argv: &[String]) -> Pid {
    let env = default_env();
    let prepared = match prepare(path, argv, &env) {
        Ok(prepared) => prepared,
        Err(code) => {
            crate::kprintln!("[exec] {}: cannot start (errno {})", path, code);
            return -1;
        }
    };
    let Prepared {
        space,
        loaded,
        rsp,
        argc,
        argv_ptr,
    } = prepared;

    let frame = TrapFrame {
        rip: loaded.entry,
        cs: (gdt::USER_CS | 3) as u64,
        rflags: 0x202,
        rsp,
        ss: (gdt::USER_DS | 3) as u64,
        rdi: argc,
        rsi: argv_ptr,
        rax: 0,
        ..TrapFrame::default()
    };

    let name = base_name(path);
    let pid = task::create_user(&name, frame, space, &name);
    if let Some(index) = task::find(pid) {
        let t = task::get(index);
        t.brk = loaded.brk_start;
        t.brk_start = loaded.brk_start;
        t.mmap_base = 0x4000_0000;
        t.linux_compat = loaded.linux_compat;
    }
    crate::kprintln!("[exec] pid {} = {} (entry {:#x})", pid, path, loaded.entry);
    pid
}

/// `execve(2)`: replace the calling task's image in place.
///
/// The new address space is built first; only then is the old one torn down,
/// so a failing `execve` leaves the caller untouched. Success is reported by
/// telling the SYSCALL stub to return into the new image (see
/// [`crate::linux::request_exec_frame`]) - this function never returns to the
/// old program.
pub fn replace(path: &str, argv: &[String], env: &[String]) -> Result<(), i64> {
    let prepared = prepare(path, argv, env)?;
    if !prepared.loaded.linux_compat {
        // A Linux process cannot turn itself into a native AtrodraX binary:
        // the entry conventions differ.
        vmm::destroy(prepared.space);
        return Err(abi::ENOEXEC);
    }

    let index = crate::sched::current_index();
    let task = task::get(index);
    let old_space = task.space;
    task.space = Some(prepared.space);
    task.brk = prepared.loaded.brk_start;
    task.brk_start = prepared.loaded.brk_start;
    task.mmap_base = 0x4000_0000;
    task.linux_compat = true;
    task.fs_base = 0;
    task.name = base_name(path);

    crate::linux::request_exec_frame(prepared.rsp, prepared.loaded.entry, 0x202);
    vmm::load(prepared.space.pml4_phys);
    if let Some(old) = old_space {
        vmm::destroy(old);
    }
    crate::kprintln!(
        "[exec] pid {} execve {} (entry {:#x})",
        task::get(index).pid,
        path,
        prepared.loaded.entry
    );
    Ok(())
}

pub fn spawn_path(path: &str) -> Pid {
    spawn(path, &[])
}

pub fn spawn_with_args(path: &str, args: &[&str]) -> Pid {
    let owned: Vec<String> = if args.is_empty() {
        vec![base_name(path)]
    } else {
        args.iter().map(|s| String::from(*s)).collect()
    };
    spawn(path, &owned)
}

pub fn print_info() {
    crate::kprintln!(
        "[exec] user stack {:#x}..{:#x}, {} tasks",
        USER_STACK_VIRT,
        USER_STACK_TOP,
        task::count()
    );
}
