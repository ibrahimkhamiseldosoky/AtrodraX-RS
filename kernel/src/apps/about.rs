//! About window.

use crate::gfx::{window};
use alloc::format;

const BG: u32 = 0x0B101A;
const TEXT: u32 = 0xCBD5E1;
const ACCENT: u32 = 0x8BE9FD;
const MUTED: u32 = 0x64748B;

pub struct About {
    pub win: Option<u32>,
    last_refresh: u64,
}

static mut ABOUT: Option<About> = None;

pub fn init() {
    unsafe { ABOUT = Some(About { win: None, last_refresh: 0 }) };
}

fn about() -> &'static mut About {
    unsafe { ABOUT.as_mut().expect("about app not initialized") }
}

fn cpu_strings() -> (alloc::string::String, alloc::string::String) {
    let vendor = crate::arch::cpu::vendor();
    let brand = crate::arch::cpu::brand();
    let vendor_end = vendor.iter().position(|b| *b == 0).unwrap_or(vendor.len());
    let brand_end = brand.iter().position(|b| *b == 0).unwrap_or(brand.len());
    (
        alloc::string::String::from_utf8_lossy(&vendor[..vendor_end]).trim().into(),
        alloc::string::String::from_utf8_lossy(&brand[..brand_end]).trim().into(),
    )
}

impl About {
    pub fn render(&mut self) {
        let Some(id) = self.win else { return };
        if !window::exists(id) {
            self.win = None;
            return;
        }
        window::clear(id, BG);
        window::draw_text(id, 12, 10, "AtrodraX", ACCENT, BG);
        window::draw_text(id, 12, 28, "version 0.1 (Rust edition)", TEXT, BG);
        window::fill_rect(id, 12, 48, window::get(id).map(|w| w.w).unwrap_or(520) - 24, 1, 0x1E2D3D);

        let (vendor, brand) = cpu_strings();
        let mut y = 58;
        let mut line = |text: alloc::string::String, color: u32| {
            window::draw_text(id, 12, y, &text, color, BG);
            y += 16;
        };
        line(format!("CPU: {} {}", vendor, brand), TEXT);
        line(format!("Cores: 1 (single-CPU kernel)"), TEXT);
        line(format!("Memory: {} MB usable", crate::boot::total_memory_bytes() / (1024 * 1024)), TEXT);
        line(format!("Free pages: {}", crate::mem::pmm::free_pages()), TEXT);
        line(format!("Tasks: {}", crate::task::count()), TEXT);
        line(format!("Windows: {}", window::count()), TEXT);
        line(format!("Framebuffer: {}x{}x{}", crate::drivers::fb::width(), crate::drivers::fb::height(), crate::drivers::fb::bpp()), TEXT);
        line(format!("FPS: {}", crate::gfx::compositor::fps()), TEXT);
        line(format!("Uptime: {} ms", crate::sync::uptime_ms()), TEXT);
        line(format!("Disk: FAT32 mounted = {}", crate::fs::fat32::mounted()), TEXT);
        line(format!("Network: {}", if crate::net::present() { crate::net::ip_string(crate::net::local_ip()) } else { alloc::string::String::from("none") }), TEXT);
        y += 6;
        window::draw_text(id, 12, y, "Written entirely in Rust: no C, no libc.", MUTED, BG);
        y += 16;
        window::draw_text(id, 12, y, "GDT/IDT/TSS, paging, heap, scheduler, drivers, TCP/IP", MUTED, BG);
        y += 16;
        window::draw_text(id, 12, y, "FAT32, ELF64 ring-3 userland and a compositing desktop.", MUTED, BG);

        window::present(id);
        self.last_refresh = crate::sync::uptime_ms();
    }
}

pub fn open() -> u32 {
    let a = about();
    if let Some(id) = a.win {
        if window::exists(id) {
            window::focus(id);
            a.render();
            return id;
        }
        a.win = None;
    }
    let id = window::create(220, 120, 560, 380, "About", window::KIND_ABOUT, BG);
    a.win = Some(id);
    a.render();
    id
}

pub fn window_id() -> Option<u32> {
    let a = about();
    match a.win {
        Some(id) if window::exists(id) => Some(id),
        _ => None,
    }
}

pub fn close_window(id: u32) {
    let a = about();
    if a.win == Some(id) {
        a.win = None;
    }
}

pub fn tick() {
    let a = about();
    if a.win.is_none() {
        return;
    }
    let now = crate::sync::uptime_ms();
    if now.saturating_sub(a.last_refresh) >= 1000 {
        a.render();
    }
}
