//! On-disk OS state, byte-compatible with the C kernel's `fs_persistence.c`.
//!
//! File layout (little endian):
//!   header  { magic 'OSST', version 1, section_count }
//!   section { id, size, payload }
//!
//! Sections: 1 = VFS snapshot (unused, written empty), 2 = settings,
//! 3 = desktop layout, 4 = package registry.
//!
//! The C kernel writes `/disk/os_state.bin` when FAT32 is mounted and falls
//! back to `/os_state.bin` in RAMFS; loading restores the theme and the
//! terminal window geometry. The VFS snapshot section was never applied by
//! the C loader either, so it is emitted with size 0.

use alloc::vec::Vec;
use crate::fs::vfs;
use crate::gfx::{theme, window};

pub const MAGIC: u32 = 0x4F53_5354; // "OSST"
pub const VERSION: u32 = 1;
const PATH_DISK: &str = "/disk/os_state.bin";
const PATH_RAM: &str = "/os_state.bin";

const SECTION_VFS: u32 = 1;
const SECTION_SETTINGS: u32 = 2;
const SECTION_DESKTOP: u32 = 3;
const SECTION_PACKAGES: u32 = 4;

#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
    pub valid: bool,
}

static mut PENDING_TERM: Layout = Layout {
    x: 0,
    y: 0,
    w: 0,
    h: 0,
    visible: false,
    valid: false,
};

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(buf: &mut Vec<u8>, value: i32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_i32(data: &[u8], offset: usize) -> Option<i32> {
    read_u32(data, offset).map(|v| v as i32)
}

fn disk_ready() -> bool {
    vfs::resolve("/disk", "/").is_some()
}

fn state_path() -> &'static str {
    if disk_ready() {
        PATH_DISK
    } else {
        PATH_RAM
    }
}

fn terminal_layout() -> Layout {
    let Some(id) = crate::apps::window_id(window::KIND_TERMINAL) else {
        return Layout::default();
    };
    let Some(win) = window::get(id) else {
        return Layout::default();
    };
    Layout {
        x: win.x,
        y: win.y,
        w: win.w,
        h: win.h,
        visible: !win.minimized,
        valid: true,
    }
}

fn files_layout() -> Layout {
    let Some(id) = crate::apps::window_id(window::KIND_FILES) else {
        return Layout::default();
    };
    let Some(win) = window::get(id) else {
        return Layout::default();
    };
    Layout {
        x: win.x,
        y: win.y,
        w: win.w,
        h: win.h,
        visible: !win.minimized,
        valid: true,
    }
}

/// Serialize the current theme and desktop layout to the state file.
pub fn save() {
    let term = terminal_layout();
    let files = files_layout();

    let mut buf: Vec<u8> = Vec::with_capacity(64);
    push_u32(&mut buf, MAGIC);
    push_u32(&mut buf, VERSION);
    push_u32(&mut buf, 4); // section_count

    // 1. VFS snapshot: intentionally empty (the C loader ignores it).
    push_u32(&mut buf, SECTION_VFS);
    push_u32(&mut buf, 0);

    // 2. Settings: theme index + window opacity (C uses 235).
    push_u32(&mut buf, SECTION_SETTINGS);
    push_u32(&mut buf, 8);
    push_u32(&mut buf, theme::index() as u32);
    push_u32(&mut buf, 235);

    // 3. Desktop layout (packed 25 bytes, same as fs_desktop_t).
    push_u32(&mut buf, SECTION_DESKTOP);
    push_u32(&mut buf, 25);
    push_i32(&mut buf, term.x);
    push_i32(&mut buf, term.y);
    push_i32(&mut buf, term.w);
    push_i32(&mut buf, term.h);
    push_i32(&mut buf, files.x);
    push_i32(&mut buf, files.y);
    buf.push(term.visible as u8);

    // 4. Package registry: empty.
    push_u32(&mut buf, SECTION_PACKAGES);
    push_u32(&mut buf, 4);
    push_u32(&mut buf, 0);

    let path = state_path();
    let Some(node) = vfs::create_file_path(path, "/") else {
        crate::kprintln!("[state] cannot create {}", path);
        return;
    };
    vfs::truncate(node, 0);
    vfs::write(node, 0, &buf);
    crate::kprintln!("[state] saved {} bytes to {}", buf.len(), path);
}

/// Read the state file and apply settings/layout. Safe to call before the
/// desktop windows exist; the terminal layout is kept pending for `open()`.
pub fn load() {
    let candidates = [PATH_DISK, PATH_RAM];
    for path in candidates {
        let Some(index) = vfs::resolve(path, "/") else {
            continue;
        };
        let data = vfs::read_all(index);
        if data.len() < 12 {
            continue;
        }
        if read_u32(&data, 0) != Some(MAGIC) || read_u32(&data, 4) != Some(VERSION) {
            crate::kprintln!("[state] {} has bad magic/version", path);
            continue;
        }
        let count = read_u32(&data, 8).unwrap_or(0).min(64);
        let mut offset = 12usize;
        let mut applied = false;
        for _ in 0..count {
            let (Some(id), Some(size)) = (read_u32(&data, offset), read_u32(&data, offset + 4)) else {
                break;
            };
            offset += 8;
            let size = size as usize;
            if offset + size > data.len() {
                break;
            }
            match id {
                SECTION_SETTINGS if size >= 8 => {
                    if let Some(index) = read_u32(&data, offset) {
                        if (index as usize) < 2 {
                            theme::set_index(index as usize);
                        }
                    }
                    applied = true;
                }
                SECTION_DESKTOP if size >= 25 => {
                    let term = Layout {
                        x: read_i32(&data, offset).unwrap_or(0),
                        y: read_i32(&data, offset + 4).unwrap_or(0),
                        w: read_i32(&data, offset + 8).unwrap_or(0),
                        h: read_i32(&data, offset + 12).unwrap_or(0),
                        visible: data[offset + 24] != 0,
                        valid: true,
                    };
                    if term.w >= 160 && term.h >= 100 {
                        unsafe {
                            PENDING_TERM = term;
                        }
                    }
                }
                _ => {}
            }
            offset += size;
        }
        if applied {
            crate::kprintln!("[state] loaded {} (theme={})", path, theme::index());
        }
        return;
    }
}

/// Terminal geometry restored from the last session, if any.
pub fn terminal_layout_hint() -> Option<Layout> {
    unsafe {
        if PENDING_TERM.valid {
            Some(PENDING_TERM)
        } else {
            None
        }
    }
}

/// Called once the terminal consumed the pending geometry.
pub fn clear_terminal_hint() {
    unsafe {
        PENDING_TERM.valid = false;
    }
}
