pub mod panic;
pub mod tests;
pub mod debug_console;
pub mod heartbeat;

pub const TAG_BOOT: &str = "BOOT";
pub const TAG_MEM: &str = "MEM ";
pub const TAG_HEAP: &str = "HEAP";
pub const TAG_VMM: &str = "VMM ";
pub const TAG_GUI: &str = "GUI ";
pub const TAG_NET: &str = "NET ";
pub const TAG_FS: &str = "FS  ";
pub const TAG_DRIVER: &str = "DRV ";
pub const TAG_INPUT: &str = "INPT";
pub const TAG_SCHED: &str = "SCHD";
pub const TAG_TEST: &str = "TEST";
pub const TAG_EXEC: &str = "EXEC";

pub fn log(tag: &str, message_args: core::fmt::Arguments) {
    crate::kprint!("[{}] ", tag);
    crate::arch::serial::write_fmt(message_args);
    crate::kprint!("\n");
}

#[macro_export]
macro_rules! diag_info {
    ($tag:expr, $($arg:tt)*) => {
        $crate::diag::log($tag, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! diag_warn {
    ($tag:expr, $($arg:tt)*) => {
        $crate::diag::log($tag, format_args!("WARN: {}", format_args!($($arg)*)))
    };
}

#[macro_export]
macro_rules! diag_error {
    ($tag:expr, $($arg:tt)*) => {
        $crate::diag::log($tag, format_args!("ERROR: {}", format_args!($($arg)*)))
    };
}
