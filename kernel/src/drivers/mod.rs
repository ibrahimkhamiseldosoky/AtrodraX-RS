pub mod e1000;
pub mod fb;
pub mod keyboard;
pub mod mouse;
pub mod pci;
pub mod rtc;
pub mod virtio_blk;

pub const FB_BLACK: u32 = 0x000000;
pub const FB_WHITE: u32 = 0xFFFFFF;
pub const FB_RED: u32 = 0xC80000;
pub const FB_GREEN: u32 = 0x00C800;
pub const FB_BLUE: u32 = 0x0000C8;
pub const FB_YELLOW: u32 = 0xFFDC00;
pub const FB_CYAN: u32 = 0x00DCDC;
pub const FB_GRAY: u32 = 0x808080;

#[inline]
pub const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    ((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF)
}

#[inline]
pub fn blend(bottom: u32, top: u32, alpha: u32) -> u32 {
    let a = alpha.min(255);
    let inv = 255 - a;
    let rb = ((top & 0xFF00FF) * a + (bottom & 0xFF00FF) * inv) >> 8;
    let g = ((top & 0x00FF00) * a + (bottom & 0x00FF00) * inv) >> 8;
    (rb & 0xFF00FF) | (g & 0x00FF00)
}
