//! CMOS real-time clock.

use crate::arch::port::{inb, outb};

#[derive(Clone, Copy, Default)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub day: u8,
    pub month: u8,
    pub year: u16,
}

fn read_cmos(reg: u8) -> u8 {
    unsafe {
        outb(0x70, reg);
        inb(0x71)
    }
}

fn bcd(value: u8) -> u8 {
    (value & 0x0F) + ((value >> 4) * 10)
}

fn update_in_progress() -> bool {
    read_cmos(0x0A) & 0x80 != 0
}

pub fn init() {
    // Make sure the RTC is in binary/24-hour mode where possible.
    unsafe {
        let status_b = read_cmos(0x0B);
        outb(0x70, 0x0B);
        outb(0x71, status_b | 0x02);
    }
    crate::kprintln!("[rtc] CMOS clock ready");
}

pub fn now() -> Time {
    for _ in 0..1_000_000 {
        if !update_in_progress() {
            break;
        }
    }
    let mut second = read_cmos(0x00);
    let mut minute = read_cmos(0x02);
    let mut hour = read_cmos(0x04);
    let mut day = read_cmos(0x07);
    let mut month = read_cmos(0x08);
    let mut year = read_cmos(0x09);
    let status_b = read_cmos(0x0B);

    if status_b & 0x04 == 0 {
        second = bcd(second);
        minute = bcd(minute);
        hour = bcd(hour & 0x7F);
        day = bcd(day);
        month = bcd(month);
        year = bcd(year);
    }
    Time {
        hour,
        minute,
        second,
        day,
        month,
        year: 2000 + year as u16,
    }
}

pub fn clock_string() -> alloc::string::String {
    let t = now();
    alloc::format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
}

pub fn date_string() -> alloc::string::String {
    let t = now();
    alloc::format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.year,
        t.month,
        t.day,
        t.hour,
        t.minute,
        t.second
    )
}
