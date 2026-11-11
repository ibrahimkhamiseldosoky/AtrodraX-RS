//! Interrupt-safe critical sections for a single-CPU kernel.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const IF_FLAG: u64 = 1 << 9;

#[inline]
pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe { asm!("pushfq; pop {}", out(reg) flags, options(nomem, nostack, preserves_flags)) };
    flags & IF_FLAG != 0
}

#[inline]
pub fn disable_interrupts() {
    unsafe { asm!("cli", options(nomem, nostack)) };
}

#[inline]
pub fn enable_interrupts() {
    unsafe { asm!("sti", options(nomem, nostack)) };
}

/// Run `f` with interrupts disabled, restoring the previous IF state.
pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let was_enabled = interrupts_enabled();
    disable_interrupts();
    let result = f();
    if was_enabled {
        enable_interrupts();
    }
    result
}

/// A very small spin lock. The kernel is single-CPU, but this also guards
/// against re-entry from an interrupt.
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> SpinGuard<'_, T> {
        let was_enabled = interrupts_enabled();
        disable_interrupts();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        SpinGuard {
            lock: self,
            restore: was_enabled,
        }
    }
}

pub struct SpinGuard<'a, T> {
    lock: &'a SpinLock<T>,
    restore: bool,
}

impl<T> core::ops::Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> core::ops::DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        if self.restore {
            enable_interrupts();
        }
    }
}

pub static TICKS: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

#[inline]
pub fn uptime_ms() -> u64 {
    ticks()
}
