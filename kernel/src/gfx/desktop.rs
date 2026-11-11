//! Desktop event loop: icons, apps menu, taskbar, window focus, dragging,
//! keyboard routing and application dispatch.

use crate::apps;
use crate::gfx::compositor::{self, Region};
use crate::gfx::{theme, window};
use crate::drivers::keyboard;
use alloc::string::String;

const KIND_EXTERNAL: u8 = 200;

struct Desktop {
    drag: Option<(u32, i32, i32)>,
    prev_left: bool,
    render_needed: bool,
    last_click_ms: u64,
    last_click_icon: i32,
}

static mut DESKTOP: Option<Desktop> = None;

fn desktop() -> &'static mut Desktop {
    unsafe { DESKTOP.as_mut().expect("desktop not initialized") }
}

pub fn init() {
    unsafe {
        DESKTOP = Some(Desktop {
            drag: None,
            prev_left: false,
            render_needed: true,
            last_click_ms: 0,
            last_click_icon: -1,
        });
    }
    apps::init();
    crate::kprintln!("[desktop] event loop initialized");
}

pub fn launch(kind: u8) {
    if kind == window::KIND_BROWSER {
        let pid = crate::exec::spawn_path("/bin/browser");
        if pid > 0 {
            crate::kprintln!("[desktop] browser pid {}", pid);
        } else {
            crate::kprintln!("[desktop] browser not available");
        }
        return;
    }
    let id = apps::open(kind);
    if id != 0 {
        desktop().render_needed = true;
    }
}

fn menu_action(index: usize) {
    match index {
        0 => launch(window::KIND_TERMINAL),
        1 => launch(window::KIND_FILES),
        2 => launch(window::KIND_SETTINGS),
        3 => launch(window::KIND_NETMON),
        4 => launch(window::KIND_EDITOR),
        5 => launch(window::KIND_ABOUT),
        6 => {
            crate::fs::persist::save();
            unsafe { crate::arch::port::outb(0x64, 0xFE) }
        }
        7 => {
            crate::fs::persist::save();
            crate::arch::cpu::halt_loop()
        }
        _ => {}
    }
}

fn close_window(id: u32) {
    let (kind, owner) = match window::get(id) {
        Some(w) => (w.kind, w.owner_pid),
        None => return,
    };
    if kind <= window::KIND_PS {
        apps::close_window(id);
    }
    if owner > 0 {
        crate::syscall::gui_forget_window(owner, id);
    }
    window::destroy(id);
    desktop().render_needed = true;
}

fn press(x: i32, y: i32) {
    if compositor::apps_button_at(x, y) {
        compositor::toggle_apps();
        desktop().render_needed = true;
        return;
    }
    if let Some(index) = compositor::apps_menu_at(x, y) {
        compositor::set_apps_open(false);
        menu_action(index);
        return;
    }
    if let Some(id) = compositor::taskbar_at(x, y) {
        if let Some(w) = window::get(id) {
            if w.minimized {
                window::restore(id);
            } else {
                window::focus(id);
            }
        }
        desktop().render_needed = true;
        return;
    }
    if let Some((id, region)) = compositor::window_region_at(x, y) {
        window::focus(id);
        let (wx, wy, kind, owner) = match window::get(id) {
            Some(w) => (w.x, w.y, w.kind, w.owner_pid),
            None => return,
        };
        match region {
            Region::Close => close_window(id),
            Region::Minimize => window::minimize(id),
            Region::Titlebar => {
                desktop().drag = Some((id, x - wx, y - wy));
            }
            Region::Border => {
                desktop().drag = Some((id, x - wx, y - wy));
            }
            Region::Client => {
                let (origin_x, origin_y) = compositor::client_origin(id);
                let local_x = x - origin_x;
                let local_y = y - origin_y;
                if owner > 0 && kind == KIND_EXTERNAL {
                    crate::syscall::gui_push_mouse(owner, id, local_x, local_y, true);
                } else {
                    apps::handle_client_click(kind, local_x, local_y);
                }
                desktop().render_needed = true;
            }
        }
        return;
    }
    if let Some(index) = compositor::icon_at(x, y) {
        let now = crate::sync::uptime_ms();
        let d = desktop();
        let double = d.last_click_icon == index as i32 && now.saturating_sub(d.last_click_ms) < 400;
        d.last_click_icon = index as i32;
        d.last_click_ms = now;
        let _ = double;
        if let Some(icon) = compositor::ICONS.get(index) {
            launch(icon.app);
        }
        return;
    }
    compositor::set_apps_open(false);
    desktop().render_needed = true;
}

pub fn handle_mouse(x: i32, y: i32, left: bool) {
    let d = desktop();
    if left && !d.prev_left {
        press(x, y);
    } else if left && d.prev_left {
        if let Some((id, offset_x, offset_y)) = d.drag {
            window::move_to(id, x - offset_x, y - offset_y);
            d.render_needed = true;
        }
    }
    if !left && d.prev_left {
        d.drag = None;
    }
    d.prev_left = left;
}

pub fn handle_key(key: u16) {
    let ctrl = keyboard::ctrl_down();
    let alt = keyboard::alt_down();

    if key == keyboard::KEY_ESC {
        if compositor::apps_open() {
            compositor::set_apps_open(false);
            desktop().render_needed = true;
            return;
        }
    }

    if alt && key == keyboard::KEY_TAB {
        cycle_focus();
        return;
    }

    match key {
        0x110 => {
            launch(window::KIND_TERMINAL);
            return;
        }
        0x111 => {
            launch(window::KIND_FILES);
            return;
        }
        0x112 => {
            launch(window::KIND_SETTINGS);
            return;
        }
        0x113 => {
            launch(window::KIND_NETMON);
            return;
        }
        0x114 => {
            launch(window::KIND_EDITOR);
            return;
        }
        0x115 => {
            launch(window::KIND_ABOUT);
            return;
        }
        _ => {}
    }

    let Some((id, kind, owner)) = window::focused_id().and_then(|id| {
        window::get(id).map(|w| (id, w.kind, w.owner_pid))
    }) else {
        return;
    };

    if owner > 0 && kind == KIND_EXTERNAL {
        if key < 0x100 {
            crate::syscall::gui_push_key(owner, id, key as u8 as i32);
        }
        return;
    }
    if key < 0x100 && key >= 0x20 {
        apps::handle_key(kind, key);
    } else {
        apps::handle_key(kind, key);
    }
    let _ = ctrl;
    desktop().render_needed = true;
}

fn cycle_focus() {
    let ids = window::sorted();
    if ids.len() < 2 {
        return;
    }
    let current = window::focused_id();
    let position = current
        .and_then(|id| ids.iter().position(|candidate| *candidate == id))
        .unwrap_or(0);
    let next = ids[(position + 1) % ids.len()];
    window::focus(next);
    desktop().render_needed = true;
}

pub fn tick() {
    apps::tick();
    if apps::terminal::is_dirty() {
        desktop().render_needed = true;
    }
}

pub fn render_needed() -> bool {
    desktop().render_needed
}

pub fn clear_render_needed() {
    desktop().render_needed = false;
}

pub fn standard_title(kind: u8) -> String {
    match kind {
        window::KIND_TERMINAL => String::from("Terminal"),
        window::KIND_FILES => String::from("Files"),
        window::KIND_SETTINGS => String::from("Settings"),
        window::KIND_EDITOR => String::from("Editor"),
        window::KIND_ABOUT => String::from("About"),
        window::KIND_NETMON => String::from("Network"),
        _ => String::from("Window"),
    }
}

pub fn screen_ready() -> bool {
    crate::drivers::fb::is_ready()
}

pub fn theme_name() -> &'static str {
    theme::current().name
}
