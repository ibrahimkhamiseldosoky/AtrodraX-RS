//! Theme engine — the same two palettes as the C AtrodraX.

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub wallpaper_top: u32,
    pub wallpaper_bottom: u32,
    pub topbar_bg: u32,
    pub topbar_border: u32,
    pub topbar_accent: u32,
    pub titlebar_focused_top: u32,
    pub titlebar_focused_bottom: u32,
    pub titlebar_focused_glow: u32,
    pub title_text_focused: u32,
    pub titlebar_unfocused: u32,
    pub title_text_unfocused: u32,
    pub border_focused: u32,
    pub border_unfocused: u32,
    pub border_width: i32,
    pub titlebar_h: i32,
    pub content_bg: u32,
    pub close_normal_fill: u32,
    pub close_normal_border: u32,
    pub close_hover_fill: u32,
    pub close_hover_border: u32,
    pub icon_bg: u32,
    pub icon_border: u32,
    pub icon_border_hover: u32,
    pub icon_label_color: u32,
    pub menu_bg: u32,
    pub menu_border: u32,
    pub menu_row_hover: u32,
    pub menu_text: u32,
    pub shadow_alpha_far: u32,
    pub shadow_alpha_mid: u32,
    pub shadow_alpha_near: u32,
    pub anim_open_ms: u32,
    pub anim_close_ms: u32,
    pub caret_blink_ms: u32,
}

pub const THEME_DARK: Theme = Theme {
    name: "Dark",
    wallpaper_top: 0x0A171C,
    wallpaper_bottom: 0x050B0E,
    topbar_bg: 0x0B161B,
    topbar_border: 0x1A4A5A,
    topbar_accent: 0x8EEBFF,
    titlebar_focused_top: 0x122A42,
    titlebar_focused_bottom: 0x081A2A,
    titlebar_focused_glow: 0x00BCD4,
    title_text_focused: 0xDBEAFE,
    titlebar_unfocused: 0x111827,
    title_text_unfocused: 0x4A5568,
    border_focused: 0x00BCD4,
    border_unfocused: 0x1E2D3D,
    border_width: 2,
    titlebar_h: 28,
    content_bg: 0x0C141C,
    close_normal_fill: 0x3D1A1A,
    close_normal_border: 0x7F2020,
    close_hover_fill: 0xE74C3C,
    close_hover_border: 0xFF6B6B,
    icon_bg: 0x101E2E,
    icon_border: 0x1E4560,
    icon_border_hover: 0x00BCD4,
    icon_label_color: 0xCBD5E1,
    menu_bg: 0x0F1F28,
    menu_border: 0x2A6A80,
    menu_row_hover: 0x18313D,
    menu_text: 0x8EEBFF,
    shadow_alpha_far: 28,
    shadow_alpha_mid: 40,
    shadow_alpha_near: 52,
    anim_open_ms: 150,
    anim_close_ms: 100,
    caret_blink_ms: 530,
};

pub const THEME_HACKER: Theme = Theme {
    name: "Hacker",
    wallpaper_top: 0x000000,
    wallpaper_bottom: 0x001100,
    topbar_bg: 0x001A00,
    topbar_border: 0x003300,
    topbar_accent: 0x00FF41,
    titlebar_focused_top: 0x001A00,
    titlebar_focused_bottom: 0x000D00,
    titlebar_focused_glow: 0x00FF41,
    title_text_focused: 0x00FF41,
    titlebar_unfocused: 0x0A0A0A,
    title_text_unfocused: 0x1A4D1A,
    border_focused: 0x00FF41,
    border_unfocused: 0x003300,
    border_width: 2,
    titlebar_h: 28,
    content_bg: 0x000D00,
    close_normal_fill: 0x1A0000,
    close_normal_border: 0x330000,
    close_hover_fill: 0xFF0000,
    close_hover_border: 0xFF4444,
    icon_bg: 0x001100,
    icon_border: 0x003300,
    icon_border_hover: 0x00FF41,
    icon_label_color: 0x00CC33,
    menu_bg: 0x000D00,
    menu_border: 0x00FF41,
    menu_row_hover: 0x003300,
    menu_text: 0x00FF41,
    shadow_alpha_far: 15,
    shadow_alpha_mid: 30,
    shadow_alpha_near: 50,
    anim_open_ms: 80,
    anim_close_ms: 60,
    caret_blink_ms: 400,
};

static mut INDEX: usize = 0;

pub fn current() -> &'static Theme {
    unsafe {
        if INDEX == 0 {
            &THEME_DARK
        } else {
            &THEME_HACKER
        }
    }
}

pub fn set_index(index: usize) {
    unsafe { INDEX = index & 1 };
}

pub fn index() -> usize {
    unsafe { INDEX }
}

pub fn toggle() {
    unsafe { INDEX ^= 1 };
}
