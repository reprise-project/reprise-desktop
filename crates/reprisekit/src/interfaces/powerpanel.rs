//! The Log Off and Turn off computer panels put together: Blue's painter, with Cancel, Restart
//! Into... and the Install updates check box drawn as Blue's or the active look's.

use crate::blue::{self, Tone};
use crate::cues::Cues;
use crate::look::{self, Look};
use crate::paint::{Backdrop, Paint};
use crate::pointer::{Interaction, Lit, lit_of};
use crate::powerpanel::{Layout, Panel, Part, restart_into_label, updates_label};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

/// The check box label's ink on the panel's dark bottom band.
const ON_BAND: Color = Color::rgb(0xFF, 0xFF, 0xFF);

/// The Blue scheme a panel drawn under `look` takes: the look's own under Blue, navy otherwise.
pub fn power_panel_variant(look: &Look) -> Tone {
    Tone::of(look)
}

/// Panel `p` laid out as `lay` onto `c`, a device-px canvas at scale 1 for output scale `s`, in
/// `v`'s colours; `text` is the host's renderer for `s` and `px` its device size. Cancel and Restart
/// Into... are push buttons of `cancel`'s look, Blue's in `v` when it is `None`. Each wears the
/// default ring while it has the focus, and the dotted rectangle too while `cues` shows it. The
/// Install updates check box is in the same look.
#[expect(clippy::too_many_arguments)]
pub fn draw_power_panel(
    c: &mut Canvas,
    text: &TextRenderer,
    v: Tone,
    cancel: Option<&Paint>,
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
    let blue = Paint {
        cs: &style.colors,
        look: &look,
        s,
        back: Backdrop::Face,
        active: true,
        text: lay.units,
        access_keys: cues.keys,
        maximized: false,
    };
    let paint = cancel.map_or(blue, |p| Paint {
        s,
        back: Backdrop::Face,
        active: true,
        text: lay.units,
        access_keys: cues.keys,
        ..*p
    });
    let mut plain = vec![(Part::Cancel, lay.cancel, tr!("cancel"))];
    if let Some(r) = lay.restart_into {
        plain.push((Part::RestartInto, r, restart_into_label()));
    }
    for (part, r, label) in plain {
        let focused = p.focus == part;
        look::widgets::draw_button(
            c,
            text,
            &paint,
            r.scaled(s),
            &label,
            focused,
            lit_of(lit, part),
            p.live(part),
            focused && cues.focus,
            px,
        );
    }
    if let (Some(b), Some(row)) = (p.updates, lay.updates) {
        let focus = p.focus == Part::Updates && cues.focus;
        let ptr = lit_of(lit, Part::Updates);
        draw_updates_box(c, text, &paint, row, b.checked, ptr, focus, ON_BAND, px);
    }
}

/// The Install updates check box across `row`, in logical px, onto `c` at `p.s`: the look's box,
/// the label in `ink` with its access key underlined while `p.access_keys`, and the dotted
/// rectangle round the label while `focus`. The framed Turn off computer dialog draws it too.
#[expect(clippy::too_many_arguments)]
pub fn draw_updates_box(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    row: Rect,
    checked: bool,
    ptr: Interaction,
    focus: bool,
    ink: Color,
    px: u32,
) {
    let s = p.s;
    let (b, label) = Layout::check_parts(row, p.text);
    look::widgets::draw_checkbox(c, p, b.scaled(s), checked, ptr);
    let words = updates_label();
    let label = label.scaled(s);
    look::widgets::draw_label(c, text, p, label, &words, ink, Align::Left, px);
    if focus {
        let w = text.measure(&crate::text::split_mnemonic(&words).0, false, px);
        let h = px as i32 + 3 * s;
        let ring = Rect::new(
            label.x - s,
            label.y + (label.height - h) / 2,
            (w + 2 * s).min(label.width + s),
            h,
        );
        look::widgets::draw_focus_ring(c, p, ring, ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::Variant;
    use crate::blue::powerpanel::{TileState, icon};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::pointer::Interaction;
    use crate::powerpanel::{Choice, First, ICON, Leave, layout};
    use crate::theme::{TextUnits, Theme};

    use crate::pins::fnv;

    fn drawn(v: Variant, p: &Panel, lit: Lit<Part>, cues: Cues, s: i32) -> Canvas {
        drawn_with(v, None, p, lit, cues, s)
    }

    fn drawn_with(
        v: Variant,
        cancel: Option<&Theme>,
        p: &Panel,
        lit: Lit<Part>,
        cues: Cues,
        s: i32,
    ) -> Canvas {
        let theme = Theme::default().scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let lay = layout(p, TextUnits::ONE);
        let mut c = Canvas::new((lay.size.0 * s) as u32, (lay.size.1 * s) as u32);
        let paint = cancel.map(|t| t.paint());
        draw_power_panel(
            &mut c,
            text.for_output(s),
            v.into(),
            paint.as_ref(),
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
        assert_eq!(
            power_panel_variant(&Theme::default().look),
            Variant::Blue.into()
        );
        assert_eq!(
            power_panel_variant(&theme_for(Interface::Glass).look),
            Variant::Blue.into()
        );
        let silver = crate::interfaces::blue_theme(Variant::Silver);
        assert_eq!(power_panel_variant(&silver.look), Variant::Silver.into());
    }

    #[test]
    fn the_panels_pin_per_state_scheme_and_scale() {
        crate::pins::fonts();
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
                let mut t = untitled(Panel::turn_off(First::StandBy {
                    live: false,
                    hibernate: true,
                }));
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
                let l = untitled(Panel::log_off(Leave::SwitchUser { live: false }));
                add("log off", drawn(v, &l, None, Cues::NONE, s));
                add(
                    "log off hot",
                    drawn(v, &l, Some((Part::Big(1), Interaction::Hot)), all, s),
                );
                let d = untitled(Panel::log_off(Leave::Disconnect));
                add("disconnect log off", drawn(v, &d, None, all, s));
                let d = untitled(Panel::turn_off(First::Disconnect));
                add("disconnect turn off", drawn(v, &d, None, Cues::NONE, s));
                let mut r = untitled(Panel::turn_off(First::StandBy {
                    live: true,
                    hibernate: false,
                }));
                r.restart_into = true;
                add("restart into", drawn(v, &r, None, Cues::NONE, s));
                add(
                    "restart into hot",
                    drawn(
                        v,
                        &r,
                        Some((Part::RestartInto, Interaction::Hot)),
                        Cues::NONE,
                        s,
                    ),
                );
                r.focus = Part::RestartInto;
                add("restart into focused", drawn(v, &r, None, all, s));
                add(
                    "restart into pressed",
                    drawn(
                        v,
                        &r,
                        Some((Part::RestartInto, Interaction::Pressed)),
                        all,
                        s,
                    ),
                );
            }
        }
        crate::pins::check("interfaces/power_panel", &got);
    }

    #[test]
    fn classics_cancel_pins_per_state_and_scale() {
        crate::pins::fonts();
        let classic = Theme::default();
        let mut got = Vec::new();
        for s in [1, 2] {
            let theme = classic.scaled(s);
            let mut t = Panel::turn_off(First::StandBy {
                live: false,
                hibernate: true,
            });
            t.title = String::new();
            let mut add = |name: &str, p: &Panel, lit: Lit<Part>, cues: Cues| {
                let c = drawn_with(Variant::Blue, Some(&theme), p, lit, cues, s);
                got.push((format!("Classic {s} {name}"), fnv(&c)));
            };
            add("turn off", &t, None, Cues::NONE);
            add(
                "cancel hot",
                &t,
                Some((Part::Cancel, Interaction::Hot)),
                Cues::NONE,
            );
            t.focus = Part::Cancel;
            add("cancel focused", &t, None, Cues::ALL);
            add(
                "cancel pressed",
                &t,
                Some((Part::Cancel, Interaction::Pressed)),
                Cues::ALL,
            );
            t.restart_into = true;
            add("restart into", &t, None, Cues::NONE);
            t.focus = Part::RestartInto;
            add("restart into focused", &t, None, Cues::ALL);
        }
        crate::pins::check("interfaces/power_panel_classic", &got);
    }

    #[test]
    fn the_install_updates_box_pins_per_state_look_and_scale() {
        crate::pins::fonts();
        let classic = Theme::default();
        let mut got = Vec::new();
        for s in [1, 2] {
            let theme = classic.scaled(s);
            for (look, cancel) in [("Blue", None), ("Classic", Some(&theme))] {
                let mut t = Panel::turn_off(First::StandBy {
                    live: true,
                    hibernate: false,
                });
                t.title = String::new();
                t.offer_updates(true, true);
                let mut add = |name: &str, p: &Panel, lit: Lit<Part>, cues: Cues| {
                    let c = drawn_with(Variant::Blue, cancel, p, lit, cues, s);
                    got.push((format!("{look} {s} {name}"), fnv(&c)));
                };
                add("checked", &t, None, Cues::NONE);
                add(
                    "hot",
                    &t,
                    Some((Part::Updates, Interaction::Hot)),
                    Cues::NONE,
                );
                add(
                    "pressed",
                    &t,
                    Some((Part::Updates, Interaction::Pressed)),
                    Cues::NONE,
                );
                t.toggle_updates();
                add("unchecked focused", &t, None, Cues::ALL);
                t.toggle_updates();
                t.restart_into = true;
                add("checked beside a greyed restart into", &t, None, Cues::NONE);
            }
            let glass = theme_for(Interface::Glass).scaled(s);
            let text = TextRenderer::load(&glass.fonts);
            let paint = glass.paint();
            let ink = glass.colors.button_text;
            let row = Rect::new(0, 0, 160, 20);
            for (name, on, ptr, focus) in [
                ("checked", true, Interaction::Idle, false),
                ("hot", true, Interaction::Hot, false),
                ("unchecked focused", false, Interaction::Idle, true),
            ] {
                let mut c = Canvas::new((row.width * s) as u32, (row.height * s) as u32);
                let px = glass.fonts.size_px;
                let t = text.for_output(s);
                draw_updates_box(&mut c, t, &paint, row, on, ptr, focus, ink, px);
                got.push((format!("Glass {s} {name}"), fnv(&c)));
            }
        }
        crate::pins::check("interfaces/power_panel_updates", &got);
    }

    #[test]
    fn the_install_updates_box_shows_only_while_offered_and_writes_white_on_the_band() {
        let mut p = Panel::turn_off(First::StandBy {
            live: true,
            hibernate: false,
        });
        let plain = drawn(Variant::Blue, &p, None, Cues::NONE, 1);
        p.offer_updates(true, true);
        let lay = layout(&p, TextUnits::ONE);
        let row = lay.updates.expect("the check box's row");
        let c = drawn(Variant::Blue, &p, None, Cues::NONE, 1);
        let (b, label) = Layout::check_parts(row, TextUnits::ONE);
        let differs = |x: i32, y: i32| c.get(x, y) != plain.get(x, y);
        assert!(differs(b.x + 6, b.y + 6), "the box is drawn");
        let white = (label.y..label.bottom())
            .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) & 0xFF_FFFF == 0xFF_FFFF)
            .count();
        assert!(white > 40, "the label is white on the band ({white} px)");
        let cancel = lay.cancel;
        assert_eq!(
            c.get(cancel.x + 4, cancel.y + 4),
            plain.get(cancel.x + 4, cancel.y + 4),
            "Cancel stays where it was"
        );
        p.offer_updates(false, false);
        assert_eq!(
            drawn(Variant::Blue, &p, None, Cues::NONE, 1).pixels,
            plain.pixels
        );
    }

    #[test]
    fn cancel_takes_the_classic_face_under_classic_and_blues_otherwise() {
        let p = Panel::turn_off(First::StandBy {
            live: false,
            hibernate: true,
        });
        let lay = layout(&p, TextUnits::ONE);
        let classic = Theme::default();
        let r = lay.cancel;
        let at = (r.x + 4, r.y + r.height / 2);
        let face = classic.colors.button_face.argb() & 0xFF_FFFF;
        let under_classic = drawn_with(Variant::Blue, Some(&classic), &p, None, Cues::NONE, 1);
        assert_eq!(
            under_classic.get(at.0, at.1) & 0xFF_FFFF,
            face,
            "Classic's Cancel is the button face"
        );
        let blue = drawn(Variant::Blue, &p, None, Cues::NONE, 1);
        assert_ne!(
            blue.get(at.0, at.1) & 0xFF_FFFF,
            face,
            "Blue's Cancel is its own push button"
        );
        let outside = (r.x - 6, r.y + r.height / 2);
        assert_eq!(
            under_classic.get(outside.0, outside.1),
            blue.get(outside.0, outside.1),
            "the band around Cancel stays Blue's"
        );
    }

    #[test]
    fn restart_into_inks_its_label_in_its_button_and_leaves_the_band_otherwise() {
        let mut p = Panel::turn_off(First::StandBy {
            live: true,
            hibernate: false,
        });
        let bare = drawn(Variant::Blue, &p, None, Cues::NONE, 1);
        p.restart_into = true;
        let lay = layout(&p, TextUnits::ONE);
        let with = drawn(Variant::Blue, &p, None, Cues::NONE, 1);
        let r = lay.restart_into.unwrap();
        let differs = |x0: i32, x1: i32, y0: i32, y1: i32| {
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|&(x, y)| bare.get(x, y) != with.get(x, y))
                .count()
        };
        assert!(
            differs(r.x + 4, r.right() - 4, r.y + 4, r.bottom() - 4) > 40,
            "a face and a label"
        );
        assert_eq!(
            differs(0, lay.size.0, 0, r.y - 2),
            0,
            "nothing above it moves"
        );
        assert_eq!(
            differs(r.right() + 2, lay.size.0, r.y, r.bottom()),
            0,
            "Cancel stays as it was"
        );
    }

    #[test]
    fn the_title_inks_white_in_the_title_band_left_of_the_logo() {
        let p = Panel::turn_off(First::StandBy {
            live: true,
            hibernate: false,
        });
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
        let p = Panel::turn_off(First::StandBy {
            live: false,
            hibernate: false,
        });
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
            let mut t = Panel::turn_off(First::StandBy {
                live: false,
                hibernate: true,
            });
            crate::raster::write_preview(
                &drawn(Variant::Blue, &t, None, Cues::NONE, s),
                &format!("power-turn-off@{s}.png"),
            );
            t.set_shift(true);
            crate::raster::write_preview(
                &drawn(Variant::Olive, &t, None, Cues::NONE, s),
                &format!("power-hibernate-olive@{s}.png"),
            );
            t.restart_into = true;
            t.focus = Part::RestartInto;
            crate::raster::write_preview(
                &drawn(Variant::Blue, &t, None, Cues::ALL, s),
                &format!("power-restart-into@{s}.png"),
            );
            crate::raster::write_preview(
                &drawn(
                    Variant::Blue,
                    &Panel::log_off(Leave::SwitchUser { live: false }),
                    None,
                    Cues::NONE,
                    s,
                ),
                &format!("power-log-off@{s}.png"),
            );
        }
    }
}
