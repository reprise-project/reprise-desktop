//! The Color button: the colour in a black box with a drop arrow, opening a palette popup of 20 swatches. Every function takes the button's logical rect; the draws scale it by `s`.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets;
use crate::look::Paint;
use crate::look::widgets::{draw_button, draw_button_face};
use crate::pointer::Interaction;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::TextRenderer;
use crate::theme::Color;
use accesskit::{NodeId, Role};

/// The popup: 20 swatches in 4 columns, an etched line, a greyed Other... and the current colour. Positions are from the button's top-left.
const POPUP: (i32, i32, i32, i32) = (0, 21, 98, 151);
const SWATCHES: usize = 20;
const COLS: usize = 4;
const SWATCH_ORIGIN: (i32, i32) = (5, 26);
const SWATCH_PITCH: i32 = 23;
const SWATCH: i32 = 19;
const POPUP_LINE: (i32, i32, i32) = (4, 92, 141);
const POPUP_OTHER: (i32, i32, i32, i32) = (4, 145, 67, 23);
const POPUP_CURRENT: (i32, i32, i32, i32) = (72, 145, 23, 23);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorButton {
    pub color: Color,
    pub palette: Vec<Color>,
    pub open: bool,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupHit {
    Swatch(usize),
    Face,
}

impl ColorButton {
    pub fn new(color: Color, palette: Vec<Color>) -> ColorButton {
        ColorButton {
            color,
            palette,
            open: false,
            enabled: true,
        }
    }

    /// A click on the button: the popup opens or closes. False when disabled.
    pub fn toggle(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        self.open = !self.open;
        true
    }

    pub fn pick(&mut self, k: usize) -> bool {
        self.open = false;
        match self.palette.get(k) {
            Some(&c) if c != self.color => {
                self.color = c;
                true
            }
            _ => false,
        }
    }
}

fn swatch_at(button: Rect, k: usize) -> (i32, i32) {
    let (c, r) = ((k % COLS) as i32, (k / COLS) as i32);
    (
        button.x + SWATCH_ORIGIN.0 + c * SWATCH_PITCH,
        button.y + SWATCH_ORIGIN.1 + r * SWATCH_PITCH,
    )
}

/// What (`x`, `y`) hits in the popup under the button at `button`, or `None` outside it. The caller checks the popup is open.
pub fn hit_popup(button: Rect, x: i32, y: i32) -> Option<PopupHit> {
    let popup = Rect::new(button.x + POPUP.0, button.y + POPUP.1, POPUP.2, POPUP.3);
    if !popup.contains(x, y) {
        return None;
    }
    for k in 0..SWATCHES {
        let (sx, sy) = swatch_at(button, k);
        if Rect::new(sx, sy, SWATCH, SWATCH).contains(x, y) {
            return Some(PopupHit::Swatch(k));
        }
    }
    Some(PopupHit::Face)
}

/// Where a face's content sits, from the button's top-left: the colour box, the etched line's x and rows, the drop arrow.
struct Face {
    swatch: (i32, i32, i32, i32),
    line: (i32, i32, i32),
    arrow: (i32, i32),
}

const CLASSIC_FACE: Face = Face {
    swatch: (5, 4, 55, 13),
    line: (63, 4, 17),
    arrow: (67, 9),
};

/// A styled look's face, its push button over the whole button: the box and line sit a pixel further in and the box is 11 px tall.
const PUSH_FACE: Face = Face {
    swatch: (6, 5, 53, 11),
    line: (62, 5, 16),
    arrow: (65, 9),
};

/// The colour in a black box, an etched line, the drop arrow: on a raised window edge in Classic, on the push button face in a styled look.
pub fn draw(c: &mut Canvas, p: &Paint, button: Rect, b: &ColorButton) {
    let Paint { cs, s, .. } = *p;
    let (color, enabled) = (b.color, b.enabled);
    let r = button.scaled(s);
    let face = if p.look.style().is_some() {
        draw_button_face(c, p, r, enabled, Interaction::Idle);
        &PUSH_FACE
    } else {
        c.fill_rect(r, cs.button_face);
        c.edge_t(r, EdgeStyle::WindowRaised, cs, s);
        &CLASSIC_FACE
    };
    let (x, y, w, h) = face.swatch;
    let swatch = Rect::new(r.x + x * s, r.y + y * s, w * s, h * s);
    c.ring_t(swatch, cs.button_text, cs.button_text, s);
    c.fill_rect(swatch.inset(s), color);
    let (lx, y0, y1) = face.line;
    c.etched_vline_t(r.x + lx * s, r.y + y0 * s, r.y + y1 * s, cs, s);
    let ink = if enabled {
        cs.button_text
    } else {
        cs.button_shadow
    };
    let (ax, ay) = face.arrow;
    widgets::draw_small_arrow(c, r.x + ax * s, r.y + ay * s, s, ink);
}

/// The open popup. Under a look with its own popup menus ([`crate::style::MenuLook`]) it is framed and shadowed as a menu with flat swatches; otherwise a raised panel of sunken swatches.
pub fn draw_popup(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    button: Rect,
    b: &ColorButton,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let (current, palette) = (b.color, b.palette.as_slice());
    let popup = Rect::new(button.x + POPUP.0, button.y + POPUP.1, POPUP.2, POPUP.3).scaled(s);
    let menu = p.look.style().and_then(|st| st.menu);
    match &menu {
        Some(m) => {
            crate::look::styled::menu::draw_shadow(m, c, popup, s);
            c.fill_rect(popup, cs.menu);
            c.ring_t(popup, m.frame, m.frame, s);
            for i in 1..3 {
                let r = popup.inset(i * s);
                c.ring_t(r, m.ring, m.ring, s);
            }
        }
        None => {
            c.fill_rect(popup, cs.button_face);
            c.edge_t(popup, EdgeStyle::WindowRaised, cs, s);
        }
    }
    for (k, col) in palette.iter().enumerate().take(SWATCHES) {
        let (x, y) = swatch_at(button, k);
        let r = Rect::new(x, y, SWATCH, SWATCH).scaled(s);
        c.fill_rect(r, *col);
        if menu.is_some() {
            c.ring_t(r, cs.button_shadow, cs.button_shadow, s);
        } else {
            c.edge_t(r, EdgeStyle::ButtonSunken, cs, s);
        }
    }
    let (x0, x1, y) = (
        (button.x + POPUP_LINE.0) * s,
        (button.x + POPUP_LINE.1) * s,
        (button.y + POPUP_LINE.2) * s,
    );
    match &menu {
        Some(m) => {
            c.fill_rect(Rect::new(x0, y, x1 - x0, s), m.rule.0);
            c.fill_rect(Rect::new(x0, y + s, x1 - x0, s), m.rule.1);
        }
        None => c.etched_hline_t(x0, x1, y, cs, s),
    }
    let other = Rect::new(
        button.x + POPUP_OTHER.0,
        button.y + POPUP_OTHER.1,
        POPUP_OTHER.2,
        POPUP_OTHER.3,
    )
    .scaled(s);
    draw_button(
        c,
        text,
        p,
        other,
        "&Other...",
        false,
        Interaction::Idle,
        false,
        false,
        px,
    );
    let cur = Rect::new(
        button.x + POPUP_CURRENT.0,
        button.y + POPUP_CURRENT.1,
        POPUP_CURRENT.2,
        POPUP_CURRENT.3,
    )
    .scaled(s);
    if menu.is_some() {
        c.fill_rect(cur, current);
        c.ring_t(cur, cs.button_shadow, cs.button_shadow, s);
        return;
    }
    c.ring_t(cur, cs.button_text, cs.button_text, s);
    c.ring_t(cur.inset(s), cs.button_hilight, cs.button_hilight, s);
    c.ring_t(cur.inset(2 * s), cs.button_text, cs.button_text, s);
    c.fill_rect(cur.inset(3 * s), current);
}

/// A colour well valued by its colour as `#rrggbb`, expanded while the palette is open.
pub fn access(
    b: &ColorButton,
    button: Rect,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::ColorWell, button);
    let c = b.color;
    n.set_value(format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b));
    n.set_expanded(b.open);
    access::focusable(&mut n, b.enabled);
    tree.push(parent, key, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;

    #[test]
    fn a_click_opens_and_a_swatch_picks() {
        let red = Color::rgb(255, 0, 0);
        let blue = Color::rgb(0, 0, 255);
        let mut b = ColorButton::new(red, vec![red, blue]);
        assert!(b.toggle() && b.open);
        let r = Rect::new(100, 100, 75, 21);
        let (sx, sy) = swatch_at(r, 1);
        assert_eq!(hit_popup(r, sx + 1, sy + 1), Some(PopupHit::Swatch(1)));
        assert_eq!(
            hit_popup(r, r.x + 1, r.y + POPUP.1 + 1),
            Some(PopupHit::Face)
        );
        assert_eq!(
            hit_popup(r, r.x + 1, r.y + 1),
            None,
            "the button is not the popup"
        );
        assert!(b.pick(1) && b.color == blue && !b.open);
        assert!(!b.pick(1), "the same colour again is no change");
        b.enabled = false;
        assert!(!b.toggle() && !b.open);
    }

    #[test]
    fn a_blue_colour_button_is_a_push_button_with_its_box_line_and_arrow() {
        use crate::interface::Interface;
        use crate::interfaces::theme_for;
        let theme = theme_for(Interface::Blue);
        let fill = Color::rgb(0x00, 0x4E, 0x98);
        let b = ColorButton::new(fill, vec![fill]);
        let (x0, y0) = (3, 3);
        let mut c = Canvas::new(81, 27);
        c.fill_rect(c.bounds(), Color::rgb(0xF4, 0xF3, 0xEE));
        draw(&mut c, &theme.paint(), Rect::new(x0, y0, 75, 21), &b);
        let at = |x: i32, y: i32| c.get(x0 + x, y0 + y);
        let border = crate::blue::Variant::Blue
            .scheme()
            .push_button
            .normal
            .rim
            .argb();
        let black = 0xFF00_0000;
        for (x, y) in [(5, 1), (40, 1), (40, 19), (1, 10), (73, 10)] {
            assert_eq!(at(x, y), border, "the push button's border at ({x}, {y})");
        }
        for (x, y) in [(6, 5), (58, 5), (6, 15), (58, 15), (30, 5), (30, 15)] {
            assert_eq!(at(x, y), black, "the box's ring at ({x}, {y})");
        }
        assert_ne!(at(5, 10), black, "the box starts at x 6");
        assert_ne!(at(59, 10), black, "the box ends at x 58");
        assert_ne!(at(30, 4), black, "the box starts at row 5");
        assert_ne!(at(30, 16), black, "the box ends at row 15");
        assert_eq!(at(7, 6), fill.argb(), "the colour inside the ring");
        assert_eq!(at(57, 14), fill.argb(), "the colour inside the ring");
        let shadow = theme.colors.button_shadow.argb();
        for y in 5..16 {
            assert_eq!(at(62, y), shadow, "the line's shadow at row {y}");
            assert_eq!(at(63, y), 0xFFFF_FFFF, "the line's hilight at row {y}");
        }
        assert_ne!(at(62, 4), shadow, "the line starts at row 5");
        assert_ne!(at(62, 16), shadow, "the line ends at row 15");
        for (row, x0a, w) in [(9, 65, 5), (10, 66, 3), (11, 67, 1)] {
            for x in x0a - 1..x0a + w + 1 {
                let inside = (x0a..x0a + w).contains(&x);
                assert_eq!(at(x, row) == black, inside, "arrow at ({x}, {row})");
            }
        }
    }

    #[test]
    fn a_greyed_blue_colour_button_draws_the_greyed_push_button() {
        use crate::interface::Interface;
        use crate::interfaces::theme_for;
        let theme = theme_for(Interface::Blue);
        let mut b = ColorButton::new(Color::rgb(0, 0, 0), vec![]);
        b.enabled = false;
        let mut c = Canvas::new(81, 27);
        draw(&mut c, &theme.paint(), Rect::new(3, 3, 75, 21), &b);
        let greyed = crate::blue::Variant::Blue.scheme().push_button.disabled.rim;
        assert_eq!(c.get(3 + 40, 3 + 1), greyed.argb(), "the greyed border");
        assert_eq!(
            c.get(3 + 67, 3 + 9),
            theme.colors.button_shadow.argb(),
            "the arrow greys"
        );
    }

    /// `draw` takes the logical rect, as `hit_popup` does: at scale 2 the colour box's inside starts at twice the logical (6, 5).
    #[test]
    fn draw_scales_the_logical_rect() {
        let theme = crate::theme::Theme::default();
        let red = Color::rgb(255, 0, 0);
        let b = ColorButton::new(red, vec![red]);
        let mut c = Canvas::new(200, 80);
        draw(
            &mut c,
            &classic_paint(&theme.colors, 2),
            Rect::new(10, 10, 75, 21),
            &b,
        );
        assert_eq!(c.get(32, 30), red.argb(), "colour at device (32, 30)");
        assert_ne!(
            c.get(31, 29),
            red.argb(),
            "the box's ring at device (31, 29)"
        );
    }
}
