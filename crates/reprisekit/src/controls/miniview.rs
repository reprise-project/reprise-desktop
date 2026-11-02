//! A mini view: the frame and close box round a window's live picture, which the host shows
//! over [`Layout::picture`].

use crate::access::accesskit::{NodeId, Role};
use crate::access::{self, Tree};
use crate::look::{panel, widgets};
use crate::raster::{Canvas, EdgeStyle, Insets, Rect};
use crate::style::{Part as StylePart, State as StyleState};
use crate::theme::{Color, Theme};

/// The frame's width as it opens.
pub const DEFAULT_W: i32 = 320;
pub const MIN_W: i32 = 120;
/// The gap to the usable area's edges as it opens.
pub const EDGE: i32 = 16;
/// The square in the lower-right corner a drag sizes it by.
pub const GRIP: i32 = 16;
/// The close box's gap to the picture's top and right edges.
const CLOSE_INSET: i32 = 4;
/// Behind the picture where its aspect leaves the frame uncovered.
const LETTERBOX: Color = Color::rgb(0, 0, 0);

/// How the interface draws the frame, at scale 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// The frame's width on every side of the picture.
    pub border: i32,
    pub close: (i32, i32),
    /// How far the canvas reaches past the frame: the glass panel's shadow.
    pub margin: Insets,
}

pub fn look(theme: &Theme) -> Look {
    let style = theme.look.style();
    let close = style
        .and_then(|st| st.part(StylePart::CloseButton))
        .map(|img| {
            let f = img.frame(StyleState::Normal);
            (f.width / img.scale, f.height / img.scale)
        })
        .unwrap_or((
            crate::classic::widgets::CLOSE_W,
            crate::classic::widgets::CLOSE_H,
        ));
    let glass = panel::glass(theme).is_some();
    Look {
        border: if glass { 6 } else { 3 },
        close,
        margin: panel::margin(theme),
    }
}

/// Where its parts lie, from the frame's top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: (i32, i32),
    pub picture: Rect,
    pub close: Rect,
    pub grip: Rect,
}

pub fn layout(look: Look, (w, h): (i32, i32)) -> Layout {
    let b = look.border;
    let picture = Rect::new(b, b, (w - 2 * b).max(1), (h - 2 * b).max(1));
    let (cw, ch) = look.close;
    Layout {
        size: (w, h),
        picture,
        close: Rect::new(
            picture.right() - CLOSE_INSET - cw,
            picture.y + CLOSE_INSET,
            cw,
            ch,
        ),
        grip: Rect::new(w - GRIP, h - GRIP, GRIP, GRIP),
    }
}

/// The frame `w` wide whose picture has the aspect of a `aspect` window.
pub fn size_for(look: Look, w: i32, aspect: (i32, i32)) -> (i32, i32) {
    let b = look.border;
    let pw = (w - 2 * b).max(1);
    let (aw, ah) = (aspect.0.max(1), aspect.1.max(1));
    let ph = ((i64::from(pw) * i64::from(ah) + i64::from(aw) / 2) / i64::from(aw)) as i32;
    (w, ph.max(1) + 2 * b)
}

/// A new mini view of `size` in `work`: its lower-right corner [`EDGE`] in from the area's, and
/// stepped up past each of the `taken` rects it would cover.
pub fn place(work: Rect, size: (i32, i32), taken: &[Rect]) -> Rect {
    let x = work.right() - EDGE - size.0;
    let mut r = Rect::new(x, work.bottom() - EDGE - size.1, size.0, size.1);
    while let Some(t) = taken.iter().find(|t| t.intersects(r)) {
        let y = t.y - EDGE - size.1;
        if y < work.y {
            return Rect::new(x, work.bottom() - EDGE - size.1, size.0, size.1);
        }
        r.y = y;
    }
    r
}

/// The frame a drag of the grip by (`dx`, `dy`) gives the one at `start`: its top-left stays,
/// its width follows whichever way the pointer went further, its picture keeps `aspect`, and it
/// stays between [`MIN_W`] and the right edge of `work`.
pub fn resized(
    look: Look,
    start: Rect,
    aspect: (i32, i32),
    (dx, dy): (i32, i32),
    work: Rect,
) -> Rect {
    let by_x = start.width + dx;
    let tall = size_for(look, start.width, aspect).1 - 2 * look.border;
    let wide = start.width - 2 * look.border;
    let tall_n = i64::from(tall.max(1));
    let by_y =
        2 * look.border + ((i64::from(tall + dy) * i64::from(wide) + tall_n / 2) / tall_n) as i32;
    let max = (work.right() - start.x).max(MIN_W);
    let w = by_x.max(by_y).clamp(MIN_W, max);
    let (w, h) = size_for(look, w, aspect);
    Rect::new(start.x, start.y, w, h)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Close,
    Grip,
    Body,
}

/// The part at (`x`, `y`) from the frame's top-left; the close box only while it shows (`hot`).
pub fn hit(l: &Layout, hot: bool, x: i32, y: i32) -> Option<Part> {
    if !Rect::new(0, 0, l.size.0, l.size.1).contains(x, y) {
        return None;
    }
    Some(if hot && l.close.contains(x, y) {
        Part::Close
    } else if l.grip.contains(x, y) {
        Part::Grip
    } else {
        Part::Body
    })
}

/// The frame at the theme's scale: under glass a glass panel with its shadow ([`Look::margin`]
/// past the frame), else Classic's raised window edge in `button_face`; the picture's rect
/// letterbox black.
pub fn draw_frame(theme: &Theme, l: &Layout) -> Canvas {
    let p = theme.paint();
    let s = p.s;
    let (w, h) = l.size;
    let pic = l.picture.scaled(s);
    if let Some((style, spec)) = panel::glass(theme) {
        return panel::draw(style, spec, w, h, s, |c| c.fill_rect(pic, LETTERBOX));
    }
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), p.cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, p.cs, s);
    c.fill_rect(pic, LETTERBOX);
    c
}

/// What shows over the picture while the pointer is on the mini view: the close box, on a
/// clear canvas the frame's size.
pub fn draw_controls(theme: &Theme, l: &Layout) -> Canvas {
    let p = theme.paint();
    let s = p.s;
    let mut c = Canvas::new((l.size.0 * s) as u32, (l.size.1 * s) as u32);
    widgets::draw_close_box(&mut c, &p, l.close.scaled(s));
    c
}

/// Its screen reader nodes under `parent`: a window named "Mini View" and its window's `title`,
/// with its close button while it shows.
pub fn access(l: &Layout, title: &str, hot: bool, tree: &mut Tree, parent: NodeId) {
    let all = Rect::new(0, 0, l.size.0, l.size.1);
    let name = format!("Mini View: {title}");
    let id = tree.push(parent, "mini", access::named(Role::Window, all, &name));
    if hot {
        tree.push(id, "close", access::named(Role::Button, l.close, "Close"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;

    const WORK: Rect = Rect::new(0, 0, 1920, 1050);

    fn classic() -> Look {
        look(&Theme::default())
    }

    #[test]
    fn it_opens_320_wide_in_the_lower_right_with_the_windows_aspect() {
        let size = size_for(classic(), DEFAULT_W, (800, 600));
        assert_eq!(size, (320, 6 + 236), "a 314 px picture at 4:3");
        let r = place(WORK, size, &[]);
        assert_eq!(r, Rect::new(1920 - 16 - 320, 1050 - 16 - 242, 320, 242));
    }

    #[test]
    fn a_second_steps_up_past_the_first_and_a_full_column_starts_over() {
        let size = (320, 200);
        let first = place(WORK, size, &[]);
        let second = place(WORK, size, &[first]);
        assert_eq!(second.y, first.y - 16 - 200);
        let low = Rect::new(0, 0, 1920, 400);
        let a = place(low, size, &[]);
        let b = place(low, size, &[a]);
        assert_eq!(b, a, "no room above: back to the corner");
    }

    #[test]
    fn the_grip_sizes_it_keeping_the_aspect_and_the_top_left() {
        let l = classic();
        let start = Rect::new(100, 100, 320, 242);
        let r = resized(l, start, (800, 600), (80, 0), WORK);
        assert_eq!(r, Rect::new(100, 100, 400, 6 + 296));
        let r = resized(l, start, (800, 600), (0, 75), WORK);
        assert_eq!(r.width, 420, "75 px down at 4:3 is 100 px across");
        let r = resized(l, start, (800, 600), (-500, -500), WORK);
        assert_eq!(r.width, MIN_W);
        let r = resized(l, start, (800, 600), (5000, 0), WORK);
        assert_eq!(r.right(), WORK.right(), "held inside the area");
    }

    #[test]
    fn hits_name_the_close_box_only_while_it_shows_and_the_corner_grip() {
        let l = layout(classic(), (320, 242));
        assert_eq!(l.picture, Rect::new(3, 3, 314, 236));
        assert_eq!(l.close, Rect::new(317 - 4 - 16, 7, 16, 14));
        let (cx, cy) = (l.close.x + 2, l.close.y + 2);
        assert_eq!(hit(&l, true, cx, cy), Some(Part::Close));
        assert_eq!(hit(&l, false, cx, cy), Some(Part::Body));
        assert_eq!(hit(&l, true, 318, 240), Some(Part::Grip));
        assert_eq!(hit(&l, true, 100, 100), Some(Part::Body));
        assert_eq!(hit(&l, true, 320, 0), None);
    }

    #[test]
    fn classic_draws_a_raised_edge_round_a_black_picture() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let l = layout(look(&theme), (200, 150));
        let c = draw_frame(&theme, &l);
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(199, 149), cs.button_dk_shadow.argb());
        assert_eq!(c.get(2, 2), cs.button_face.argb());
        assert_eq!(c.get(3, 3), LETTERBOX.argb());
        let over = draw_controls(&theme, &l);
        assert_eq!(over.get(50, 50) >> 24, 0, "clear over the picture");
        assert_eq!(over.get(l.close.x, l.close.y) >> 24, 0xFF, "the close box");
    }

    #[test]
    fn glass_draws_its_panel_with_the_shadow_past_the_frame() {
        let theme = theme_for(Interface::Glass);
        let lk = look(&theme);
        assert_eq!(lk.border, 6);
        let l = layout(lk, (200, 150));
        let c = draw_frame(&theme, &l);
        let m = lk.margin;
        assert_eq!(
            (c.width as i32, c.height as i32),
            (200 + m.left + m.right, 150 + m.top + m.bottom)
        );
        let edge = c.get(m.left + 2, m.top + 75) >> 24;
        assert!(
            edge > 0 && edge < 0xFF,
            "translucent glass round the picture"
        );
        assert_eq!(c.get(m.left + 100, m.top + 75), LETTERBOX.argb());
    }

    #[test]
    fn access_names_the_window_and_its_close_button_while_it_shows() {
        let l = layout(classic(), (320, 242));
        let mut tree = Tree::window("Mini views", (320, 242));
        access(&l, "top", true, &mut tree, crate::access::ROOT);
        assert!(tree.find(Role::Window, "Mini View: top").is_some());
        assert!(tree.find(Role::Button, "Close").is_some());
        let mut tree = Tree::window("Mini views", (320, 242));
        access(&l, "top", false, &mut tree, crate::access::ROOT);
        assert!(tree.find(Role::Button, "Close").is_none());
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                for b in c.get(x, y).to_le_bytes() {
                    h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
                }
            }
        }
        h
    }

    #[test]
    fn the_frame_and_close_box_pin_in_every_interface_at_scale_1_and_2() {
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = theme_for(i).scaled(scale);
                let l = layout(look(&theme), (240, 180));
                got.push((
                    i,
                    scale,
                    fnv(&draw_frame(&theme, &l)),
                    fnv(&draw_controls(&theme, &l)),
                ));
            }
        }
        assert_eq!(got, PINS, "pins changed; now:\n{got:#x?}");
    }

    const PINS: &[(Interface, i32, u64, u64)] = &[
        (
            Interface::Classic,
            1,
            0x56c2c0843362a7d8,
            0x3fd0ebbd130386c8,
        ),
        (
            Interface::Classic,
            2,
            0xe952af93569430d5,
            0x7d7bc126e3ac63b5,
        ),
        (Interface::Blue, 1, 0x3ce0336a41fae004, 0xc4e33266901af903),
        (Interface::Blue, 2, 0x05483dc8155527c5, 0xdc28cf8e1e6ce6c3),
        (Interface::Glass, 1, 0x449b36160c7624a9, 0xb92c3a6e501cd0ab),
        (Interface::Glass, 2, 0x70120dd545d8b812, 0xd1367e792d27e172),
    ];
}
