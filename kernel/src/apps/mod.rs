//! Built-in desktop applications.

pub mod about;
pub mod editor;
pub mod files;
pub mod netmon;
pub mod settings;
pub mod terminal;

use crate::gfx::window;

pub fn init() {
    terminal::init();
    files::init();
    settings::init();
    about::init();
    netmon::init();
    editor::init();
    crate::kprintln!("[apps] terminal, files, settings, about, netmon, editor ready");
}

pub fn open(kind: u8) -> u32 {
    match kind {
        window::KIND_TERMINAL => terminal::open(),
        window::KIND_FILES => files::open(),
        window::KIND_SETTINGS => settings::open(),
        window::KIND_ABOUT => about::open(),
        window::KIND_NETMON => netmon::open(),
        window::KIND_EDITOR => editor::open(),
        _ => 0,
    }
}

pub fn close_window(id: u32) {
    terminal::close_window(id);
    files::close_window(id);
    settings::close_window(id);
    about::close_window(id);
    netmon::close_window(id);
    editor::close_window(id);
}

pub fn window_id(kind: u8) -> Option<u32> {
    match kind {
        window::KIND_TERMINAL => terminal::window_id(),
        window::KIND_FILES => files::window_id(),
        window::KIND_SETTINGS => settings::window_id(),
        window::KIND_ABOUT => about::window_id(),
        window::KIND_NETMON => netmon::window_id(),
        window::KIND_EDITOR => editor::window_id(),
        _ => None,
    }
}

pub fn tick() {
    terminal::tick();
    files::tick();
    settings::tick();
    about::tick();
    netmon::tick();
    editor::tick();
}

pub fn handle_key(kind: u8, key: u16) {
    match kind {
        window::KIND_TERMINAL => terminal::handle_key(key),
        window::KIND_FILES => files::handle_key_pub(key),
        window::KIND_SETTINGS => settings::handle_key_pub(key),
        window::KIND_EDITOR => editor::handle_key_pub(key),
        window::KIND_NETMON => netmon::handle_key_pub(key),
        _ => {}
    }
}

pub fn handle_client_click(kind: u8, local_x: i32, local_y: i32) {
    match kind {
        window::KIND_TERMINAL => terminal::handle_click(local_y),
        window::KIND_FILES => files::handle_click_pub(local_y),
        window::KIND_SETTINGS => settings::handle_click_pub(local_x, local_y),
        window::KIND_EDITOR => editor::handle_click_pub(local_x, local_y),
        window::KIND_NETMON => netmon::handle_click_pub(local_x, local_y),
        _ => {}
    }
}

pub fn stdio_write(pid: i32, bytes: &[u8]) -> i32 {
    terminal::stdio_write(pid, bytes)
}

pub fn stdio_read(pid: i32, buffer: &mut [u8]) -> i32 {
    terminal::stdio_read(pid, buffer)
}

pub fn notify_death(pid: i32) {
    terminal::notify_death(pid);
}
