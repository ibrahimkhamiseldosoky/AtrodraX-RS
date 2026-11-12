//! Round-robin scheduler with cooperative kernel/user context switching.

use crate::arch::{cpu, idt::TrapFrame, tss};
use crate::mem::vmm;
use crate::task::{self, Task, TASK_DEAD, TASK_READY, TASK_RUNNING, TASK_SLEEPING, TASK_ZOMBIE};
use core::ptr::addr_of_mut;

pub const KERNEL: i32 = -1;
const SLICE_TICKS: u64 = 10;

static mut CURRENT: i32 = KERNEL;
static mut KERNEL_SP: u64 = 0;
static mut KERNEL_TSS_TOP: u64 = 0;
static mut ROUND_ROBIN: usize = 0;

extern "C" {
    fn context_switch(old_sp: *mut u64, new_sp: u64);
}

pub fn init() {
    unsafe {
        KERNEL_TSS_TOP = tss::rsp0();
        CURRENT = KERNEL;
    }
    crate::kprintln!("[sched] round-robin scheduler ready");
}

pub fn current_index() -> usize {
    unsafe {
        if CURRENT < 0 {
            0
        } else {
            CURRENT as usize
        }
    }
}

pub fn current() -> Option<&'static mut Task> {
    unsafe {
        if CURRENT < 0 {
            None
        } else {
            Some(task::get(CURRENT as usize))
        }
    }
}

pub fn current_pid() -> i32 {
    current().map(|t| t.pid).unwrap_or(0)
}

pub fn activate(task: &Task) {
    tss::set_rsp0(task.kstack_top);
    // SYSCALL does not switch stacks through the TSS, so the Linux entry stub
    // needs the same kernel stack address in its own slot.
    crate::linux::set_kernel_stack(task.kstack_top);
    if let Some(space) = task.space {
        vmm::load_space(&space);
    } else {
        vmm::load(vmm::kernel_cr3());
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        // FS.base is used by Linux-compatible TLS (musl). Always write it,
        // including zero, or a task resumes with the previous task's TLS.
        let lo = task.fs_base as u32;
        let hi = (task.fs_base >> 32) as u32;
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC000_0100u32,
            in("eax") lo,
            in("edx") hi,
            options(nostack)
        );
    }
}

fn activate_kernel() {
    unsafe { tss::set_rsp0(KERNEL_TSS_TOP) };
    vmm::load(vmm::kernel_cr3());
}

/// Called from the timer IRQ (CS may be ring 0 or ring 3).
pub fn on_timer(frame: &mut TrapFrame) {
    if frame.cs & 3 != 3 {
        // Interrupted the kernel; just accumulate time.
        unsafe {
            if CURRENT >= 0 {
                task::get(CURRENT as usize).cpu_ticks += 1;
            }
        }
        return;
    }

    let ticks = crate::sync::ticks();
    unsafe {
        let cur_index = CURRENT;
        if cur_index >= 0 {
            let cur = task::get(cur_index as usize);
            cur.cpu_ticks += 1;
            if cur.state == TASK_RUNNING {
                cur.state = TASK_READY;
            }
        }

        // Wake sleeping tasks.
        for t in task::table().iter_mut() {
            if t.state == TASK_SLEEPING && t.wake_tick <= ticks {
                t.state = TASK_READY;
            }
        }

        if ticks % SLICE_TICKS == 0 {
            switch_to_kernel(cur_index);
            return;
        }

        if let Some(next) = pick_next(cur_index) {
            if next != cur_index {
                switch_to(cur_index, next);
            } else if cur_index >= 0 {
                task::get(cur_index as usize).state = TASK_RUNNING;
            }
        } else {
            switch_to_kernel(cur_index);
        }
    }
}

unsafe fn pick_next(current: i32) -> Option<i32> {
    let table = task::table();
    if table.is_empty() {
        return None;
    }
    let len = table.len();
    let start = if current >= 0 {
        (current as usize + 1) % len
    } else {
        ROUND_ROBIN % len
    };
    for offset in 0..len {
        let index = (start + offset) % len;
        if table[index].state == TASK_READY {
            ROUND_ROBIN = (index + 1) % len;
            return Some(index as i32);
        }
    }
    None
}

unsafe fn switch_to(_old: i32, next: i32) {
    let next_task = task::get(next as usize);
    next_task.state = TASK_RUNNING;
    let old_index = CURRENT;
    CURRENT = next;
    activate(next_task);
    if old_index >= 0 {
        let old_task = task::get(old_index as usize);
        context_switch(addr_of_mut!(old_task.kernel_sp), next_task.kernel_sp);
    } else {
        // Shouldn't happen, but recover by entering through the trampoline.
        context_switch(addr_of_mut!(KERNEL_SP), next_task.kernel_sp);
    }
}

unsafe fn switch_to_kernel(old: i32) {
    if KERNEL_SP == 0 {
        cpu::halt_loop();
    }
    CURRENT = KERNEL;
    activate_kernel();
    if old >= 0 {
        let old_task = task::get(old as usize);
        context_switch(addr_of_mut!(old_task.kernel_sp), KERNEL_SP);
    } else {
        context_switch(addr_of_mut!(KERNEL_SP), KERNEL_SP);
    }
}

/// Called from the kernel main loop to grant the user tasks a time slice.
pub fn run_user_slice() {
    unsafe {
        if CURRENT != KERNEL {
            return;
        }
        let next = match pick_next(KERNEL) {
            Some(n) => n,
            None => return,
        };
        let next_task = task::get(next as usize);
        next_task.state = TASK_RUNNING;
        CURRENT = next;
        activate(next_task);
        context_switch(addr_of_mut!(KERNEL_SP), next_task.kernel_sp);
        // We get here when a task yields back to the kernel.
        CURRENT = KERNEL;
        activate_kernel();
        cpu::enable_interrupts();
    }
}

/// Block the current task and return to the kernel. Returns when the task is
/// scheduled again.
pub fn block_and_switch(setup: impl FnOnce(&mut Task)) {
    unsafe {
        let index = CURRENT;
        if index < 0 {
            return;
        }
        let task = task::get(index as usize);
        setup(task);
        switch_to_kernel(index);
        // Resumed: restore this task's address space.
        let task = task::get(index as usize);
        activate(task);
    }
}

pub fn yield_now() {
    unsafe {
        let index = CURRENT;
        if index >= 0 {
            task::get(index as usize).state = TASK_READY;
            switch_to_kernel(index);
        }
    }
}

pub fn sleep_ms(ms: u64) {
    let wake = crate::sync::ticks() + ms.max(1);
    block_and_switch(|task| {
        task.state = TASK_SLEEPING;
        task.wake_tick = wake;
    });
}

pub fn exit_current(code: i32) -> ! {
    unsafe {
        let index = CURRENT;
        if index >= 0 {
            let task = task::get(index as usize);
            task.exit_code = code;
            task.state = if task.ppid > 0 { TASK_ZOMBIE } else { TASK_DEAD };
            let ppid = task.ppid;
            if ppid > 0 {
                if let Some(parent_index) = task::find(ppid) {
                    let parent = task::get(parent_index);
                    if parent.state == 2 {
                        parent.state = TASK_READY;
                    }
                }
            }
            switch_to_kernel(index);
        }
    }
    cpu::halt_loop()
}

pub fn kill_current(_frame: &mut TrapFrame) -> ! {
    exit_current(-11)
}

pub fn wait_for(pid: i32) -> i32 {
    loop {
        match task::find(pid) {
            Some(index) => {
                let child = task::get(index);
                if child.state == TASK_ZOMBIE || child.state == TASK_DEAD {
                    let code = child.exit_code;
                    child.state = TASK_DEAD;
                    return code;
                }
            }
            None => return -1,
        }
        block_and_switch(|task| {
            task.state = 2; // TASK_BLOCKED
            task.waiting_for = pid;
        });
    }
}

pub fn reap_dead() {
    let table = task::table();
    let mut i = 0;
    while i < table.len() {
        if table[i].state == TASK_DEAD && table[i].ppid != 0 {
            // Keep dead tasks visible for ps, but mark their memory reusable.
            i += 1;
        } else {
            i += 1;
        }
    }
}
