//! Linux x86-64 ABI compatibility layer (`syscall`/`sysret`).
//!
//! Native AtrodraX programs use `int 0x80` (see [`crate::syscall`]). Linux
//! binaries use the `SYSCALL` instruction instead, which is a completely
//! separate entry path:
//!
//! * `EFER.SCE` enables the instruction.
//! * `STAR` supplies the kernel/user segment selectors.
//! * `LSTAR` points at [`linux_syscall_entry`].
//! * `SFMASK` clears IF on entry so the handler runs atomically.
//!
//! `SYSCALL` does not switch stacks, so the entry stub loads the running
//! task's kernel stack from [`linux_kernel_rsp0`] (kept in sync by the
//! scheduler) and builds a per-task frame before calling [`dispatch`].
//!
//! Only static Linux binaries are supported: there is no ELF interpreter
//! loader, so dynamically linked programs are rejected by [`crate::elf`].

pub mod abi;
mod sys;

use core::arch::global_asm;

global_asm!(include_str!("entry.S"), options(att_syntax));

pub const MSR_EFER: u32 = 0xC000_0080;
pub const MSR_STAR: u32 = 0xC000_0081;
pub const MSR_LSTAR: u32 = 0xC000_0082;
pub const MSR_SFMASK: u32 = 0xC000_0084;
pub const EFER_SCE: u64 = 1 << 0;

/// `STAR[47:32]` = SYSCALL CS (SS = +8) = 0x08/0x10, matching `arch::gdt`.
/// `STAR[63:48]` = SYSRET  CS (SS = +8) = 0x20/0x18.
const STAR_VALUE: u64 = 0x0010_0008_0000_0000;
/// Clear IF on entry; the CPU leaves our frame otherwise interruptible while
/// RSP still points at a user stack for a few instructions.
const SFMASK_VALUE: u64 = 0x200;

extern "C" {
    fn linux_syscall_entry();
    static mut linux_kernel_rsp0: u64;
}

/// Point the SYSCALL entry stub at the current task's kernel stack.
///
/// Called by the scheduler whenever it activates a task, right after
/// `tss::set_rsp0`, because `SYSCALL` bypasses the TSS stack switch that
/// interrupts get.
pub fn set_kernel_stack(top: u64) {
    unsafe { linux_kernel_rsp0 = top };
}

pub fn init() {
    unsafe {
        crate::arch::cpu::wrmsr(MSR_STAR, STAR_VALUE);
        crate::arch::cpu::wrmsr(MSR_LSTAR, linux_syscall_entry as *const () as u64);
        crate::arch::cpu::wrmsr(MSR_SFMASK, SFMASK_VALUE);
        let efer = crate::arch::cpu::rdmsr(MSR_EFER);
        crate::arch::cpu::wrmsr(MSR_EFER, efer | EFER_SCE);
    }
    set_kernel_stack(crate::arch::tss::rsp0());
    crate::kprintln!(
        "[linux] SYSCALL/SYSRET ABI enabled (LSTAR={:#x})",
        linux_syscall_entry as *const () as u64
    );
}

/// Frame the SYSCALL stub should return into instead of the caller, used by
/// `execve`. Written by [`request_exec_frame`]; consumed by `entry.S`.
#[no_mangle]
pub static mut linux_exec_pending: u64 = 0;
#[no_mangle]
pub static mut linux_exec_rsp: u64 = 0;
#[no_mangle]
pub static mut linux_exec_rip: u64 = 0;
#[no_mangle]
pub static mut linux_exec_flags: u64 = 0;

/// Make the current `SYSCALL` return into `rip` with a fresh stack.
///
/// `execve` replaces the task's address space; the syscall frame on the kernel
/// stack still describes the old program, so the stub overwrites it before
/// `sysret`.
pub fn request_exec_frame(rsp: u64, rip: u64, rflags: u64) {
    unsafe {
        linux_exec_rsp = rsp;
        linux_exec_rip = rip;
        linux_exec_flags = rflags;
        linux_exec_pending = 1;
    }
}

/// Register state captured by the SYSCALL stub for the current call.
///
/// `fork`/`clone` need the caller's RIP/RSP/RFLAGS and callee-saved registers
/// so the child resumes in the same place as the parent; `execve` needs the
/// same RSP/RIP/RFLAGS to return into a new image. The layout is mirrored by
/// `entry.S`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct UserContext {
    pub num: u64,
    pub a0: u64,
    pub a1: u64,
    pub a2: u64,
    pub a3: u64,
    pub a4: u64,
    pub a5: u64,
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

/// Called from `entry.S` with a pointer to the saved [`UserContext`].
#[no_mangle]
pub extern "C" fn linux_syscall_dispatch(context: *const UserContext) -> u64 {
    let context = unsafe { &*context };
    sys::dispatch(context)
}
