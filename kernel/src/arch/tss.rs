//! 64-bit Task State Segment. Supplies RSP0 for ring-3 -> ring-0 transitions.

use core::mem::size_of;
use core::ptr::addr_of_mut;

pub const TSS_SELECTOR: u16 = 0x28;

#[repr(C, packed)]
pub struct Tss {
    reserved0: u32,
    pub rsp0: u64,
    rsp1: u64,
    rsp2: u64,
    reserved1: u64,
    ist1: u64,
    ist2: u64,
    ist3: u64,
    ist4: u64,
    ist5: u64,
    ist6: u64,
    ist7: u64,
    reserved2: u64,
    reserved3: u16,
    iomap_base: u16,
}

impl Tss {
    const fn zeroed() -> Self {
        Self {
            reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved1: 0,
            ist1: 0,
            ist2: 0,
            ist3: 0,
            ist4: 0,
            ist5: 0,
            ist6: 0,
            ist7: 0,
            reserved2: 0,
            reserved3: 0,
            iomap_base: 0,
        }
    }
}

static mut TSS: Tss = Tss::zeroed();

extern "C" {
    static boot_stack_top: u8;
}

pub fn init() {
    unsafe {
        addr_of_mut!(TSS.iomap_base).write(size_of::<Tss>() as u16);
        addr_of_mut!(TSS.rsp0).write(core::ptr::addr_of!(boot_stack_top) as u64);
    }
}

pub fn set_rsp0(value: u64) {
    unsafe { addr_of_mut!(TSS.rsp0).write(value) };
}

pub fn rsp0() -> u64 {
    unsafe { core::ptr::addr_of!(TSS.rsp0).read() }
}

pub fn base() -> u64 {
    core::ptr::addr_of!(TSS) as u64
}

pub fn limit() -> u64 {
    (size_of::<Tss>() - 1) as u64
}
