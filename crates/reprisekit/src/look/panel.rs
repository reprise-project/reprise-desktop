//! A panel floating over the desktop (the media keys' panel, the launcher)
//! under a look with glass: the style's [`Part::PreviewFrame`] at the panel's
//! size with its shadow, and white text over it. The host puts the blurred
//! backdrop under the panel's rect, corners cut by [`PanelSpec::corner`].

use crate::raster::{Canvas, Insets};
use crate::style::{GlowMask, PanelSpec, Part, State, Style, TitleGlow};
use crate::theme::{Color, Theme};

pub const INK: Color = Color::rgb(0xFF, 0xFF, 0xFF);
/// Ink for text that cannot be used, on glass.
pub const DIM_INK: Color = Color::rgb(0xC0, 0xC0, 0xC0);

/// The style a panel in `theme` draws its glass with, and the panel's spec; None without glass.
pub fn glass(theme: &Theme) -> Option<(&Style, PanelSpec)> {
    let style = theme.look.style()?;
    Some((style, style.glass?.panel))
}

/// How far a panel's canvas reaches past the panel at scale 1: the shadow under glass, else nothing.
pub fn margin(theme: &Theme) -> Insets {
    glass(theme).map_or(Insets::ZERO, |(_, p)| p.shadow)
}

/// A `w`x`h` panel at scale `s` (logical): glass and shadow, with `body` drawn over the glass
/// in panel coordinates on a clear canvas. The canvas reaches `spec.shadow` past the panel on each side.
pub fn draw(
    style: &Style,
    spec: PanelSpec,
    w: i32,
    h: i32,
    s: i32,
    body: impl FnOnce(&mut Canvas),
) -> Canvas {
    paint(style, spec, (w, h), s, Part::PreviewFrame, false, body)
}

/// [`draw`] for a panel that is all text and must read over any backdrop: denser glass
/// ([`Part::InkPanel`]) and the style's [`Style::panel_halo`] under everything `body` draws.
pub fn draw_inked(
    style: &Style,
    spec: PanelSpec,
    w: i32,
    h: i32,
    s: i32,
    body: impl FnOnce(&mut Canvas),
) -> Canvas {
    paint(style, spec, (w, h), s, Part::InkPanel, true, body)
}

fn paint(
    style: &Style,
    spec: PanelSpec,
    (w, h): (i32, i32),
    s: i32,
    part: Part,
    halo: bool,
    body: impl FnOnce(&mut Canvas),
) -> Canvas {
    let m = spec.shadow;
    let mut c = Canvas::new(
        ((w + m.left + m.right) * s) as u32,
        ((h + m.top + m.bottom) * s) as u32,
    );
    let all = c.bounds();
    style.draw(&mut c, all, part, State::Normal, s);
    let mut inner = Canvas::new((w * s) as u32, (h * s) as u32);
    body(&mut inner);
    if let Some(halo) = style.panel_halo.filter(|_| halo) {
        inner = haloed(&inner, halo, s);
    }
    c.blit_over(m.left * s, m.top * s, &inner);
    c
}

/// `ink` over a soft dark halo following its coverage, so white ink reads on a pale backdrop.
fn haloed(ink: &Canvas, halo: TitleGlow, s: i32) -> Canvas {
    let (w, h) = (ink.width as usize, ink.height as usize);
    let coverage: Vec<u8> = ink.pixels.iter().map(|&px| (px >> 24) as u8).collect();
    let alpha = (halo.alpha)(&GlowMask {
        coverage: &coverage,
        width: w,
        height: h,
        rows: (0, h),
        active: true,
        scale: s,
    });
    let mut c = Canvas::new(ink.width, ink.height);
    let all = c.bounds();
    c.blit_mask_over(0, 0, w, h, &alpha, halo.color, all);
    c.blit_over(0, 0, ink);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::raster::Rect;

    #[test]
    fn under_glass_the_panel_is_translucent_inside_its_shadow_and_body_ink_lands_on_it() {
        let theme = theme_for(Interface::Glass);
        let (style, spec) = glass(&theme).expect("Glass has a panel");
        assert_eq!(margin(&theme), Insets::new(12, 11, 11, 8));
        let red = Color::rgb(0xFF, 0, 0);
        let c = draw(style, spec, 100, 60, 1, |c| {
            c.fill_rect(Rect::new(40, 20, 4, 4), red)
        });
        assert_eq!((c.width, c.height), (123, 79), "100x60 and the shadow");
        let a = |x: i32, y: i32| c.get(x, y) >> 24;
        assert!(a(12 + 50, 11 + 50) > 0, "glass at the panel's middle");
        assert!(a(12 + 50, 11 + 50) < 0xFF, "and it is translucent");
        assert_eq!(a(0, 0), 0, "nothing past the shadow's corner");
        assert_eq!(c.get(12 + 41, 11 + 21), red.argb(), "the body at (41, 21)");
    }

    fn glass_theme(mode: crate::glass::Mode) -> Theme {
        let g = crate::glass::Settings {
            mode,
            ..Default::default()
        };
        crate::interfaces::theme_with(Interface::Glass, Default::default(), &g)
    }

    /// `px` (premultiplied) laid over opaque white, as its grey level.
    fn over_white(px: u32) -> u32 {
        let a = px >> 24;
        let g = (px >> 8) & 0xFF;
        g + (255 - a)
    }

    #[test]
    fn an_inked_panel_darkens_a_white_backdrop_round_its_ink_in_both_modes() {
        for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
            for s in [1, 2] {
                let theme = glass_theme(mode).scaled(s);
                let (style, spec) = glass(&theme).unwrap();
                let ink =
                    |c: &mut Canvas| c.fill_rect(Rect::new(40 * s, 20 * s, 4 * s, 4 * s), INK);
                let plain = draw(style, spec, 100, 60, s, ink);
                let inked = draw_inked(style, spec, 100, 60, s, ink);
                let (l, t) = (12 * s, 11 * s);
                let at = |c: &Canvas, x: i32, y: i32| over_white(c.get(l + x * s, t + y * s));
                let what = format!("{mode:?}@{s}");
                assert!(
                    at(&inked, 80, 45) + 40 < at(&plain, 80, 45),
                    "{what}: denser glass away from the ink, {} against {}",
                    at(&inked, 80, 45),
                    at(&plain, 80, 45)
                );
                assert!(
                    at(&inked, 45, 21) + 40 < at(&inked, 80, 45),
                    "{what}: a halo beside the ink, {} against {}",
                    at(&inked, 45, 21),
                    at(&inked, 80, 45)
                );
                assert_eq!(
                    inked.get(l + 41 * s, t + 21 * s),
                    INK.argb(),
                    "{what}: the ink"
                );
            }
        }
    }

    #[test]
    fn classic_and_blue_have_no_glass_panel() {
        for i in [Interface::Classic, Interface::Blue] {
            let theme = theme_for(i);
            assert!(glass(&theme).is_none(), "{i:?}");
            assert_eq!(margin(&theme), Insets::ZERO, "{i:?}");
        }
    }
}
