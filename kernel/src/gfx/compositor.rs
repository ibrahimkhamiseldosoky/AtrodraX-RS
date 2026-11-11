//! Desktop compositor: wallpaper, topbar, windows, taskbar, cursor.

use crate::drivers::{blend, fb, mouse};
use crate::gfx::font::{draw_text, CHAR_H, CHAR_W};
use crate::gfx::theme::Theme;
use crate::gfx::{theme, window};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const SCREEN_W: i32 = 1024;
pub const SCREEN_H: i32 = 768;
pub const TOPBAR_H: i32 = 36;
pub const TASKBAR_H: i32 = 36;
pub const CLOSE_BTN: i32 = 14;
pub const ICON_W: i32 = 48;
pub const ICON_H: i32 = 48;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Client,
    Titlebar,
    Close,
    Minimize,
    Border,
}

#[derive(Clone, Copy)]
pub struct DesktopIcon {
    pub label: &'static str,
    pub app: u8,
}

pub const ICONS: [DesktopIcon; 5] = [
    DesktopIcon { label: "Terminal", app: crate::gfx::window::KIND_TERMINAL },
    DesktopIcon { label: "Files", app: crate::gfx::window::KIND_FILES },
    DesktopIcon { label: "Settings", app: crate::gfx::window::KIND_SETTINGS },
    DesktopIcon { label: "Net", app: crate::gfx::window::KIND_NETMON },
    DesktopIcon { label: "About", app: crate::gfx::window::KIND_ABOUT },
];

pub const MENU_ITEMS: [&str; 8] = [
    "Terminal",
    "Files",
    "Settings",
    "Network",
    "Editor",
    "About",
    "Reboot",
    "Shutdown",
];

#[derive(Clone, Copy, Default)]
struct State {
    apps_open: bool,
    apps_hover: i32,
    icon_hover: i32,
    taskbar_hover: i32,
    frames: u32,
    fps: u32,
    last_second: u64,
    caret_phase: bool,
    accent_phase: i32,
}

static mut STATE: State = State {
    apps_open: false,
    apps_hover: -1,
    icon_hover: -1,
    taskbar_hover: -1,
    frames: 0,
    fps: 0,
    last_second: u64::MAX,
    caret_phase: false,
    accent_phase: 0,
};

const CURSOR: [[u8; 12]; 19] = [
    [1,0,0,0,0,0,0,0,0,0,0,0],
    [1,1,0,0,0,0,0,0,0,0,0,0],
    [1,3,1,0,0,0,0,0,0,0,0,0],
    [1,2,3,1,0,0,0,0,0,0,0,0],
    [1,2,2,2,1,0,0,0,0,0,0,0],
    [1,2,2,2,2,1,0,0,0,0,0,0],
    [1,2,2,2,2,2,1,0,0,0,0,0],
    [1,2,2,2,2,2,2,1,0,0,0,0],
    [1,2,2,2,2,2,2,2,1,0,0,0],
    [1,2,2,2,2,2,2,2,2,1,0,0],
    [1,2,2,2,2,2,2,2,2,2,1,0],
    [1,2,2,2,2,2,1,1,1,1,1,1],
    [1,2,2,2,1,2,2,1,0,0,0,0],
    [1,2,2,1,0,1,2,2,1,0,0,0],
    [1,2,1,0,0,1,2,2,1,0,0,0],
    [1,1,0,0,0,0,1,2,2,1,0,0],
    [1,0,0,0,0,0,1,2,2,1,0,0],
    [0,0,0,0,0,0,0,1,2,1,0,0],
    [0,0,0,0,0,0,0,0,1,0,0,0],
];

pub fn init() {
    unsafe {
        STATE = State {
            apps_open: false,
            apps_hover: -1,
            icon_hover: -1,
            taskbar_hover: -1,
            frames: 0,
            fps: 0,
            last_second: u64::MAX,
            caret_phase: false,
            accent_phase: 0,
        };
    }
    crate::kprintln!("[gui] compositor initialized ({}x{})", SCREEN_W, SCREEN_H);
}

pub fn apps_open() -> bool {
    unsafe { STATE.apps_open }
}

pub fn set_apps_open(open: bool) {
    unsafe {
        STATE.apps_open = open;
        STATE.apps_hover = -1;
    }
}

pub fn toggle_apps() {
    let open = apps_open();
    set_apps_open(!open);
}

pub fn caret_visible() -> bool {
    unsafe { STATE.caret_phase }
}

fn blend_rect(x: i32, y: i32, w: i32, h: i32, color: u32, alpha: u32) {
    fb::blend_rect(x, y, w, h, color, alpha);
}

fn text(x: i32, y: i32, s: &str, fg: u32) {
    let buffer = fb::backbuffer();
    draw_text(buffer, fb::width(), x, y, s, fg, 0, true);
}

fn text_on(x: i32, y: i32, s: &str, fg: u32, bg: u32) {
    let buffer = fb::backbuffer();
    draw_text(buffer, fb::width(), x, y, s, fg, bg, false);
}

fn clipped(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.into()
    } else {
        let mut out: String = text.chars().take(max_chars.saturating_sub(3)).collect();
        out.push_str("...");
        out
    }
}

fn wallpaper(t: &Theme) {
    let buffer = fb::backbuffer();
    let width = fb::width();
    let height = fb::height();
    for y in 0..height {
        let blend_amount = (y * 255 / height.max(1)) as u32;
        let color = blend(t.wallpaper_top, t.wallpaper_bottom, blend_amount);
        let row = &mut buffer[y * width..(y + 1) * width];
        for pixel in row.iter_mut() {
            *pixel = color;
        }
    }

    // Faint grid.
    for gy in (TOPBAR_H + 4..height as i32).step_by(24) {
        for x in (0..width as i32).step_by(1) {
            if x % 2 == 0 {
                fb::blendpixel(x, gy, 0x1A3048, 18);
            }
        }
    }
    for gx in (TOPBAR_H + 4..height as i32).step_by(24) {
        for y in (TOPBAR_H..height as i32).step_by(1) {
            if y % 2 == 0 {
                fb::blendpixel(gx, y, 0x1A3048, 18);
            }
        }
    }

    // Animated accent line.
    let span = height as i32 - TOPBAR_H - 96;
    let phase = unsafe { STATE.accent_phase };
    let gy = TOPBAR_H + 48 + (phase % span.max(1));
    for x in 0..width as i32 {
        fb::blendpixel(x, gy, t.topbar_accent, 18);
    }

    // Vignette strips.
    blend_rect(0, TOPBAR_H, width as i32, 56, 0x000208, 22);
    blend_rect(0, TOPBAR_H, 56, height as i32 - TOPBAR_H, 0x000208, 20);
    blend_rect(width as i32 - 56, TOPBAR_H, 56, height as i32 - TOPBAR_H, 0x000208, 20);
}

fn draw_icons(t: &Theme) {
    let icon_hover = unsafe { STATE.icon_hover };
    for (index, icon) in ICONS.iter().enumerate() {
        let ix = 24;
        let iy = 64 + index as i32 * 76;
        let hovered = icon_hover == index as i32;
        let inset = if hovered { 2 } else { 4 };
        let bg = if hovered {
            blend(t.icon_bg, 0x1A3048, 120)
        } else {
            t.icon_bg
        };
        fb::fill_rect(ix + inset, iy + inset, ICON_W - inset * 2, ICON_H - inset * 2, bg);
        fb::rect_outline(
            ix + inset,
            iy + inset,
            ICON_W - inset * 2,
            ICON_H - inset * 2,
            if hovered { t.icon_border_hover } else { t.icon_border },
        );
        draw_icon_glyph(icon.app, ix + ICON_W / 2, iy + ICON_H / 2, t, hovered);
        let label = icon.label;
        let lx = ix + (ICON_W - label.len() as i32 * CHAR_W as i32) / 2;
        text(lx, iy + ICON_H + 3, label, if hovered { t.icon_border_hover } else { t.icon_label_color });
    }
}

fn draw_icon_glyph(kind: u8, cx: i32, cy: i32, t: &Theme, hovered: bool) {
    let color = if hovered { t.icon_border_hover } else { t.topbar_accent };
    match kind {
        window::KIND_TERMINAL => {
            fb::fill_rect(cx - 10, cy - 8, 20, 16, 0x081018);
            fb::rect_outline(cx - 10, cy - 8, 20, 16, color);
            fb::fill_rect(cx - 6, cy - 2, 2, 2, color);
            fb::fill_rect(cx - 3, cy + 1, 2, 2, color);
            fb::fill_rect(cx, cy - 4, 6, 2, 0x00FF41);
        }
        window::KIND_FILES => {
            fb::fill_rect(cx - 10, cy - 7, 20, 15, 0x101820);
            fb::rect_outline(cx - 10, cy - 7, 20, 15, color);
            fb::fill_rect(cx - 8, cy - 5, 7, 3, color);
            fb::fill_rect(cx - 8, cy, 16, 1, 0x335577);
            fb::fill_rect(cx - 8, cy + 3, 12, 1, 0x335577);
        }
        window::KIND_SETTINGS => {
            fb::fill_rect(cx - 8, cy - 8, 16, 16, 0x101820);
            fb::rect_outline(cx - 8, cy - 8, 16, 16, color);
            fb::fill_rect(cx - 2, cy - 2, 4, 4, color);
            fb::fill_rect(cx - 2, cy - 8, 4, 2, color);
            fb::fill_rect(cx - 2, cy + 6, 4, 2, color);
            fb::fill_rect(cx - 8, cy - 2, 2, 4, color);
            fb::fill_rect(cx + 6, cy - 2, 2, 4, color);
        }
        window::KIND_NETMON => {
            fb::fill_rect(cx - 10, cy - 8, 20, 16, 0x081018);
            fb::rect_outline(cx - 10, cy - 8, 20, 16, color);
            fb::fill_rect(cx - 7, cy - 4, 5, 5, color);
            fb::fill_rect(cx + 2, cy - 4, 5, 5, color);
            fb::fill_rect(cx - 7, cy + 4, 5, 5, color);
            fb::fill_rect(cx + 2, cy + 4, 5, 5, color);
        }
        _ => {
            draw_circle(cx, cy, 9, color);
            text(cx - 4, cy - 8, "i", color);
        }
    }
}

fn draw_topbar(t: &Theme) {
    fb::fill_rect(0, 0, SCREEN_W, TOPBAR_H, t.topbar_bg);
    fb::fill_rect(0, TOPBAR_H - 1, SCREEN_W, 1, t.topbar_border);
    text(12, 10, "ATRODRAX", t.topbar_accent);

    let apps_x = 104;
    let apps_y = 8;
    let apps_w = 74;
    let apps_h = 20;
    let hovered = unsafe { STATE.apps_hover == 100 };
    let bg = if hovered {
        blend(t.menu_row_hover, t.topbar_accent, 24)
    } else {
        blend(t.topbar_bg, t.topbar_accent, 14)
    };
    fb::fill_rect(apps_x, apps_y, apps_w, apps_h, bg);
    fb::rect_outline(apps_x, apps_y, apps_w, apps_h, t.topbar_border);
    text(apps_x + 9, apps_y + 3, "APPS", t.topbar_accent);
    fb::fill_rect(apps_x + apps_w - 10, apps_y + 9, 4, 2, t.topbar_accent);

    if let Some(focused) = window::focused_id() {
        let title = clipped(&window::title(focused), 34);
        fb::fill_rect(194, 6, 320, 24, blend(t.topbar_bg, 0x000000, 40));
        fb::rect_outline(194, 6, 320, 24, blend(t.topbar_border, 0x000000, 30));
        text(204, 10, &title, t.title_text_focused);
    }

    let theme_name = t.name;
    let clock = crate::drivers::rtc::clock_string();
    let right = SCREEN_W - 16;
    let clock_x = right - clock.len() as i32 * CHAR_W as i32;
    text(clock_x, 10, &clock, t.topbar_accent);
    let theme_x = clock_x - 90;
    text(theme_x, 10, theme_name, t.topbar_accent);
}

fn draw_taskbar(t: &Theme) {
    let y = SCREEN_H - TASKBAR_H;
    fb::fill_rect(0, y, SCREEN_W, TASKBAR_H, t.topbar_bg);
    fb::fill_rect(0, y, SCREEN_W, 1, t.topbar_border);
    let mut x = 8;
    for w in window::all().iter() {
        let width = 150.min(SCREEN_W - x - 8);
        if width <= 20 {
            break;
        }
        let active = w.focused && !w.minimized;
        let bg = if active {
            blend(t.menu_row_hover, t.topbar_accent, 20)
        } else if w.minimized {
            blend(t.topbar_bg, 0x000000, 45)
        } else {
            blend(t.topbar_bg, 0x1A3048, 50)
        };
        fb::fill_rect(x, y + 5, width, 26, bg);
        fb::rect_outline(x, y + 5, width, 26, if active { t.border_focused } else { t.topbar_border });
        let label = clipped(&w.title, ((width - 12) / CHAR_W as i32) as usize);
        let fg = if active {
            t.topbar_accent
        } else if w.minimized {
            t.title_text_unfocused
        } else {
            t.title_text_unfocused
        };
        text(x + 6, y + 10, &label, fg);
        x += width + 6;
    }
}

fn draw_window(w: &mut window::Window, t: &Theme) {
    let border = t.border_width;
    let title_h = t.titlebar_h;
    let x = w.x;
    let y = w.y;
    let width = w.w;
    let height = w.h;

    // Drop shadow.
    for offset in 0..6 {
        let alpha = 40 - offset * 5;
        blend_rect(x + 4 + offset, y + 6 + offset, width, height, 0x000000, alpha as u32);
    }

    // Border and titlebar.
    fb::fill_rect(x, y, width, height, t.content_bg);
    fb::rect_outline(x, y, width, height, if w.focused { t.border_focused } else { t.border_unfocused });

    let title_color_top = if w.focused { t.titlebar_focused_top } else { t.titlebar_unfocused };
    let title_color_bottom = if w.focused { t.titlebar_focused_bottom } else { t.titlebar_unfocused };
    for row in 0..title_h {
        let color = blend(title_color_top, title_color_bottom, (row * 255 / title_h.max(1)) as u32);
        fb::fill_rect(x + 1, y + 1 + row, width - 2, 1, color);
    }
    if w.focused {
        fb::fill_rect(x + 1, y + title_h, width - 2, 1, t.titlebar_focused_glow);
    }

    let title = clipped(&w.title, ((width - 70) / CHAR_W as i32).max(1) as usize);
    text(x + 10, y + 7, &title, if w.focused { t.title_text_focused } else { t.title_text_unfocused });

    // Close and minimize buttons.
    let close_x = x + width - 20;
    let min_x = close_x - 20;
    let close_fill = if w.close_hover { t.close_hover_fill } else { t.close_normal_fill };
    let close_border = if w.close_hover { t.close_hover_border } else { t.close_normal_border };
    fb::fill_rect(close_x, y + 7, CLOSE_BTN, CLOSE_BTN, close_fill);
    fb::rect_outline(close_x, y + 7, CLOSE_BTN, CLOSE_BTN, close_border);
    for i in 0..6 {
        fb::putpixel(close_x + 4 + i, y + 11 + i, close_border);
        fb::putpixel(close_x + 9 - i, y + 11 + i, close_border);
    }
    let min_fill = if w.min_hover { 0xFFD166 } else { 0x3D351A };
    fb::fill_rect(min_x, y + 7, CLOSE_BTN, CLOSE_BTN, min_fill);
    fb::rect_outline(min_x, y + 7, CLOSE_BTN, CLOSE_BTN, 0x7F6A20);
    fb::fill_rect(min_x + 4, y + 14, 6, 2, 0x7F6A20);

    // Content.
    let content_x = x + 1;
    let content_y = y + title_h + 1;
    let content_w = width - 2;
    let content_h = height - title_h - 2;
    if content_w > 0 && content_h > 0 {
        let fb_width = fb::width();
        let buffer = fb::backbuffer();
        let src_stride = w.w as usize;
        for row in 0..content_h {
            let dst_y = content_y + row;
            if dst_y < 0 || dst_y >= fb::height() as i32 {
                continue;
            }
            let dst_x = content_x;
            let copy_w = content_w.min(fb_width as i32 - dst_x);
            if copy_w <= 0 {
                continue;
            }
            let src_start = row as usize * src_stride;
            let src_end = src_start + copy_w as usize;
            if src_end <= w.fb.len() {
                let dst = dst_y as usize * fb_width + dst_x as usize;
                buffer[dst..dst + copy_w as usize].copy_from_slice(&w.fb[src_start..src_end]);
            }
        }
    }
}

fn draw_apps_menu(t: &Theme) {
    if !apps_open() {
        return;
    }
    let x = 104;
    let y = TOPBAR_H + 6;
    let width = 160;
    let row_h = 24;
    let height = row_h * MENU_ITEMS.len() as i32 + 8;
    blend_rect(x + 4, y + 4, width, height, 0x000000, 60);
    fb::fill_rect(x, y, width, height, t.menu_bg);
    fb::rect_outline(x, y, width, height, t.menu_border);
    let hover = unsafe { STATE.apps_hover };
    for (index, item) in MENU_ITEMS.iter().enumerate() {
        let row_y = y + 4 + index as i32 * row_h;
        if hover == index as i32 {
            fb::fill_rect(x + 4, row_y, width - 8, row_h, t.menu_row_hover);
        }
        fb::fill_rect(x + 10, row_y + 10, 2, 2, t.topbar_accent);
        text(x + 20, row_y + 4, item, t.menu_text);
    }
}

fn draw_cursor() {
    let state = mouse::state();
    for (row, bits) in CURSOR.iter().enumerate() {
        for (col, value) in bits.iter().enumerate() {
            if *value == 0 {
                continue;
            }
            let color = match value {
                1 => 0x000000,
                2 => 0xFFFFFF,
                _ => 0x808080,
            };
            fb::putpixel(state.x + col as i32, state.y + row as i32, color);
        }
    }
}

pub fn compose() {
    if !fb::is_ready() {
        return;
    }
    let t = *theme::current();
    wallpaper(&t);
    draw_icons(&t);
    for id in window::sorted() {
        if let Some(w) = window::get(id) {
            draw_window(w, &t);
            window::clear_invalid(id);
        }
    }
    draw_topbar(&t);
    draw_apps_menu(&t);
    draw_taskbar(&t);
    draw_cursor();
}

pub fn present() {
    fb::present();
}

pub fn tick() {
    unsafe {
        STATE.frames += 1;
        STATE.accent_phase = (STATE.accent_phase + 3) % 600;
        let ticks = crate::sync::ticks();
        let second = ticks / 1000;
        if STATE.last_second != second {
            STATE.last_second = second;
            STATE.fps = STATE.frames;
            STATE.frames = 0;
            STATE.caret_phase = !STATE.caret_phase;
        }
    }
}

pub fn fps() -> u32 {
    unsafe { STATE.fps }
}

pub fn set_icon_hover(index: i32) {
    unsafe { STATE.icon_hover = index };
}

pub fn icon_hover() -> i32 {
    unsafe { STATE.icon_hover }
}

pub fn set_taskbar_hover(index: i32) {
    unsafe { STATE.taskbar_hover = index };
}

pub fn set_apps_hover(index: i32) {
    unsafe { STATE.apps_hover = index };
}

pub fn icon_at(mx: i32, my: i32) -> Option<usize> {
    for (index, _) in ICONS.iter().enumerate() {
        let ix = 24;
        let iy = 64 + index as i32 * 76;
        if mx >= ix && mx < ix + ICON_W && my >= iy && my < iy + ICON_H + 16 {
            return Some(index);
        }
    }
    None
}

pub fn apps_button_at(mx: i32, my: i32) -> bool {
    mx >= 104 && mx < 178 && my >= 8 && my < 28
}

pub fn apps_menu_at(mx: i32, my: i32) -> Option<usize> {
    if !apps_open() {
        return None;
    }
    let x = 104;
    let y = TOPBAR_H + 6;
    let width = 160;
    let row_h = 24;
    if mx < x || mx >= x + width || my < y + 4 {
        return None;
    }
    let index = (my - y - 4) / row_h;
    if index >= 0 && (index as usize) < MENU_ITEMS.len() {
        Some(index as usize)
    } else {
        None
    }
}

pub fn taskbar_at(mx: i32, my: i32) -> Option<u32> {
    let y = SCREEN_H - TASKBAR_H;
    if my < y || my >= SCREEN_H {
        return None;
    }
    let mut x = 8;
    for w in window::all().iter() {
        let width = 150.min(SCREEN_W - x - 8);
        if mx >= x && mx < x + width {
            return Some(w.id);
        }
        x += width + 6;
    }
    None
}

pub fn window_region_at(mx: i32, my: i32) -> Option<(u32, Region)> {
    let id = window::at_point(mx, my)?;
    let w = window::get(id)?;
    let border = theme::current().border_width;
    if mx >= w.x && mx < w.x + w.w && my >= w.y && my < w.y + w.h {
        let close_x = w.x + w.w - 20;
        let min_x = close_x - 20;
        if my >= w.y + 7 && my < w.y + 7 + CLOSE_BTN {
            if mx >= close_x && mx < close_x + CLOSE_BTN {
                return Some((id, Region::Close));
            }
            if mx >= min_x && mx < min_x + CLOSE_BTN {
                return Some((id, Region::Minimize));
            }
        }
        if my < w.y + theme::current().titlebar_h {
            return Some((id, Region::Titlebar));
        }
        if mx < w.x + border || mx >= w.x + w.w - border || my >= w.y + w.h - border {
            return Some((id, Region::Border));
        }
        return Some((id, Region::Client));
    }
    None
}

pub fn client_origin(id: u32) -> (i32, i32) {
    if let Some(w) = window::get(id) {
        (w.x + 1, w.y + theme::current().titlebar_h + 1)
    } else {
        (0, 0)
    }
}

pub fn screen_size() -> (i32, i32) {
    (fb::width() as i32, fb::height() as i32)
}

/// Integer midpoint circle — keeps the kernel free of SSE/FPU state.
fn draw_circle(cx: i32, cy: i32, radius: i32, color: u32) {
    let mut x = radius;
    let mut y = 0;
    let mut err = 1 - radius;
    while x >= y {
        fb::putpixel(cx + x, cy + y, color);
        fb::putpixel(cx + y, cy + x, color);
        fb::putpixel(cx - y, cy + x, color);
        fb::putpixel(cx - x, cy + y, color);
        fb::putpixel(cx - x, cy - y, color);
        fb::putpixel(cx - y, cy - x, color);
        fb::putpixel(cx + y, cy - x, color);
        fb::putpixel(cx + x, cy - y, color);
        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}
