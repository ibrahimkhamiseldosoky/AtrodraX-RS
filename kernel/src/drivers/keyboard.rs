//! PS/2 keyboard driver (scan code set 1) with a key-event ring buffer.

use crate::arch::port::inb;

pub const KEY_ESC: u16 = 0x1B;
pub const KEY_ENTER: u16 = 0x0A;
pub const KEY_BACKSPACE: u16 = 0x08;
pub const KEY_TAB: u16 = 0x09;
pub const KEY_UP: u16 = 0x100;
pub const KEY_DOWN: u16 = 0x101;
pub const KEY_LEFT: u16 = 0x102;
pub const KEY_RIGHT: u16 = 0x103;
pub const KEY_HOME: u16 = 0x104;
pub const KEY_END: u16 = 0x105;
pub const KEY_PGUP: u16 = 0x106;
pub const KEY_PGDN: u16 = 0x107;
pub const KEY_DELETE: u16 = 0x108;
pub const KEY_INSERT: u16 = 0x109;
pub const KEY_F1: u16 = 0x110;

const RING_SIZE: usize = 256;
static mut RING: [u16; RING_SIZE] = [0; RING_SIZE];
static mut HEAD: usize = 0;
static mut TAIL: usize = 0;
static mut SHIFT: bool = false;
static mut CTRL: bool = false;
static mut ALT: bool = false;
static mut CAPS: bool = false;
static mut EXTENDED: bool = false;
static mut RELEASE: bool = false;

const UNSHIFTED: [u8; 128] = [
    0, 27, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 8, 9,
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0, b'a', b's',
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\', b'z', b'x', b'c', b'v',
    b'b', b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ', 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const SHIFTED: [u8; 128] = [
    0, 27, b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')', b'_', b'+', 8, 9,
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I', b'O', b'P', b'{', b'}', b'\n', 0, b'A', b'S',
    b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':', b'"', b'~', 0, b'|', b'Z', b'X', b'C', b'V',
    b'B', b'N', b'M', b'<', b'>', b'?', 0, b'*', 0, b' ', 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

pub fn init() {
    unsafe {
        HEAD = 0;
        TAIL = 0;
        SHIFT = false;
        CTRL = false;
        ALT = false;
        CAPS = false;
        EXTENDED = false;
        RELEASE = false;
    }
    crate::kprintln!("[kbd] PS/2 keyboard ready (IRQ1)");
}

fn push(key: u16) {
    unsafe {
        let next = (HEAD + 1) % RING_SIZE;
        if next != TAIL {
            RING[HEAD] = key;
            HEAD = next;
        }
    }
}

pub fn poll() -> Option<u16> {
    unsafe {
        if TAIL == HEAD {
            None
        } else {
            let key = RING[TAIL];
            TAIL = (TAIL + 1) % RING_SIZE;
            Some(key)
        }
    }
}

pub fn has_input() -> bool {
    unsafe { HEAD != TAIL }
}

pub fn ctrl_down() -> bool {
    unsafe { CTRL }
}

pub fn alt_down() -> bool {
    unsafe { ALT }
}

pub fn shift_down() -> bool {
    unsafe { SHIFT }
}

pub fn on_irq() {
    let scancode = unsafe { inb(0x60) };
    handle_scancode(scancode);
}

fn handle_scancode(sc: u8) {
    unsafe {
        if sc == 0xE0 {
            EXTENDED = true;
            return;
        }
        if sc == 0xE1 {
            EXTENDED = false;
            return;
        }
        let released = sc & 0x80 != 0;
        let code = sc & 0x7F;

        if EXTENDED {
            EXTENDED = false;
            if !released {
                let key = match code {
                    0x48 => KEY_UP,
                    0x50 => KEY_DOWN,
                    0x4B => KEY_LEFT,
                    0x4D => KEY_RIGHT,
                    0x47 => KEY_HOME,
                    0x4F => KEY_END,
                    0x49 => KEY_PGUP,
                    0x51 => KEY_PGDN,
                    0x53 => KEY_DELETE,
                    0x52 => KEY_INSERT,
                    _ => 0,
                };
                if key != 0 {
                    push(key);
                }
            }
            return;
        }

        match code {
            0x2A | 0x36 => {
                SHIFT = !released;
                return;
            }
            0x1D => {
                CTRL = !released;
                return;
            }
            0x38 => {
                ALT = !released;
                return;
            }
            0x3A => {
                if !released {
                    CAPS = !CAPS;
                }
                return;
            }
            _ => {}
        }

        if released {
            return;
        }

        let key = match code {
            0x01 => KEY_ESC,
            0x1C => KEY_ENTER,
            0x0E => KEY_BACKSPACE,
            0x0F => KEY_TAB,
            0x3B..=0x44 => KEY_F1 + (code - 0x3B) as u16,
            0x57 => KEY_F1 + 10,
            0x58 => KEY_F1 + 11,
            _ => {
                let table = if SHIFT { &SHIFTED } else { &UNSHIFTED };
                let mut ch = table[code as usize];
                if CAPS && ch.is_ascii_alphabetic() {
                    ch = if SHIFT {
                        ch.to_ascii_lowercase()
                    } else {
                        ch.to_ascii_uppercase()
                    };
                }
                ch as u16
            }
        };

        if key != 0 {
            push(key);
        }
    }
}
