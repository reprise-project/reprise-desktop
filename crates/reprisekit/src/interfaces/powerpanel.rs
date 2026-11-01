//! The Log Off and Turn off computer panels put together: Blue's painter and Blue's push button
//! for Cancel, under whichever interface is on.

use crate::blue::{self, Variant};
use crate::cues::Cues;
use crate::look::{self, Look};
use crate::paint::{Backdrop, Paint};
use crate::pointer::{Lit, lit_of};
use crate::powerpanel::{Layout, Panel, Part};
use crate::raster::Canvas;
use crate::text::TextRenderer;

/// The Blue scheme a panel drawn under `look` takes: the look's own under Blue, navy otherwise.
pub fn power_panel_variant(look: &Look) -> Variant {
    look.style()
        .and_then(|st| Variant::ALL.into_iter().find(|v| v.style_name() == st.name))
        .unwrap_or_default()
}

/// Panel `p` laid out as `lay` onto `c`, a device-px canvas at scale 1 for output scale `s`, in
/// `v`'s colours; `text` is the host's renderer for `s` and `px` its device size. Cancel wears the
/// default ring while it has the focus, and the dotted rectangle too while `cues` shows it.
#[expect(clippy::too_many_arguments)]
pub fn draw_power_panel(
    c: &mut Canvas,
    text: &TextRenderer,
    v: Variant,
    p: &Panel,
    lay: &Layout,
    lit: Lit<Part>,
    cues: Cues,
    px: u32,
    s: i32,
) {
    blue::powerpanel::draw(c, text, v, p, lay, lit, cues, px, s);
    let style = blue::style_for(v).at_scale(s);
    let look = Look::Styled(style.clone());
    let paint = Paint {
        cs: &style.colors,
        look: &look,
        s,
        back: Backdrop::Face,
        active: true,
        text: lay.units,
        access_keys: cues.keys,
    };
    let focused = p.focus == Part::Cancel;
    look::widgets::draw_button(
        c,
        text,
        &paint,
        lay.cancel.scaled(s),
        &tr!("cancel"),
        focused,
        lit_of(lit, Part::Cancel),
        true,
        focused && cues.focus,
        px,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::powerpanel::{TileState, icon};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::pointer::Interaction;
    use crate::powerpanel::{Choice, ICON, layout};
    use crate::theme::{TextUnits, Theme};

    fn fnv(c: &Canvas) -> u64 {
        let mut x = 0xCBF2_9CE4_8422_2325_u64;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                x = (x ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
            }
        };
        eat(c.width);
        eat(c.height);
        for &p in &c.pixels {
            eat(p);
        }
        x
    }

    fn drawn(v: Variant, p: &Panel, lit: Lit<Part>, cues: Cues, s: i32) -> Canvas {
        let theme = Theme::default().scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let lay = layout(p, TextUnits::ONE);
        let mut c = Canvas::new((lay.size.0 * s) as u32, (lay.size.1 * s) as u32);
        draw_power_panel(
            &mut c,
            text.for_output(s),
            v,
            p,
            &lay,
            lit,
            cues,
            theme.fonts.size_px,
            s,
        );
        c
    }

    #[test]
    fn the_panel_takes_blues_scheme_under_blue_and_navy_elsewhere() {
        assert_eq!(power_panel_variant(&Theme::default().look), Variant::Blue);
        assert_eq!(
            power_panel_variant(&theme_for(Interface::Glass).look),
            Variant::Blue
        );
        let silver = crate::interfaces::blue_theme(Variant::Silver);
        assert_eq!(power_panel_variant(&silver.look), Variant::Silver);
    }

    #[test]
    fn the_panels_pin_per_state_scheme_and_scale() {
        if !crate::text::test_tahoma() {
            crate::pins::skip!("no Tahoma for the labels");
        }
        let all = Cues {
            focus: true,
            keys: true,
        };
        // The title's face is whatever the system has; the labels and Cancel are in the pinned
        // Tahoma.
        let untitled = |p: Panel| Panel {
            title: String::new(),
            ..p
        };
        let mut got = Vec::new();
        for v in [Variant::Blue, Variant::Silver] {
            for s in [1, 2] {
                let mut t = untitled(Panel::turn_off(false, true));
                let mut add = |name: &str, c: Canvas| {
                    got.push((format!("{v:?} {s} {name}"), fnv(&c)));
                };
                add("turn off", drawn(v, &t, None, Cues::NONE, s));
                add(
                    "restart hot",
                    drawn(v, &t, Some((Part::Big(2), Interaction::Hot)), Cues::NONE, s),
                );
                add(
                    "turn off pressed",
                    drawn(v, &t, Some((Part::Big(1), Interaction::Pressed)), all, s),
                );
                t.set_shift(true);
                add("hibernate", drawn(v, &t, None, all, s));
                t.focus = Part::Cancel;
                add("cancel focused", drawn(v, &t, None, all, s));
                add(
                    "cancel pressed",
                    drawn(v, &t, Some((Part::Cancel, Interaction::Pressed)), all, s),
                );
                let l = untitled(Panel::log_off(false));
                add("log off", drawn(v, &l, None, Cues::NONE, s));
                add(
                    "log off hot",
                    drawn(v, &l, Some((Part::Big(1), Interaction::Hot)), all, s),
                );
            }
        }
        crate::pins::check("interfaces/power_panel", &got);
    }

    #[test]
    fn the_title_inks_white_in_the_title_band_left_of_the_logo() {
        let p = Panel::turn_off(true, false);
        let lay = layout(&p, TextUnits::ONE);
        let c = drawn(Variant::Blue, &p, None, Cues::NONE, 1);
        let t = lay.title;
        let white = |x0: i32, x1: i32| {
            (t.y..t.bottom())
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) & 0xFF_FFFF == 0xFF_FFFF)
                .count()
        };
        assert!(white(t.x, t.x + 150) > 60, "the title");
        assert!(white(t.right() - 43, t.right() - 11) > 60, "the logo");
    }

    #[test]
    fn the_tiles_sit_in_their_cells_and_a_greyed_one_lets_the_band_through() {
        let p = Panel::turn_off(false, false);
        let lay = layout(&p, TextUnits::ONE);
        let c = drawn(Variant::Blue, &p, None, Cues::NONE, 1);
        let (at, _) = lay.cell_parts(lay.cells[1]);
        let focused = icon(Choice::TurnOff, TileState::Focus, ICON, 1);
        assert_eq!(
            c.get(at.x + 16, at.y + 2),
            focused.get(16, 2),
            "Turn Off has the focus"
        );
        let (dead, _) = lay.cell_parts(lay.cells[0]);
        let band = c.get(dead.x - 4, dead.y + 16);
        let in_tile = c.get(dead.x + 3, dead.y + 26);
        let lum = |p: u32| ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF);
        assert!(
            lum(in_tile).abs_diff(lum(band)) < 120,
            "the greyed Stand By is mostly band"
        );
    }

    #[test]
    #[ignore]
    fn render_power_panels() {
        for s in [1, 2] {
            let mut t = Panel::turn_off(false, true);
            crate::raster::write_preview(
                &drawn(Variant::Blue, &t, None, Cues::NONE, s),
                &format!("power-turn-off@{s}.png"),
            );
            t.set_shift(true);
            crate::raster::write_preview(
                &drawn(Variant::Olive, &t, None, Cues::NONE, s),
                &format!("power-hibernate-olive@{s}.png"),
            );
            crate::raster::write_preview(
                &drawn(Variant::Blue, &Panel::log_off(false), None, Cues::NONE, s),
                &format!("power-log-off@{s}.png"),
            );
        }
    }
}
