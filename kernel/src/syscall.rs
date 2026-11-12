//! Native AtrodraX syscall ABI (int 0x80).
//!
//! `rax` selects the call, `rdi`/`rsi`/`rdx`/`r10` carry arguments and `rax`
//! carries the result. The numbers and semantics match the C edition.

use crate::apps;
use crate::arch::idt::TrapFrame;
use crate::fs::vfs;
use crate::gfx::{theme, window};
use crate::mem::vmm::{self, AddrSpace};
use crate::net;
use crate::sched;
use crate::task::{self, Fd, TASK_DEAD, TASK_SLEEPING, TASK_ZOMBIE};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

pub const SYS_EXIT: u64 = 0;
pub const SYS_PRINT: u64 = 1;
pub const SYS_GETPID: u64 = 2;
pub const SYS_SLEEP: u64 = 3;
pub const SYS_OPEN: u64 = 4;
pub const SYS_READ: u64 = 5;
pub const SYS_WRITE: u64 = 6;
pub const SYS_CLOSE: u64 = 7;
pub const SYS_EXEC: u64 = 8;
pub const SYS_FORK: u64 = 9;
pub const SYS_WAIT: u64 = 10;
pub const SYS_KILL: u64 = 11;
pub const SYS_BRK: u64 = 12;
pub const SYS_READDIR: u64 = 13;
pub const SYS_STAT: u64 = 14;
pub const SYS_DUP: u64 = 15;
pub const SYS_PIPE: u64 = 16;
pub const SYS_GUI: u64 = 17;
pub const SYS_THEME: u64 = 18;
pub const SYS_NET: u64 = 19;
pub const SYS_SOCKET: u64 = 20;
pub const SYS_BIND: u64 = 21;
pub const SYS_CONNECT: u64 = 22;
pub const SYS_SEND: u64 = 23;
pub const SYS_RECV: u64 = 24;
pub const SYS_SENDTO: u64 = 25;
pub const SYS_RECVFROM: u64 = 26;
pub const SYS_SOCKCLOSE: u64 = 27;
pub const SYS_LISTEN: u64 = 28;
pub const SYS_ACCEPT: u64 = 29;
pub const SYS_SOCKSTATE: u64 = 30;
pub const SYS_PROCINFO: u64 = 31;
pub const SYS_EXEC_ARGV: u64 = 32;
pub const SYS_FCNTL: u64 = 33;
pub const SYS_POLL: u64 = 34;
pub const SYS_SYNC: u64 = 35;
pub const SYS_LSEEK: u64 = 36;
pub const SYS_UNLINK: u64 = 37;
pub const SYS_MKDIR: u64 = 38;
pub const SYS_CHDIR: u64 = 39;
pub const SYS_GETCWD: u64 = 40;
pub const SYS_TRUNCATE: u64 = 41;

const O_CREAT: i32 = 0x40;
const O_TRUNC: i32 = 0x200;
const O_APPEND: i32 = 0x400;

const CONSOLE_NODE: usize = crate::task::CONSOLE_NODE;
const PIPE_BASE: usize = 0x4000_0000;
const PIPE_CAP: usize = 4096;

const KIND_EXTERNAL: u8 = 200;

/* ── Pipes ─────────────────────────────────────────────────────────────── */

struct Pipe {
    buffer: Vec<u8>,
    read_refs: i32,
    write_refs: i32,
}

static mut PIPES: Option<Vec<Option<Pipe>>> = None;

fn pipes() -> &'static mut Vec<Option<Pipe>> {
    unsafe {
        if PIPES.is_none() {
            PIPES = Some(Vec::new());
        }
        PIPES.as_mut().unwrap()
    }
}

fn pipe_node(id: usize, write: bool) -> usize {
    PIPE_BASE | (id << 1) | if write { 1 } else { 0 }
}

fn pipe_parts(node: usize) -> Option<(usize, bool)> {
    if node < PIPE_BASE {
        return None;
    }
    let raw = node - PIPE_BASE;
    Some((raw >> 1, raw & 1 != 0))
}

fn pipe_read(node: usize, max: usize) -> i32 {
    let Some((id, write)) = pipe_parts(node) else {
        return -1;
    };
    if write {
        return -1;
    }
    let list = pipes();
    let Some(Some(pipe)) = list.get_mut(id) else {
        return -1;
    };
    let take = max.min(pipe.buffer.len());
    pipe.buffer.drain(..take);
    take as i32
}

fn pipe_write(node: usize, data: &[u8]) -> i32 {
    let Some((id, write)) = pipe_parts(node) else {
        return -1;
    };
    if !write {
        return -1;
    }
    let list = pipes();
    let Some(Some(pipe)) = list.get_mut(id) else {
        return -1;
    };
    if pipe.read_refs <= 0 {
        return -1;
    }
    let space = PIPE_CAP.saturating_sub(pipe.buffer.len());
    let take = data.len().min(space);
    pipe.buffer.extend_from_slice(&data[..take]);
    take as i32
}

fn pipe_release(node: usize) {
    let Some((id, write)) = pipe_parts(node) else {
        return;
    };
    let list = pipes();
    if let Some(Some(pipe)) = list.get_mut(id) {
        if write {
            pipe.write_refs -= 1;
        } else {
            pipe.read_refs -= 1;
        }
    }
}

/* ── GUI event queue ───────────────────────────────────────────────────── */

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GuiMsg {
    version: u32,
    kind: u32,
    win_id: u32,
    flags: u32,
    arg0: u64,
    arg1: u64,
    arg2: u64,
    sender_pid: i32,
}

static mut GUI_EVENTS: Option<Vec<(i32, GuiMsg)>> = None;
static mut GUI_TEXT_PENDING: Option<(i32, u32, i32, i32, u32, u32)> = None;

fn gui_events() -> &'static mut Vec<(i32, GuiMsg)> {
    unsafe {
        if GUI_EVENTS.is_none() {
            GUI_EVENTS = Some(Vec::new());
        }
        GUI_EVENTS.as_mut().unwrap()
    }
}

pub fn gui_push_key(pid: i32, win: u32, ch: i32) {
    gui_events().push((
        pid,
        GuiMsg {
            version: 1,
            kind: 35,
            win_id: win,
            flags: 0,
            arg0: ch as u32 as u64,
            arg1: 0,
            arg2: 0,
            sender_pid: 0,
        },
    ));
}

pub fn gui_push_mouse(pid: i32, win: u32, x: i32, y: i32, down: bool) {
    gui_events().push((
        pid,
        GuiMsg {
            version: 1,
            kind: if down { 33 } else { 32 },
            win_id: win,
            flags: 0,
            arg0: ((x as u32 as u64) << 32) | (y as u32 as u64),
            arg1: 0,
            arg2: 0,
            sender_pid: 0,
        },
    ));
}

pub fn gui_forget_window(pid: i32, win: u32) {
    gui_events().retain(|(owner, message)| *owner != pid || message.win_id != win);
}

/* ── user memory helpers ───────────────────────────────────────────────── */

pub(crate) fn current_index() -> Option<usize> {
    if sched::current().is_some() {
        Some(sched::current_index())
    } else {
        None
    }
}

pub(crate) fn current_space() -> Option<AddrSpace> {
    sched::current().and_then(|t| t.space)
}

pub(crate) fn copy_from_user(space: &AddrSpace, address: u64, len: usize) -> Option<Vec<u8>> {
    if len == 0 {
        return Some(Vec::new());
    }
    if address == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(len);
    let mut offset = 0usize;
    while offset < len {
        let va = address + offset as u64;
        let physical = vmm::translate(space, va)?;
        let page_offset = (va & 0xFFF) as usize;
        let chunk = (4096 - page_offset).min(len - offset);
        unsafe {
            let slice =
                core::slice::from_raw_parts(vmm::phys_to_virt(physical) as *const u8, chunk);
            out.extend_from_slice(slice);
        }
        offset += chunk;
    }
    Some(out)
}

pub(crate) fn copy_to_user(space: &AddrSpace, address: u64, data: &[u8]) -> bool {
    vmm::write_user(space, address, data)
}

pub(crate) fn read_cstr(space: &AddrSpace, address: u64, max: usize) -> Option<String> {
    let mut out = Vec::new();
    for index in 0..max {
        let physical = vmm::translate(space, address + index as u64)?;
        let byte = unsafe { *(vmm::phys_to_virt(physical) as *const u8) };
        if byte == 0 {
            break;
        }
        out.push(byte);
    }
    String::from_utf8(out).ok()
}

fn read_u32_user(space: &AddrSpace, address: u64) -> Option<u32> {
    let bytes = copy_from_user(space, address, 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn write_u32_user(space: &AddrSpace, address: u64, value: u32) -> bool {
    copy_to_user(space, address, &value.to_le_bytes())
}

fn write_i32_user(space: &AddrSpace, address: u64, value: i32) -> bool {
    copy_to_user(space, address, &value.to_le_bytes())
}

fn ensure_std_fds(index: usize) {
    let t = task::get(index);
    for slot in 0..3 {
        if t.fds[slot].is_none() {
            t.fds[slot] = Some(Fd {
                node: CONSOLE_NODE,
                offset: 0,
                flags: 0,
            });
        }
    }
}

fn task_pid(index: usize) -> i32 {
    task::get(index).pid
}

/* ── file descriptor helpers ───────────────────────────────────────────── */

fn fd_entry(index: usize, fd: i32) -> Option<Fd> {
    task::get_fd(index, fd).copied()
}

fn fd_read(index: usize, fd: i32, max: usize) -> i32 {
    let Some(entry) = fd_entry(index, fd) else {
        return -1;
    };
    if entry.node == CONSOLE_NODE {
        let mut temp = vec![0u8; max.min(4096)];
        let n = apps::stdio_read(task_pid(index), &mut temp);
        if n > 0 {
            return n;
        }
        sched::yield_now();
        return 0;
    }
    if pipe_parts(entry.node).is_some() {
        let mut temp = vec![0u8; max.min(4096)];
        let n = pipe_read(entry.node, temp.len());
        if n <= 0 {
            return 0;
        }
        temp.truncate(n as usize);
        let space = match current_space() {
            Some(space) => space,
            None => return -1,
        };
        // The caller copies out afterwards; stash length through buffer path.
        let _ = space;
        return n;
    }
    let data = vfs::read(entry.node, entry.offset, max);
    let space = match current_space() {
        Some(space) => space,
        None => return -1,
    };
    if !copy_to_user(&space, fd_last_buffer_ptr(), &data) {
        return -1;
    }
    if let Some(entry) = task::get_fd(index, fd) {
        entry.offset += data.len();
    }
    let _ = max;
    data.len() as i32
}

// Kept for clarity: syscall_write/read use explicit buffers, so this function
// is only a placeholder for the generic fd path.
fn fd_last_buffer_ptr() -> u64 {
    0
}

fn fd_write(index: usize, fd: i32, data: &[u8]) -> i32 {
    let Some(entry) = fd_entry(index, fd) else {
        return -1;
    };
    if entry.node == CONSOLE_NODE {
        let n = apps::stdio_write(task_pid(index), data);
        if n >= 0 {
            return n;
        }
        let text = String::from_utf8_lossy(data);
        crate::arch::serial::write_str(&text);
        return data.len() as i32;
    }
    if pipe_parts(entry.node).is_some() {
        return pipe_write(entry.node, data);
    }
    let n = vfs::write(entry.node, entry.offset, data);
    if let Some(entry) = task::get_fd(index, fd) {
        entry.offset += n;
    }
    n as i32
}

pub(crate) fn close_fd(index: usize, fd: i32) -> i32 {
    let Some(entry) = fd_entry(index, fd) else {
        return -1;
    };
    if pipe_parts(entry.node).is_some() {
        pipe_release(entry.node);
    }
    if task::close_fd(index, fd) {
        0
    } else {
        -1
    }
}

/* ── syscall implementations ───────────────────────────────────────────── */

fn sys_open(index: usize, path: &str, flags: i32) -> i64 {
    ensure_std_fds(index);
    let cwd = task::get(index).cwd.clone();
    let mut node = vfs::resolve(path, &cwd);
    if node.is_none() && flags & O_CREAT != 0 {
        node = vfs::create_file_path(path, &cwd);
    }
    let Some(node) = node else {
        return -1;
    };
    if vfs::is_directory(node) && flags & 0x3 != 0 {
        return -1;
    }
    if flags & O_TRUNC != 0 && !vfs::is_directory(node) {
        vfs::truncate(node, 0);
    }
    let fd = task::alloc_fd(index, node, 0);
    if fd < 0 {
        return -1;
    }
    if flags & O_APPEND != 0 {
        if let Some(entry) = task::get_fd(index, fd) {
            entry.offset = vfs::size_of(node);
        }
    }
    fd as i64
}

fn sys_read(f: &mut TrapFrame, index: usize) -> u64 {
    let fd = f.rdi as i32;
    let buffer = f.rsi;
    let len = f.rdx as usize;
    if len == 0 {
        return 0;
    }
    let Some(space) = current_space() else {
        return u64::MAX;
    };
    let Some(entry) = fd_entry(index, fd) else {
        return u64::MAX;
    };
    if entry.node == CONSOLE_NODE {
        let mut temp = vec![0u8; len.min(4096)];
        let n = apps::stdio_read(task_pid(index), &mut temp);
        if n > 0 {
            if !copy_to_user(&space, buffer, &temp[..n as usize]) {
                return u64::MAX;
            }
            return n as u64;
        }
        sched::yield_now();
        return 0;
    }
    if pipe_parts(entry.node).is_some() {
        let mut temp = vec![0u8; len.min(4096)];
        let n = pipe_read(entry.node, temp.len());
        if n <= 0 {
            return 0;
        }
        if !copy_to_user(&space, buffer, &temp[..n as usize]) {
            return u64::MAX;
        }
        return n as u64;
    }
    let data = vfs::read(entry.node, entry.offset, len);
    if !copy_to_user(&space, buffer, &data) {
        return u64::MAX;
    }
    if let Some(entry) = task::get_fd(index, fd) {
        entry.offset += data.len();
    }
    data.len() as u64
}

fn sys_write(f: &mut TrapFrame, index: usize) -> u64 {
    let fd = f.rdi as i32;
    let buffer = f.rsi;
    let len = f.rdx as usize;
    if len == 0 {
        return 0;
    }
    let Some(space) = current_space() else {
        return u64::MAX;
    };
    let Some(data) = copy_from_user(&space, buffer, len.min(1024 * 1024)) else {
        return u64::MAX;
    };
    if fd == 1 || fd == 2 {
        let console = fd_entry(index, fd)
            .map(|entry| entry.node == CONSOLE_NODE)
            .unwrap_or(true);
        if console {
            let n = apps::stdio_write(task_pid(index), &data);
            if n >= 0 {
                return n as u64;
            }
            let text = String::from_utf8_lossy(&data);
            crate::arch::serial::write_str(&text);
            return data.len() as u64;
        }
    }
    let n = fd_write(index, fd, &data);
    if n < 0 {
        u64::MAX
    } else {
        n as u64
    }
}

fn sys_exec(index: usize, path: &str, argv: &[String]) -> i64 {
    let cwd = task::get(index).cwd.clone();
    let full = if path.starts_with('/') {
        path.to_string()
    } else if vfs::resolve(path, &cwd).is_some() {
        vfs::join_cwd(&cwd, path)
    } else {
        format!("/bin/{}", path)
    };
    let pid = crate::exec::spawn(&full, argv);
    if pid > 0 {
        if let Some(child) = task::find(pid) {
            task::get(child).ppid = task_pid(index);
        }
    }
    pid as i64
}

fn sys_fork(f: &TrapFrame, index: usize) -> i64 {
    let parent_pid = task::get(index).pid;
    let parent_name = task::get(index).name.clone();
    let parent_cwd = task::get(index).cwd.clone();
    let (brk, brk_start, linux_compat, fs_base, mmap_base) = {
        let parent = task::get(index);
        (
            parent.brk,
            parent.brk_start,
            parent.linux_compat,
            parent.fs_base,
            parent.mmap_base,
        )
    };
    let Some(parent_space) = task::get(index).space else {
        return -1;
    };
    let Some(child_space) = vmm::copy_space(&parent_space) else {
        return -1;
    };
    let mut frame = *f;
    frame.rax = 0;
    let child = task::create_user(&parent_name, frame, child_space, &parent_name);
    if let Some(child_index) = task::find(child) {
        let fds = task::get(index).fds.clone();
        let child_task = task::get(child_index);
        child_task.ppid = parent_pid;
        child_task.cwd = parent_cwd;
        child_task.brk = brk;
        child_task.brk_start = brk_start;
        child_task.mmap_base = mmap_base;
        child_task.linux_compat = linux_compat;
        child_task.fs_base = fs_base;
        child_task.fds = fds;
        for entry in child_task.fds.iter().flatten() {
            if pipe_parts(entry.node).is_some() {
                let Some((id, write)) = pipe_parts(entry.node) else { continue };
                let list = pipes();
                if let Some(Some(pipe)) = list.get_mut(id) {
                    if write {
                        pipe.write_refs += 1;
                    } else {
                        pipe.read_refs += 1;
                    }
                }
            }
        }
    }
    child as i64
}

fn sys_wait(index: usize, pid: i32, status_out: u64) -> i64 {
    let caller = task_pid(index);
    let mut has_children = false;
    let mut found: Option<(i32, i32)> = None;
    for t in task::table().iter() {
        if t.ppid == caller && t.pid != caller {
            has_children = true;
            if (pid == -1 || t.pid == pid) && t.state == TASK_ZOMBIE {
                found = Some((t.pid, t.exit_code));
                break;
            }
        }
    }
    if let Some((child_pid, code)) = found {
        if let Some(child_index) = task::find(child_pid) {
            task::get(child_index).state = TASK_DEAD;
        }
        if status_out != 0 {
            if let Some(space) = current_space() {
                write_i32_user(&space, status_out, code);
            }
        }
        return child_pid as i64;
    }
    if has_children {
        -2
    } else {
        -1
    }
}

fn sys_kill(pid: i32) -> i64 {
    match task::find(pid) {
        Some(index) => {
            let parent = task::get(index).ppid;
            task::get(index).state = TASK_DEAD;
            if let Some(parent_index) = task::find(parent) {
                if task::get(parent_index).state == 2 {
                    task::get(parent_index).state = task::TASK_READY;
                }
            }
            0
        }
        None => -1,
    }
}

fn sys_brk(index: usize, requested: u64) -> u64 {
    let (mut current, start) = {
        let t = task::get(index);
        (t.brk, t.brk_start)
    };
    if current == 0 {
        current = if start != 0 { start } else { 0x2000_0000 };
        task::get(index).brk = current;
        task::get(index).brk_start = current;
    }
    if requested == 0 {
        return current;
    }
    if requested > 0x3000_0000 {
        return current;
    }
    let Some(space) = task::get(index).space else {
        return current;
    };
    if requested > current {
        let old_page = (current + 0xFFF) & !0xFFF;
        let new_page = (requested + 0xFFF) & !0xFFF;
        if new_page > old_page {
            vmm::map_user_range(&space, old_page, new_page, true);
        }
    }
    task::get(index).brk = requested;
    requested
}

fn sys_readdir(path: &str, buffer: u64, len: usize, space: &AddrSpace) -> i64 {
    let cwd = current_index()
        .map(|index| task::get(index).cwd.clone())
        .unwrap_or_else(|| String::from("/"));
    let Some(index) = vfs::resolve(path, &cwd) else {
        return -1;
    };
    if !vfs::is_directory(index) {
        return -1;
    }
    let mut out = Vec::new();
    for (name, _kind, _child) in vfs::list(index) {
        if out.len() + name.len() + 2 > len {
            break;
        }
        out.extend_from_slice(name.as_bytes());
        out.push(0);
    }
    if out.len() < len {
        out.push(0);
    }
    if !copy_to_user(space, buffer, &out) {
        return -1;
    }
    out.len() as i64
}

fn sys_stat(path: &str, out: u64, space: &AddrSpace) -> i64 {
    let cwd = current_index()
        .map(|index| task::get(index).cwd.clone())
        .unwrap_or_else(|| String::from("/"));
    let Some(index) = vfs::resolve(path, &cwd) else {
        return -1;
    };
    let directory = vfs::is_directory(index);
    let mode: u32 = if directory { 0x4000 | 0x01C0 } else { 0x8000 | 0x01C0 };
    let size = vfs::size_of(index) as u32;
    let kind: u32 = if directory { 1 } else { 0 };
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&mode.to_le_bytes());
    data.extend_from_slice(&size.to_le_bytes());
    data.extend_from_slice(&kind.to_le_bytes());
    if !copy_to_user(space, out, &data) {
        return -1;
    }
    0
}

fn sys_dup(index: usize, old_fd: i32, new_fd: i32) -> i64 {
    ensure_std_fds(index);
    let Some(entry) = fd_entry(index, old_fd) else {
        return -1;
    };
    if new_fd == -1 {
        let fd = task::alloc_fd(index, entry.node, entry.flags);
        if fd < 0 {
            return -1;
        }
        if let Some(dest) = task::get_fd(index, fd) {
            dest.offset = entry.offset;
            dest.flags = entry.flags;
        }
        if pipe_parts(entry.node).is_some() {
            let Some((id, write)) = pipe_parts(entry.node) else { return fd as i64 };
            let list = pipes();
            if let Some(Some(pipe)) = list.get_mut(id) {
                if write {
                    pipe.write_refs += 1;
                } else {
                    pipe.read_refs += 1;
                }
            }
        }
        return fd as i64;
    }
    if new_fd < 0 || new_fd as usize >= task::MAX_FDS {
        return -1;
    }
    if old_fd == new_fd {
        return new_fd as i64;
    }
    close_fd(index, new_fd);
    let t = task::get(index);
    t.fds[new_fd as usize] = Some(Fd {
        node: entry.node,
        offset: entry.offset,
        flags: entry.flags,
    });
    if pipe_parts(entry.node).is_some() {
        let Some((id, write)) = pipe_parts(entry.node) else { return new_fd as i64 };
        let list = pipes();
        if let Some(Some(pipe)) = list.get_mut(id) {
            if write {
                pipe.write_refs += 1;
            } else {
                pipe.read_refs += 1;
            }
        }
    }
    new_fd as i64
}

fn sys_pipe(index: usize, out: u64) -> i64 {
    let Some(space) = current_space() else {
        return -1;
    };
    let list = pipes();
    let id = list.len();
    list.push(Some(Pipe {
        buffer: Vec::new(),
        read_refs: 1,
        write_refs: 1,
    }));
    ensure_std_fds(index);
    let read_fd = task::alloc_fd(index, pipe_node(id, false), 0);
    let write_fd = task::alloc_fd(index, pipe_node(id, true), 0);
    if read_fd < 0 || write_fd < 0 {
        if read_fd >= 0 {
            task::close_fd(index, read_fd);
        }
        return -1;
    }
    let mut data = Vec::with_capacity(8);
    data.extend_from_slice(&read_fd.to_le_bytes());
    data.extend_from_slice(&write_fd.to_le_bytes());
    if !copy_to_user(&space, out, &data) {
        return -1;
    }
    0
}

fn sys_lseek(index: usize, fd: i32, offset: i64, whence: i32) -> i64 {
    let Some(entry) = fd_entry(index, fd) else {
        return -1;
    };
    if entry.node == CONSOLE_NODE || pipe_parts(entry.node).is_some() || vfs::is_directory(entry.node) {
        return -1;
    }
    let base = match whence {
        0 => 0i64,
        1 => entry.offset as i64,
        2 => vfs::size_of(entry.node) as i64,
        _ => return -1,
    };
    let position = base + offset;
    if position < 0 || position > 64 * 1024 * 1024 {
        return -1;
    }
    if let Some(entry) = task::get_fd(index, fd) {
        entry.offset = position as usize;
    }
    position
}

fn sys_getcwd(index: usize, out: u64, size: usize) -> i64 {
    let Some(space) = current_space() else {
        return -1;
    };
    let cwd = task::get(index).cwd.clone();
    let need = cwd.len() + 1;
    if size < need {
        return -1;
    }
    if !copy_to_user(&space, out, cwd.as_bytes()) {
        return -1;
    }
    if !copy_to_user(&space, out + cwd.len() as u64, &[0]) {
        return -1;
    }
    (need - 1) as i64
}

fn sys_chdir(index: usize, path: &str) -> i64 {
    let cwd = task::get(index).cwd.clone();
    let Some(node) = vfs::resolve(path, &cwd) else {
        return -1;
    };
    if !vfs::is_directory(node) {
        return -1;
    }
    task::get(index).cwd = vfs::path_of(node);
    0
}

fn sys_procinfo(out: u64, max: usize) -> i64 {
    let Some(space) = current_space() else {
        return -1;
    };
    if max == 0 || max > 256 {
        return -1;
    }
    let mut data = Vec::with_capacity(max * 44);
    let mut count = 0usize;
    for t in task::table().iter() {
        if count >= max {
            break;
        }
        data.extend_from_slice(&t.pid.to_le_bytes());
        data.extend_from_slice(&t.ppid.to_le_bytes());
        data.extend_from_slice(&(t.state as i32).to_le_bytes());
        let mut name = [0u8; 32];
        let bytes = t.name.as_bytes();
        let copy = bytes.len().min(31);
        name[..copy].copy_from_slice(&bytes[..copy]);
        data.extend_from_slice(&name);
        count += 1;
    }
    if !copy_to_user(&space, out, &data) {
        return -1;
    }
    count as i64
}

/* ── GUI server ───────────────────────────────────────────────────────── */

fn gui_syscall(index: usize, cmd: u64, a1: u64, a2: u64) -> u64 {
    let pid = task_pid(index);
    match cmd {
        0 => pid as u64, // GUI_CONNECT
        1 => {
            // GUI_CREATE_WINDOW: a1 = pack(x,y), a2 = pack(w,h)
            let x = ((a1 >> 16) as u16 as i16) as i32;
            let y = ((a1 & 0xFFFF) as u16 as i16) as i32;
            let w = ((a2 >> 16) as u16 as i16) as i32;
            let h = ((a2 & 0xFFFF) as u16 as i16) as i32;
            let id = window::create(
                x,
                y,
                w.max(160),
                h.max(100),
                "Window",
                KIND_EXTERNAL,
                0x0B101A,
            );
            if let Some(window) = window::get(id) {
                window.owner_pid = pid;
            }
            id as u64
        }
        2 => {
            let id = a1 as u32;
            if let Some(window) = window::get(id) {
                if window.owner_pid == pid {
                    window::destroy(id);
                }
            }
            0
        }
        3 => {
            // GUI_DRAW_RECT: a1 = color<<32 | win, a2 = pack(x,y)<<32 | pack(w,h)
            let win = a1 as u32;
            let color = (a1 >> 32) as u32;
            let xy = (a2 >> 32) as u32;
            let wh = a2 as u32;
            let x = ((xy >> 16) as u16 as i16) as i32;
            let y = ((xy & 0xFFFF) as u16 as i16) as i32;
            let w = ((wh >> 16) as u16 as i16) as i32;
            let h = ((wh & 0xFFFF) as u16 as i16) as i32;
            if window_owned(win, pid) {
                window::fill_rect(win, x, y, w, h, color);
            }
            0
        }
        4 => {
            // Two-step: first stores style/position, second supplies the text.
            if (a1 >> 32) != 0 {
                let win = a1 as u32;
                let fg = (a1 >> 32) as u32;
                let bg = (a2 >> 32) as u32;
                let xy = a2 as u32;
                let x = ((xy >> 16) as u16 as i16) as i32;
                let y = ((xy & 0xFFFF) as u16 as i16) as i32;
                unsafe {
                    GUI_TEXT_PENDING = Some((pid, win, x, y, fg, bg));
                }
            } else {
                let win = a1 as u32;
                let Some(space) = current_space() else { return 0 };
                let Some(text) = read_cstr(&space, a2, 512) else { return 0 };
                let pending = unsafe { GUI_TEXT_PENDING };
                if let Some((owner, pwin, x, y, fg, bg)) = pending {
                    if owner == pid && pwin == win && window_owned(win, pid) {
                        window::draw_text(win, x, y, &text, fg, bg);
                    }
                }
                unsafe { GUI_TEXT_PENDING = None };
            }
            0
        }
        5 => {
            if window_owned(a1 as u32, pid) {
                window::mark_dirty(a1 as u32);
            }
            0
        }
        6 => {
            let win = a1 as u32;
            let color = a2 as u32;
            if window_owned(win, pid) {
                window::clear(win, color);
            }
            0
        }
        7 => {
            let win = a1 as u32;
            let color = (a1 >> 32) as u32;
            let xy = a2 as u32;
            let x = ((xy >> 16) as u16 as i16) as i32;
            let y = ((xy & 0xFFFF) as u16 as i16) as i32;
            if window_owned(win, pid) {
                window::draw_pixel(win, x, y, color);
            }
            0
        }
        8 => 0, // GUI_KEY_EVENT (kernel-to-app)
        9 => 0, // GUI_MOUSE_EVENT
        10 => 0,
        11 => 0,
        12 => 0,
        13 => {
            // GUI_POLL_EVENT
            let Some(space) = current_space() else { return 0 };
            let events = gui_events();
            if let Some(position) = events.iter().position(|(owner, _)| *owner == pid) {
                let (_, message) = events.remove(position);
                let mut data = Vec::with_capacity(48);
                data.extend_from_slice(&message.version.to_le_bytes());
                data.extend_from_slice(&message.kind.to_le_bytes());
                data.extend_from_slice(&message.win_id.to_le_bytes());
                data.extend_from_slice(&message.flags.to_le_bytes());
                data.extend_from_slice(&message.arg0.to_le_bytes());
                data.extend_from_slice(&message.arg1.to_le_bytes());
                data.extend_from_slice(&message.arg2.to_le_bytes());
                data.extend_from_slice(&message.sender_pid.to_le_bytes());
                data.extend_from_slice(&[0u8; 4]);
                if copy_to_user(&space, a1, &data) {
                    1
                } else {
                    0
                }
            } else {
                0
            }
        }
        14 => {
            let win = a1 as u32;
            if !window_owned(win, pid) {
                return 0;
            }
            let Some(space) = current_space() else { return 0 };
            if let Some(title) = read_cstr(&space, a2, 64) {
                window::set_title(win, &title);
            }
            0
        }
        15 => {
            let win = a1 as u32;
            let packed = a2 as u32;
            let x = ((packed >> 16) as u16 as i16) as i32;
            let y = ((packed & 0xFFFF) as u16 as i16) as i32;
            if window_owned(win, pid) {
                window::move_to(win, x, y);
            }
            0
        }
        16 => {
            let (width, height) = (
                crate::drivers::fb::width() as u64,
                crate::drivers::fb::height() as u64,
            );
            (width << 32) | height
        }
        17 => pid as u64,
        18 => 0,
        19 => 0,
        20 => {
            // GUI_GET_OWNED_FB_INFO
            let Some(space) = current_space() else { return u64::MAX };
            let mut info = [0u8; 12];
            if let Some(window) = window::all().iter().find(|w| w.owner_pid == pid) {
                info[0..4].copy_from_slice(&(window.id as i32).to_le_bytes());
                info[4..8].copy_from_slice(&window.w.to_le_bytes());
                info[8..12].copy_from_slice(&window.h.to_le_bytes());
                if copy_to_user(&space, a1, &info) {
                    0
                } else {
                    u64::MAX
                }
            } else {
                u64::MAX
            }
        }
        21 | 22 => u64::MAX,
        23 => {
            if window_owned(a1 as u32, pid) {
                window::mark_dirty(a1 as u32);
            }
            0
        }
        24 => gui_blit(pid, a1, a2),
        _ => u64::MAX,
    }
}

fn window_owned(id: u32, pid: i32) -> bool {
    window::get(id)
        .map(|window| window.owner_pid == pid)
        .unwrap_or(false)
}

fn gui_blit(pid: i32, win: u64, args: u64) -> u64 {
    let win = win as u32;
    if !window_owned(win, pid) {
        return u64::MAX;
    }
    let Some(space) = current_space() else {
        return u64::MAX;
    };
    let Some(data) = copy_from_user(&space, args, 32) else {
        return u64::MAX;
    };
    let pixels = u64::from_le_bytes(data[0..8].try_into().unwrap());
    let src_w = i32::from_le_bytes(data[8..12].try_into().unwrap());
    let src_h = i32::from_le_bytes(data[12..16].try_into().unwrap());
    let dst_x = i32::from_le_bytes(data[16..20].try_into().unwrap());
    let dst_y = i32::from_le_bytes(data[20..24].try_into().unwrap());
    let dst_w = i32::from_le_bytes(data[24..28].try_into().unwrap());
    let dst_h = i32::from_le_bytes(data[28..32].try_into().unwrap());
    if src_w <= 0 || src_h <= 0 || pixels == 0 {
        return u64::MAX;
    }
    let target_w = if dst_w <= 0 { src_w } else { dst_w };
    let target_h = if dst_h <= 0 { src_h } else { dst_h };
    let source = match copy_from_user(&space, pixels, (src_w * src_h * 4) as usize) {
        Some(source) => source,
        None => return u64::MAX,
    };
    if let Some(target) = window::get(win) {
        for y in 0..target_h {
            for x in 0..target_w {
                let sx = (x * src_w / target_w).clamp(0, src_w - 1);
                let sy = (y * src_h / target_h).clamp(0, src_h - 1);
                let offset = ((sy * src_w + sx) * 4) as usize;
                let argb = u32::from_le_bytes([
                    source[offset],
                    source[offset + 1],
                    source[offset + 2],
                    source[offset + 3],
                ]);
                let alpha = (argb >> 24) & 0xFF;
                if alpha == 0 {
                    continue;
                }
                let color = argb & 0xFFFFFF;
                let px = dst_x + x;
                let py = dst_y + y;
                if px < 0 || py < 0 || px >= target.w || py >= target.h {
                    continue;
                }
                let index = py as usize * target.w as usize + px as usize;
                if index < target.fb.len() {
                    target.fb[index] = if alpha >= 255 {
                        color
                    } else {
                        crate::drivers::blend(target.fb[index], color, alpha)
                    };
                }
            }
        }
        target.dirty = true;
    }
    0
}

/* ── socket helpers ────────────────────────────────────────────────────── */

fn port_arg(value: u64) -> u16 {
    u16::from_be(value as u16)
}

fn sys_sendto(slot: i32, buffer: u64, args: u64) -> i64 {
    let Some(space) = current_space() else {
        return -1;
    };
    let Some(packet) = copy_from_user(&space, args, 8) else {
        return -1;
    };
    let ip = u32::from_le_bytes([packet[0], packet[1], packet[2], packet[3]]);
    let port_be = u16::from_le_bytes([packet[4], packet[5]]);
    let len = u16::from_le_bytes([packet[6], packet[7]]) as usize;
    let Some(data) = copy_from_user(&space, buffer, len) else {
        return -1;
    };
    net::socket_sendto(slot, &data, ip, u16::from_be(port_be)) as i64
}

fn sys_recvfrom(slot: i32, buffer: u64, args: u64) -> i64 {
    let Some(space) = current_space() else {
        return -1;
    };
    let Some(packet) = copy_from_user(&space, args, 8) else {
        return -1;
    };
    let len = u16::from_le_bytes([packet[6], packet[7]]) as usize;
    let mut temp = vec![0u8; len.min(65535)];
    let n = net::socket_read_into(slot, &mut temp);
    if n <= 0 {
        return 0;
    }
    if !copy_to_user(&space, buffer, &temp[..n as usize]) {
        return -1;
    }
    let mut out = [0u8; 8];
    out[0..4].copy_from_slice(&net::socket_src_ip(slot).to_le_bytes());
    out[4..6].copy_from_slice(&net::socket_src_port(slot).to_be_bytes());
    out[6..8].copy_from_slice(&(n as u16).to_le_bytes());
    let _ = copy_to_user(&space, args, &out);
    n as i64
}

fn sys_poll(fds: u64, nfds: usize) -> i64 {
    let Some(space) = current_space() else {
        return -1;
    };
    if nfds == 0 || nfds > 64 {
        return -1;
    }
    let mut ready = 0i64;
    for index in 0..nfds {
        let base = fds + (index * 12) as u64;
        let Some(entry) = copy_from_user(&space, base, 12) else {
            return -1;
        };
        let fd = i32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let events = i32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
        let mut revents = 0i32;
        if fd >= 0 {
            if net::socket_state(fd) >= 0 {
                if events & 1 != 0 && net::socket_peek(fd) > 0 {
                    revents |= 1;
                }
                if events & 2 != 0 {
                    revents |= 2;
                }
                if net::socket_state(fd) == net::SOCK_STATE_CLOSED {
                    revents |= 8;
                }
            } else if fd == 0 {
                if apps::terminal::window_id().is_some() {
                    revents |= 1;
                }
            } else {
                revents |= 4;
            }
        }
        if revents != 0 {
            ready += 1;
        }
        let _ = copy_to_user(&space, base + 8, &revents.to_le_bytes());
    }
    ready
}

/* ── dispatcher ────────────────────────────────────────────────────────── */

pub fn init() {
    unsafe {
        PIPES = Some(Vec::new());
        GUI_EVENTS = Some(Vec::new());
    }
    crate::kprintln!("[syscall] int 0x80 ABI ready (SYS_EXIT..SYS_TRUNCATE)");
}

pub fn dispatch(f: &mut TrapFrame) {
    let number = f.rax;
    let arg0 = f.rdi;
    let arg1 = f.rsi;
    let arg2 = f.rdx;
    let Some(index) = current_index() else {
        f.rax = u64::MAX;
        return;
    };
    if task::get(index).state == TASK_SLEEPING {
        // A sleeping task that comes back through a syscall is runnable again.
        task::get(index).state = task::TASK_RUNNING;
    }
    f.rax = match number {
        SYS_EXIT => {
            let pid = task_pid(index);
            apps::notify_death(pid);
            for fd in 0..task::MAX_FDS as i32 {
                if fd_entry(index, fd).is_some() {
                    close_fd(index, fd);
                }
            }
            sched::exit_current(arg0 as i32);
        }
        SYS_PRINT => {
            if let Some(space) = current_space() {
                if let Some(text) = read_cstr(&space, arg0, 1024) {
                    crate::kprint!("{}", text);
                    crate::arch::serial::write_str(&text);
                }
            }
            0
        }
        SYS_GETPID => task_pid(index) as u64,
        SYS_SLEEP => {
            let ticks = (arg0.max(1)) as u64;
            let task = task::get(index);
            task.wake_tick = crate::sync::ticks() + ticks;
            task.state = TASK_SLEEPING;
            // Return immediately; the timer wakes us.
            0
        }
        SYS_OPEN => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            match read_cstr(&space, arg0, 256) {
                Some(path) => sys_open(index, &path, arg1 as i32) as u64,
                None => u64::MAX,
            }
        }
        SYS_READ => sys_read(f, index),
        SYS_WRITE => sys_write(f, index),
        SYS_CLOSE => close_fd(index, arg0 as i32) as u64,
        SYS_EXEC => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            match read_cstr(&space, arg0, 256) {
                Some(path) => sys_exec(index, &path, &[]) as u64,
                None => u64::MAX,
            }
        }
        SYS_EXEC_ARGV => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            let argc = arg2 as i32;
            if argc < 0 || argc > 16 {
                u64::MAX
            } else {
                let path = read_cstr(&space, arg0, 256).unwrap_or_default();
                let mut argv: Vec<String> = Vec::new();
                for slot in 0..argc as usize {
                    let Some(pointer) = read_u32_user(&space, arg1 + (slot * 8) as u64) else {
                        argv.push(String::new());
                        continue;
                    };
                    argv.push(read_cstr(&space, pointer as u64, 128).unwrap_or_default());
                }
                sys_exec(index, &path, &argv) as u64
            }
        }
        SYS_FORK => sys_fork(f, index) as u64,
        SYS_WAIT => sys_wait(index, arg0 as i32, arg1) as u64,
        SYS_KILL => sys_kill(arg0 as i32) as u64,
        SYS_BRK => sys_brk(index, arg0),
        SYS_READDIR => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            match read_cstr(&space, arg0, 256) {
                Some(path) => sys_readdir(&path, arg1, arg2 as usize, &space) as u64,
                None => u64::MAX,
            }
        }
        SYS_STAT => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            match read_cstr(&space, arg0, 256) {
                Some(path) => sys_stat(&path, arg1, &space) as u64,
                None => u64::MAX,
            }
        }
        SYS_DUP => sys_dup(index, arg0 as i32, arg1 as i32) as u64,
        SYS_PIPE => sys_pipe(index, arg0) as u64,
        SYS_GUI => gui_syscall(index, arg0, arg1, arg2),
        SYS_THEME => {
            theme::set_index(if arg0 == 1 { 1 } else { 0 });
            crate::fs::persist::save();
            0
        }
        SYS_NET => {
            if arg0 == 6 {
                let Some(space) = current_space() else {
                    f.rax = u64::MAX;
                    return;
                };
                match read_cstr(&space, arg1, 256) {
                    Some(name) => {
                        net::dns_lookup(&name);
                        0
                    }
                    None => u64::MAX,
                }
            } else {
                net::syscall(arg0, arg1)
            }
        }
        SYS_SOCKET => net::socket_create(arg0 as i32, arg1 as i32, arg2 as i32) as u64,
        SYS_BIND => net::socket_bind(arg0 as i32, arg1 as u32, port_arg(arg2)) as u64,
        SYS_CONNECT => net::socket_connect(arg0 as i32, arg1 as u32, port_arg(arg2)) as u64,
        SYS_SEND => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            match copy_from_user(&space, arg1, arg2 as usize) {
                Some(data) => net::socket_send(arg0 as i32, &data) as u64,
                None => u64::MAX,
            }
        }
        SYS_RECV => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            let mut temp = vec![0u8; (arg2 as usize).min(65535)];
            let n = net::socket_read_into(arg0 as i32, &mut temp);
            if n > 0 {
                copy_to_user(&space, arg1, &temp[..n as usize]);
            }
            if n < 0 {
                u64::MAX
            } else {
                n as u64
            }
        }
        SYS_SENDTO => sys_sendto(arg0 as i32, arg1, arg2) as u64,
        SYS_RECVFROM => sys_recvfrom(arg0 as i32, arg1, arg2) as u64,
        SYS_SOCKCLOSE => net::socket_close(arg0 as i32) as u64,
        SYS_LISTEN => net::socket_listen(arg0 as i32, arg1 as i32) as u64,
        SYS_ACCEPT => {
            let (slot, ip, port) = net::socket_accept(arg0 as i32);
            if slot < 0 {
                u64::MAX
            } else {
                if let Some(space) = current_space() {
                    if arg1 != 0 {
                        write_u32_user(&space, arg1, ip);
                    }
                    if arg2 != 0 {
                        copy_to_user(&space, arg2, &port.to_be_bytes());
                    }
                }
                slot as u64
            }
        }
        SYS_SOCKSTATE => net::socket_state(arg0 as i32) as u64,
        SYS_PROCINFO => sys_procinfo(arg0, arg1 as usize) as u64,
        SYS_FCNTL => {
            if arg1 == 3 {
                0
            } else {
                0
            }
        }
        SYS_POLL => sys_poll(arg0, arg1 as usize) as u64,
        SYS_SYNC => 0,
        SYS_LSEEK => sys_lseek(index, arg0 as i32, arg1 as i64, arg2 as i32) as u64,
        SYS_UNLINK => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            let cwd = task::get(index).cwd.clone();
            match read_cstr(&space, arg0, 256) {
                Some(path) => {
                    if vfs::unlink(&path, &cwd) || vfs::rmdir(&path, &cwd) {
                        0
                    } else {
                        u64::MAX
                    }
                }
                None => u64::MAX,
            }
        }
        SYS_MKDIR => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            let cwd = task::get(index).cwd.clone();
            match read_cstr(&space, arg0, 256) {
                Some(path) => {
                    if vfs::mkdir(&path, &cwd) {
                        0
                    } else {
                        u64::MAX
                    }
                }
                None => u64::MAX,
            }
        }
        SYS_CHDIR => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            match read_cstr(&space, arg0, 256) {
                Some(path) => sys_chdir(index, &path) as u64,
                None => u64::MAX,
            }
        }
        SYS_GETCWD => sys_getcwd(index, arg0, arg1 as usize) as u64,
        SYS_TRUNCATE => {
            let Some(space) = current_space() else {
                f.rax = u64::MAX;
                return;
            };
            let cwd = task::get(index).cwd.clone();
            match read_cstr(&space, arg0, 256) {
                Some(path) => match vfs::resolve(&path, &cwd) {
                    Some(node) => {
                        if vfs::truncate(node, arg1 as usize) {
                            0
                        } else {
                            u64::MAX
                        }
                    }
                    None => u64::MAX,
                },
                None => u64::MAX,
            }
        }
        _ => u64::MAX,
    };
}

pub fn print_info() {
    crate::kprintln!(
        "[syscall] native ABI: {} calls, {} pipes, {} GUI events queued",
        42,
        pipes().len(),
        gui_events().len()
    );
}

pub fn _unused() {
    let _ = TASK_DEAD;
}
