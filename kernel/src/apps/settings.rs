//! Settings app — system information and theme selection.

use crate::gfx::{theme, window};
use alloc::format;

const TEXT: u32 = 0xCBD5E1;
const MUTED: u32 = 0x4A5568;
const LINE: u32 = 0x1E2D3D;

pub struct Settings {
    pub win: Option<u32>,
    dirty: bool,
    last_refresh: u64,
    theme_y: i32,
}

static mut SETTINGS: Option<Settings> = None;

pub fn init() {
    unsafe {
        SETTINGS = Some(Settings {
            win: None,
            dirty: true,
            last_refresh: 0,
            theme_y: 194,
        });
    }
}

fn settings() -> &'static mut Settings {
    unsafe { SETTINGS.as_mut().expect("settings app not initialized") }
}

impl Settings {
    pub fn render(&mut self) {
        let Some(id) = self.win else { return };
        if !window::exists(id) {
            self.win = None;
            return;
        }
        let th = *theme::current();
        let bg = th.content_bg;
        let accent = th.topbar_accent;
        window::clear(id, bg);

        window::fill_rect(id, 0, 0, window::get(id).map(|w| w.w).unwrap_or(440), 28, th.titlebar_unfocused);
        window::draw_text(id, 10, 8, "Settings", accent, th.titlebar_unfocused);

        let mut y = 36;
        window::draw_text(id, 10, y, "SYSTEM", accent, bg);
        y += 18;
        window::fill_rect(id, 10, y, window::get(id).map(|w| w.w).unwrap_or(440) - 20, 1, LINE);
        y += 8;

        window::draw_text(id, 10, y, "AtrodraX v0.1  |  x86-64  |  Rust", TEXT, bg);
        y += 16;
        let tasks = crate::task::count();
        window::draw_text(id, 10, y, &format!("Tasks: {}", tasks), TEXT, bg);
        y += 16;
        let total_mb = crate::boot::total_memory_bytes() / (1024 * 1024);
        window::draw_text(id, 10, y, &format!("RAM: {} MB usable", total_mb), TEXT, bg);
        y += 16;
        window::draw_text(
            id,
            10,
            y,
            &format!("Free pages: {}", crate::mem::pmm::free_pages()),
            TEXT,
            bg,
        );
        y += 16;
        window::draw_text(
            id,
            10,
            y,
            &format!(
                "Framebuffer: {}x{}x{}",
                crate::drivers::fb::width(),
                crate::drivers::fb::height(),
                crate::drivers::fb::bpp()
            ),
            TEXT,
            bg,
        );
        y += 24;

        window::draw_text(id, 10, y, "THEME", accent, bg);
        y += 18;
        window::fill_rect(id, 10, y, window::get(id).map(|w| w.w).unwrap_or(440) - 20, 1, LINE);
        y += 8;

        window::draw_text(id, 10, y, &format!("Active: {}", th.name), TEXT, bg);
        y += 18;

        let dark_active = theme::index() == 0;
        let hacker_active = theme::index() == 1;
        self.theme_y = y;
        let dark_bg = if dark_active { accent } else { 0x0F1B2C };
        let dark_txt = if dark_active { 0x000000 } else { TEXT };
        let hack_bg = if hacker_active { 0x00FF41 } else { 0x001100 };
        let hack_txt = if hacker_active { 0x000000 } else { 0x00CC33 };

        window::fill_rect(id, 10, y, 120, 22, dark_bg);
        window::rect_outline(id, 10, y, 120, 22, LINE);
        window::draw_text(id, 18, y + 7, "1  Dark", dark_txt, dark_bg);
        window::fill_rect(id, 140, y, 120, 22, hack_bg);
        window::rect_outline(id, 140, y, 120, 22, 0x003300);
        window::draw_text(id, 148, y + 7, "2  Hacker", hack_txt, hack_bg);

        y += 30;
        window::draw_text(id, 10, y, "Press 1 or 2 to switch theme", MUTED, bg);
        y += 18;
        window::draw_text(id, 10, y, &format!("Uptime: {} ms", crate::sync::uptime_ms()), MUTED, bg);

        window::present(id);
        self.dirty = false;
        self.last_refresh = crate::sync::uptime_ms();
    }

    pub fn handle_key(&mut self, key: u16) {
        match key {
            0x31 => theme::set_index(0),
            0x32 => theme::set_index(1),
            _ => return,
        }
        crate::fs::persist::save();
        self.dirty = true;
    }

    pub fn handle_click(&mut self, local_x: i32, local_y: i32) {
        if local_y >= self.theme_y && local_y < self.theme_y + 22 {
            if (10..130).contains(&local_x) {
                theme::set_index(0);
                crate::fs::persist::save();
                self.dirty = true;
            } else if (140..260).contains(&local_x) {
                theme::set_index(1);
                crate::fs::persist::save();
                self.dirty = true;
            }
        }
    }
}

pub fn open() -> u32 {
    let s = settings();
    if let Some(id) = s.win {
        if window::exists(id) {
            window::focus(id);
            s.render();
            return id;
        }
        s.win = None;
    }
    let id = window::create(240, 120, 440, 320, "Settings", window::KIND_SETTINGS, theme::current().content_bg);
    s.win = Some(id);
    s.render();
    id
}

pub fn window_id() -> Option<u32> {
    let s = settings();
    match s.win {
        Some(id) if window::exists(id) => Some(id),
        _ => None,
    }
}

pub fn close_window(id: u32) {
    let s = settings();
    if s.win == Some(id) {
        s.win = None;
    }
}

pub fn handle_key_pub(key: u16) {
    settings().handle_key(key);
}

pub fn handle_click_pub(x: i32, y: i32) {
    settings().handle_click(x, y);
}

pub fn tick() {
    let s = settings();
    if s.win.is_none() {
        return;
    }
    let now = crate::sync::uptime_ms();
    if now.saturating_sub(s.last_refresh) >= 1000 {
        s.render();
    }
}

pub fn mark_dirty() {
    settings().dirty = true;
}
