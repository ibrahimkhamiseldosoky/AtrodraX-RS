//! Legacy virtio-blk PCI driver (QEMU `virtio-blk-pci`, transitional device).
//!
//! Uses the legacy I/O BAR interface with a single split virtqueue. Requests
//! are synchronous: a 3-descriptor chain (header, data, status) is submitted
//! and the used ring is polled until completion.

use crate::arch::port::{inb, inl, inw, outb, outl, outw};
use crate::drivers::pci;
use crate::mem::pmm;
use core::sync::atomic::{fence, Ordering};

const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_BLK_LEGACY: u16 = 0x1001;

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;

const REQ_READ: u32 = 0;
const REQ_WRITE: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct Desc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[derive(Clone, Copy, Default)]
struct UsedElem {
    id: u32,
    len: u32,
}

struct Device {
    io: u16,
    queue: u64,
    request: u64,
    capacity: u64,
    queue_size: u16,
    avail_idx: u16,
    used_seen: u16,
}

static mut DEV: Option<Device> = None;

pub fn present() -> bool {
    unsafe { DEV.is_some() }
}

pub fn sector_count() -> u64 {
    unsafe { DEV.as_ref().map(|d| d.capacity).unwrap_or(0) }
}

pub fn init() {
    let dev = match pci::find(VIRTIO_VENDOR, VIRTIO_BLK_LEGACY)
        .or_else(|| pci::find(VIRTIO_VENDOR, 0x1000))
    {
        Some(d) => d,
        None => {
            crate::kprintln!("[virtio] no virtio-blk device found");
            return;
        }
    };
    pci::enable_bus_master(&dev);
    let bar0 = dev.bar[0];
    if bar0 & 0x1 == 0 {
        crate::kprintln!("[virtio] device is modern-only (BAR0 not I/O); unsupported");
        return;
    }
    let io = pci::bar_io(bar0);

    unsafe {
        // Reset, acknowledge, driver.
        outb(io + 0x12, 0);
        for _ in 0..100_000 {
            if inb(io + 0x12) == 0 {
                break;
            }
        }
        outb(io + 0x12, 1); // ACKNOWLEDGE
        outb(io + 0x12, 3); // ACKNOWLEDGE | DRIVER
        outl(io + 0x04, 0); // no optional features

        // Select queue 0 and read its maximum size.
        outw(io + 0x0E, 0);
        let queue_size = inw(io + 0x0C);
        if queue_size < 3 {
            crate::kprintln!("[virtio] unusable queue size {}", queue_size);
            return;
        }

        let queue = match pmm::alloc_n(5) {
            Some(p) => p,
            None => {
                crate::kprintln!("[virtio] queue allocation failed");
                return;
            }
        };

        // Legacy split-virtqueue layout:
        //   descriptors | available ring | padding to 4 KiB | used ring | requests
        let desc_bytes = 16 * queue_size as u64;
        let used_offset = (desc_bytes + 4 + 2 * queue_size as u64 + 0xFFF) & !0xFFF;
        let request_offset = (used_offset + 4 + 8 * queue_size as u64 + 0x1FF) & !0x1FF;
        if request_offset + 1536 > 5 * 4096 {
            crate::kprintln!("[virtio] queue size {} needs a larger arena", queue_size);
            return;
        }
        core::ptr::write_bytes(queue as *mut u8, 0, 5 * 4096);

        // Legacy queue address register takes the page frame number.
        outl(io + 0x08, (queue >> 12) as u32);

        outb(io + 0x12, 7); // DRIVER | DRIVER_OK

        let capacity = inl(io + 0x14) as u64 | ((inl(io + 0x18) as u64) << 32);

        DEV = Some(Device {
            io,
            queue,
            request: queue + request_offset,
            capacity,
            queue_size,
            avail_idx: 0,
            used_seen: 0,
        });

        crate::kprintln!(
            "[virtio] blk at pci {:02x}:{:02x}.{} io={:#x} qsize={} sectors={} ({} MiB)",
            dev.bus,
            dev.slot,
            dev.func,
            io,
            queue_size,
            capacity,
            capacity * 512 / (1024 * 1024)
        );
    }
}

unsafe fn desc_base(queue: u64) -> *mut Desc {
    queue as *mut Desc
}

unsafe fn avail_ring(queue: u64, size: u16) -> *mut u16 {
    // flags(2) + idx(2) + ring[size], after the descriptor table.
    (queue as *mut u8).add(16 * size as usize + 4) as *mut u16
}

unsafe fn avail_idx_ptr(queue: u64, size: u16) -> *mut u16 {
    (queue as *mut u8).add(16 * size as usize + 2) as *mut u16
}

unsafe fn used_base(queue: u64, size: u16) -> *mut u8 {
    let offset = (16 * size as u64 + 4 + 2 * size as u64 + 0xFFF) & !0xFFF;
    (queue as *mut u8).add(offset as usize)
}

fn submit(dev: &mut Device, write: bool, lba: u64, data: &mut [u8]) -> bool {
    unsafe {
        let q = dev.queue;
        let req = dev.request;
        let hdr = req as *mut u8;
        core::ptr::write_bytes(hdr, 0, 16);
        let kind: u32 = if write { REQ_WRITE } else { REQ_READ };
        core::ptr::write_unaligned(hdr as *mut u32, kind);
        core::ptr::write_unaligned(hdr.add(8) as *mut u64, lba);

        let data_ptr = (req + 512) as *mut u8;
        if write {
            core::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr, 512);
        } else {
            core::ptr::write_bytes(data_ptr, 0, 512);
        }
        let status_ptr = (req + 1024) as *mut u8;
        *status_ptr = 0xFF;

        let desc = desc_base(q);
        (*desc.add(0)) = Desc {
            addr: req,
            len: 16,
            flags: VIRTQ_DESC_F_NEXT,
            next: 1,
        };
        (*desc.add(1)) = Desc {
            addr: req + 512,
            len: 512,
            flags: VIRTQ_DESC_F_NEXT | if write { 0 } else { VIRTQ_DESC_F_WRITE },
            next: 2,
        };
        (*desc.add(2)) = Desc {
            addr: req + 1024,
            len: 1,
            flags: VIRTQ_DESC_F_WRITE,
            next: 0,
        };

        let avail = avail_ring(q, dev.queue_size);
        let slot = (dev.avail_idx % dev.queue_size) as usize;
        *avail.add(slot) = 0; // head descriptor index
        let idx_ptr = avail_idx_ptr(q, dev.queue_size);
        dev.avail_idx = dev.avail_idx.wrapping_add(1);
        core::ptr::write_volatile(idx_ptr, dev.avail_idx);
        fence(Ordering::SeqCst);

        outw(dev.io + 0x10, 0); // notify queue 0

        let used = used_base(q, dev.queue_size);
        let used_idx_ptr = used.add(2) as *mut u16;
        let start = core::ptr::read_volatile(used_idx_ptr);
        let mut spins = 0u64;
        loop {
            fence(Ordering::SeqCst);
            let now = core::ptr::read_volatile(used_idx_ptr);
            if now != start {
                dev.used_seen = now;
                let used_ring = used.add(4) as *mut UsedElem;
                let elem_index = ((now.wrapping_sub(1)) % dev.queue_size) as usize;
                let elem = core::ptr::read_volatile(used_ring.add(elem_index));
                let _ = elem.len;
                break;
            }
            spins += 1;
            if spins > 50_000_000 {
                crate::kprintln!("[virtio] request timeout");
                return false;
            }
            core::hint::spin_loop();
        }

        if *status_ptr != 0 {
            crate::kprintln!("[virtio] request failed status={}", *status_ptr);
            return false;
        }
        if !write {
            core::ptr::copy_nonoverlapping(data_ptr, data.as_mut_ptr(), 512);
        }
        true
    }
}

pub fn read_sector(lba: u64, buf: &mut [u8]) -> bool {
    if buf.len() < 512 {
        return false;
    }
    unsafe {
        match DEV.as_mut() {
            Some(dev) if lba < dev.capacity => submit(dev, false, lba, &mut buf[..512]),
            _ => false,
        }
    }
}

pub fn write_sector(lba: u64, buf: &[u8]) -> bool {
    if buf.len() < 512 {
        return false;
    }
    let mut copy = [0u8; 512];
    copy.copy_from_slice(&buf[..512]);
    unsafe {
        match DEV.as_mut() {
            Some(dev) if lba < dev.capacity => submit(dev, true, lba, &mut copy),
            _ => false,
        }
    }
}

pub fn read_sectors(lba: u64, count: u32, buf: &mut [u8]) -> bool {
    if buf.len() < count as usize * 512 {
        return false;
    }
    for index in 0..count as usize {
        if !read_sector(lba + index as u64, &mut buf[index * 512..]) {
            return false;
        }
    }
    true
}

pub fn write_sectors(lba: u64, count: u32, buf: &[u8]) -> bool {
    if buf.len() < count as usize * 512 {
        return false;
    }
    for index in 0..count as usize {
        if !write_sector(lba + index as u64, &buf[index * 512..]) {
            return false;
        }
    }
    true
}

pub fn print_info() {
    if present() {
        crate::kprintln!(
            "[virtio] {} sectors, {} MiB",
            sector_count(),
            sector_count() * 512 / (1024 * 1024)
        );
    } else {
        crate::kprintln!("[virtio] not initialized");
    }
}
