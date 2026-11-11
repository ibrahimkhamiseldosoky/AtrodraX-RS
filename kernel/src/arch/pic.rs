//! Dual 8259A PIC remapped to vectors 0x20..0x2F.

use crate::arch::port::{inb, io_wait, outb};

pub const PIC1_CMD: u16 = 0x20;
pub const PIC1_DATA: u16 = 0x21;
pub const PIC2_CMD: u16 = 0xA0;
pub const PIC2_DATA: u16 = 0xA1;
pub const PIC_EOI: u8 = 0x20;
pub const IRQ_BASE: u8 = 0x20;

pub fn init() {
    unsafe {
        outb(PIC1_CMD, 0x11);
        io_wait();
        outb(PIC2_CMD, 0x11);
        io_wait();
        outb(PIC1_DATA, IRQ_BASE);
        io_wait();
        outb(PIC2_DATA, IRQ_BASE + 8);
        io_wait();
        outb(PIC1_DATA, 0x04);
        io_wait();
        outb(PIC2_DATA, 0x02);
        io_wait();
        outb(PIC1_DATA, 0x01);
        io_wait();
        outb(PIC2_DATA, 0x01);
        io_wait();

        // IRQ0 timer, IRQ1 keyboard, IRQ2 cascade, IRQ12 mouse.
        outb(PIC1_DATA, 0xF8);
        outb(PIC2_DATA, 0xEF);
    }
    crate::kprintln!("[pic] remapped to 0x20..0x2F (timer, keyboard, mouse enabled)");
}

pub fn send_eoi(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(PIC2_CMD, PIC_EOI);
        }
        outb(PIC1_CMD, PIC_EOI);
    }
}

pub fn mask(irq: u8) {
    unsafe {
        let (port, bit) = if irq < 8 {
            (PIC1_DATA, irq)
        } else {
            (PIC2_DATA, irq - 8)
        };
        let value = inb(port) | (1 << bit);
        outb(port, value);
    }
}

pub fn unmask(irq: u8) {
    unsafe {
        let (port, bit) = if irq < 8 {
            (PIC1_DATA, irq)
        } else {
            (PIC2_DATA, irq - 8)
        };
        let value = inb(port) & !(1 << bit);
        outb(port, value);
    }
}
