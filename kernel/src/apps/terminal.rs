//! Terminal emulator and the built-in kernel shell.
//!
//! The terminal keeps a scrollback grid that is rendered into its own window.
//! If `/bin/shell` exists, a real ring-3 shell is launched and its stdin/stdout
//! are routed through this buffer; otherwise a small built-in shell handles the
//! commands directly.

use crate::exec;
use crate::fs::vfs;
use crate::gfx::{font, theme, window};
use crate::net;
use crate::task;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

pub const COLS: usize = 100;
pub const ROWS: usize = 34;
pub const SCROLLBACK: usize = 512;
pub const INPUT_CAP: usize = 1024;

const DEFAULT_FG: u32 = 0xD7E3EA;
const DEFAULT_BG: u32 = 0x050B0E;
const ACCENT: u32 = 0x00BCD4;

const ANSI_COLORS: [u32; 16] = [
    0x000000, 0xC80000, 0x00C800, 0xC8C800, 0x0000C8, 0xC800C8, 0x00C8C8, 0xC8C8C8,
    0x555555, 0xFF5555, 0x55FF55, 0xFFFF55, 0x5555FF, 0xFF55FF, 0x55FFFF, 0xFFFFFF,
];

#[derive(Clone, Copy)]
struct Cell {
    ch: u8,
    fg: u32,
    bg: u32,
}

impl Cell {
    const fn blank() -> Self {
        Self {
            ch: b' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
        }
    }
}

pub struct Terminal {
    pub win: Option<u32>,
    pub shell_pid: i32,
    cells: Vec<Cell>,
    write_row: usize,
    scroll_off: i32,
    cursor_col: usize,
    cursor_row: usize,
    cur_fg: u32,
    cur_bg: u32,
    bold: bool,
    esc_state: u8,
    esc_buf: [u8; 16],
    esc_len: usize,
    input: [u8; INPUT_CAP],
    in_head: usize,
    in_tail: usize,
    line: String,
    cwd: String,
    dirty: bool,
    caret_visible: bool,
    caret_last: u64,
    last_client_render: u64,
}

static mut TERM: Option<Terminal> = None;

pub fn init() {
    unsafe {
        TERM = Some(Terminal::new());
    }
}

fn term() -> &'static mut Terminal {
    unsafe { TERM.as_mut().expect("terminal not initialized") }
}

impl Terminal {
    fn new() -> Self {
        let mut t = Self {
            win: None,
            shell_pid: -1,
            cells: vec![Cell::blank(); SCROLLBACK * COLS],
            write_row: 0,
            scroll_off: 0,
            cursor_col: 0,
            cursor_row: ROWS - 1,
            cur_fg: DEFAULT_FG,
            cur_bg: DEFAULT_BG,
            bold: false,
            esc_state: 0,
            esc_buf: [0; 16],
            esc_len: 0,
            input: [0; INPUT_CAP],
            in_head: 0,
            in_tail: 0,
            line: String::new(),
            cwd: String::from("/"),
            dirty: true,
            caret_visible: true,
            caret_last: 0,
            last_client_render: 0,
        };
        t.reset_buffer();
        t
    }

    fn reset_buffer(&mut self) {
        for cell in self.cells.iter_mut() {
            *cell = Cell::blank();
        }
        self.write_row = 0;
        self.scroll_off = 0;
        self.cursor_col = 0;
        self.cursor_row = ROWS - 1;
        self.cur_fg = DEFAULT_FG;
        self.cur_bg = DEFAULT_BG;
        self.bold = false;
        self.esc_state = 0;
        self.esc_len = 0;
        self.dirty = true;
    }

    fn viewport_row(&self, vrow: usize) -> usize {
        let bottom = (self.write_row + SCROLLBACK - (self.scroll_off.max(0) as usize % SCROLLBACK)) % SCROLLBACK;
        (bottom + SCROLLBACK - (ROWS - 1) + vrow) % SCROLLBACK
    }

    fn scroll(&mut self) {
        self.write_row = (self.write_row + 1) % SCROLLBACK;
        for col in 0..COLS {
            let index = self.write_row * COLS + col;
            self.cells[index] = Cell {
                ch: b' ',
                fg: self.cur_fg,
                bg: self.cur_bg,
            };
        }
        self.cursor_col = 0;
        self.cursor_row = ROWS - 1;
    }

    fn apply_sgr(&mut self, param: i32) {
        match param {
            0 => {
                self.cur_fg = DEFAULT_FG;
                self.cur_bg = DEFAULT_BG;
                self.bold = false;
            }
            1 => self.bold = true,
            22 => self.bold = false,
            30..=37 => {
                let mut idx = (param - 30) as usize;
                if self.bold {
                    idx += 8;
                }
                self.cur_fg = ANSI_COLORS[idx.min(15)];
            }
            39 => self.cur_fg = DEFAULT_FG,
            40..=47 => self.cur_bg = ANSI_COLORS[(param - 40) as usize],
            49 => self.cur_bg = DEFAULT_BG,
            90..=97 => self.cur_fg = ANSI_COLORS[(param - 90 + 8) as usize],
            100..=107 => self.cur_bg = ANSI_COLORS[(param - 100 + 8) as usize],
            _ => {}
        }
    }

    fn process_escape(&mut self) {
        if self.esc_len == 0 {
            return;
        }
        let cmd = self.esc_buf[self.esc_len - 1];
        match cmd {
            b'm' => {
                let mut param = 0i32;
                for index in 0..self.esc_len {
                    let ch = self.esc_buf[index];
                    if ch.is_ascii_digit() {
                        param = param * 10 + (ch - b'0') as i32;
                    } else if ch == b';' || ch == b'm' {
                        if param == 0 && ch == b';' {
                            param = 0;
                        }
                        self.apply_sgr(param);
                        param = 0;
                    }
                }
            }
            b'J' => self.reset_buffer(),
            b'H' | b'f' => self.cursor_col = 0,
            _ => {}
        }
    }

    pub fn putc(&mut self, c: u8) {
        if self.esc_state == 1 {
            if c == b'[' {
                self.esc_state = 2;
                self.esc_len = 0;
            } else {
                self.esc_state = 0;
            }
            return;
        }
        if self.esc_state == 2 {
            if self.esc_len < 15 {
                self.esc_buf[self.esc_len] = c;
                self.esc_len += 1;
            }
            if c.is_ascii_alphabetic() {
                self.process_escape();
                self.esc_state = 0;
                self.esc_len = 0;
            }
            return;
        }

        match c {
            0x1B => {
                self.esc_state = 1;
                return;
            }
            b'\r' => {
                self.cursor_col = 0;
                return;
            }
            b'\n' => {
                self.cursor_col = 0;
                self.cursor_row += 1;
                if self.cursor_row >= ROWS {
                    self.scroll();
                }
                self.dirty = true;
                return;
            }
            b'\t' => {
                let next = (self.cursor_col + 8) & !7;
                while self.cursor_col < next && self.cursor_col < COLS {
                    self.putc(b' ');
                }
                return;
            }
            0x08 => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                    let row = self.viewport_row(self.cursor_row);
                    self.cells[row * COLS + self.cursor_col] = Cell {
                        ch: b' ',
                        fg: self.cur_fg,
                        bg: self.cur_bg,
                    };
                }
                self.dirty = true;
                return;
            }
            _ => {}
        }

        if self.cursor_col >= COLS {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= ROWS {
                self.scroll();
            }
        }
        if (0x20..=0x7E).contains(&c) {
            let row = self.viewport_row(self.cursor_row);
            self.cells[row * COLS + self.cursor_col] = Cell {
                ch: c,
                fg: self.cur_fg,
                bg: self.cur_bg,
            };
            self.cursor_col += 1;
        }
        self.dirty = true;
    }

    pub fn write_str(&mut self, text: &str) {
        for byte in text.bytes() {
            self.putc(byte);
        }
    }

    fn push_input(&mut self, byte: u8) {
        let next = (self.in_head + 1) % INPUT_CAP;
        if next != self.in_tail {
            self.input[self.in_head] = byte;
            self.in_head = next;
        }
    }

    pub fn pop_input(&mut self) -> Option<u8> {
        if self.in_head == self.in_tail {
            None
        } else {
            let byte = self.input[self.in_tail];
            self.in_tail = (self.in_tail + 1) % INPUT_CAP;
            Some(byte)
        }
    }

    fn drain_user_input(&mut self, buf: &mut [u8]) -> i32 {
        let mut count = 0;
        while count < buf.len() {
            match self.pop_input() {
                Some(byte) => {
                    buf[count] = byte;
                    count += 1;
                }
                None => break,
            }
        }
        count as i32
    }

    pub fn render(&mut self) {
        let Some(id) = self.win else { return };
        if !window::exists(id) {
            self.win = None;
            return;
        }
        let bg = 0x050B0E;
        window::clear(id, bg);
        for vrow in 0..ROWS {
            let row = self.viewport_row(vrow);
            let y = 8 + vrow as i32 * font::CHAR_H as i32;
            let mut line = String::with_capacity(COLS);
            for col in 0..COLS {
                let ch = self.cells[row * COLS + col].ch;
                line.push(if ch == 0 { ' ' } else { ch as char });
            }
            window::draw_text(id, 8, y, &line, DEFAULT_FG, DEFAULT_BG);
            for col in 0..COLS {
                let cell = self.cells[row * COLS + col];
                if cell.fg != DEFAULT_FG || cell.bg != DEFAULT_BG {
                    let text = [cell.ch.max(32)];
                    let text = core::str::from_utf8(&text).unwrap_or(" ");
                    window::draw_text(id, 8 + col as i32 * 8, y, text, cell.fg, cell.bg);
                }
            }
        }
        if self.caret_visible && self.scroll_off == 0 && self.shell_pid <= 0 {
            let cx = 8 + self.cursor_col as i32 * 8;
            let cy = 8 + self.cursor_row as i32 * font::CHAR_H as i32 + 14;
            window::fill_rect(id, cx, cy, 8, 2, ACCENT);
        }
        window::present(id);
        self.dirty = false;
        self.last_client_render = crate::sync::uptime_ms();
    }

    pub fn flush_if_dirty(&mut self) {
        if self.dirty {
            self.render();
        }
    }

    fn prompt(&mut self) {
        let cwd = self.cwd.clone();
        let line = format!("{} $ ", cwd);
        self.write_str(&line);
    }

    fn echo_line(&mut self) {
        let line = self.line.clone();
        self.putc(b'\n');
        self.execute(&line);
        self.prompt();
    }

    fn reset_line(&mut self) {
        self.line.clear();
    }

    fn execute(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        let tokens = split_args(trimmed);
        let cmd = tokens.first().map(|s| s.as_str()).unwrap_or("");
        let args: Vec<&str> = tokens.iter().skip(1).map(|s| s.as_str()).collect();
        match cmd {
            "help" => {
                self.write_str(
                    "AtrodraX shell (built-in)\r\n\
                     commands:\r\n\
                     \x20 help clear echo pwd ls cat cd mkdir touch rm\r\n\
                     \x20 date uptime mem ps net ping dns theme about\r\n\
                     \x20 exec <file> [args...]  run a userland ELF\r\n\
                     \x20 edit [file]           open the editor\r\n\
                     \x20 reboot shutdown\r\n",
                );
            }
            "clear" => {
                self.reset_buffer();
            }
            "echo" => {
                let mut text = String::new();
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        text.push(' ');
                    }
                    text.push_str(arg);
                }
                self.write_str(&text);
                self.putc(b'\n');
            }
            "pwd" => {
                let cwd = self.cwd.clone();
                self.write_str(&cwd);
                self.putc(b'\n');
            }
            "cd" => {
                let target = args.first().copied().unwrap_or("/");
                match vfs::resolve(target, &self.cwd) {
                    Some(index) if vfs::is_directory(index) => {
                        self.cwd = vfs::path_of(index);
                    }
                    _ => self.write_str("cd: no such directory\r\n"),
                }
            }
            "ls" => {
                let target = args.first().copied().unwrap_or(".");
                match vfs::resolve(target, &self.cwd) {
                    Some(index) if vfs::is_directory(index) => {
                        self.write_str(&format!("{}\r\n", vfs::path_of(index)));
                        for (name, kind, child) in vfs::list(index) {
                            let marker = if kind == vfs::KIND_DIR || kind == vfs::KIND_FAT_DIR {
                                "d"
                            } else {
                                "-"
                            };
                            self.write_str(&format!(
                                "{} {:8}  {}\r\n",
                                marker,
                                vfs::size_of(child),
                                name
                            ));
                        }
                    }
                    Some(index) => {
                        self.write_str(&format!("{} {:8}\r\n", vfs::name_of(index), vfs::size_of(index)));
                    }
                    None => self.write_str("ls: not found\r\n"),
                }
            }
            "cat" => {
                let Some(target) = args.first() else {
                    self.write_str("usage: cat <file>\r\n");
                    return;
                };
                match vfs::resolve(target, &self.cwd) {
                    Some(index) if !vfs::is_directory(index) => {
                        let data = vfs::read_all(index);
                        let text = String::from_utf8_lossy(&data);
                        self.write_str(&text);
                        if !text.ends_with('\n') {
                            self.putc(b'\n');
                        }
                    }
                    _ => self.write_str("cat: not found\r\n"),
                }
            }
            "mkdir" => {
                let Some(target) = args.first() else {
                    self.write_str("usage: mkdir <dir>\r\n");
                    return;
                };
                if vfs::mkdir(target, &self.cwd) {
                    self.write_str("ok\r\n");
                } else {
                    self.write_str("mkdir: failed\r\n");
                }
            }
            "touch" => {
                let Some(target) = args.first() else {
                    self.write_str("usage: touch <file>\r\n");
                    return;
                };
                if vfs::create_file_path(target, &self.cwd).is_some() {
                    self.write_str("ok\r\n");
                } else {
                    self.write_str("touch: failed\r\n");
                }
            }
            "rm" => {
                let Some(target) = args.first() else {
                    self.write_str("usage: rm <file>\r\n");
                    return;
                };
                if vfs::unlink(target, &self.cwd) || vfs::rmdir(target, &self.cwd) {
                    self.write_str("ok\r\n");
                } else {
                    self.write_str("rm: failed\r\n");
                }
            }
            "date" => {
                let text = crate::drivers::rtc::date_string();
                self.write_str(&text);
                self.putc(b'\n');
            }
            "uptime" => {
                let ms = crate::sync::uptime_ms();
                let text = format!("up {} ms ({} ticks)\r\n", ms, crate::sync::ticks());
                self.write_str(&text);
            }
            "mem" => {
                let text = format!(
                    "memory: {} pages free, {} used, heap {} KiB free\r\n",
                    crate::mem::pmm::free_pages(),
                    crate::mem::pmm::used_pages(),
                    crate::mem::heap::stats().0 / 1024
                );
                self.write_str(&text);
            }
            "ps" => {
                self.write_str("pid ppid state   name\r\n");
                let mut lines = Vec::new();
                for t in task::table().iter() {
                    lines.push(format!(
                        "{:3} {:4} {:7} {}\r\n",
                        t.pid,
                        t.ppid,
                        task::state_name(t.state),
                        t.name
                    ));
                }
                for line in lines {
                    self.write_str(&line);
                }
            }
            "net" => {
                let text = format!(
                    "mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\r\n\
                     ip  {}  gw {}  dns {}\r\n\
                     rx {} tx {} icmp {} udp {} tcp {}\r\n",
                    net::mac()[0],
                    net::mac()[1],
                    net::mac()[2],
                    net::mac()[3],
                    net::mac()[4],
                    net::mac()[5],
                    net::ip_string(net::local_ip()),
                    net::ip_string(net::gateway()),
                    net::ip_string(net::dns_server()),
                    net::stats().0,
                    net::stats().1,
                    net::stats().2,
                    net::stats().3,
                    crate::sync::ticks() % 1_000_000,
                );
                self.write_str(&text);
            }
            "ping" => {
                let Some(target) = args.first() else {
                    self.write_str("usage: ping <ip>\r\n");
                    return;
                };
                match parse_ipv4(target) {
                    Some(ip) => {
                        net::ping(ip);
                        self.write_str(&format!("ping sent to {}\r\n", net::ip_string(ip)));
                    }
                    None => self.write_str("ping: bad address\r\n"),
                }
            }
            "dns" => {
                let Some(name) = args.first() else {
                    self.write_str("usage: dns <hostname>\r\n");
                    return;
                };
                net::dns_lookup(name);
                self.write_str(&format!("resolving {}...\r\n", name));
            }
            "theme" => {
                match args.first().copied() {
                    Some("hacker") | Some("1") => crate::gfx::theme::set_index(1),
                    Some("dark") | Some("0") => crate::gfx::theme::set_index(0),
                    _ => crate::gfx::theme::toggle(),
                }
                self.write_str(&format!("theme: {}\r\n", theme::current().name));
            }
            "about" => {
                self.write_str(
                    "AtrodraX v0.1 (Rust edition)\r\n\
                     x86-64, long mode, ring-3 userland, FAT32, TCP/IP\r\n\
                     (c) AtrodraX project\r\n",
                );
            }
            "edit" => {
                let path = args.first().copied().unwrap_or("/home/untitled.txt");
                crate::apps::editor::open_path(path);
                self.write_str(&format!("editor: {}\r\n", path));
            }
            "exec" | "run" => {
                let Some(path) = args.first() else {
                    self.write_str("usage: exec <file> [args...]\r\n");
                    return;
                };
                let full = if path.starts_with('/') {
                    path.to_string()
                } else {
                    format!("/bin/{}", path)
                };
                let mut argv = vec![path.to_string()];
                for arg in args.iter().skip(1) {
                    argv.push(arg.to_string());
                }
                let pid = exec::spawn(&full, &argv);
                if pid > 0 {
                    self.shell_pid = pid;
                    if let Some(index) = task::find(pid) {
                        task::get(index).ppid = sched_pid();
                    }
                    self.write_str(&format!("started pid {}\r\n", pid));
                } else {
                    self.write_str("exec: failed\r\n");
                }
            }
            "reboot" => {
                self.write_str("rebooting...\r\n");
                unsafe { crate::arch::port::outb(0x64, 0xFE) };
            }
            "shutdown" | "halt" => {
                self.write_str("halted.\r\n");
                crate::arch::cpu::halt_loop();
            }
            other => {
                self.write_str(&format!("{}: command not found\r\n", other));
            }
        }
    }

    pub fn handle_key(&mut self, key: u16, ctrl: bool) {
        if key >= 0x100 {
            match key {
                crate::drivers::keyboard::KEY_PGUP => self.scroll_by(3),
                crate::drivers::keyboard::KEY_PGDN => self.scroll_by(-3),
                _ => {}
            }
            return;
        }
        let ch = key as u8;
        if ctrl {
            match ch {
                b'u' => {
                    self.scroll_by(3);
                    return;
                }
                b'd' => {
                    self.scroll_by(-3);
                    return;
                }
                b'l' => {
                    self.reset_buffer();
                    self.prompt();
                    return;
                }
                b'c' => {
                    self.reset_line();
                    self.putc(b'^');
                    self.putc(b'C');
                    self.putc(b'\n');
                    self.prompt();
                    return;
                }
                _ => {}
            }
        }

        if self.shell_pid > 0 && self.process_alive(self.shell_pid) {
            if self.scroll_off > 0 {
                self.scroll_off = 0;
                self.dirty = true;
            }
            if ch == b'\r' || ch == b'\n' {
                self.push_input(b'\n');
            } else {
                self.push_input(ch);
            }
            return;
        }

        if self.scroll_off > 0 {
            self.scroll_off = 0;
            self.dirty = true;
        }
        match ch {
            b'\r' | b'\n' => {
                self.echo_line();
            }
            0x08 | 0x7F => {
                if self.line.pop().is_some() {
                    self.putc(0x08);
                }
            }
            0x09 => {
                self.line.push(' ');
                self.putc(b' ');
            }
            0x1B => {
                self.reset_line();
                self.putc(b'\n');
                self.prompt();
            }
            c if (0x20..=0x7E).contains(&c) => {
                self.line.push(c as char);
                self.putc(c);
            }
            _ => {}
        }
    }

    fn scroll_by(&mut self, amount: i32) {
        let max_scroll = (SCROLLBACK - ROWS) as i32;
        self.scroll_off = (self.scroll_off + amount).clamp(0, max_scroll);
        self.dirty = true;
    }

    fn process_alive(&self, pid: i32) -> bool {
        match task::find(pid) {
            Some(index) => {
                let state = task::get(index).state;
                state != task::TASK_DEAD && state != task::TASK_ZOMBIE
            }
            None => false,
        }
    }

    fn start_shell(&mut self) {
        if self.shell_pid > 0 && self.process_alive(self.shell_pid) {
            return;
        }
        if vfs::resolve("/bin/shell", "/").is_none() {
            self.write_str("AtrodraX built-in shell. Type 'help'.\r\n");
            self.prompt();
            return;
        }
        let pid = exec::spawn_path("/bin/shell");
        if pid > 0 {
            self.shell_pid = pid;
            if let Some(index) = task::find(pid) {
                task::get(index).ppid = sched_pid();
            }
            self.write_str("[terminal] user shell started\r\n");
        } else {
            self.write_str("AtrodraX built-in shell. Type 'help'.\r\n");
            self.prompt();
        }
    }

    fn check_foreground(&mut self) {
        if self.shell_pid > 0 && !self.process_alive(self.shell_pid) {
            self.shell_pid = -1;
            self.putc(b'\n');
            self.prompt();
        }
    }
}

fn sched_pid() -> i32 {
    crate::sched::current_pid()
}

/// Split a command line into arguments, honouring single and double quotes so
/// that `busybox sh -c 'echo hi | cat'` arrives as one script argument.
fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for ch in line.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    started = true;
                }
                c if c.is_whitespace() => {
                    if started || !current.is_empty() {
                        out.push(current.clone());
                        current.clear();
                        started = false;
                    }
                }
                c => {
                    current.push(c);
                    started = true;
                }
            },
        }
    }
    if started || !current.is_empty() {
        out.push(current);
    }
    out
}

fn parse_ipv4(text: &str) -> Option<u32> {    let mut octets = [0u8; 4];
    let mut count = 0;
    for part in text.split('.') {
        if count >= 4 {
            return None;
        }
        octets[count] = part.parse::<u8>().ok()?;
        count += 1;
    }
    if count != 4 {
        return None;
    }
    Some(((octets[0] as u32) << 24) | ((octets[1] as u32) << 16) | ((octets[2] as u32) << 8) | octets[3] as u32)
}

/* ── Public app entry points ───────────────────────────────────────────── */

pub fn open() -> u32 {
    let t = term();
    if let Some(id) = t.win {
        if window::exists(id) {
            window::focus(id);
            t.render();
            return id;
        }
        t.win = None;
    }
    let hint = crate::fs::persist::terminal_layout_hint();
    let (x, y, w, h) = match hint {
        Some(l) => (l.x, l.y, l.w, l.h),
        None => (108, 78, 808, 608),
    };
    if hint.is_some() {
        crate::fs::persist::clear_terminal_hint();
    }
    let id = window::create(x, y, w, h, "Terminal", window::KIND_TERMINAL, DEFAULT_BG);
    t.win = Some(id);
    if t.cells.iter().all(|c| c.ch == b' ') {
        t.start_shell();
    }
    t.render();
    id
}

pub fn window_id() -> Option<u32> {
    let t = term();
    match t.win {
        Some(id) if window::exists(id) => Some(id),
        _ => None,
    }
}

pub fn close_window(id: u32) {
    let t = term();
    if t.win == Some(id) {
        t.win = None;
    }
}

pub fn handle_key(key: u16) {
    term().handle_key(key, crate::drivers::keyboard::ctrl_down());
}

pub fn handle_click(local_y: i32) {
    let t = term();
    let _ = local_y;
    if t.scroll_off > 0 {
        t.scroll_off = 0;
        t.dirty = true;
    }
}

pub fn tick() {
    let t = term();
    if !crate::drivers::fb::is_ready() {
        return;
    }
    let now = crate::sync::uptime_ms();
    let blink = theme::current().caret_blink_ms as u64;
    if now.saturating_sub(t.caret_last) >= blink.max(100) {
        t.caret_visible = !t.caret_visible;
        t.caret_last = now;
        if t.win.is_some() {
            t.dirty = true;
        }
    }
    t.check_foreground();
    if t.dirty && now.saturating_sub(t.last_client_render) >= 16 {
        t.flush_if_dirty();
    }
}

pub fn is_dirty() -> bool {
    term().dirty
}

pub fn notify_death(pid: i32) {
    let t = term();
    if t.shell_pid == pid {
        t.shell_pid = -1;
        t.putc(b'\n');
        t.prompt();
    }
}

pub fn stdio_write(pid: i32, bytes: &[u8]) -> i32 {
    let t = term();
    if !t.is_terminal_process(pid) {
        return -1;
    }
    for byte in bytes {
        t.putc(*byte);
        // Mirror program output to the serial console for headless debugging.
        crate::arch::serial::write_byte(*byte);
    }
    bytes.len() as i32
}

pub fn stdio_read(pid: i32, buffer: &mut [u8]) -> i32 {
    let t = term();
    if !t.is_terminal_process(pid) {
        return -1;
    }
    t.drain_user_input(buffer)
}

impl Terminal {
    fn is_terminal_process(&self, pid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        if self.shell_pid < 0 {
            return false;
        }
        let mut current = pid;
        for _ in 0..16 {
            if current == self.shell_pid {
                return true;
            }
            match task::find(current) {
                Some(index) => {
                    let ppid = task::get(index).ppid;
                    if ppid == current || ppid <= 0 {
                        return false;
                    }
                    current = ppid;
                }
                None => return false,
            }
        }
        false
    }
}

pub fn inject(text: &str) {
    let t = term();
    for byte in text.bytes() {
        t.push_input(byte);
    }
}
