//! Create ring-3 tasks from ELF files in the VFS.

use crate::arch::gdt;
use crate::arch::idt::TrapFrame;
use crate::elf;
use crate::fs::vfs;
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
    let (rsp, argc, argv_ptr) = build_stack(&space, &arguments);

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
