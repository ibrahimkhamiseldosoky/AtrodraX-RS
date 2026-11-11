//! Serial debug console. Commands are read from COM1 and executed in the
//! kernel main loop, independent of the graphical desktop.

use crate::arch::serial;
use crate::fs::vfs;
use crate::task;

const LINE_MAX: usize = 160;
static mut LINE: [u8; LINE_MAX] = [0; LINE_MAX];
static mut LEN: usize = 0;

pub fn init() {
    unsafe { LEN = 0 };
    crate::kprintln!("[console] serial debug console ready (type 'help')");
}

fn print_line(s: &str) {
    crate::kprint!("{}", s);
    crate::arch::serial::write_str(s);
}

fn prompt() {
    crate::arch::serial::write_str("atrodrax> ");
}

pub fn poll() {
    loop {
        let byte = match serial::try_read() {
            Some(b) => b,
            None => break,
        };
        match byte {
            b'\r' | b'\n' => {
                serial::write_str("\r\n");
                execute();
                unsafe { LEN = 0 };
                prompt();
            }
            0x08 | 0x7F => unsafe {
                if LEN > 0 {
                    LEN -= 1;
                    serial::write_str("\x08 \x08");
                }
            },
            b if b >= 0x20 && b < 0x7F => unsafe {
                if LEN < LINE_MAX - 1 {
                    LINE[LEN] = b;
                    LEN += 1;
                    let mut tmp = [0u8; 1];
                    tmp[0] = b;
                    if let Ok(s) = core::str::from_utf8(&tmp) {
                        serial::write_str(s);
                    }
                }
            },
            _ => {}
        }
    }
}

fn line_str() -> &'static str {
    unsafe {
        let len = LEN;
        core::str::from_utf8(&LINE[..len]).unwrap_or("")
    }
}

fn execute() {
    let line = line_str().trim();
    if line.is_empty() {
        return;
    }
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("");
    match cmd {
        "help" | "?" => print_line(
            "commands: help ps mem fs net gfx lspci tasks uptime tests theme reboot halt\r\n",
        ),
        "ps" | "tasks" => {
            for t in task::table().iter() {
                let line = alloc::format!(
                    "{:3} {:3} {:6} {}\r\n",
                    t.pid,
                    t.ppid,
                    task::state_name(t.state),
                    t.name
                );
                print_line(&line);
            }
        }
        "mem" => {
            crate::mem::pmm::print_info();
            crate::mem::heap::print_info();
        }
        "fs" => {
            vfs::print_info();
            let line = alloc::format!(
                "fat32: {} mounted, {} clusters\r\n",
                if crate::fs::fat32::mounted() { "yes" } else { "no" },
                crate::fs::fat32::info().cluster_count
            );
            print_line(&line);
        }
        "net" => crate::net::print_info(),
        "gfx" => {
            let line = alloc::format!(
                "framebuffer {}x{} fps={} windows={}\r\n",
                crate::drivers::fb::width(),
                crate::drivers::fb::height(),
                crate::gfx::compositor::fps(),
                crate::gfx::window::count()
            );
            print_line(&line);
        }
        "lspci" => crate::drivers::pci::print_info(),
        "uptime" => {
            let line = alloc::format!(
                "{} ticks ({} ms)\r\n",
                crate::sync::ticks(),
                crate::sync::uptime_ms()
            );
            print_line(&line);
        }
        "tests" => crate::diag::tests::run_all(),
        "theme" => {
            crate::gfx::theme::toggle();
            crate::fs::persist::save();
            let line = alloc::format!("theme: {}\r\n", crate::gfx::theme::current().name);
            print_line(&line);
        }
        "reboot" => {
            print_line("rebooting...\r\n");
            crate::fs::persist::save();
            unsafe { crate::arch::port::outb(0x64, 0xFE) };
        }
        "halt" => {
            print_line("halted.\r\n");
            crate::fs::persist::save();
            crate::arch::cpu::halt_loop();
        }
        other => {
            let line = alloc::format!("unknown command: {}\r\n", other);
            print_line(&line);
        }
    }
    let _ = arg;
}
