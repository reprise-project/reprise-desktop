//! A flyout's panel: the box a tray icon or the clock opens (the hidden icons, month calendar, volume
//! popup, notification history, clipboard history, emoji panel). Classic and Blue draw it on a 2 px
//! raised window edge. A look with a [`FlyoutLook`] draws it as a small window: the window's border all
//! round, [`ControlMetrics::flyout_border`] wide, the contents on the window colour inside it, and
//! [`Part::FlyoutShadow`] round the whole.
//!
//! A host shows the canvas [`shadowed`] returns `margin` up and left of the panel's place, takes the
//! pointer on the panel alone, and puts the blurred backdrop under the border. A flyout with no backdrop
//! under it (a program's own) draws its border opaque ([`Backdrop::None`]).
//!
//! [`ControlMetrics::flyout_border`]: crate::metrics::ControlMetrics::flyout_border

use crate::raster::{Canvas, EdgeStyle, Insets, Rect};
use crate::style::{FlyoutLook, Part, State, Style};
use crate::theme::{Color, Theme};

pub const EDGE: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    Blurred,
    None,
}

/// The style a flyout in `theme` is framed with and how; None under a look that keeps Classic's raised panel.
pub fn look(theme: &Theme) -> Option<(&Style, FlyoutLook)> {
    let st = theme.look.style()?;
    Some((st, st.flyout?))
}

pub fn border(theme: &Theme) -> i32 {
    theme.look.controls().flyout_border
}

/// How far a flyout's canvas reaches past the panel in `theme`'s look, at scale 1: its shadow, or nothing.
pub fn margin(theme: &Theme) -> Insets {
    look(theme).map_or(Insets::ZERO, |(_, f)| f.shadow)
}

/// A window's border all round `all` (device px), `f` device px wide, through `style`'s frame parts at
/// scale `s`, laid over what `c` holds. The top border is the bottom one turned over.
pub fn draw_border(c: &mut Canvas, style: &Style, all: Rect, f: i32, s: i32) {
    let (w, h) = (all.width, all.height);
    let mut b = Canvas::new(w.max(1) as u32, h.max(1) as u32);
    style.draw(
        &mut b,
        Rect::new(0, h - f, w, f),
        Part::FrameBottom,
        State::Normal,
        s,
    );
    let mut top = Canvas::new(w.max(1) as u32, f.max(1) as u32);
    style.draw(
        &mut top,
        Rect::new(0, 0, w, f),
        Part::FrameBottom,
        State::Normal,
        s,
    );
    for y in 0..f {
        for x in 0..w {
            b.put_raw(x, y, top.get(x, f - 1 - y));
        }
    }
    let side = h - 2 * f;
    style.draw(
        &mut b,
        Rect::new(0, f, f, side),
        Part::FrameLeft,
        State::Normal,
        s,
    );
    style.draw(
        &mut b,
        Rect::new(w - f, f, f, side),
        Part::FrameRight,
        State::Normal,
        s,
    );
    c.blit_over(all.x, all.y, &b);
}

/// The panel filling `all` (device px) at `theme`'s scale: `fill` inside Classic's raised edge, or the
/// look's border round the window colour. Returns what the panel leaves inside its edge or border.
pub fn draw_panel(theme: &Theme, c: &mut Canvas, all: Rect, fill: Color, under: Backdrop) -> Rect {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let f = border(theme) * s;
    match look(theme) {
        Some((st, fl)) => {
            let inside = all.inset(f);
            match under {
                Backdrop::Blurred => draw_border(c, st, all, f, s),
                Backdrop::None => {
                    let mut b = Canvas::new(all.width.max(1) as u32, all.height.max(1) as u32);
                    let bounds = b.bounds();
                    draw_border(&mut b, st, bounds, f, s);
                    opaque_over(&mut b, fl.opaque_under);
                    c.blit_over(all.x, all.y, &b);
                }
            }
            c.fill_rect(inside, cs.window);
            inside
        }
        None => {
            c.fill_rect(all, fill);
            c.edge_t(all, EdgeStyle::WindowRaised, cs, s);
            all.inset(EDGE * s)
        }
    }
}

/// Every pixel of `c` the border touches, laid over `under` and made opaque; pixels outside its corners stay clear.
fn opaque_over(c: &mut Canvas, under: Color) {
    let (r, g, b) = (u32::from(under.r), u32::from(under.g), u32::from(under.b));
    for px in c.pixels.iter_mut() {
        let a = *px >> 24;
        if a == 0 {
            continue;
        }
        let ch = |sh: u32, u: u32| (((*px >> sh) & 0xFF) + u * (255 - a) / 255).min(255) << sh;
        *px = 0xFF00_0000 | ch(16, r) | ch(8, g) | ch(0, b);
    }
}

/// The look's shadow round a flyout filling `panel` (device px) at scale `s`, laid over what `c` holds; nothing for a look without one.
pub fn draw_shadow(theme: &Theme, c: &mut Canvas, panel: Rect) {
    let Some((st, fl)) = look(theme) else {
        return;
    };
    let s = theme.scale.max(1);
    let m = fl.shadow.scaled(s);
    let r = Rect::new(
        panel.x - m.left,
        panel.y - m.top,
        panel.width + m.left + m.right,
        panel.height + m.top + m.bottom,
    );
    st.draw(c, r, Part::FlyoutShadow, State::Normal, s);
}

/// `panel`, a flyout drawn at `theme`'s scale, with the look's shadow round it: a canvas [`margin`]
/// bigger on each side, or `panel` itself under a look without one.
pub fn shadowed(theme: &Theme, panel: Canvas) -> Canvas {
    let m = margin(theme);
    if m == Insets::ZERO {
        return panel;
    }
    let s = theme.scale.max(1);
    let m = m.scaled(s);
    let (w, h) = (panel.width as i32, panel.height as i32);
    let mut c = Canvas::new((w + m.left + m.right) as u32, (h + m.top + m.bottom) as u32);
    let at = Rect::new(m.left, m.top, w, h);
    draw_shadow(theme, &mut c, at);
    c.blit_over(at.x, at.y, &panel);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;

    #[test]
    fn classic_and_blue_keep_the_raised_panel_with_no_shadow() {
        for i in [Interface::Classic, Interface::Blue] {
            let theme = theme_for(i);
            assert_eq!(border(&theme), 2, "{i:?}");
            assert_eq!(margin(&theme), Insets::ZERO, "{i:?}");
            let mut c = Canvas::new(40, 30);
            let all = c.bounds();
            let inside = draw_panel(
                &theme,
                &mut c,
                all,
                theme.colors.button_face,
                Backdrop::Blurred,
            );
            assert_eq!(inside, Rect::new(2, 2, 36, 26), "{i:?}");
            assert_eq!(c.get(20, 15), theme.colors.button_face.argb(), "{i:?}");
            let same = shadowed(&theme, c.clone());
            assert_eq!((same.width, same.height), (40, 30), "{i:?}");
        }
    }

    #[test]
    fn glass_frames_a_flyout_as_a_window_round_white() {
        let theme = theme_for(Interface::Glass);
        assert_eq!(border(&theme), 8);
        assert_eq!(margin(&theme), Insets::new(13, 12, 18, 19));
        let mut c = Canvas::new(100, 80);
        let all = c.bounds();
        let inside = draw_panel(
            &theme,
            &mut c,
            all,
            theme.colors.button_face,
            Backdrop::Blurred,
        );
        assert_eq!(inside, Rect::new(8, 8, 84, 64));
        assert_eq!(c.get(50, 40), 0xFFFF_FFFF, "the window colour inside");
        let alpha = |c: &Canvas, x, y| c.get(x, y) >> 24;
        for (x, y) in [(50, 0), (0, 40), (99, 40), (50, 79)] {
            assert!(alpha(&c, x, y) > 0x80, "outline at ({x}, {y})");
        }
        for (x, y) in [(50, 4), (4, 40), (95, 40), (50, 75)] {
            assert!(alpha(&c, x, y) < 0xFF, "glass at ({x}, {y})");
        }
        assert_eq!(alpha(&c, 0, 0), 0, "a rounded corner");
        let mut o = Canvas::new(100, 80);
        let all = o.bounds();
        draw_panel(
            &theme,
            &mut o,
            all,
            theme.colors.button_face,
            Backdrop::None,
        );
        for (x, y) in [(50, 4), (4, 40), (95, 40), (50, 75)] {
            assert_eq!(alpha(&o, x, y), 0xFF, "opaque glass at ({x}, {y})");
        }
        assert_eq!(alpha(&o, 0, 0), 0, "the corner stays clear");
    }

    #[test]
    fn the_glass_shadow_falls_13_left_12_up_18_right_and_19_down() {
        let theme = theme_for(Interface::Glass);
        let mut panel = Canvas::new(100, 80);
        let all = panel.bounds();
        draw_panel(
            &theme,
            &mut panel,
            all,
            theme.colors.button_face,
            Backdrop::Blurred,
        );
        let c = shadowed(&theme, panel);
        assert_eq!((c.width, c.height), (131, 111));
        let alpha = |x, y| c.get(x, y) >> 24;
        for ((x, y), want) in [
            ((13 + 50, 11), 0.45),
            ((13 + 50, 12 + 80), 0.64),
            ((12, 12 + 40), 0.49),
            ((13 + 100, 12 + 40), 0.62),
        ] {
            let got = f64::from(alpha(x, y)) / 255.0;
            assert!(
                (got - want).abs() < 0.01,
                "({x}, {y}): {want}, got {got:.3}"
            );
        }
        assert!(alpha(0, 50) < alpha(12, 50), "fading out to the left");
        assert!(alpha(130, 50) < alpha(113, 50), "and to the right");
        assert_eq!(c.get(13 + 50, 12 + 40), 0xFFFF_FFFF, "the panel on top");
    }
}
