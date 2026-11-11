//! Intel 82540EM (e1000) gigabit Ethernet driver with polled RX/TX rings.

use crate::drivers::pci;
use crate::mem::pmm;
use core::ptr::{read_volatile, write_volatile};

const REG_CTRL: u32 = 0x0000;
const REG_STATUS: u32 = 0x0008;
const REG_EERD: u32 = 0x0014;
const REG_ICR: u32 = 0x00C0;
const REG_IMC: u32 = 0x00D8;
const REG_RCTL: u32 = 0x0100;
const REG_TCTL: u32 = 0x0400;
const REG_TIPG: u32 = 0x0410;
const REG_RDBAL: u32 = 0x2800;
const REG_RDBAH: u32 = 0x2804;
const REG_RDLEN: u32 = 0x2808;
const REG_RDH: u32 = 0x2810;
const REG_RDT: u32 = 0x2818;
const REG_TDBAL: u32 = 0x3800;
const REG_TDBAH: u32 = 0x3804;
const REG_TDLEN: u32 = 0x3808;
const REG_TDH: u32 = 0x3810;
const REG_TDT: u32 = 0x3818;
const REG_RAL0: u32 = 0x5400;
const REG_RAH0: u32 = 0x5404;

const CTRL_SLU: u32 = 1 << 6;
const CTRL_RST: u32 = 1 << 26;

const RCTL_EN: u32 = 1 << 1;
const RCTL_SBP: u32 = 1 << 2;
const RCTL_UPE: u32 = 1 << 3;
const RCTL_MPE: u32 = 1 << 4;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_SECRC: u32 = 1 << 26;

const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3;
const TCTL_CT: u32 = 0x10 << 4;
const TCTL_COLD: u32 = 0x40 << 12;

const RX_COUNT: usize = 32;
const TX_COUNT: usize = 8;
const BUF_SIZE: usize = 2048;

#[repr(C)]
#[derive(Clone, Copy)]
struct RxDesc {
    addr: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TxDesc {
    addr: u64,
    length: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

struct Nic {
    mmio: u64,
    mac: [u8; 6],
    rx_desc: u64,
    tx_desc: u64,
    rx_bufs: u64,
    tx_bufs: u64,
    rx_cur: usize,
    tx_cur: usize,
    rx_packets: u64,
    tx_packets: u64,
}

static mut NIC: Option<Nic> = None;

#[inline]
unsafe fn read32(nic: &Nic, offset: u32) -> u32 {
    read_volatile((nic.mmio + offset as u64) as *const u32)
}

#[inline]
unsafe fn write32(nic: &Nic, offset: u32, value: u32) {
    write_volatile((nic.mmio + offset as u64) as *mut u32, value)
}

pub fn present() -> bool {
    unsafe { NIC.is_some() }
}

pub fn mac() -> [u8; 6] {
    unsafe { NIC.as_ref().map(|n| n.mac).unwrap_or([0; 6]) }
}

pub fn stats() -> (u64, u64) {
    unsafe {
        NIC.as_ref()
            .map(|n| (n.rx_packets, n.tx_packets))
            .unwrap_or((0, 0))
    }
}

fn read_eeprom_word(nic: &Nic, address: u8) -> Option<u16> {
    unsafe {
        write32(nic, REG_EERD, 1 | ((address as u32) << 8));
        for _ in 0..100_000 {
            let value = read32(nic, REG_EERD);
            if value & 0x10 != 0 {
                return Some((value >> 16) as u16);
            }
            core::hint::spin_loop();
        }
    }
    None
}

pub fn init() {
    let dev = match pci::find(0x8086, 0x100E)
        .or_else(|| pci::find(0x8086, 0x100F))
        .or_else(|| pci::find(0x8086, 0x10D3))
        .or_else(|| pci::find_class(0x02, 0x00))
    {
        Some(d) => d,
        None => {
            crate::kprintln!("[e1000] no Intel NIC found");
            return;
        }
    };
    pci::enable_bus_master(&dev);
    let mmio = pci::bar_mem(dev.bar[0]).max(pci::bar_mem(dev.bar[1]));
    if mmio == 0 {
        crate::kprintln!("[e1000] NIC has no MMIO BAR");
        return;
    }

    let rx_desc = match pmm::alloc_n(1) {
        Some(p) => p,
        None => return,
    };
    let tx_desc = match pmm::alloc_n(1) {
        Some(p) => p,
        None => return,
    };
    let rx_bufs = match pmm::alloc_n((RX_COUNT * BUF_SIZE).div_ceil(4096)) {
        Some(p) => p,
        None => return,
    };
    let tx_bufs = match pmm::alloc_n((TX_COUNT * BUF_SIZE).div_ceil(4096)) {
        Some(p) => p,
        None => return,
    };

    let mut nic = Nic {
        mmio,
        mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
        rx_desc,
        tx_desc,
        rx_bufs,
        tx_bufs,
        rx_cur: 0,
        tx_cur: 0,
        rx_packets: 0,
        tx_packets: 0,
    };

    unsafe {
        // Reset the controller.
        let ctrl = read32(&nic, REG_CTRL);
        write32(&nic, REG_CTRL, ctrl | CTRL_RST);
        for _ in 0..1_000_000 {
            if read32(&nic, REG_CTRL) & CTRL_RST == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        write32(&nic, REG_IMC, 0xFFFF_FFFF);
        let _ = read32(&nic, REG_ICR);

        // MAC from EEPROM, falling back to the QEMU default.
        let mut mac = [0u8; 6];
        for i in 0..3 {
            if let Some(word) = read_eeprom_word(&nic, i as u8) {
                mac[i * 2] = word as u8;
                mac[i * 2 + 1] = (word >> 8) as u8;
            }
        }
        if mac.iter().all(|b| *b != 0 && *b != 0xFF) {
            nic.mac = mac;
        }

        write32(&nic, REG_RAL0, u32::from_le_bytes([nic.mac[0], nic.mac[1], nic.mac[2], nic.mac[3]]));
        write32(
            &nic,
            REG_RAH0,
            u32::from_le_bytes([nic.mac[4], nic.mac[5], 0, 0]) | 0x8000_0000,
        );

        // Clear the multicast table.
        for index in 0..128 {
            write32(&nic, 0x5200 + index * 4, 0);
        }

        let ctrl = read32(&nic, REG_CTRL);
        write32(&nic, REG_CTRL, ctrl | CTRL_SLU);

        // RX ring.
        for index in 0..RX_COUNT {
            let desc = (rx_desc as *mut RxDesc).add(index);
            (*desc).addr = rx_bufs + (index * BUF_SIZE) as u64;
            (*desc).length = 0;
            (*desc).status = 0;
            (*desc).errors = 0;
            (*desc).special = 0;
        }
        write32(&nic, REG_RDBAL, rx_desc as u32);
        write32(&nic, REG_RDBAH, (rx_desc >> 32) as u32);
        write32(&nic, REG_RDLEN, (RX_COUNT * 16) as u32);
        write32(&nic, REG_RDH, 0);
        write32(&nic, REG_RDT, (RX_COUNT - 1) as u32);
        write32(
            &nic,
            REG_RCTL,
            RCTL_EN | RCTL_SBP | RCTL_UPE | RCTL_MPE | RCTL_BAM | RCTL_SECRC,
        );

        // TX ring.
        for index in 0..TX_COUNT {
            let desc = (tx_desc as *mut TxDesc).add(index);
            (*desc).addr = tx_bufs + (index * BUF_SIZE) as u64;
            (*desc).length = 0;
            (*desc).cso = 0;
            (*desc).cmd = 0;
            (*desc).status = 1;
            (*desc).css = 0;
            (*desc).special = 0;
        }
        write32(&nic, REG_TDBAL, tx_desc as u32);
        write32(&nic, REG_TDBAH, (tx_desc >> 32) as u32);
        write32(&nic, REG_TDLEN, (TX_COUNT * 16) as u32);
        write32(&nic, REG_TDH, 0);
        write32(&nic, REG_TDT, 0);
        write32(&nic, REG_TIPG, 10 | (8 << 10) | (6 << 20));
        write32(&nic, REG_TCTL, TCTL_EN | TCTL_PSP | TCTL_CT | TCTL_COLD);

        NIC = Some(nic);
    }

    let mac = mac();
    crate::kprintln!(
        "[e1000] {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} @ {:#x} (pci {:02x}:{:02x}.{})",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5],
        mmio,
        dev.bus,
        dev.slot,
        dev.func
    );
}

pub fn tx(frame: &[u8]) -> bool {
    let nic = match unsafe { NIC.as_mut() } {
        Some(n) => n,
        None => return false,
    };
    if frame.is_empty() || frame.len() > BUF_SIZE {
        return false;
    }
    unsafe {
        let slot = nic.tx_cur;
        let desc = (nic.tx_desc as *mut TxDesc).add(slot);
        let buffer = (nic.tx_bufs + (slot * BUF_SIZE) as u64) as *mut u8;
        core::ptr::copy_nonoverlapping(frame.as_ptr(), buffer, frame.len());
        let length = frame.len().max(60);
        if length > frame.len() {
            core::ptr::write_bytes(buffer.add(frame.len()), 0, length - frame.len());
        }
        (*desc).addr = buffer as u64;
        (*desc).length = length as u16;
        (*desc).cmd = 0x0B; // EOP | IFCS | RS
        (*desc).status = 0;

        write32(nic, REG_TDT, ((slot + 1) % TX_COUNT) as u32);

        for _ in 0..10_000_000 {
            if (*desc).status & 0x01 != 0 {
                nic.tx_cur = (slot + 1) % TX_COUNT;
                nic.tx_packets += 1;
                return true;
            }
            core::hint::spin_loop();
        }
    }
    crate::kprintln!("[e1000] tx timeout");
    false
}

pub fn rx_poll() {
    let nic = match unsafe { NIC.as_mut() } {
        Some(n) => n,
        None => return,
    };
    unsafe {
        for _ in 0..RX_COUNT {
            let slot = nic.rx_cur;
            let desc = (nic.rx_desc as *mut RxDesc).add(slot);
            if (*desc).status & 0x01 == 0 {
                break;
            }
            let length = (*desc).length as usize;
            let errors = (*desc).errors;
            if errors == 0 && length > 14 && length <= BUF_SIZE {
                let frame = core::slice::from_raw_parts((*desc).addr as *const u8, length);
                let mut copy = alloc::vec![0u8; length];
                copy.copy_from_slice(frame);
                crate::net::receive(&copy);
                nic.rx_packets += 1;
            }
            (*desc).status = 0;
            (*desc).length = 0;
            write32(nic, REG_RDT, slot as u32);
            nic.rx_cur = (slot + 1) % RX_COUNT;
        }
    }
}

pub fn print_info() {
    let (rx, tx) = stats();
    crate::kprintln!("[e1000] rx={} tx={}", rx, tx);
}
