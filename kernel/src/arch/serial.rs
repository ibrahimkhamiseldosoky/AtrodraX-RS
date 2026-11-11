//! COM1 serial output — the kernel's primary diagnostic channel.

use crate::arch::port::{inb, outb};
use crate::sync::without_interrupts;
use core::fmt;

const COM1: u16 = 0x3F8;

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // disable interrupts
        outb(COM1 + 3, 0x80); // DLAB
        outb(COM1, 0x01); // 115200 baud
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // FIFO, 14-byte threshold
        outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR
    }
}

fn transmit_empty() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

pub fn write_byte(byte: u8) {
    // Convert bare newlines to CRLF for serial terminals.
    if byte == b'\n' {
        write_raw(b'\r');
    }
    write_raw(byte);
}

fn write_raw(byte: u8) {
    let mut spins = 0u32;
    while !transmit_empty() {
        spins += 1;
        if spins > 1_000_000 {
            return;
        }
        core::hint::spin_loop();
    }
    unsafe { outb(COM1, byte) };
}

pub fn write_str(s: &str) {
    for b in s.bytes() {
        write_byte(b);
    }
}

/// Non-blocking serial receive (COM1). Used by the debug console.
pub fn try_read() -> Option<u8> {
    unsafe {
        if inb(COM1 + 5) & 1 != 0 {
            Some(inb(COM1))
        } else {
            None
        }
    }
}

pub fn write_fmt(args: fmt::Arguments) {
    struct Writer;
    impl fmt::Write for Writer {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            serial_write(s);
            Ok(())
        }
    }
    without_interrupts(|| {
        let _ = fmt::Write::write_fmt(&mut Writer, args);
    });
}

fn serial_write(s: &str) {
    for b in s.bytes() {
        write_byte(b);
    }
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        $crate::arch::serial::write_fmt(format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($fmt:expr) => { $crate::kprint!(concat!($fmt, "\n")) };
    ($fmt:expr, $($arg:tt)*) => { $crate::kprint!(concat!($fmt, "\n"), $($arg)*) };
}
