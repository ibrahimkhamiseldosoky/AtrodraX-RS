//! PS/2 mouse driver (IRQ12).

use crate::arch::port::{inb, outb};

const DATA_PORT: u16 = 0x60;
const STATUS_PORT: u16 = 0x64;
const COMMAND_PORT: u16 = 0x64;

#[derive(Clone, Copy, Default)]
pub struct MouseState {
    pub x: i32,
    pub y: i32,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

static mut STATE: MouseState = MouseState {
    x: 512,
    y: 384,
    left: false,
    right: false,
    middle: false,
};
static mut PACKET: [u8; 3] = [0; 3];
static mut PACKET_INDEX: usize = 0;

fn wait_input() {
    for _ in 0..100_000 {
        unsafe {
            if inb(STATUS_PORT) & 2 == 0 {
                return;
            }
        }
    }
}

fn wait_output() {
    for _ in 0..100_000 {
        unsafe {
            if inb(STATUS_PORT) & 1 != 0 {
                return;
            }
        }
    }
}

unsafe fn command(cmd: u8) {
    wait_input();
    outb(COMMAND_PORT, cmd);
}

unsafe fn data(value: u8) {
    wait_input();
    outb(DATA_PORT, value);
}

pub fn init() {
    unsafe {
        command(0xA8); // enable AUX port
        command(0x20);
        wait_output();
        let mut status = inb(DATA_PORT);
        status |= 0x02;
        status &= !0x20;
        command(0x60);
        data(status);

        command(0xD4);
        data(0xF4); // enable data reporting
        wait_output();
        let _ack = inb(DATA_PORT);

        STATE = MouseState {
            x: 512,
            y: 384,
            left: false,
            right: false,
            middle: false,
        };
        PACKET_INDEX = 0;
    }
    crate::kprintln!("[mouse] PS/2 mouse ready (IRQ12)");
}

pub fn state() -> MouseState {
    unsafe { STATE }
}

pub fn set_position(x: i32, y: i32) {
    unsafe {
        STATE.x = x.clamp(0, crate::drivers::fb::width() as i32 - 1);
        STATE.y = y.clamp(0, crate::drivers::fb::height() as i32 - 1);
    }
}

pub fn on_irq() {
    let byte = unsafe { inb(DATA_PORT) };
    unsafe {
        match PACKET_INDEX {
            0 => {
                if byte & 0x08 == 0 {
                    return;
                }
                PACKET[0] = byte;
                PACKET_INDEX = 1;
            }
            1 => {
                PACKET[1] = byte;
                PACKET_INDEX = 2;
            }
            _ => {
                PACKET[2] = byte;
                PACKET_INDEX = 0;
                let flags = PACKET[0];
                if flags & 0xC0 != 0 {
                    return;
                }
                let mut dx = PACKET[1] as i32;
                let mut dy = PACKET[2] as i32;
                if flags & 0x10 != 0 {
                    dx -= 256;
                }
                if flags & 0x20 != 0 {
                    dy -= 256;
                }
                let max_x = crate::drivers::fb::width() as i32 - 1;
                let max_y = crate::drivers::fb::height() as i32 - 1;
                STATE.x = (STATE.x + dx).clamp(0, max_x.max(0));
                STATE.y = (STATE.y - dy).clamp(0, max_y.max(0));
                STATE.left = flags & 0x01 != 0;
                STATE.right = flags & 0x02 != 0;
                STATE.middle = flags & 0x04 != 0;
            }
        }
    }
}
