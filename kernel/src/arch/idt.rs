//! Interrupt Descriptor Table and the common interrupt dispatcher.

use crate::arch::{cpu, pic, timer};
use crate::drivers::{keyboard, mouse};
use crate::sched;
use crate::syscall;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn empty() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::empty(); 256];

extern "C" {
    static isr_stub_table: [u64; 64];
    fn isr_128();
    fn isr_63();
}

fn set_gate(vector: usize, handler: u64, dpl3: bool) {
    let base = handler;
    let attr = if dpl3 { 0xEE } else { 0x8E };
    unsafe {
        let entry = &mut IDT[vector];
        entry.offset_low = base as u16;
        entry.selector = super::gdt::KERNEL_CS;
        entry.ist = 0;
        entry.type_attr = attr;
        entry.offset_mid = (base >> 16) as u16;
        entry.offset_high = (base >> 32) as u32;
        entry.reserved = 0;
    }
}

pub fn init() {
    unsafe {
        for vector in 0..256 {
            let handler = if vector < 64 {
                isr_stub_table[vector]
            } else if vector == 0x80 {
                isr_128 as *const () as u64
            } else {
                isr_63 as *const () as u64
            };
            set_gate(vector, handler, vector == 0x80);
        }

        let idtr = Idtr {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        core::arch::asm!("lidt [{}]", in(reg) &idtr, options(nostack));
    }
    crate::kprintln!("[idt] 256 gates installed (int 0x80 DPL=3)");
}

fn exception_name(vector: u64) -> &'static str {
    match vector {
        0 => "divide error",
        1 => "debug",
        2 => "nmi",
        3 => "breakpoint",
        4 => "overflow",
        5 => "bound range",
        6 => "invalid opcode",
        7 => "device not available",
        8 => "double fault",
        10 => "invalid TSS",
        11 => "segment not present",
        12 => "stack fault",
        13 => "general protection fault",
        14 => "page fault",
        16 => "x87 floating point",
        17 => "alignment check",
        18 => "machine check",
        19 => "simd floating point",
        20 => "virtualization",
        30 => "security",
        _ => "reserved",
    }
}

/// Called from the assembly ISR trampoline with a pointer to the saved frame.
#[no_mangle]
pub extern "C" fn interrupt_dispatch(frame: *mut TrapFrame) {
    let f = unsafe { &mut *frame };
    match f.vector {
        0x20 => {
            timer::on_irq();
            pic::send_eoi(0);
            sched::on_timer(f);
        }
        0x21 => {
            keyboard::on_irq();
            pic::send_eoi(1);
        }
        0x2C => {
            mouse::on_irq();
            pic::send_eoi(12);
        }
        0x80 => {
            syscall::dispatch(f);
        }
        vector if vector < 32 => {
            let cr2 = cpu::read_cr2();
            crate::kprintln!(
                "[fault] {} (vector {}) rip={:#x} cs={:#x} err={:#x} cr2={:#x}",
                exception_name(vector),
                vector,
                f.rip,
                f.cs,
                f.error,
                cr2
            );
            if f.cs & 3 == 3 {
                crate::diag::panic::kill_user_on_fault(f);
            } else {
                crate::diag::panic::report_frame(f);
            }
        }
        _ => {
            if (0x20..0x30).contains(&f.vector) {
                pic::send_eoi((f.vector - 0x20) as u8);
            }
        }
    }
}
