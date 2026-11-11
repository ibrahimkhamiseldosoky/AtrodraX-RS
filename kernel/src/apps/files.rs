//! Files browser app.

use crate::fs::vfs;
use crate::gfx::window;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const BG: u32 = 0x0B101A;
const ACCENT: u32 = 0x8BE9FD;
const TEXT: u32 = 0xE2E8F0;
const MUTED: u32 = 0x94A3B8;
const ROW_BG: u32 = 0x123041;
const DIR: u32 = 0x00BCD4;
const FILE: u32 = 0x475569;

const KEY_W: u16 = 0x77;
const KEY_S: u16 = 0x73;
const KEY_K: u16 = 0x6B;
const KEY_J: u16 = 0x6A;

pub struct Files {
    pub win: Option<u32>,
    dir: usize,
    items: Vec<(String, u8, usize)>,
    show_parent: bool,
    selected: i32,
    dirty: bool,
}

static mut FILES: Option<Files> = None;

pub fn init() {
    unsafe {
        FILES = Some(Files {
            win: None,
            dir: vfs::root(),
            items: Vec::new(),
            show_parent: false,
            selected: 0,
            dirty: true,
        });
    }
}

fn files() -> &'static mut Files {
    unsafe { FILES.as_mut().expect("files app not initialized") }
}

fn window_width(id: u32) -> i32 {
    window::get(id).map(|w| w.w).unwrap_or(480)
}

fn window_height(id: u32) -> i32 {
    window::get(id).map(|w| w.h).unwrap_or(420)
}

impl Files {
    fn refresh(&mut self) {
        self.dir = if self.dir < vfs::fs().nodes.len() {
            self.dir
        } else {
            vfs::root()
        };
        self.items = vfs::list(self.dir);
        self.show_parent = self.dir != vfs::root();
        let total = self.items.len() as i32 + if self.show_parent { 1 } else { 0 };
        if self.selected >= total {
            self.selected = 0;
        }
        self.dirty = true;
    }

    fn path(&self) -> String {
        vfs::path_of(self.dir)
    }

    pub fn render(&mut self) {
        let Some(id) = self.win else { return };
        if !window::exists(id) {
            self.win = None;
            return;
        }
        let width = window_width(id);
        window::clear(id, BG);
        window::draw_text(id, 10, 8, "Files", ACCENT, BG);
        let path = self.path();
        window::draw_text(id, 10, 26, &path, 0x7DD3FC, BG);
        window::draw_text(id, 10, 44, "w/s select  enter open  backspace up", MUTED, BG);
        window::fill_rect(id, 8, 62, width - 16, 1, 0x1E2D3D);

        let mut y = 68;
        if self.show_parent {
            if self.selected == 0 {
                window::fill_rect(id, 8, y - 1, width - 16, 17, ROW_BG);
            }
            window::draw_text(id, 14, y, "..", TEXT, if self.selected == 0 { ROW_BG } else { BG });
            y += 18;
        }
        for (index, (name, kind, child)) in self.items.iter().enumerate() {
            let row = index as i32 + if self.show_parent { 1 } else { 0 };
            let selected = row == self.selected;
            if selected {
                window::fill_rect(id, 8, y - 1, width - 16, 17, ROW_BG);
            }
            let is_dir = *kind == vfs::KIND_DIR || *kind == vfs::KIND_FAT_DIR;
            let color = if is_dir { DIR } else { FILE };
            window::fill_rect(id, 12, y + 3, 8, 8, color);
            let label = if is_dir {
                format!("{}/", name)
            } else {
                format!("{}  ({} B)", name, vfs::size_of(*child))
            };
            window::draw_text(id, 26, y, &label, TEXT, if selected { ROW_BG } else { BG });
            y += 18;
            if y > window_height(id) - 20 {
                break;
            }
        }
        window::present(id);
        self.dirty = false;
    }

    fn open_selected(&mut self) {
        if self.show_parent && self.selected == 0 {
            if let Some(parent) = vfs::parent_of(self.dir) {
                self.dir = parent;
                self.selected = 0;
                self.refresh();
            }
            return;
        }
        let index = self.selected - if self.show_parent { 1 } else { 0 };
        if index < 0 || index as usize >= self.items.len() {
            return;
        }
        let (name, kind, child) = self.items[index as usize].clone();
        if kind == vfs::KIND_DIR || kind == vfs::KIND_FAT_DIR {
            self.dir = child;
            self.selected = 0;
            self.refresh();
        } else {
            let base = self.path();
            let path = if base == "/" {
                format!("/{}", name)
            } else {
                format!("{}/{}", base, name)
            };
            crate::apps::editor::open_path(&path);
        }
    }

    pub fn handle_key(&mut self, key: u16) {
        let total = self.items.len() as i32 + if self.show_parent { 1 } else { 0 };
        if total == 0 {
            return;
        }
        match key {
            KEY_W | KEY_K | 0x100 => {
                self.selected -= 1;
                if self.selected < 0 {
                    self.selected = total - 1;
                }
                self.dirty = true;
            }
            KEY_S | KEY_J | 0x101 => {
                self.selected += 1;
                if self.selected >= total {
                    self.selected = 0;
                }
                self.dirty = true;
            }
            0x0A | 0x0D => self.open_selected(),
            0x08 | 0x7F => {
                if let Some(parent) = vfs::parent_of(self.dir) {
                    self.dir = parent;
                    self.selected = 0;
                    self.refresh();
                }
            }
            _ => {}
        }
    }

    pub fn handle_click(&mut self, local_y: i32) {
        let mut row = (local_y - 68) / 18;
        if row < 0 {
            return;
        }
        if self.show_parent {
            if row == 0 {
                self.selected = 0;
                self.dirty = true;
                return;
            }
            row -= 1;
        }
        if row as usize >= self.items.len() {
            return;
        }
        self.selected = row + if self.show_parent { 1 } else { 0 };
        self.dirty = true;
        self.open_selected();
    }
}

pub fn open() -> u32 {
    let f = files();
    if let Some(id) = f.win {
        if window::exists(id) {
            window::focus(id);
            f.render();
            return id;
        }
        f.win = None;
    }
    f.dir = vfs::root();
    f.selected = 0;
    f.refresh();
    let id = window::create(180, 110, 500, 420, "Files", window::KIND_FILES, BG);
    f.win = Some(id);
    f.render();
    id
}

pub fn window_id() -> Option<u32> {
    let f = files();
    match f.win {
        Some(id) if window::exists(id) => Some(id),
        _ => None,
    }
}

pub fn close_window(id: u32) {
    let f = files();
    if f.win == Some(id) {
        f.win = None;
    }
}

pub fn handle_key_pub(key: u16) {
    files().handle_key(key);
}

pub fn handle_click_pub(local_y: i32) {
    files().handle_click(local_y);
}

pub fn tick() {
    let f = files();
    if f.win.is_some() {
        f.refresh();
        if f.dirty {
            f.render();
        }
    }
}

pub fn refresh() {
    files().refresh();
}

pub fn open_path(path: &str) {
    let f = files();
    if let Some(index) = vfs::resolve(path, "/") {
        if vfs::is_directory(index) {
            f.dir = index;
            f.selected = 0;
            f.refresh();
        }
    }
}

pub fn _unused() {
    let _ = MUTED.to_string();
}
