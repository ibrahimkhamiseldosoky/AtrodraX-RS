//! Physical memory manager — a bitmap covering the first 4 GiB.

use crate::boot;
use crate::mem::{FOUR_GIB, PAGE_SIZE};
use core::ptr::addr_of_mut;

pub const TOTAL_PAGES: usize = (FOUR_GIB as usize) / PAGE_SIZE;
const BITMAP_BYTES: usize = TOTAL_PAGES / 8;

static mut BITMAP: [u8; BITMAP_BYTES] = [0xFF; BITMAP_BYTES];
static mut FREE_PAGES: usize = 0;
static mut USED_PAGES: usize = 0;
static mut LAST_ALLOC: usize = 0;

extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

#[inline]
unsafe fn bitmap_get(index: usize) -> bool {
    (BITMAP[index / 8] & (1 << (index % 8))) != 0
}

#[inline]
unsafe fn bitmap_set(index: usize, used: bool) {
    let mask = 1u8 << (index % 8);
    if used {
        BITMAP[index / 8] |= mask;
    } else {
        BITMAP[index / 8] &= !mask;
    }
}

unsafe fn mark_used(start: u64, len: u64) {
    let first = (start as usize / PAGE_SIZE).min(TOTAL_PAGES);
    let last = (((start.saturating_add(len)) as usize).saturating_add(PAGE_SIZE - 1) / PAGE_SIZE).min(TOTAL_PAGES);
    let mut i = first;
    while i < last {
        if !bitmap_get(i) {
            bitmap_set(i, true);
            if FREE_PAGES > 0 {
                FREE_PAGES -= 1;
            }
            USED_PAGES += 1;
        }
        i += 1;
    }
}

unsafe fn mark_free(start: u64, len: u64) {
    let first = (start as usize / PAGE_SIZE).min(TOTAL_PAGES);
    let last = (((start.saturating_add(len)) as usize) / PAGE_SIZE).min(TOTAL_PAGES);
    let mut i = first;
    while i < last {
        if bitmap_get(i) {
            bitmap_set(i, false);
            FREE_PAGES += 1;
            if USED_PAGES > 0 {
                USED_PAGES -= 1;
            }
        }
        i += 1;
    }
}

pub fn init() {
    unsafe {
        for byte in BITMAP.iter_mut() {
            *byte = 0xFF;
        }
        FREE_PAGES = 0;
        USED_PAGES = TOTAL_PAGES;
        LAST_ALLOC = 0;

        // Start by reserving everything below 1 MiB (BIOS, VGA, EBDA).
        mark_used(0, 0x10_0000);

        for region in boot::regions() {
            if region.kind == 1 && region.base < FOUR_GIB {
                let clipped = region.len.min(FOUR_GIB - region.base);
                mark_free(region.base, clipped);
            }
        }

        // Reserve the kernel image (.text/.rodata/.data/.bss including heap).
        let kstart = core::ptr::addr_of!(__kernel_start) as u64;
        let kend = core::ptr::addr_of!(__kernel_end) as u64;
        mark_used(kstart, kend.saturating_sub(kstart));

        // Reserve the multiboot information snapshot.
        let (mb2, mb2_len) = boot::mb2_range();
        mark_used(mb2 as u64, mb2_len as u64 + PAGE_SIZE as u64);

        // Reserve the framebuffer.
        let fb = boot::framebuffer();
        if fb.address != 0 {
            mark_used(fb.address, fb.pitch as u64 * fb.height as u64);
        }

        // Reserve multiboot modules.
        for region in boot::regions() {
            if region.kind == 0xFFFF_FFFF {
                mark_used(region.base, region.len);
            }
        }

        crate::kprintln!(
            "[pmm] {} MiB usable, {} free pages, kernel {:#x}..{:#x}",
            boot::total_memory_bytes() / (1024 * 1024),
            FREE_PAGES,
            kstart,
            kend
        );
    }
}

pub fn alloc_page() -> Option<u64> {
    alloc_n(1).map(|p| p)
}

pub fn alloc_n(count: usize) -> Option<u64> {
    unsafe {
        if count == 0 || count > FREE_PAGES {
            return None;
        }
        let mut run_start = 0usize;
        let mut run = 0usize;
        let mut index = LAST_ALLOC;
        let mut scanned = 0usize;
        while scanned < TOTAL_PAGES {
            if index >= TOTAL_PAGES {
                index = 0;
            }
            if !bitmap_get(index) {
                if run == 0 {
                    run_start = index;
                }
                run += 1;
                if run == count {
                    for i in run_start..run_start + count {
                        bitmap_set(i, true);
                    }
                    FREE_PAGES -= count;
                    USED_PAGES += count;
                    LAST_ALLOC = run_start + count;
                    let phys = (run_start * PAGE_SIZE) as u64;
                    zero_range(phys, (count * PAGE_SIZE) as u64);
                    return Some(phys);
                }
            } else {
                run = 0;
            }
            index += 1;
            scanned += 1;
        }
    }
    None
}

pub fn free_page(page: u64) {
    unsafe {
        let index = page as usize / PAGE_SIZE;
        if index < TOTAL_PAGES && bitmap_get(index) {
            bitmap_set(index, false);
            FREE_PAGES += 1;
            USED_PAGES -= 1;
        }
    }
}

pub fn free_n(base: u64, count: usize) {
    for i in 0..count {
        free_page(base + (i * PAGE_SIZE) as u64);
    }
}

pub fn zero_range(addr: u64, len: u64) {
    unsafe {
        // `addr` is physical; reach it through the kernel's direct map rather
        // than dereferencing the physical address (which a user mapping could
        // shadow).
        let virt = crate::mem::vmm::phys_to_virt(addr);
        core::ptr::write_bytes(virt as *mut u8, 0, len as usize);
    }
}

pub fn free_pages() -> usize {
    unsafe { FREE_PAGES }
}

pub fn used_pages() -> usize {
    unsafe { USED_PAGES }
}

pub fn total_pages() -> usize {
    TOTAL_PAGES
}

pub fn print_info() {
    crate::kprintln!(
        "[pmm] total {} pages, free {}, used {}",
        total_pages(),
        free_pages(),
        used_pages()
    );
}
