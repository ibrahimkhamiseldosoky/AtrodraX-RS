//! Programmable Interval Timer at 1000 Hz.

use crate::arch::port::outb;
use crate::sync::TICKS;
use core::sync::atomic::Ordering;

const PIT_CHANNEL0: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;
pub const FREQ_HZ: u32 = 1000;

pub fn init() {
    let divisor = (1_193_182u32 / FREQ_HZ) as u16;
    unsafe {
        outb(PIT_COMMAND, 0x36);
        outb(PIT_CHANNEL0, (divisor & 0xFF) as u8);
        outb(PIT_CHANNEL0, (divisor >> 8) as u8);
    }
    crate::kprintln!("[timer] PIT configured for {} Hz", FREQ_HZ);
}

pub fn on_irq() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn uptime_ms() -> u64 {
    ticks()
}
