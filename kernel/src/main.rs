#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![allow(
    static_mut_refs,
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_unsafe,
    non_snake_case,
    unreachable_code,
    overflowing_literals,
    improper_ctypes,
    asm_sub_register
)]

extern crate alloc;

#[macro_use]
pub mod diag;
pub mod arch;
pub mod boot;
pub mod sync;
pub mod mem;
pub mod drivers;
pub mod fs;
pub mod net;
pub mod gfx;
pub mod apps;
pub mod task;
pub mod sched;
pub mod elf;
pub mod exec;
pub mod syscall;
pub mod system;

use core::panic::PanicInfo;

/* ── Multiboot2 header ──────────────────────────────────────────────────────
 * magic, arch, length, checksum, framebuffer tag (1024x768x32), end tag.
 */
#[repr(C, align(8))]
struct Mb2Header([u32; 12]);

#[link_section = ".multiboot2"]
#[used]
static MB2_HEADER: Mb2Header = Mb2Header([
    0xE852_50D6,
    0,
    48,
    0x17AD_AEFA,
    5,      /* framebuffer tag */
    20,
    1024,
    768,
    32,
    0,      /* padding */
    0,      /* end tag */
    8,
]);

core::arch::global_asm!(include_str!("entry.S"), options(att_syntax));

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    diag::panic::report(info);
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    diag::panic::alloc_failure(layout);
}

/// 64-bit kernel entry point, called from `entry.S`.
#[no_mangle]
pub extern "C" fn kernel_main(magic: u32, mb2_phys: u64) -> ! {
    arch::serial::init();
    kprintln!("");
    kprintln!("===================== AtrodraX v0.1 (Rust) =====================");
    kprintln!("[boot] long mode active, kernel entered");

    if magic != 0x36D7_6289 {
        kprintln!("[boot] FATAL: not booted by a multiboot2 loader (magic={:#x})", magic);
        diag::panic::halt_forever()
    }

    boot::init(mb2_phys as usize);

    arch::gdt::init();
    arch::tss::init();
    arch::idt::init();
    arch::pic::init();

    mem::pmm::init();
    mem::heap::init();
    mem::vmm::init();

    arch::timer::init();
    drivers::fb::init();
    drivers::rtc::init();

    syscall::init();
    task::init();
    sched::init();
    elf::selftest();

    drivers::keyboard::init();
    drivers::mouse::init();
    drivers::pci::init();
    drivers::e1000::init();
    drivers::virtio_blk::init();

    fs::vfs::init();
    fs::ramfs::populate();
    fs::fat32::mount();
    fs::persist::load();

    gfx::window::init();
    gfx::font::init();
    gfx::compositor::init();

    net::init();

    diag::tests::run_all();
    arch::cpu::report();

    kprintln!("[boot] enabling interrupts");
    unsafe { arch::cpu::enable_interrupts() };

    system::run()
}
