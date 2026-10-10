//! The Pointers page's host side: installed XCursor themes and the preview bitmap.

use reprisekit::icontheme::IconTheme;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::theme::Color;

/// The pointers the preview shows, each its XCursor names (the CSS one first, older X11 ones
/// after).
const ROLES: &[&[&str]] = &[
    &["default", "left_ptr", "arrow"],
    &["help", "question_arrow", "whats_this"],
    &["progress", "left_ptr_watch", "half-busy"],
    &["wait", "watch"],
    &["crosshair", "cross", "tcross"],
    &["text", "xterm", "ibeam"],
    &["pointer", "hand2", "pointing_hand"],
    &["not-allowed", "crossed_circle", "forbidden"],
    &["fleur", "all-scroll", "move"],
    &["ns-resize", "sb_v_double_arrow", "v_double_arrow"],
    &["ew-resize", "sb_h_double_arrow", "h_double_arrow"],
    &["nwse-resize", "bd_double_arrow", "size_fdiag"],
    &["nesw-resize", "fd_double_arrow", "size_bdiag"],
];

const PREVIEW_MAX: u32 = 48;

pub fn themes() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for root in IconTheme::default_roots() {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in rd.flatten() {
            if !e.path().join("cursors").is_dir() {
                continue;
            }
            if let Some(name) = e.file_name().to_str()
                && !out.iter().any(|n| n == name)
            {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

fn load(theme: &xcursor::CursorTheme, names: &[&str], size: u32) -> Option<Canvas> {
    let path = names.iter().find_map(|n| theme.load_icon(n))?;
    let bytes = std::fs::read(path).ok()?;
    let images = xcursor::parser::parse_xcursor(&bytes)?;
    let best = images
        .iter()
        .min_by_key(|i| (i.size.abs_diff(size), i.size))?
        .size;
    let img = images.iter().find(|i| i.size == best)?;
    let pixels = img
        .pixels_rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_le_bytes(*p))
        .collect();
    Some(Canvas::from_pixels(img.width, img.height, pixels))
}

/// The preview bitmap, `w` x `h`: `theme`'s pointers at `size` px (0 for 24) in equal cells. An
/// empty `theme` is `default`; a missing pointer leaves its cell blank.
pub fn preview(theme: &str, size: i64, (w, h): (u32, u32), window: Color) -> Canvas {
    let mut c = Canvas::new(w, h);
    c.fill_rect(Rect::new(0, 0, w as i32, h as i32), window);
    let name = if theme.is_empty() { "default" } else { theme };
    let size = match size {
        s if s > 0 => (s as u32).min(PREVIEW_MAX),
        _ => 24,
    };
    let theme = xcursor::CursorTheme::load(name);
    let cell = (size + size / 2) as i32;
    let per_row = (w as i32 / cell).max(1);
    let rows = (ROLES.len() as i32 + per_row - 1) / per_row;
    let x0 = (w as i32 - per_row.min(ROLES.len() as i32) * cell) / 2;
    let y0 = ((h as i32 - rows * cell) / 2).max(0);
    for (i, names) in ROLES.iter().enumerate() {
        let Some(p) = load(&theme, names, size) else {
            continue;
        };
        let (col, row) = (i as i32 % per_row, i as i32 / per_row);
        let x = x0 + col * cell + (cell - p.width as i32) / 2;
        let y = y0 + row * cell + (cell - p.height as i32) / 2;
        c.blit_over(x, y, &p);
    }
    c
}
