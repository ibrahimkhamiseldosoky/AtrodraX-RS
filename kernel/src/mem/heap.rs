//! Kernel heap: an interrupt-safe first-fit free-list allocator.
//!
//! The arena is a large `static` in .bss, so it is part of the kernel image
//! reservation and needs no special mapping.

use crate::sync::without_interrupts;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{addr_of_mut, null_mut};

const HEAP_SIZE: usize = 64 * 1024 * 1024;
const HDR_SIZE: usize = 24; // start: usize, size: usize, next: *mut FreeBlock
const MIN_SPLIT: usize = 32;

#[repr(align(4096))]
struct HeapArena([u8; HEAP_SIZE]);

static mut ARENA: HeapArena = HeapArena([0; HEAP_SIZE]);

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

#[repr(C)]
struct AllocHeader {
    start: usize,
    size: usize,
    next: *mut FreeBlock,
}

pub struct FreeListAllocator {
    head: *mut FreeBlock,
    initialized: bool,
}

unsafe impl Sync for FreeListAllocator {}

#[inline]
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

impl FreeListAllocator {
    pub const fn empty() -> Self {
        Self {
            head: null_mut(),
            initialized: false,
        }
    }

    unsafe fn init(&mut self) {
        if self.initialized {
            return;
        }
        let base = core::ptr::addr_of_mut!(ARENA.0) as *mut u8;
        let block = base as *mut FreeBlock;
        (*block).size = HEAP_SIZE;
        (*block).next = null_mut();
        self.head = block;
        self.initialized = true;
    }

    unsafe fn alloc_inner(&mut self, layout: Layout) -> *mut u8 {
        if !self.initialized {
            self.init();
        }
        let align = layout.align().max(16);
        let size = layout.size().max(1);

        let mut prev: *mut FreeBlock = null_mut();
        let mut cur = self.head;
        while !cur.is_null() {
            let block_start = cur as usize;
            let block_size = (*cur).size;
            let payload = align_up(block_start + HDR_SIZE, align);
            let mut used = align_up((payload + size).saturating_sub(block_start), 16);

            if used <= block_size {
                if block_size - used >= HDR_SIZE + MIN_SPLIT {
                    let rest = (block_start + used) as *mut FreeBlock;
                    (*rest).size = block_size - used;
                    (*rest).next = (*cur).next;
                    if prev.is_null() {
                        self.head = rest;
                    } else {
                        (*prev).next = rest;
                    }
                } else {
                    used = block_size;
                    if prev.is_null() {
                        self.head = (*cur).next;
                    } else {
                        (*prev).next = (*cur).next;
                    }
                }

                let header = (payload - HDR_SIZE) as *mut AllocHeader;
                (*header).start = block_start;
                (*header).size = used;
                (*header).next = null_mut();
                return payload as *mut u8;
            }
            prev = cur;
            cur = (*cur).next;
        }
        null_mut()
    }

    unsafe fn dealloc_inner(&mut self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        let heap_start = core::ptr::addr_of!(ARENA.0) as usize;
        let heap_end = heap_start + HEAP_SIZE;
        let addr = ptr as usize;
        if addr < heap_start + HDR_SIZE || addr >= heap_end {
            return;
        }

        let header = (addr - HDR_SIZE) as *const AllocHeader;
        let start = (*header).start;
        let size = (*header).size;
        if start < heap_start || start >= heap_end || size == 0 || start + size > heap_end {
            return;
        }

        let block = start as *mut FreeBlock;
        (*block).size = size;
        (*block).next = null_mut();

        let mut prev: *mut FreeBlock = null_mut();
        let mut cur = self.head;
        while !cur.is_null() && (cur as usize) < start {
            prev = cur;
            cur = (*cur).next;
        }

        (*block).next = cur;
        if prev.is_null() {
            self.head = block;
        } else {
            (*prev).next = block;
        }

        if !cur.is_null() && start + (*block).size == cur as usize {
            (*block).size += (*cur).size;
            (*block).next = (*cur).next;
        }
        if !prev.is_null() && (prev as usize) + (*prev).size == start {
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
        }
    }
}

static mut ALLOCATOR: FreeListAllocator = FreeListAllocator::empty();

pub fn init() {
    unsafe {
        let allocator = addr_of_mut!(ALLOCATOR);
        (*allocator).init();
    }
    crate::kprintln!("[heap] {} MiB free-list arena initialized", HEAP_SIZE / (1024 * 1024));
}

pub fn stats() -> (usize, usize) {
    unsafe {
        let allocator = addr_of_mut!(ALLOCATOR);
        let mut free = 0usize;
        let mut blocks = 0usize;
        let mut cur = (*allocator).head;
        while !cur.is_null() {
            free += (*cur).size;
            blocks += 1;
            cur = (*cur).next;
        }
        (free, blocks)
    }
}

pub fn print_info() {
    let (free, blocks) = stats();
    crate::kprintln!("[heap] {} KiB free in {} blocks", free / 1024, blocks);
}

unsafe impl GlobalAlloc for FreeListAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        without_interrupts(|| {
            let allocator = addr_of_mut!(ALLOCATOR);
            (*allocator).alloc_inner(layout)
        })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        without_interrupts(|| {
            let allocator = addr_of_mut!(ALLOCATOR);
            (*allocator).dealloc_inner(ptr)
        })
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: FreeListAllocator = FreeListAllocator::empty();
