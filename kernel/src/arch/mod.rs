pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod pic;
pub mod serial;
pub mod timer;
pub mod tss;

pub mod port {
    use core::arch::asm;

    #[inline]
    pub unsafe fn outb(port: u16, value: u8) {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }

    #[inline]
    pub unsafe fn inb(port: u16) -> u8 {
        let value: u8;
        asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
        value
    }

    #[inline]
    pub unsafe fn outw(port: u16, value: u16) {
        asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
    }

    #[inline]
    pub unsafe fn inw(port: u16) -> u16 {
        let value: u16;
        asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags));
        value
    }

    #[inline]
    pub unsafe fn outl(port: u16, value: u32) {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }

    #[inline]
    pub unsafe fn inl(port: u16) -> u32 {
        let value: u32;
        asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
        value
    }

    #[inline]
    pub unsafe fn io_wait() {
        outb(0x80, 0);
    }
}
