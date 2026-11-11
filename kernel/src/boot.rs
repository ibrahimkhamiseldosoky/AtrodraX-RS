//! Multiboot2 information parsing.

use core::ptr::{read_unaligned, addr_of};

#[derive(Clone, Copy, Default)]
pub struct MemoryRegion {
    pub base: u64,
    pub len: u64,
    pub kind: u32,
}

#[derive(Clone, Copy, Default)]
pub struct FramebufferInfo {
    pub address: u64,
    pub pitch: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u8,
    pub kind: u8,
}

const MAX_REGIONS: usize = 64;
static mut REGIONS: [MemoryRegion; MAX_REGIONS] = [MemoryRegion { base: 0, len: 0, kind: 0 }; MAX_REGIONS];
static mut REGION_COUNT: usize = 0;
static mut MB2_PHYS: usize = 0;
static mut MB2_SIZE: usize = 0;
static mut FB: FramebufferInfo = FramebufferInfo {
    address: 0,
    pitch: 0,
    width: 0,
    height: 0,
    bpp: 0,
    kind: 0,
};

pub fn init(mb2_phys: usize) {
    unsafe {
        MB2_PHYS = mb2_phys;
        let total = read_unaligned(mb2_phys as *const u32) as usize;
        MB2_SIZE = total;
        crate::kprintln!("[mb2] info at {:#x}, {} bytes", mb2_phys, total);

        let mut ptr = mb2_phys + 8;
        let end = mb2_phys + total;

        // A memory map tag is authoritative.  GRUB also emits a basic-meminfo
        // tag, which covers the same RAM; using both would double-count every
        // usable page (and inflate the PMM free counters).
        let mut has_mmap = false;
        let mut scan = ptr;
        while scan + 8 <= end {
            let kind = read_unaligned(scan as *const u32);
            let size = read_unaligned((scan + 4) as *const u32) as usize;
            if kind == 0 || size < 8 {
                break;
            }
            if kind == 6 {
                has_mmap = true;
                break;
            }
            scan = (scan + size + 7) & !7;
        }

        while ptr + 8 <= end {
            let kind = read_unaligned(ptr as *const u32);
            let size = read_unaligned((ptr + 4) as *const u32) as usize;
            if kind == 0 || size < 8 {
                break;
            }
            match kind {
                3 => parse_module(ptr),
                4 => {
                    if !has_mmap {
                        parse_basic_meminfo(ptr);
                    }
                }
                6 => parse_mmap(ptr, size),
                8 => parse_framebuffer(ptr),
                _ => {}
            }
            ptr = (ptr + size + 7) & !7;
        }

        crate::kprintln!(
            "[mb2] {} memory regions, framebuffer {}x{}x{} @ {:#x} pitch {}",
            REGION_COUNT,
            FB.width,
            FB.height,
            FB.bpp,
            FB.address,
            FB.pitch
        );
    }
}

unsafe fn parse_basic_meminfo(ptr: usize) {
    let mem_lower = read_unaligned((ptr + 8) as *const u32) as u64;
    let mem_upper = read_unaligned((ptr + 12) as *const u32) as u64;
    if REGION_COUNT < MAX_REGIONS {
        REGIONS[REGION_COUNT] = MemoryRegion {
            base: 0,
            len: mem_lower * 1024,
            kind: 1,
        };
        REGION_COUNT += 1;
    }
    if REGION_COUNT < MAX_REGIONS {
        REGIONS[REGION_COUNT] = MemoryRegion {
            base: 0x10_0000,
            len: mem_upper * 1024,
            kind: 1,
        };
        REGION_COUNT += 1;
    }
}

unsafe fn parse_mmap(ptr: usize, _size: usize) {
    let entry_size = read_unaligned((ptr + 8) as *const u32) as usize;
    let mut entry = ptr + 16;
    let end = ptr + _size;
    let mut accepted = 0;
    while entry + entry_size <= end && entry_size >= 24 {
        let base = read_unaligned(entry as *const u64);
        let len = read_unaligned((entry + 8) as *const u64);
        let kind = read_unaligned((entry + 16) as *const u32);
        // Prefer the mmap over the basic meminfo tags: drop duplicates later.
        if REGION_COUNT < MAX_REGIONS && len > 0 {
            REGIONS[REGION_COUNT] = MemoryRegion { base, len, kind };
            REGION_COUNT += 1;
            accepted += 1;
        }
        entry += entry_size;
    }
    crate::kprintln!("[mb2] mmap: {} entries", accepted);
}

unsafe fn parse_framebuffer(ptr: usize) {
    FB.address = read_unaligned((ptr + 8) as *const u64);
    FB.pitch = read_unaligned((ptr + 16) as *const u32);
    FB.width = read_unaligned((ptr + 20) as *const u32);
    FB.height = read_unaligned((ptr + 24) as *const u32);
    FB.bpp = read_unaligned((ptr + 28) as *const u8);
    FB.kind = read_unaligned((ptr + 29) as *const u8);
}

unsafe fn parse_module(ptr: usize) {
    let start = read_unaligned((ptr + 8) as *const u32) as u64;
    let end = read_unaligned((ptr + 12) as *const u32) as u64;
    if REGION_COUNT < MAX_REGIONS {
        REGIONS[REGION_COUNT] = MemoryRegion {
            base: start,
            len: end.saturating_sub(start),
            kind: 0xFFFF_FFFF,
        };
        REGION_COUNT += 1;
    }
}

pub fn regions() -> &'static [MemoryRegion] {
    unsafe {
        let ptr = addr_of!(REGIONS) as *const MemoryRegion;
        core::slice::from_raw_parts(ptr, REGION_COUNT)
    }
}

pub fn framebuffer() -> FramebufferInfo {
    unsafe { FB }
}

pub fn mb2_range() -> (usize, usize) {
    unsafe { (MB2_PHYS, MB2_SIZE) }
}

pub fn total_memory_bytes() -> u64 {
    regions()
        .iter()
        .filter(|r| r.kind == 1)
        .map(|r| r.len)
        .sum()
}
