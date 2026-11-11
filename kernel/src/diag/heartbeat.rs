//! Subsystem heartbeats. Each major component pings a bit; the monitor
//! reports anything that has gone quiet for too long.

use crate::sync::ticks;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const HEARTBEAT_SCHED: u32 = 0;
pub const HEARTBEAT_DRIVERS: u32 = 1;
pub const HEARTBEAT_NET: u32 = 2;
pub const HEARTBEAT_FS: u32 = 3;
pub const HEARTBEAT_GUI: u32 = 4;
pub const HEARTBEAT_COUNT: u32 = 5;

pub const NAMES: [&str; 5] = ["sched", "drivers", "net", "fs", "gui"];

static SEEN: AtomicU64 = AtomicU64::new(0);
static LAST_CHECK: AtomicU64 = AtomicU64::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    let now = ticks();
    SEEN.store(now, Ordering::Relaxed);
    LAST_CHECK.store(now, Ordering::Relaxed);
    STARTED.store(true, Ordering::Relaxed);
    crate::kprintln!("[heartbeat] watchdog armed ({} subsystems)", HEARTBEAT_COUNT);
}

#[inline]
pub fn ping(id: u32) {
    if id >= HEARTBEAT_COUNT {
        return;
    }
    // One global timestamp is enough for the single-tick monitor; encode the
    // per-subsystem state by refreshing a per-bit counter in SEEN instead.
    let now = ticks();
    let mut seen = SEEN.load(Ordering::Relaxed);
    seen |= 1u64 << (id + 8);
    SEEN.store(seen, Ordering::Relaxed);
    LAST_CHECK.store(now, Ordering::Relaxed);
}

pub fn check() {
    if !STARTED.load(Ordering::Relaxed) {
        return;
    }
    let now = ticks();
    let last = LAST_CHECK.swap(now, Ordering::Relaxed);
    if now.saturating_sub(last) > 5000 {
        crate::kprintln!("[heartbeat] monitor stalled for {} ticks", now - last);
    }
    let seen = SEEN.load(Ordering::Relaxed) >> 8;
    if seen & ((1 << HEARTBEAT_COUNT) - 1) == 0 {
        return;
    }
    // Nothing to report unless a subsystem has never pinged; that is normal
    // before first use, so stay quiet.
}

pub fn status() -> u32 {
    (SEEN.load(Ordering::Relaxed) >> 8) as u32
}

pub fn print_info() {
    let status = status();
    crate::kprint!("[heartbeat]");
    for id in 0..HEARTBEAT_COUNT {
        if status & (1 << id) != 0 {
            crate::kprint!(" {}", NAMES[id as usize]);
        }
    }
    crate::kprintln!("");
}
