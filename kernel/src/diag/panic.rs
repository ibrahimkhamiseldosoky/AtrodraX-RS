//! Panic, fatal-fault and allocator-failure reporting.

use crate::arch::idt::TrapFrame;
use core::panic::PanicInfo;
use core::alloc::Layout;

pub fn halt_forever() -> ! {
    loop {
        crate::arch::cpu::halt();
    }
}

pub fn report(info: &PanicInfo) -> ! {
    unsafe { crate::arch::cpu::disable_interrupts() };
    crate::kprintln!("");
    crate::kprintln!("==================== KERNEL PANIC ====================");
    if let Some(location) = info.location() {
        crate::kprintln!("at {}:{}:{}", location.file(), location.line(), location.column());
    }
    crate::kprint!("message: ");
    crate::kprint!("{}", info.message());
    crate::kprintln!("");
    crate::kprintln!("system halted.");
    halt_forever()
}

pub fn alloc_failure(layout: Layout) -> ! {
    crate::kprintln!("[heap] allocation failed: size={} align={}", layout.size(), layout.align());
    crate::kprintln!("==================== KERNEL PANIC ====================");
    crate::kprintln!("system halted.");
    halt_forever()
}

pub fn report_frame(frame: &TrapFrame) -> ! {
    unsafe { crate::arch::cpu::disable_interrupts() };
    crate::kprintln!("");
    crate::kprintln!("============== FATAL CPU EXCEPTION ==============");
    crate::kprintln!("vector={} error={:#x} rip={:#x} cs={:#x} rflags={:#x}", frame.vector, frame.error, frame.rip, frame.cs, frame.rflags);
    crate::kprintln!("rsp={:#x} ss={:#x} cr2={:#x} cr3={:#x}", frame.rsp, frame.ss, crate::arch::cpu::read_cr2(), crate::arch::cpu::read_cr3());
    crate::kprintln!("rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x}", frame.rax, frame.rbx, frame.rcx, frame.rdx);
    crate::kprintln!("rsi={:#x} rdi={:#x} rbp={:#x} r8 ={:#x}", frame.rsi, frame.rdi, frame.rbp, frame.r8);
    crate::kprintln!("r9 ={:#x} r10={:#x} r11={:#x} r12={:#x}", frame.r9, frame.r10, frame.r11, frame.r12);
    crate::kprintln!("r13={:#x} r14={:#x} r15={:#x}", frame.r13, frame.r14, frame.r15);
    crate::kprintln!("system halted.");
    halt_forever()
}

pub fn kill_user_on_fault(frame: &mut TrapFrame) -> ! {
    crate::kprintln!("[fault] terminating ring-3 task (rip={:#x})", frame.rip);
    crate::sched::kill_current(frame);
    // kill_current() switches away permanently.
    halt_forever()
}
