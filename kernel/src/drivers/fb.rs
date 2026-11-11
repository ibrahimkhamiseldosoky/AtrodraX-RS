//! Linear framebuffer driver with a software backbuffer.

use crate::boot;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr;

#[derive(Clone, Copy, Default)]
pub struct Framebuffer {
    pub address: u64,
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: u8,
    pub ready: bool,
}

static mut FB: Framebuffer = Framebuffer {
    address: 0,
    width: 0,
    height: 0,
    pitch: 0,
    bpp: 0,
    ready: false,
};

static mut BACKBUFFER: Option<Vec<u32>> = None;

pub fn init() {
    let info = boot::framebuffer();
    unsafe {
        FB = Framebuffer {
            address: info.address,
            width: info.width as usize,
            height: info.height as usize,
            pitch: info.pitch as usize,
            bpp: info.bpp,
            ready: info.address != 0 && info.width >= 320 && info.height >= 200,
        };
        if FB.ready {
            let pixels = FB.width * FB.height;
            BACKBUFFER = Some(vec![0u32; pixels]);
            clear(0x050B0E);
            present();
        }
    }
    if is_ready() {
        crate::kprintln!(
            "[fb] {}x{}x{} pitch {} @ {:#x}",
            width(),
            height(),
            bpp(),
            pitch(),
            address()
        );
    } else {
        crate::kprintln!("[fb] no usable framebuffer; graphics disabled");
    }
}

pub fn is_ready() -> bool {
    unsafe { FB.ready }
}

pub fn width() -> usize {
    unsafe { FB.width }
}

pub fn height() -> usize {
    unsafe { FB.height }
}

pub fn pitch() -> usize {
    unsafe { FB.pitch }
}

pub fn bpp() -> u8 {
    unsafe { FB.bpp }
}

pub fn address() -> u64 {
    unsafe { FB.address }
}

pub fn backbuffer() -> &'static mut [u32] {
    unsafe {
        BACKBUFFER
            .as_mut()
            .map(|v| v.as_mut_slice())
            .unwrap_or(&mut [])
    }
}

#[inline]
pub fn putpixel(x: i32, y: i32, color: u32) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y, w, h) = (x as usize, y as usize, width(), height());
    if x >= w || y >= h {
        return;
    }
    unsafe {
        if let Some(buffer) = BACKBUFFER.as_mut() {
            buffer[y * w + x] = color & 0xFFFFFF;
        }
    }
}

#[inline]
pub fn blendpixel(x: i32, y: i32, color: u32, alpha: u32) {
    if alpha == 0 {
        return;
    }
    if alpha >= 255 {
        putpixel(x, y, color);
        return;
    }
    if x < 0 || y < 0 || x as usize >= width() || y as usize >= height() {
        return;
    }
    let old = getpixel(x, y);
    putpixel(x, y, super::blend(old, color, alpha));
}

#[inline]
pub fn getpixel(x: i32, y: i32) -> u32 {
    if x < 0 || y < 0 || x as usize >= width() || y as usize >= height() {
        return 0;
    }
    unsafe {
        BACKBUFFER
            .as_ref()
            .map(|v| v[y as usize * width() + x as usize])
            .unwrap_or(0)
    }
}

pub fn clear(color: u32) {
    unsafe {
        if let Some(buffer) = BACKBUFFER.as_mut() {
            for pixel in buffer.iter_mut() {
                *pixel = color & 0xFFFFFF;
            }
        }
    }
}

pub fn fill_rect(x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = ((x + w) as usize).min(width());
    let y1 = ((y + h) as usize).min(height());
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let color = color & 0xFFFFFF;
    unsafe {
        if let Some(buffer) = BACKBUFFER.as_mut() {
            let stride = width();
            for row in y0..y1 {
                let start = row * stride + x0;
                let end = row * stride + x1;
                for pixel in &mut buffer[start..end] {
                    *pixel = color;
                }
            }
        }
    }
}

pub fn blend_rect(x: i32, y: i32, w: i32, h: i32, color: u32, alpha: u32) {
    if alpha == 0 || w <= 0 || h <= 0 {
        return;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(width() as i32);
    let y1 = (y + h).min(height() as i32);
    unsafe {
        if let Some(buffer) = BACKBUFFER.as_mut() {
            let stride = width();
            for row in y0..y1 {
                for col in x0..x1 {
                    let idx = row as usize * stride + col as usize;
                    buffer[idx] = super::blend(buffer[idx], color, alpha);
                }
            }
        }
    }
}

pub fn rect_outline(x: i32, y: i32, w: i32, h: i32, color: u32) {
    fill_rect(x, y, w, 1, color);
    fill_rect(x, y + h - 1, w, 1, color);
    fill_rect(x, y, 1, h, color);
    fill_rect(x + w - 1, y, 1, h, color);
}

pub fn present() {
    if !is_ready() {
        return;
    }
    unsafe {
        let fb = &FB;
        let Some(buffer) = BACKBUFFER.as_ref() else {
            return;
        };
        if fb.bpp == 32 {
            if fb.pitch == fb.width * 4 {
                ptr::copy_nonoverlapping(
                    buffer.as_ptr() as *const u8,
                    fb.address as *mut u8,
                    fb.width * fb.height * 4,
                );
            } else {
                for y in 0..fb.height {
                    ptr::copy_nonoverlapping(
                        buffer.as_ptr().add(y * fb.width) as *const u8,
                        (fb.address as *mut u8).add(y * fb.pitch),
                        fb.width * 4,
                    );
                }
            }
        } else if fb.bpp == 24 {
            for y in 0..fb.height {
                let dst = (fb.address as *mut u8).add(y * fb.pitch);
                for x in 0..fb.width {
                    let c = buffer[y * fb.width + x];
                    *dst.add(x * 3 + 0) = (c & 0xFF) as u8;
                    *dst.add(x * 3 + 1) = ((c >> 8) & 0xFF) as u8;
                    *dst.add(x * 3 + 2) = ((c >> 16) & 0xFF) as u8;
                }
            }
        }
    }
}

pub fn scroll_up(pixels: i32) {
    if pixels <= 0 {
        return;
    }
    unsafe {
        let w = width();
        let h = height();
        if let Some(buffer) = BACKBUFFER.as_mut() {
            for y in 0..h.saturating_sub(pixels as usize) {
                for x in 0..w {
                    buffer[y * w + x] = buffer[(y + pixels as usize) * w + x];
                }
            }
            for y in h.saturating_sub(pixels as usize)..h {
                for x in 0..w {
                    buffer[y * w + x] = 0;
                }
            }
        }
    }
}
