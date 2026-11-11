//! Kernel main loop: GUI composition, input polling, networking, userspace
//! scheduling and diagnostics.
//!
//! The kernel owns the display and the interrupt-driven drivers.  It composes
//! the desktop at a fixed rate and, between frames, hands the CPU to ring-3
//! tasks for a cooperative slice.  This keeps the GUI responsive while user
//! programs still make progress.

use crate::diag::{debug_console, heartbeat};
use crate::drivers::{keyboard, mouse};
use crate::gfx::{compositor, desktop, window};
use crate::sched;

/// Target frame interval in milliseconds (~60 Hz).
const FRAME_MS: u64 = 16;
/// How often to poll the serial debug console / heartbeat monitor.
const HOUSEKEEPING_MS: u64 = 100;

fn init() {
    desktop::init();
    debug_console::init();
    heartbeat::init();

    // Open the terminal so the machine is immediately usable.
    desktop::launch(window::KIND_TERMINAL);

    crate::kprintln!("[system] entering main loop");
}

fn pump_input(last: &mut mouse::MouseState, dirty: &mut bool) {
    while let Some(key) = keyboard::poll() {
        desktop::handle_key(key);
        *dirty = true;
    }

    let current = mouse::state();
    if current.x != last.x || current.y != last.y {
        desktop::handle_mouse(current.x, current.y, current.left);
        *dirty = true;
    } else if current.left != last.left {
        desktop::handle_mouse(current.x, current.y, current.left);
        *dirty = true;
    }
    *last = current;
}

fn compose_frame() {
    compositor::compose();
    compositor::present();
    compositor::tick();
}

pub fn run() -> ! {
    init();

    let mut last_mouse = mouse::state();
    let mut last_frame = crate::sync::uptime_ms();
    let mut last_housekeeping = last_frame;

    loop {
        let now = crate::sync::uptime_ms();

        // Network receive/ARP/DHCP/TCP timers.
        crate::net::poll();

        // Input devices and desktop dispatch.
        let mut dirty = false;
        pump_input(&mut last_mouse, &mut dirty);

        // Application-level timers (clock, terminal caret, editor blinking).
        desktop::tick();

        if dirty || desktop::render_needed() || now.saturating_sub(last_frame) >= FRAME_MS {
            if !desktop::screen_ready() {
                // No framebuffer: still run userspace and diagnostics.
                sched::run_user_slice();
                crate::arch::cpu::halt();
                continue;
            }
            compose_frame();
            desktop::clear_render_needed();
            last_frame = now;
            heartbeat::ping(heartbeat::HEARTBEAT_GUI);
        }

        // Serial maintenance console.
        if now.saturating_sub(last_housekeeping) >= HOUSEKEEPING_MS {
            debug_console::poll();
            heartbeat::check();
            heartbeat::ping(heartbeat::HEARTBEAT_DRIVERS);
            heartbeat::ping(heartbeat::HEARTBEAT_FS);
            heartbeat::ping(heartbeat::HEARTBEAT_NET);
            heartbeat::ping(heartbeat::HEARTBEAT_SCHED);
            last_housekeeping = now;
        }

        // Let ring-3 tasks run for a cooperative slice.  The timer IRQ can
        // preempt them and bring control back through `run_user_slice`.
        sched::run_user_slice();
    }
}
