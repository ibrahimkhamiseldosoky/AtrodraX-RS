//! PSF2-derived 8x16 bitmap font and drawing helpers.
//!
//! The embedded PSF2 data stores each 8-pixel row LSB-first (bit 0 = leftmost
//! pixel), matching the original C kernel's `fb_psf2_render_glyph_to_buf`;
//! reading it MSB-first mirrors every glyph horizontally.

pub const CHAR_W: usize = 8;
pub const CHAR_H: usize = 16;
pub const GLYPH_BYTES: usize = 16;
pub const FONT_DATA: &[u8; 4096] = include_bytes!("../../assets/font8x16.bin");

pub fn init() {
    crate::kprintln!("[font] 8x16 PSF2 font loaded (256 glyphs)");
}

#[inline]
pub fn glyph(ch: u8) -> &'static [u8; 16] {
    let index = ch as usize;
    let offset = index * GLYPH_BYTES;
    unsafe { &*(FONT_DATA.as_ptr().add(offset) as *const [u8; 16]) }
}

pub fn text_width(text: &str) -> i32 {
    text.chars().count() as i32 * CHAR_W as i32
}

pub fn draw_char(
    buf: &mut [u32],
    stride: usize,
    x: i32,
    y: i32,
    ch: u8,
    fg: u32,
    bg: u32,
    transparent: bool,
) {
    let rows = glyph(ch);
    for (row, bits) in rows.iter().enumerate() {
        let py = y + row as i32;
        if py < 0 {
            continue;
        }
        for col in 0..8 {
            let px = x + col as i32;
            if px < 0 {
                continue;
            }
            let set = bits & (1 << col) != 0;
            if !set && transparent {
                continue;
            }
            let index = py as usize * stride + px as usize;
            if index < buf.len() {
                buf[index] = if set { fg } else { bg } & 0xFFFFFF;
            }
        }
    }
}

pub fn draw_text(
    buf: &mut [u32],
    stride: usize,
    x: i32,
    y: i32,
    text: &str,
    fg: u32,
    bg: u32,
    transparent: bool,
) {
    let mut cx = x;
    for ch in text.bytes() {
        draw_char(buf, stride, cx, y, ch, fg, bg, transparent);
        cx += CHAR_W as i32;
    }
}

pub fn draw_text_clipped(
    buf: &mut [u32],
    stride: usize,
    buf_w: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    text: &str,
    fg: u32,
    bg: u32,
    transparent: bool,
) {
    if y < 0 || y as usize + CHAR_H > buf_h {
        return;
    }
    let mut cx = x;
    for ch in text.bytes() {
        if cx >= 0 && (cx as usize) < buf_w {
            let rows = glyph(ch);
            for (row, bits) in rows.iter().enumerate() {
                let py = y as usize + row;
                for col in 0..8 {
                    let px = cx as usize + col;
                    if px >= buf_w {
                        continue;
                    }
                    let set = bits & (1 << col) != 0;
                    if !set && transparent {
                        continue;
                    }
                    let index = py * stride + px;
                    if index < buf.len() {
                        buf[index] = if set { fg } else { bg } & 0xFFFFFF;
                    }
                }
            }
        }
        cx += CHAR_W as i32;
    }
}
