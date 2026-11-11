//! Process/task table.

use crate::arch::idt::TrapFrame;
use crate::mem::vmm::AddrSpace;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub type Pid = i32;

pub const MAX_TASKS: usize = 64;
pub const MAX_FDS: usize = 16;
pub const KSTACK_SIZE: usize = 32 * 1024;

/// Sentinel file descriptor node for the task's controlling terminal.
pub const CONSOLE_NODE: usize = usize::MAX;

pub const TASK_READY: u32 = 0;
pub const TASK_RUNNING: u32 = 1;
pub const TASK_BLOCKED: u32 = 2;
pub const TASK_SLEEPING: u32 = 3;
pub const TASK_DEAD: u32 = 4;
pub const TASK_ZOMBIE: u32 = 5;

#[derive(Clone, Copy)]
pub struct Fd {
    pub node: usize,
    pub offset: usize,
    pub flags: i32,
}

pub struct Task {
    pub pid: Pid,
    pub ppid: Pid,
    pub name: String,
    pub state: u32,
    pub exit_code: i32,
    pub frame: TrapFrame,
    pub kstack: Vec<u8>,
    pub kstack_top: u64,
    pub kernel_sp: u64,
    pub space: Option<AddrSpace>,
    pub brk: u64,
    pub brk_start: u64,
    pub wake_tick: u64,
    pub waiting_for: Pid,
    pub fds: Vec<Option<Fd>>,
    pub cwd: String,
    pub linux_compat: bool,
    pub fs_base: u64,
    pub start_tick: u64,
    pub cpu_ticks: u64,
    pub argv0: String,
}

static mut TASKS: Option<Vec<Task>> = None;

extern "C" {
    fn context_switch(old_sp: *mut u64, new_sp: u64);
}

pub fn init() {
    unsafe {
        TASKS = Some(Vec::with_capacity(MAX_TASKS));
    }
    crate::kprintln!("[task] process table initialized (max {})", MAX_TASKS);
}

pub fn table() -> &'static mut Vec<Task> {
    unsafe { TASKS.as_mut().expect("task table not initialized") }
}

pub fn count() -> usize {
    table().len()
}

pub fn next_pid() -> Pid {
    static mut PID: Pid = 1;
    unsafe {
        let pid = PID;
        PID += 1;
        pid
    }
}

/// Build the initial kernel stack image used by context_switch:
/// six callee-saved registers, then a return address to user_trampoline.
fn build_initial_stack(top: u64) -> u64 {
    let sp = top - 64;
    unsafe {
        let p = sp as *mut u64;
        for i in 0..6 {
            *p.add(i) = 0;
        }
        *p.add(6) = user_trampoline as *const () as u64;
    }
    sp
}

pub fn create_user(name: &str, frame: TrapFrame, space: AddrSpace, argv0: &str) -> Pid {
    let pid = next_pid();
    let mut kstack = vec![0u8; KSTACK_SIZE];
    let base = kstack.as_mut_ptr() as u64;
    let top = (base + KSTACK_SIZE as u64) & !15;
    let kernel_sp = build_initial_stack(top);

    let task = Task {
        pid,
        ppid: 0,
        name: String::from(name),
        state: TASK_READY,
        exit_code: 0,
        frame,
        kstack,
        kstack_top: top,
        kernel_sp,
        space: Some(space),
        brk: 0,
        brk_start: 0,
        wake_tick: 0,
        waiting_for: 0,
        fds: vec![
            Some(Fd { node: CONSOLE_NODE, offset: 0, flags: 0 }),
            Some(Fd { node: CONSOLE_NODE, offset: 0, flags: 0 }),
            Some(Fd { node: CONSOLE_NODE, offset: 0, flags: 0 }),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
        cwd: String::from("/"),
        linux_compat: false,
        fs_base: 0,
        start_tick: crate::sync::ticks(),
        cpu_ticks: 0,
        argv0: String::from(argv0),
    };
    table().push(task);
    pid
}

pub fn find(pid: Pid) -> Option<usize> {
    table().iter().position(|t| t.pid == pid)
}

pub fn find_index(pid: Pid) -> Option<usize> {
    find(pid)
}

pub fn get(index: usize) -> &'static mut Task {
    &mut table()[index]
}

pub fn alloc_fd(index: usize, node: usize, flags: i32) -> i32 {
    let task = get(index);
    for (slot, entry) in task.fds.iter_mut().enumerate() {
        if entry.is_none() {
            *entry = Some(Fd {
                node,
                offset: 0,
                flags,
            });
            return slot as i32;
        }
    }
    -1
}

pub fn get_fd(index: usize, fd: i32) -> Option<&'static mut Fd> {
    if fd < 0 || fd as usize >= MAX_FDS {
        return None;
    }
    let task = get(index);
    task.fds[fd as usize].as_mut()
}

pub fn close_fd(index: usize, fd: i32) -> bool {
    if fd < 0 || fd as usize >= MAX_FDS {
        return false;
    }
    let task = get(index);
    if task.fds[fd as usize].is_some() {
        task.fds[fd as usize] = None;
        true
    } else {
        false
    }
}

pub fn print_info() {
    crate::kprintln!("[task] pid  ppid  state  name");
    for t in table().iter() {
        crate::kprintln!(
            "[task] {:3}  {:4}  {:5}  {}",
            t.pid,
            t.ppid,
            state_name(t.state),
            t.name
        );
    }
}

pub fn state_name(state: u32) -> &'static str {
    match state {
        TASK_READY => "ready",
        TASK_RUNNING => "run",
        TASK_BLOCKED => "block",
        TASK_SLEEPING => "sleep",
        TASK_DEAD => "dead",
        TASK_ZOMBIE => "zombie",
        _ => "?",
    }
}

/// Entry trampoline used the first time a task is scheduled.
#[no_mangle]
pub extern "C" fn user_trampoline() -> ! {
    let index = crate::sched::current_index();
    let task = get(index);
    task.state = TASK_RUNNING;
    crate::sched::activate(task);
    unsafe {
        enter_user(&task.frame as *const TrapFrame);
    }
}

extern "C" {
    fn enter_user(frame: *const TrapFrame) -> !;
}

/// Switch away from the current task. `setup` is called while the task is
/// still current, before the context switch.
pub fn block_current(setup: impl FnOnce(&mut Task)) {
    crate::sched::block_and_switch(setup);
}
