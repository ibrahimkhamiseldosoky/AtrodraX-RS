//! CPU identification, control-register helpers, and idle loops.

use core::arch::asm;

#[inline]
fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx:e}, ebx",
            "pop rbx",
            ebx = out(reg) ebx,
            inout("eax") leaf => eax,
            out("ecx") ecx,
            out("edx") edx,
            options(preserves_flags),
        );
    }
    (eax, ebx, ecx, edx)
}

pub fn vendor() -> [u8; 12] {
    let mut out = [0u8; 12];
    let (_, ebx, ecx, edx) = cpuid(0);
    out[0..4].copy_from_slice(&ebx.to_le_bytes());
    out[4..8].copy_from_slice(&edx.to_le_bytes());
    out[8..12].copy_from_slice(&ecx.to_le_bytes());
    out
}

pub fn brand() -> [u8; 48] {
    let mut out = [0u8; 48];
    for (index, leaf) in (0x8000_0002u32..=0x8000_0004).enumerate() {
        let (eax, ebx, ecx, edx) = cpuid(leaf);
        let bytes = [
            eax.to_le_bytes(),
            ebx.to_le_bytes(),
            ecx.to_le_bytes(),
            edx.to_le_bytes(),
        ];
        let offset = index * 16;
        for i in 0..16 {
            out[offset + i] = bytes[i / 4][i % 4];
        }
    }
    out
}

pub fn report() {
    let v = vendor();
    let vendor_str = core::str::from_utf8(&v).unwrap_or("unknown");
    let b = brand();
    let end = b.iter().position(|x| *x == 0).unwrap_or(b.len());
    let brand_str = core::str::from_utf8(&b[..end]).unwrap_or("unknown");
    crate::kprintln!("[cpu] vendor: {}", vendor_str.trim());
    crate::kprintln!("[cpu] model:  {}", brand_str.trim());
}

#[inline]
pub fn read_cr2() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

#[inline]
pub fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

#[inline]
pub unsafe fn write_cr3(value: u64) {
    asm!("mov cr3, {}", in(reg) value, options(nostack));
}

#[inline]
pub unsafe fn invlpg(vaddr: u64) {
    asm!("invlpg [{}]", in(reg) vaddr, options(nostack, preserves_flags));
}

#[inline]
pub fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((hi as u64) << 32) | lo as u64
}

#[inline]
pub unsafe fn wrmsr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline]
pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe { asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    ((hi as u64) << 32) | lo as u64
}

#[inline]
pub unsafe fn enable_interrupts() {
    asm!("sti", options(nomem, nostack));
}

#[inline]
pub unsafe fn disable_interrupts() {
    asm!("cli", options(nomem, nostack));
}

#[inline]
pub fn halt() {
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
}

#[inline]
pub fn halt_loop() -> ! {
    loop {
        unsafe { asm!("cli; hlt", options(nomem, nostack, preserves_flags)) };
    }
}
