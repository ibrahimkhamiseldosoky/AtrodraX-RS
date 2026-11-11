//! PCI configuration-space enumeration.

use crate::arch::port::{inl, outl};

#[derive(Clone, Copy, Default)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub header_type: u8,
    pub bar: [u32; 6],
    pub irq: u8,
}

pub const MAX_DEVICES: usize = 48;
static mut DEVICES: [PciDevice; MAX_DEVICES] = [PciDevice {
    bus: 0,
    slot: 0,
    func: 0,
    vendor: 0,
    device: 0,
    class: 0,
    subclass: 0,
    prog_if: 0,
    header_type: 0,
    bar: [0; 6],
    irq: 0,
}; MAX_DEVICES];
static mut COUNT: usize = 0;

fn config_addr(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
}

pub fn read32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    unsafe {
        outl(0xCF8, config_addr(bus, slot, func, offset));
        inl(0xCFC)
    }
}

pub fn write32(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    unsafe {
        outl(0xCF8, config_addr(bus, slot, func, offset));
        outl(0xCFC, value);
    }
}

pub fn init() {
    unsafe {
        COUNT = 0;
        for bus in 0..=255u16 {
            for slot in 0..32u8 {
                let vendor = (read32(bus as u8, slot, 0, 0) & 0xFFFF) as u16;
                if vendor == 0xFFFF {
                    continue;
                }
                let header_type = ((read32(bus as u8, slot, 0, 0x0C) >> 16) & 0xFF) as u8;
                let functions = if header_type & 0x80 != 0 { 8 } else { 1 };
                for func in 0..functions as u8 {
                    let id = read32(bus as u8, slot, func, 0);
                    let vendor = (id & 0xFFFF) as u16;
                    if vendor == 0xFFFF {
                        continue;
                    }
                    if COUNT >= MAX_DEVICES {
                        break;
                    }
                    let class_reg = read32(bus as u8, slot, func, 0x08);
                    let mut dev = PciDevice {
                        bus: bus as u8,
                        slot,
                        func,
                        vendor,
                        device: (id >> 16) as u16,
                        class: ((class_reg >> 24) & 0xFF) as u8,
                        subclass: ((class_reg >> 16) & 0xFF) as u8,
                        prog_if: ((class_reg >> 8) & 0xFF) as u8,
                        header_type,
                        bar: [0; 6],
                        irq: (read32(bus as u8, slot, func, 0x3C) & 0xFF) as u8,
                    };
                    for (index, bar) in dev.bar.iter_mut().enumerate() {
                        *bar = read32(bus as u8, slot, func, 0x10 + (index as u8) * 4);
                    }
                    DEVICES[COUNT] = dev;
                    COUNT += 1;
                }
            }
        }
    }
    crate::kprintln!("[pci] {} devices enumerated", count());
}

pub fn count() -> usize {
    unsafe { COUNT }
}

pub fn devices() -> &'static [PciDevice] {
    unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(DEVICES) as *const PciDevice, COUNT) }
}

pub fn find(vendor: u16, device: u16) -> Option<PciDevice> {
    devices()
        .iter()
        .copied()
        .find(|d| d.vendor == vendor && d.device == device)
}

pub fn find_class(class: u8, subclass: u8) -> Option<PciDevice> {
    devices()
        .iter()
        .copied()
        .find(|d| d.class == class && d.subclass == subclass)
}

pub fn bar_io(bar: u32) -> u16 {
    (bar & !0x3) as u16
}

pub fn bar_mem(bar: u32) -> u64 {
    (bar & !0xF) as u64
}

pub fn enable_bus_master(dev: &PciDevice) {
    let command = read32(dev.bus, dev.slot, dev.func, 0x04);
    write32(dev.bus, dev.slot, dev.func, 0x04, command | 0x7);
}

pub fn print_info() {
    for d in devices().iter().take(12) {
        crate::kprintln!(
            "[pci] {:02x}:{:02x}.{} {:04x}:{:04x} class {:02x}:{:02x}",
            d.bus,
            d.slot,
            d.func,
            d.vendor,
            d.device,
            d.class,
            d.subclass
        );
    }
}
