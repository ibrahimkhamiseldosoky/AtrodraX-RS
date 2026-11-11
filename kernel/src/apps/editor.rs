//! Small text editor app.

use crate::fs::vfs;
use crate::gfx::{font, window};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = 0x0B101A;
const TEXT: u32 = 0xD7E3EA;
const ACCENT: u32 = 0x8BE9FD;
const MUTED: u32 = 0x64748B;
const CARET: u32 = 0x00BCD4;

pub struct Editor {
    pub win: Option<u32>,
    path: String,
    text: Vec<char>,
    cursor: usize,
    scroll: usize,
    dirty: bool,
    modified: bool,
}

static mut EDITOR: Option<Editor> = None;

pub fn init() {
    unsafe {
        EDITOR = Some(Editor {
            win: None,
            path: String::new(),
            text: Vec::new(),
            cursor: 0,
            scroll: 0,
            dirty: true,
            modified: false,
        });
    }
}

fn editor() -> &'static mut Editor {
    unsafe { EDITOR.as_mut().expect("editor not initialized") }
}

impl Editor {
    fn load(&mut self, path: &str) {
        self.path = String::from(path);
        self.text.clear();
        if let Some(index) = vfs::resolve(path, "/") {
            if !vfs::is_directory(index) {
                let data = vfs::read_all(index);
                let text = String::from_utf8_lossy(&data);
                self.text.extend(text.chars());
            }
        }
        self.cursor = self.text.len();
        self.scroll = 0;
        self.dirty = true;
        self.modified = false;
    }

    fn save(&mut self) {
        if self.path.is_empty() {
            self.path = String::from("/home/untitled.txt");
        }
        let text: String = self.text.iter().collect();
        match vfs::create_file_path(&self.path, "/") {
            Some(index) => {
                vfs::truncate(index, 0);
                vfs::write(index, 0, text.as_bytes());
                self.modified = false;
                if let Some(id) = self.win {
                    window::set_title(id, &format!("Editor - {}", self.path));
                }
            }
            None => {}
        }
        self.dirty = true;
    }

    fn line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for (index, ch) in self.text.iter().enumerate() {
            if index >= self.cursor {
                break;
            }
            if *ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn move_cursor(&mut self, key: u16) {
        match key {
            crate::drivers::keyboard::KEY_LEFT => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            crate::drivers::keyboard::KEY_RIGHT => {
                if self.cursor < self.text.len() {
                    self.cursor += 1;
                }
            }
            crate::drivers::keyboard::KEY_UP => {
                let (line, col) = self.line_col();
                if line > 0 {
                    self.cursor = self.line_start(line - 1) + col.min(self.line_len(line - 1));
                }
            }
            crate::drivers::keyboard::KEY_DOWN => {
                let (line, col) = self.line_col();
                if line + 1 < self.line_count() {
                    self.cursor = self.line_start(line + 1) + col.min(self.line_len(line + 1));
                }
            }
            crate::drivers::keyboard::KEY_HOME => {
                let (line, _) = self.line_col();
                self.cursor = self.line_start(line);
            }
            crate::drivers::keyboard::KEY_END => {
                let (line, _) = self.line_col();
                self.cursor = self.line_start(line) + self.line_len(line);
            }
            crate::drivers::keyboard::KEY_PGUP => {
                let (line, col) = self.line_col();
                let target = line.saturating_sub(20);
                self.cursor = self.line_start(target) + col.min(self.line_len(target));
            }
            crate::drivers::keyboard::KEY_PGDN => {
                let (line, col) = self.line_col();
                let target = (line + 20).min(self.line_count().saturating_sub(1));
                self.cursor = self.line_start(target) + col.min(self.line_len(target));
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn line_start(&self, line: usize) -> usize {
        let mut current = 0;
        for (index, ch) in self.text.iter().enumerate() {
            if current == line {
                return index;
            }
            if *ch == '\n' {
                current += 1;
            }
        }
        self.text.len()
    }

    fn line_len(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let mut len = 0;
        for ch in self.text.iter().skip(start) {
            if *ch == '\n' {
                break;
            }
            len += 1;
        }
        len
    }

    fn line_count(&self) -> usize {
        1 + self.text.iter().filter(|c| **c == '\n').count()
    }

    pub fn handle_key(&mut self, key: u16) {
        if crate::drivers::keyboard::ctrl_down() {
            if key == 0x13 {
                self.save();
                return;
            }
        }
        if key >= 0x100 {
            self.move_cursor(key);
            return;
        }
        let ch = key as u8;
        match ch {
            0x08 | 0x7F => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.text.remove(self.cursor);
                    self.modified = true;
                    self.dirty = true;
                }
            }
            0x0A | 0x0D => {
                self.text.insert(self.cursor, '\n');
                self.cursor += 1;
                self.modified = true;
                self.dirty = true;
            }
            0x09 => {
                for _ in 0..4 {
                    self.text.insert(self.cursor, ' ');
                    self.cursor += 1;
                }
                self.modified = true;
                self.dirty = true;
            }
            c if (0x20..=0x7E).contains(&c) => {
                self.text.insert(self.cursor, c as char);
                self.cursor += 1;
                self.modified = true;
                self.dirty = true;
            }
            _ => {}
        }
    }

    pub fn handle_click(&mut self, local_x: i32, local_y: i32) {
        let line = ((local_y - 8).max(0) / font::CHAR_H as i32) as usize + self.scroll;
        let col = ((local_x - 8).max(0) / font::CHAR_W as i32) as usize;
        let line = line.min(self.line_count().saturating_sub(1));
        let col = col.min(self.line_len(line));
        self.cursor = self.line_start(line) + col;
        self.dirty = true;
    }

    pub fn render(&mut self) {
        let Some(id) = self.win else { return };
        if !window::exists(id) {
            self.win = None;
            return;
        }
        let height = window::get(id).map(|w| w.h).unwrap_or(440);
        let width = window::get(id).map(|w| w.w).unwrap_or(640);
        window::clear(id, BG);
        window::draw_text(id, 10, 8, &format!("{}", self.path), ACCENT, BG);
        window::draw_text(
            id,
            width - 120,
            8,
            if self.modified { "[modified] Ctrl+S" } else { "Ctrl+S saves" },
            MUTED,
            BG,
        );

        let rows = ((height - 28) / font::CHAR_H as i32).max(1) as usize;
        let (cursor_line, cursor_col) = self.line_col();
        if cursor_line < self.scroll {
            self.scroll = cursor_line;
        } else if cursor_line >= self.scroll + rows {
            self.scroll = cursor_line + 1 - rows;
        }

        let mut line = 0usize;
        let mut y = 26i32;
        let mut col = 0i32;
        let mut current = String::new();
        for ch in self.text.iter() {
            if line < self.scroll {
                if *ch == '\n' {
                    line += 1;
                }
                continue;
            }
            if line >= self.scroll + rows {
                break;
            }
            if *ch == '\n' {
                window::draw_text(id, 10, y, &current, TEXT, BG);
                y += font::CHAR_H as i32;
                current.clear();
                line += 1;
                col = 0;
                continue;
            }
            current.push(*ch);
            col += 1;
        }
        if line >= self.scroll && line < self.scroll + rows {
            window::draw_text(id, 10, y, &current, TEXT, BG);
        }
        let _ = col;

        let caret_y = 26 + (cursor_line.saturating_sub(self.scroll)) as i32 * font::CHAR_H as i32;
        let caret_x = 10 + cursor_col as i32 * font::CHAR_W as i32;
        window::fill_rect(id, caret_x, caret_y, 2, font::CHAR_H as i32, CARET);

        window::present(id);
        self.dirty = false;
    }
}

pub fn open() -> u32 {
    open_path("/home/untitled.txt")
}

pub fn open_path(path: &str) -> u32 {
    let e = editor();
    if let Some(id) = e.win {
        if window::exists(id) {
            e.load(path);
            window::focus(id);
            window::set_title(id, &format!("Editor - {}", path));
            e.render();
            return id;
        }
        e.win = None;
    }
    e.load(path);
    let id = window::create(160, 96, 640, 440, "Editor", window::KIND_EDITOR, BG);
    window::set_title(id, &format!("Editor - {}", path));
    e.win = Some(id);
    e.render();
    id
}

pub fn window_id() -> Option<u32> {
    let e = editor();
    match e.win {
        Some(id) if window::exists(id) => Some(id),
        _ => None,
    }
}

pub fn close_window(id: u32) {
    let e = editor();
    if e.win == Some(id) {
        e.win = None;
    }
}

pub fn handle_key_pub(key: u16) {
    editor().handle_key(key);
}

pub fn handle_click_pub(x: i32, y: i32) {
    editor().handle_click(x, y);
}

pub fn tick() {
    let e = editor();
    if e.win.is_some() && e.dirty {
        e.render();
    }
}
