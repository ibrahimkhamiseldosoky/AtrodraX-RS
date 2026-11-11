//! Global Descriptor Table: kernel/user code+data and the TSS descriptor.

use core::arch::asm;
use crate::arch::tss;

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const USER_DS: u16 = 0x18;
pub const USER_CS: u16 = 0x20;

const GDT_LEN: usize = 7;
static mut GDT: [u64; GDT_LEN] = [0; GDT_LEN];

#[repr(C, packed)]
struct Gdtr {
    limit: u16,
    base: u64,
}

impl Gdtr {
    fn new() -> Self {
        Self {
            limit: (GDT_LEN * 8 - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        }
    }
}

extern "C" {
    fn gdt_flush(gdtr: *const Gdtr);
}

pub fn init() {
    unsafe {
        GDT[0] = 0;
        GDT[1] = 0x00AF_9A00_0000_FFFF; // kernel code
        GDT[2] = 0x00AF_9200_0000_FFFF; // kernel data
        GDT[3] = 0x00AF_F200_0000_FFFF; // user data
        GDT[4] = 0x00AF_FA00_0000_FFFF; // user code

        let base = tss::base();
        let limit = tss::limit();
        let low = (limit & 0xFFFF)
            | ((base & 0xFF_FFFF) << 16)
            | (0x89u64 << 40)
            | (((limit >> 16) & 0xF) << 48)
            | (((base >> 24) & 0xFF) << 56);
        let high = base >> 32;
        GDT[5] = low;
        GDT[6] = high;

        let gdtr = Gdtr::new();
        gdt_flush(&gdtr);

        let selector = tss::TSS_SELECTOR;
        asm!("ltr {0:x}", in(reg) selector, options(nostack));
    }
}
