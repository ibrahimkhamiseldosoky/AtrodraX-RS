//! Virtual memory: per-process page tables cloned from the kernel identity map.
//!
//! The boot page tables identity-map the first 4 GiB with 2 MiB pages. A new
//! address space gets copies of the PDPT and the four page directories, so the
//! kernel mappings stay shared in content but user mappings never mutate the
//! boot tables. Splitting a 2 MiB entry creates an identity-mapped page table
//! with supervisor-only entries, then individual user PTEs are tagged U/S.

use crate::arch::cpu;
use crate::mem::pmm;
use crate::mem::PAGE_SIZE;

pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITABLE: u64 = 1 << 1;
pub const PTE_USER: u64 = 1 << 2;
pub const PTE_PCD: u64 = 1 << 4;
pub const PTE_PWT: u64 = 1 << 3;
pub const PTE_HUGE: u64 = 1 << 7;
pub const PTE_NX: u64 = 1 << 63;
const PHYS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Where the kernel's direct map of physical memory lives. PML4 slot 511
/// aliases the boot page tables, so `DIRECT_MAP_BASE + paddr` always refers to
/// physical memory regardless of what the user half of the address space maps.
/// The kernel therefore never treats a physical address as a pointer.
pub const DIRECT_MAP_BASE: u64 = 0xFFFF_FF80_0000_0000;

#[inline]
pub fn phys_to_virt(paddr: u64) -> u64 {
    DIRECT_MAP_BASE + paddr
}


#[derive(Clone, Copy)]
pub struct AddrSpace {
    pub pml4_phys: u64,
}

extern "C" {
    static boot_pml4: u8;
    static boot_pdpt: u8;
    static boot_pd0: u8;
    static boot_pd1: u8;
    static boot_pd2: u8;
    static boot_pd3: u8;
}

static mut KERNEL_CR3: u64 = 0;

pub fn init() {
    unsafe {
        KERNEL_CR3 = core::ptr::addr_of!(boot_pml4) as u64;
    }
    crate::kprintln!("[vmm] kernel CR3 = {:#x}", kernel_cr3());
}

pub fn kernel_cr3() -> u64 {
    unsafe { KERNEL_CR3 }
}

pub fn load(cr3: u64) {
    unsafe { cpu::write_cr3(cr3) };
}

unsafe fn table_at(phys: u64) -> *mut u64 {
    phys_to_virt(phys) as *mut u64
}

pub fn create() -> Option<AddrSpace> {
    unsafe {
        let pml4 = pmm::alloc_page()?;
        let pdpt = pmm::alloc_page()?;
        let mut pds = [0u64; 4];
        for slot in pds.iter_mut() {
            *slot = pmm::alloc_page()?;
        }

        // Pages come from the physical allocator uninitialized; clear the top
        // two levels so stale content cannot act as page-table entries.
        core::ptr::write_bytes(table_at(pml4) as *mut u8, 0u8, 512 * 8);
        core::ptr::write_bytes(table_at(pdpt) as *mut u8, 0u8, 512 * 8);

        // Clone the four 1 GiB page directories.
        let boot_pds = [
            core::ptr::addr_of!(boot_pd0) as u64,
            core::ptr::addr_of!(boot_pd1) as u64,
            core::ptr::addr_of!(boot_pd2) as u64,
            core::ptr::addr_of!(boot_pd3) as u64,
        ];
        for i in 0..4 {
            core::ptr::copy_nonoverlapping(
                boot_pds[i] as *const u64,
                pds[i] as *mut u64,
                512,
            );
            *table_at(pdpt).add(i) = pds[i] | PTE_PRESENT | PTE_WRITABLE | PTE_USER;
        }
        *table_at(pml4) = pdpt | PTE_PRESENT | PTE_WRITABLE | PTE_USER;
        // Share the kernel's direct map (PML4 slot 511). The boot tables it
        // points at are never modified, so every address space gets the same
        // supervisor-only view of physical memory.
        let boot_pdpt_phys = core::ptr::addr_of!(boot_pdpt) as u64;
        *table_at(pml4).add(511) = boot_pdpt_phys | PTE_PRESENT | PTE_WRITABLE;

        Some(AddrSpace { pml4_phys: pml4 })
    }
}

pub fn load_space(space: &AddrSpace) {
    load(space.pml4_phys);
}

unsafe fn ensure_page_table(space: &AddrSpace, pd_index: usize) -> Option<*mut u64> {
    let pml4 = table_at(space.pml4_phys);
    let pdpt_phys = (*pml4) & PHYS_MASK;
    if pdpt_phys == 0 {
        return None;
    }
    let pdpt = table_at(pdpt_phys);
    let pd_phys = (*pdpt.add(pd_index)) & PHYS_MASK;
    if pd_phys == 0 {
        return None;
    }
    let pd = table_at(pd_phys);
    Some(pd)
}

/// Map one 4 KiB user page at `vaddr` to `paddr`.
pub fn map_user_page(space: &AddrSpace, vaddr: u64, paddr: u64, writable: bool) -> bool {
    let vaddr = vaddr & !0xFFF;
    if vaddr >= crate::mem::FOUR_GIB {
        return false;
    }
    let pdpt_index = ((vaddr >> 30) & 0x1FF) as usize;
    let pd_index = ((vaddr >> 21) & 0x1FF) as usize;
    let pt_index = ((vaddr >> 12) & 0x1FF) as usize;

    unsafe {
        let pd = match ensure_page_table(space, pdpt_index) {
            Some(pd) => pd,
            None => return false,
        };

        let mut entry = *pd.add(pd_index);
        if entry & PTE_HUGE != 0 {
            // Split the 2 MiB mapping into a 512-entry page table. The new
            // table must keep the original identity mapping: entry `i` covers
            // `base + i * 4 KiB`, not `i * 2 MiB`.
            let pt_phys = match pmm::alloc_page() {
                Some(p) => p,
                None => return false,
            };
            let pt = table_at(pt_phys);
            let base = entry & 0x000F_FFFF_FFE0_0000;
            for i in 0..512 {
                *pt.add(i) = (base + (i as u64 * PAGE_SIZE as u64)) | PTE_PRESENT | PTE_WRITABLE;
            }
            entry = pt_phys | PTE_PRESENT | PTE_WRITABLE | PTE_USER;
            *pd.add(pd_index) = entry;
        } else if entry & PTE_PRESENT == 0 {
            let pt_phys = match pmm::alloc_page() {
                Some(p) => p,
                None => return false,
            };
            entry = pt_phys | PTE_PRESENT | PTE_WRITABLE | PTE_USER;
            *pd.add(pd_index) = entry;
        }

        let pt_phys = entry & PHYS_MASK;
        if pt_phys == 0 {
            return false;
        }
        let pt = table_at(pt_phys);
        let mut flags = PTE_PRESENT | PTE_USER;
        if writable {
            flags |= PTE_WRITABLE;
        }
        *pt.add(pt_index) = (paddr & PHYS_MASK) | flags;
        cpu::invlpg(vaddr);
        true
    }
}

pub fn alloc_user_page(space: &AddrSpace, vaddr: u64, writable: bool) -> Option<u64> {
    let paddr = pmm::alloc_page()?;
    if map_user_page(space, vaddr, paddr, writable) {
        Some(paddr)
    } else {
        pmm::free_page(paddr);
        None
    }
}

pub fn map_user_range(space: &AddrSpace, start: u64, end: u64, writable: bool) -> bool {
    let mut vaddr = start & !0xFFF;
    let end = (end + 0xFFF) & !0xFFF;
    while vaddr < end {
        if !is_4k_mapped(space, vaddr) && alloc_user_page(space, vaddr, writable).is_none() {
            return false;
        }
        vaddr += PAGE_SIZE as u64;
    }
    true
}

/// True when a 4 KiB PTE already backs `vaddr` (a 2 MiB huge mapping does not
/// count, because user mappings need per-page control).
pub fn is_4k_mapped(space: &AddrSpace, vaddr: u64) -> bool {
    unsafe {
        let pml4 = table_at(space.pml4_phys);
        let pdpt_phys = (*pml4) & PHYS_MASK;
        if pdpt_phys == 0 {
            return false;
        }
        let pdpt = table_at(pdpt_phys);
        let pd_phys = (*pdpt.add(((vaddr >> 30) & 0x1FF) as usize)) & PHYS_MASK;
        if pd_phys == 0 {
            return false;
        }
        let pd = table_at(pd_phys);
        let pd_entry = *pd.add(((vaddr >> 21) & 0x1FF) as usize);
        if pd_entry & PTE_PRESENT == 0 || pd_entry & PTE_HUGE != 0 {
            return false;
        }
        let pt_phys = pd_entry & PHYS_MASK;
        if pt_phys == 0 {
            return false;
        }
        let pt = table_at(pt_phys);
        (*pt.add(((vaddr >> 12) & 0x1FF) as usize)) & (PTE_PRESENT | PTE_USER)
            == (PTE_PRESENT | PTE_USER)
    }
}

pub fn translate(space: &AddrSpace, vaddr: u64) -> Option<u64> {    unsafe {
        let pml4 = table_at(space.pml4_phys);
        let pdpt_phys = (*pml4) & PHYS_MASK;
        if pdpt_phys == 0 {
            return None;
        }
        let pdpt = table_at(pdpt_phys);
        let pd_phys = (*pdpt.add(((vaddr >> 30) & 0x1FF) as usize)) & PHYS_MASK;
        if pd_phys == 0 {
            return None;
        }
        let pd = table_at(pd_phys);
        let pd_entry = *pd.add(((vaddr >> 21) & 0x1FF) as usize);
        if pd_entry & PTE_PRESENT == 0 {
            return None;
        }
        if pd_entry & PTE_HUGE != 0 {
            return Some((pd_entry & 0x000F_FFFF_FFE0_0000) + (vaddr & 0x1F_FFFF));
        }
        let pt_phys = pd_entry & PHYS_MASK;
        if pt_phys == 0 {
            return None;
        }
        let pt = table_at(pt_phys);
        let entry = *pt.add(((vaddr >> 12) & 0x1FF) as usize);
        if entry & PTE_PRESENT == 0 {
            return None;
        }
        Some((entry & PHYS_MASK) + (vaddr & 0xFFF))
    }
}

pub fn destroy(space: AddrSpace) {
    unsafe {
        let pml4 = table_at(space.pml4_phys);
        let pdpt_phys = (*pml4) & PHYS_MASK;
        if pdpt_phys != 0 {
            let pdpt = table_at(pdpt_phys);
            for pdpt_index in 0..4 {
                let pd_phys = (*pdpt.add(pdpt_index)) & PHYS_MASK;
                if pd_phys == 0 {
                    continue;
                }
                let pd = table_at(pd_phys);
                for pd_index in 0..512 {
                    let pd_entry = *pd.add(pd_index);
                    if pd_entry & PTE_PRESENT == 0 || pd_entry & PTE_HUGE != 0 {
                        continue;
                    }
                    let pt_phys = pd_entry & PHYS_MASK;
                    if pt_phys == 0 {
                        continue;
                    }
                    let pt = table_at(pt_phys);
                    let mut has_user = false;
                    for pt_index in 0..512 {
                        let entry = *pt.add(pt_index);
                        if entry & PTE_PRESENT != 0 && entry & PTE_USER != 0 {
                            has_user = true;
                            pmm::free_page(entry & PHYS_MASK);
                        }
                    }
                    if has_user {
                        pmm::free_page(pt_phys);
                    }
                    // Keep non-user (kernel identity) page tables: they are
                    // copies, but only free them when they contain user pages.
                }
                pmm::free_page(pd_phys);
            }
            pmm::free_page(pdpt_phys);
        }
        pmm::free_page(space.pml4_phys);
    }
}

/// Deep-copy all user mappings into a new address space (fork).
pub fn copy_space(source: &AddrSpace) -> Option<AddrSpace> {
    let dest = create()?;
    unsafe {
        let src_pml4 = table_at(source.pml4_phys);
        let src_pdpt = table_at((*src_pml4) & PHYS_MASK);
        for pdpt_index in 0..4 {
            let pd_phys = (*src_pdpt.add(pdpt_index)) & PHYS_MASK;
            if pd_phys == 0 {
                continue;
            }
            let pd = table_at(pd_phys);
            for pd_index in 0..512 {
                let pd_entry = *pd.add(pd_index);
                if pd_entry & PTE_PRESENT == 0 || pd_entry & PTE_HUGE != 0 {
                    continue;
                }
                let pt = table_at(pd_entry & PHYS_MASK);
                for pt_index in 0..512 {
                    let entry = *pt.add(pt_index);
                    if entry & PTE_PRESENT != 0 && entry & PTE_USER != 0 {
                        let vaddr = ((pdpt_index as u64) << 30)
                            | ((pd_index as u64) << 21)
                            | ((pt_index as u64) << 12);
                        let new_page = pmm::alloc_page()?;
                        core::ptr::copy_nonoverlapping(
                            phys_to_virt(entry & PHYS_MASK) as *const u8,
                            phys_to_virt(new_page) as *mut u8,
                            PAGE_SIZE,
                        );
                        map_user_page(&dest, vaddr, new_page, entry & PTE_WRITABLE != 0);
                    }
                }
            }
        }
    }
    Some(dest)
}

/// Copy bytes into a user address space through the kernel's direct map.
pub fn write_user(space: &AddrSpace, vaddr: u64, data: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset < data.len() {
        let va = vaddr + offset as u64;
        let pa = match translate(space, va) {
            Some(pa) => pa,
            None => return false,
        };
        let page_off = (va & 0xFFF) as usize;
        let chunk = (PAGE_SIZE - page_off).min(data.len() - offset);
        unsafe {
            core::ptr::copy_nonoverlapping(
                data.as_ptr().add(offset),
                phys_to_virt(pa) as *mut u8,
                chunk,
            );
        }
        offset += chunk;
    }
    true
}
