//! Window objects and the global window list.

use crate::gfx::{font, theme};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

pub const WINDOW_TITLE_MAX: usize = 48;
pub const MAX_WINDOWS: usize = 16;

pub const KIND_TERMINAL: u8 = 0;
pub const KIND_FILES: u8 = 1;
pub const KIND_SETTINGS: u8 = 2;
pub const KIND_EDITOR: u8 = 3;
pub const KIND_ABOUT: u8 = 4;
pub const KIND_NETMON: u8 = 5;
pub const KIND_IMAGE: u8 = 6;
pub const KIND_DEMO: u8 = 7;
pub const KIND_BROWSER: u8 = 8;
pub const KIND_PS: u8 = 9;

pub struct Window {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub fb: Vec<u32>,
    pub title: String,
    pub kind: u8,
    pub visible: bool,
    pub minimized: bool,
    pub focused: bool,
    pub z: i32,
    pub dirty: bool,
    pub close_hover: bool,
    pub min_hover: bool,
    pub close_requested: bool,
    pub content_bg: u32,
    pub owner_pid: i32,
    pub created_ms: u64,
    pub anim: i32,
    pub pre_min_x: i32,
    pub pre_min_y: i32,
    pub invalid: Vec<(i32, i32, i32, i32)>,
}

static mut WINDOWS: Option<Vec<Window>> = None;
static mut NEXT_ID: u32 = 1;
static mut Z_TOP: i32 = 1;

pub fn init() {
    unsafe {
        WINDOWS = Some(Vec::with_capacity(MAX_WINDOWS));
        NEXT_ID = 1;
        Z_TOP = 1;
    }
}

pub fn all() -> &'static mut Vec<Window> {
    unsafe { WINDOWS.as_mut().expect("window manager not initialized") }
}

pub fn count() -> usize {
    all().len()
}

pub fn create(x: i32, y: i32, w: i32, h: i32, title: &str, kind: u8, content_bg: u32) -> u32 {
    let id = unsafe {
        let id = NEXT_ID;
        NEXT_ID += 1;
        id
    };
    let w = w.max(160);
    let h = h.max(100);
    let window = Window {
        id,
        x,
        y,
        w,
        h,
        fb: vec![content_bg & 0xFFFFFF; (w * h) as usize],
        title: title.to_string(),
        kind,
        visible: true,
        minimized: false,
        focused: false,
        z: 1,
        dirty: true,
        close_hover: false,
        min_hover: false,
        close_requested: false,
        content_bg,
        owner_pid: 0,
        created_ms: crate::sync::uptime_ms(),
        anim: 0,
        pre_min_x: x,
        pre_min_y: y,
        invalid: Vec::new(),
    };
    all().push(window);
    focus(id);
    id
}

pub fn get(id: u32) -> Option<&'static mut Window> {
    all().iter_mut().find(|w| w.id == id)
}

pub fn exists(id: u32) -> bool {
    all().iter().any(|w| w.id == id)
}

pub fn destroy(id: u32) {
    all().retain(|w| w.id != id);
}

pub fn focus(id: u32) {
    unsafe { Z_TOP += 1 };
    let z = unsafe { Z_TOP };
    for window in all().iter_mut() {
        window.focused = window.id == id;
        if window.id == id {
            window.z = z;
            window.minimized = false;
            window.visible = true;
        }
    }
}

pub fn focused() -> Option<&'static mut Window> {
    all().iter_mut().find(|w| w.focused && !w.minimized)
}

pub fn focused_id() -> Option<u32> {
    all()
        .iter()
        .find(|w| w.focused && !w.minimized)
        .map(|w| w.id)
}

pub fn at_point(x: i32, y: i32) -> Option<u32> {
    let mut best: Option<(i32, u32)> = None;
    for w in all().iter() {
        if !w.visible || w.minimized {
            continue;
        }
        if x >= w.x && x < w.x + w.w && y >= w.y && y < w.y + w.h {
            match best {
                Some((z, _)) if z >= w.z => {}
                _ => best = Some((w.z, w.id)),
            }
        }
    }
    best.map(|(_, id)| id)
}

pub fn sorted() -> Vec<u32> {
    let mut ids: Vec<(i32, u32)> = all()
        .iter()
        .filter(|w| w.visible && !w.minimized)
        .map(|w| (w.z, w.id))
        .collect();
    ids.sort_by_key(|(z, _)| *z);
    ids.into_iter().map(|(_, id)| id).collect()
}

pub fn move_to(id: u32, x: i32, y: i32) {
    if let Some(w) = get(id) {
        w.x = x;
        w.y = y;
        w.dirty = true;
    }
}

pub fn resize(id: u32, width: i32, height: i32) {
    let (width, height) = (width.max(160), height.max(100));
    if let Some(w) = get(id) {
        if w.w == width && w.h == height {
            return;
        }
        w.w = width;
        w.h = height;
        w.fb = vec![w.content_bg & 0xFFFFFF; (width * height) as usize];
        w.dirty = true;
    }
}

pub fn minimize(id: u32) {
    if let Some(w) = get(id) {
        w.pre_min_x = w.x;
        w.pre_min_y = w.y;
        w.minimized = true;
        w.focused = false;
    }
}

pub fn restore(id: u32) {
    if let Some(w) = get(id) {
        w.minimized = false;
        w.x = w.pre_min_x;
        w.y = w.pre_min_y;
    }
    focus(id);
}

pub fn mark_dirty(id: u32) {
    if let Some(w) = get(id) {
        w.dirty = true;
    }
}

pub fn clear(id: u32, color: u32) {
    if let Some(w) = get(id) {
        for pixel in w.fb.iter_mut() {
            *pixel = color & 0xFFFFFF;
        }
        w.dirty = true;
    }
}

#[inline]
pub fn putpixel(w: &mut Window, x: i32, y: i32, color: u32) {
    if x < 0 || y < 0 || x >= w.w || y >= w.h {
        return;
    }
    let index = y as usize * w.w as usize + x as usize;
    if index < w.fb.len() {
        w.fb[index] = color & 0xFFFFFF;
    }
}

pub fn draw_pixel(id: u32, x: i32, y: i32, color: u32) {
    if let Some(w) = get(id) {
        putpixel(w, x, y, color);
    }
}

pub fn fill_rect(id: u32, x: i32, y: i32, width: i32, height: i32, color: u32) {
    if let Some(w) = get(id) {
        let color = color & 0xFFFFFF;
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + width).min(w.w);
        let y1 = (y + height).min(w.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let stride = w.w as usize;
        for row in y0..y1 {
            let start = row as usize * stride + x0 as usize;
            let end = row as usize * stride + x1 as usize;
            for pixel in &mut w.fb[start..end] {
                *pixel = color;
            }
        }
    }
}

pub fn blend_rect(id: u32, x: i32, y: i32, width: i32, height: i32, color: u32, alpha: u32) {
    if let Some(w) = get(id) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + width).min(w.w);
        let y1 = (y + height).min(w.h);
        let stride = w.w as usize;
        for row in y0..y1 {
            for col in x0..x1 {
                let index = row as usize * stride + col as usize;
                w.fb[index] = super::super::drivers::blend(w.fb[index], color, alpha);
            }
        }
    }
}

pub fn rect_outline(id: u32, x: i32, y: i32, width: i32, height: i32, color: u32) {
    fill_rect(id, x, y, width, 1, color);
    fill_rect(id, x, y + height - 1, width, 1, color);
    fill_rect(id, x, y, 1, height, color);
    fill_rect(id, x + width - 1, y, 1, height, color);
}

pub fn draw_text(id: u32, x: i32, y: i32, text: &str, fg: u32, bg: u32) {
    if let Some(w) = get(id) {
        let stride = w.w as usize;
        font::draw_text_clipped(&mut w.fb, stride, w.w as usize, w.h as usize, x, y, text, fg, bg, false);
    }
}

pub fn draw_text_transparent(id: u32, x: i32, y: i32, text: &str, fg: u32) {
    if let Some(w) = get(id) {
        let stride = w.w as usize;
        font::draw_text_clipped(&mut w.fb, stride, w.w as usize, w.h as usize, x, y, text, fg, 0, true);
    }
}

pub fn present(id: u32) {
    mark_dirty(id);
}

pub fn tile_texture(id: u32) {
    // Small helper used by the image-viewer demo: draw a gradient.
    if let Some(w) = get(id) {
        let t = theme::current();
        for y in 0..w.h {
            let color = super::super::drivers::blend(
                t.wallpaper_top,
                t.wallpaper_bottom,
                ((y as u32) * 255 / w.h.max(1) as u32).min(255),
            );
            for x in 0..w.w {
                let index = y as usize * w.w as usize + x as usize;
                w.fb[index] = color;
            }
        }
    }
}

pub fn title(id: u32) -> String {
    get(id).map(|w| w.title.clone()).unwrap_or_default()
}

pub fn set_title(id: u32, text: &str) {
    if let Some(w) = get(id) {
        w.title = text.to_string();
        if w.title.len() > WINDOW_TITLE_MAX {
            w.title.truncate(WINDOW_TITLE_MAX);
        }
    }
}

pub fn invalidate(id: u32, _x: i32, _y: i32, _w: i32, _h: i32) {
    mark_dirty(id);
}

pub fn invalidate_all(id: u32) {
    mark_dirty(id);
}

pub fn clear_invalid(id: u32) {
    if let Some(w) = get(id) {
        w.invalid.clear();
        w.dirty = false;
    }
}
