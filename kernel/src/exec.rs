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

pub const USER_STACK_VIRT: u64 = 0x1000_0000;
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
fn build_linux_stack(space: &vmm::AddrSpace, argv: &[String], loaded: &elf::Loaded) -> u64 {
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

    let words = 1 + pointers.len() as u64 + 1 + 1 + auxv.len() as u64 * 2 + 2;
    let sp = (cursor - words * 8) & !0xF;

    write_u64(space, sp, pointers.len() as u64);
    let mut at = sp + 8;
    for pointer in &pointers {
        write_u64(space, at, *pointer);
        at += 8;
    }
    write_u64(space, at, 0); // argv terminator
    at += 8;
    write_u64(space, at, 0); // envp terminator (empty environment)
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

/// Load `path` and create a new task. Returns the PID, or -1 on failure.
pub fn spawn(path: &str, argv: &[String]) -> Pid {
    let node = match resolve_path(path) {
        Some(node) => node,
        None => {
            crate::kprintln!("[exec] {}: not found", path);
            return -1;
        }
    };
    if vfs::is_directory(node) {
        crate::kprintln!("[exec] {}: is a directory", path);
        return -1;
    }
    let image = vfs::read_all(node);
    if image.is_empty() {
        crate::kprintln!("[exec] {}: empty image", path);
        return -1;
    }
    if !elf::is_elf(&image) {
        crate::kprintln!("[exec] {}: not an ELF executable", path);
        return -1;
    }

    let space = match vmm::create() {
        Some(space) => space,
        None => {
            crate::kprintln!("[exec] out of memory creating address space");
            return -1;
        }
    };

    let loaded = match elf::load(&space, &image) {
        Some(loaded) => loaded,
        None => {
            vmm::destroy(space);
            return -1;
        }
    };

    if !vmm::map_user_range(&space, USER_STACK_VIRT, USER_STACK_TOP, true) {
        crate::kprintln!("[exec] failed to map user stack");
        vmm::destroy(space);
        return -1;
    }

    let arguments: Vec<String> = if argv.is_empty() {
        vec![base_name(path)]
    } else {
        argv.to_vec()
    };
    let (rsp, argc, argv_ptr) = if loaded.linux_compat {
        (build_linux_stack(&space, &arguments, &loaded), 0, 0)
    } else {
        build_stack(&space, &arguments)
    };

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
