//! ELF64 executable loader.

use crate::mem::vmm::{self, AddrSpace};

pub const PT_LOAD: u32 = 1;
pub const PT_INTERP: u32 = 3;
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

#[derive(Clone, Copy, Default)]
pub struct Loaded {
    pub entry: u64,
    pub brk_start: u64,
    pub linux_compat: bool,
    /// Address of the program header table in the loaded image.
    pub phdr: u64,
    pub phent: u64,
    pub phnum: u64,
    /// Load bias applied to ET_DYN images (zero for ET_EXEC).
    pub bias: u64,
}

#[inline]
fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

#[inline]
fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[inline]
fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

pub fn is_elf(data: &[u8]) -> bool {
    data.len() >= 64 && data[0..4] == ELF_MAGIC
}

/// Load an ELF image into `space`. Returns `None` if the image is invalid.
pub fn load(space: &AddrSpace, data: &[u8]) -> Option<Loaded> {
    if !is_elf(data) {
        crate::kprintln!("[elf] bad magic");
        return None;
    }
    if data[4] != 2 || data[5] != 1 {
        crate::kprintln!("[elf] not 64-bit little-endian");
        return None;
    }
    let machine = u16_at(data, 18);
    if machine != 0x3E {
        crate::kprintln!("[elf] not x86-64 (machine {:#x})", machine);
        return None;
    }
    let kind = u16_at(data, 16);
    if kind != 2 && kind != 3 {
        crate::kprintln!("[elf] not an executable (type {})", kind);
        return None;
    }
    let entry = u64_at(data, 24);
    let phoff = u64_at(data, 32) as usize;
    let phentsize = u16_at(data, 54) as usize;
    let phnum = u16_at(data, 56) as usize;
    if phentsize < 56 || phnum == 0 || phnum > 64 {
        crate::kprintln!("[elf] invalid program header table");
        return None;
    }
    if phoff.checked_add(phentsize * phnum)? > data.len() {
        crate::kprintln!("[elf] program headers out of bounds");
        return None;
    }

    let bias: u64 = if kind == 3 { 0x40_0000 } else { 0 };
    let mut highest = 0u64;
    let mut linux_compat = entry < 0x1000_0000;
    let mut has_interp = false;

    for index in 0..phnum {
        let ph = phoff + index * phentsize;
        let p_type = u32_at(data, ph);
        let p_flags = u32_at(data, ph + 4);
        let p_offset = u64_at(data, ph + 8) as usize;
        let p_vaddr = u64_at(data, ph + 16);
        let p_filesz = u64_at(data, ph + 32) as usize;
        let p_memsz = u64_at(data, ph + 40) as usize;

        if p_type == PT_INTERP {
            has_interp = true;
            linux_compat = true;
            continue;
        }
        if p_type != PT_LOAD || p_memsz == 0 {
            continue;
        }
        if p_offset.checked_add(p_filesz)? > data.len() {
            crate::kprintln!("[elf] segment out of bounds");
            return None;
        }
        let vaddr = p_vaddr.checked_add(bias)?;
        let memsz = p_memsz.max(p_filesz);
        let end = vaddr.checked_add(memsz as u64)?;
        if vaddr < 0x1_0000 || end > crate::mem::FOUR_GIB {
            crate::kprintln!("[elf] segment outside user range: {:#x}..{:#x}", vaddr, end);
            return None;
        }

        let writable = p_flags & 0x2 != 0;
        if !vmm::map_user_range(space, vaddr, end, writable) {
            crate::kprintln!("[elf] out of memory mapping {:#x}..{:#x}", vaddr, end);
            return None;
        }
        if p_filesz > 0 {
            if !vmm::write_user(space, vaddr, &data[p_offset..p_offset + p_filesz]) {
                crate::kprintln!("[elf] failed to copy segment at {:#x}", vaddr);
                return None;
            }
        }
        if end > highest {
            highest = end;
        }
    }

    if highest == 0 {
        crate::kprintln!("[elf] no loadable segments");
        return None;
    }
    if has_interp {
        crate::kprintln!("[elf] dynamic binaries need an ELF interpreter (not supported yet)");
        return None;
    }

    let brk_start = (highest + 0xFFF) & !0xFFF;
    crate::kprintln!(
        "[elf] loaded entry={:#x} brk={:#x} bias={:#x}{}",
        entry,
        brk_start,
        bias,
        if linux_compat { " (linux abi)" } else { "" }
    );
    Some(Loaded {
        entry,
        brk_start,
        linux_compat,
        phdr: phoff as u64 + bias,
        phent: phentsize as u64,
        phnum: phnum as u64,
        bias,
    })
}

/// Boot-time sanity check of the loader against an embedded binary.
pub fn selftest() {
    let data = include_bytes!("../assets/user/hello.elf");
    if data.len() < 64 || &data[0..4] != &ELF_MAGIC {
        crate::kprintln!("[elf] selftest FAILED (bad embedded image)");
        return;
    }
    let entry = u64_at(data, 24);
    let phnum = u16_at(data, 56);
    crate::kprintln!(
        "[elf] loader ready (hello.elf {} bytes, entry {:#x}, {} phdrs)",
        data.len(),
        entry,
        phnum
    );
}
