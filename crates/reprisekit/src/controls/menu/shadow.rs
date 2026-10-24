use crate::raster::{Canvas, Rect};
use crate::theme::Theme;

const SHADOW_ALPHA: f32 = 0.5;

pub fn draw_shadow(w: i32, h: i32, size: i32) -> Canvas {
    let mut c = Canvas::new((w + size).max(1) as u32, (h + size).max(1) as u32);
    if size <= 0 {
        return c;
    }
    let fade = |past: i32| -> f32 { (size - past) as f32 / size as f32 };
    let mut put = |x: i32, y: i32| {
        let fx = if x >= w { fade(x - w) } else { 1.0 };
        let fy = if y >= h { fade(y - h) } else { 1.0 };
        let a = (SHADOW_ALPHA * fx * fy * 255.0).round() as u32;
        c.put_raw(x, y, a << 24);
    };
    for y in size..h + size {
        for x in w.max(size)..w + size {
            put(x, y);
        }
    }
    for y in h.max(size)..h + size {
        for x in size..w {
            put(x, y);
        }
    }
    c
}

pub fn draw_shadow_above(w: i32, h: i32, size: i32, floor: i32) -> Canvas {
    cut_at(draw_shadow(w, h, size), floor)
}

fn cut_at(full: Canvas, floor: i32) -> Canvas {
    let rows = floor.clamp(1, full.height as i32);
    let mut c = Canvas::new(full.width, rows as u32);
    c.blit_over(0, 0, &full);
    c
}

pub(super) fn menu_look(theme: &Theme) -> Option<crate::style::MenuLook> {
    theme.look.style().and_then(|st| st.menu)
}

/// Whether [`PopupMenu::draw`] casts `theme`'s menu shadow itself (or none, for a menu look
/// without one); a host that draws Classic's ([`draw_shadow`]) under a chain skips it then.
pub fn draws_own_shadow(theme: &Theme) -> bool {
    menu_look(theme).is_some()
}

/// How far right and down of a [`PopupMenu`] level its own shadow reaches, logical px.
pub fn level_shadow(theme: &Theme) -> i32 {
    match menu_look(theme).and_then(|m| m.shadow) {
        Some(_) => crate::style::DropShadow::N as i32,
        None => 0,
    }
}

pub fn shadow_reach(theme: &Theme) -> i32 {
    match menu_look(theme) {
        Some(_) => level_shadow(theme) * theme.scale.max(1),
        None => theme.metrics.menu_shadow,
    }
}

pub fn draw_menu_shadow(theme: &Theme, w: i32, h: i32, floor: i32) -> Canvas {
    let reach = shadow_reach(theme);
    match menu_look(theme) {
        Some(m) => {
            let s = theme.scale.max(1);
            let mut c = Canvas::new((w + reach).max(1) as u32, (h + reach).max(1) as u32);
            crate::look::styled::menu::draw_shadow(&m, &mut c, Rect::new(0, 0, w, h), s);
            cut_at(c, floor)
        }
        None => draw_shadow_above(w, h, reach, floor),
    }
}
