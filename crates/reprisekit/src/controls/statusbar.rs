//! The status bar: sunken panels along the bottom of a window, with the
//! sizing grip in the corner. A 22 px bar of 18 px panels with 2 px of face
//! above and below, a 2 px gap, text 11 px in. Panels span the whole client
//! width; the grip reserves nothing and sits 1 px inside the last panel's
//! recess. A panel may carry a 16 px icon.

use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::look::widgets::statusbar as widgets;
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::TextUnits;
use accesskit::{NodeId, Role};

/// The bar's height in Classic; a look sets its own ([`height`]).
pub const HEIGHT: i32 = 22;
pub const PANEL_H: i32 = 18;
/// A panel's text inset (Notepad's 11, Explorer's 2); programs set `Panel::pad`.
pub const TEXT_PAD: i32 = 11;
pub const EXPLORER_TEXT_PAD: i32 = 2;
/// A panel's icon starts 2 px in and its text 22 px in.
pub const ICON_PAD: i32 = 2;
pub const ICON_PX: i32 = 16;
pub const ICON_TEXT_PAD: i32 = 22;
const GAP: i32 = 2;
const GRIP: i32 = 16;

fn grip_rect(r: Rect) -> Rect {
    Rect::new(r.right() - GRIP - 1, r.y + GAP + 1, GRIP, GRIP)
}

/// Whether (`x`, `y`) is on the grip drawn in `r`; a press there resizes from the lower-right corner.
pub fn hit_grip(sb: &StatusBar, r: Rect, x: i32, y: i32) -> bool {
    let g = grip_rect(r);
    sb.grip && Rect::new(g.x, g.y, r.right() - g.x, r.bottom() - g.y).contains(x, y)
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Panel {
    pub text: String,
    pub width: i32,
    pub pad: i32,
    /// A 16 px icon left of the text, already at the drawing scale.
    pub icon: Option<Canvas>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StatusBar {
    pub panels: Vec<Panel>,
    pub grip: bool,
}

/// The panels in `r`, their fixed widths and their height grown with the text by `u`.
pub fn layout(sb: &StatusBar, r: Rect, u: TextUnits) -> Vec<Rect> {
    let fixed: i32 = sb
        .panels
        .iter()
        .filter(|p| p.width > 0)
        .map(|p| u.len(p.width))
        .sum();
    let gaps = GAP * (sb.panels.len().saturating_sub(1)) as i32;
    let flex = sb.panels.iter().filter(|p| p.width <= 0).count().max(1) as i32;
    let spare = (r.width - fixed - gaps).max(0) / flex;
    let mut out = Vec::with_capacity(sb.panels.len());
    let mut x = r.x;
    for p in &sb.panels {
        let w = if p.width > 0 { u.len(p.width) } else { spare };
        out.push(Rect::new(x, r.y + GAP, w, u.len(PANEL_H)));
        x += w + GAP;
    }
    out
}

/// The bar's height in a look with controls `cm`, grown with its text.
pub fn height(cm: &ControlMetrics) -> i32 {
    cm.status_bar
}

pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, sb: &StatusBar, r: Rect, px: u32) {
    let Paint { cs, s, .. } = *p;
    let bar = r.scaled(s);
    widgets::draw_bar(c, p, bar);
    let rects = layout(sb, r, p.text);
    for (i, (panel, rect)) in sb.panels.iter().zip(&rects).enumerate() {
        let next = rects.get(i + 1).map(|n| n.x * s);
        let boxes = widgets::draw_pane(c, p, bar, rect.scaled(s), next, sb.grip, i == 0);
        let pr = boxes.text;
        let pad = match &panel.icon {
            Some(ic) => {
                let ib = boxes.icon;
                let iy = ib.y + (ib.height - ic.height as i32) / 2;
                c.blit_over(ib.x + ICON_PAD * s, iy, ic);
                ICON_TEXT_PAD
            }
            None => panel.pad,
        };
        if panel.text.is_empty() {
            continue;
        }
        let t = Rect::new(pr.x + pad * s, pr.y, pr.width - pad * s, pr.height);
        text.draw(c, t, &panel.text, false, cs.button_text, Align::Left, px);
    }
    if sb.grip {
        widgets::draw_grip(c, p, bar, grip_rect(r).scaled(s));
    }
}

pub fn access(sb: &StatusBar, r: Rect, u: TextUnits, tree: &mut Tree, parent: NodeId) -> NodeId {
    let id = tree.push(parent, "status", access::node(Role::Status, r));
    for (i, (p, pr)) in sb.panels.iter().zip(layout(sb, r, u)).enumerate() {
        if !p.text.is_empty() {
            tree.push(id, i, access::label(pr, &p.text, true));
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::text::TextRenderer;
    use crate::theme::{Color, FontConfig, Theme};

    fn bar() -> StatusBar {
        StatusBar {
            panels: vec![
                Panel {
                    text: String::new(),
                    width: 0,
                    pad: TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: "Ln 1, Col 1".into(),
                    width: 356,
                    pad: TEXT_PAD,
                    icon: None,
                },
            ],
            grip: true,
        }
    }

    /// Notepad's bar: a 1432 px client, panel 1 1074 wide from x 0, a 2 px gap,
    /// panel 2 356 wide, both 18 px tall 2 px down.
    /// At 200% text a panel's fixed width and its height grow with the bar.
    #[test]
    fn grown_panels_take_the_texts_width_and_height() {
        let u = TextUnits::new(22, 11);
        let cm = ControlMetrics::CLASSIC.grown(u);
        assert_eq!(height(&cm), 44);
        let r = Rect::new(0, 100, 1432, height(&cm));
        let lay = layout(&bar(), r, u);
        assert_eq!(lay[1], Rect::new(1432 - 712, 102, 712, 36));
        assert_eq!(lay[0], Rect::new(0, 102, 1432 - 712 - 2, 36));
    }

    #[test]
    fn panels_land_on_their_pixels() {
        let r = Rect::new(0, 746, 1432, HEIGHT);
        let lay = layout(&bar(), r, TextUnits::ONE);
        assert_eq!(lay.len(), 2);
        assert_eq!(lay[0], Rect::new(0, 748, 1074, PANEL_H));
        assert_eq!(lay[1], Rect::new(1076, 748, 356, PANEL_H));
        assert_eq!(lay[1].x - lay[0].right(), 2);
        assert_eq!(lay[0].y - r.y, 2);
        assert_eq!(r.bottom() - lay[0].bottom(), 2);
    }

    #[test]
    fn a_panel_recess_is_shadow_over_hilight() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let r = Rect::new(0, 0, 200, HEIGHT);
        let mut c = Canvas::new(200, HEIGHT as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        let sb = StatusBar {
            panels: vec![Panel {
                text: String::new(),
                width: 100,
                pad: TEXT_PAD,
                icon: None,
            }],
            grip: false,
        };
        draw(&mut c, &text, &classic_paint(cs, 1), &sb, r, 11);
        let p = layout(&sb, r, TextUnits::ONE)[0];
        assert_eq!(c.get(p.x + 5, p.y), cs.button_shadow.argb());
        assert_eq!(c.get(p.x, p.y + 5), cs.button_shadow.argb());
        assert_eq!(c.get(p.x + 5, p.bottom() - 1), cs.button_hilight.argb());
        assert_eq!(c.get(p.right() - 1, p.y + 5), cs.button_hilight.argb());
        assert_eq!(c.get(p.x + 5, p.y + 5), cs.button_face.argb());
    }

    /// A panel's text lands 11 px inside it. The expected inset is a literal, not
    /// `TEXT_PAD`, which `draw` also uses.
    #[test]
    fn panel_text_starts_at_its_inset() {
        const EXPECTED_TEXT_X: i32 = 11;
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let label = "Ln 1, Col 1";
        let r = Rect::new(0, 0, 200, HEIGHT);
        let sb = StatusBar {
            panels: vec![Panel {
                text: label.into(),
                width: 150,
                pad: TEXT_PAD,
                icon: None,
            }],
            grip: false,
        };
        let mut c = Canvas::new(200, HEIGHT as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &sb, r, 11);

        let ink = |canvas: &Canvas, b: Rect| {
            (b.x..b.right())
                .find(|&x| (b.y..b.bottom()).any(|y| canvas.get(x, y) == cs.button_text.argb()))
        };
        // The glyph's left bearing is not part of the pad: measure it by drawing from a known origin.
        let mut scratch = Canvas::new(200, HEIGHT as u32);
        scratch.fill_rect(scratch.bounds(), cs.button_face);
        let origin = Rect::new(0, 0, 200, HEIGHT);
        text.draw(
            &mut scratch,
            origin,
            label,
            false,
            cs.button_text,
            Align::Left,
            11,
        );
        let bearing = ink(&scratch, origin).expect("the label inks something");

        let p = layout(&sb, r, TextUnits::ONE)[0];
        let first = ink(&c, p).expect("the panel's text inks something");
        assert_eq!(
            first,
            p.x + EXPECTED_TEXT_X + bearing,
            "a panel's text starts 11 px inside it"
        );
    }

    /// The panels fill the whole bar (1074 + 2 + 356 = 1432): the grip overlays
    /// the last panel's recess, so layout is the same with or without it.
    #[test]
    fn the_grip_does_not_take_space_from_the_panels() {
        let r = Rect::new(0, 0, 400, HEIGHT);
        let with = layout(&bar(), r, TextUnits::ONE);
        let without = layout(
            &StatusBar {
                grip: false,
                ..bar()
            },
            r,
            TextUnits::ONE,
        );
        assert_eq!(with, without, "the grip must not change panel layout");

        // The grip sits inside the last panel's rect; on a 1432 px bar at (1415, 3),
        // pinned exactly since containment also admits a grip flush to the corner.
        let last = *with.last().unwrap();
        let g = grip_rect(r);
        assert!(g.x >= last.x && g.right() <= last.right());
        assert!(g.y >= last.y && g.bottom() <= last.bottom());

        let wide = Rect::new(0, 0, 1432, HEIGHT);
        assert_eq!(grip_rect(wide), Rect::new(1415, 3, GRIP, GRIP));
    }

    /// The grip answers the pointer over its 16 px square out to the bar's corner, not when undrawn.
    #[test]
    fn the_grip_is_hit_over_its_square() {
        let r = Rect::new(0, 100, 400, HEIGHT);
        let sb = bar();
        assert!(hit_grip(&sb, r, 383, 103), "the square's top-left");
        assert!(hit_grip(&sb, r, 399, 121), "the bar's corner");
        assert!(!hit_grip(&sb, r, 382, 110), "left of the square");
        assert!(!hit_grip(&sb, r, 390, 102), "above the square");
        let off = StatusBar { grip: false, ..sb };
        assert!(!hit_grip(&off, r, 390, 110), "no grip, no hit");
    }

    /// Explorer's last panel: the icon's box starts 2 px right of the panel's
    /// left ring and one row below its top ring, and the text inks from 22 px in.
    /// The literals are not the constants, so moving a constant fails here.
    #[test]
    fn a_panel_icon_lands_where_explorer_puts_it() {
        const PANEL_X: i32 = 694;
        const ICON_X: i32 = 696;
        const ICON_TOP: i32 = 646;
        const TEXT_X: i32 = 716;
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let label = "My Computer";
        let ink_color = Color::rgb(0, 0, 255);
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.fill_rect(Rect::new(0, 0, ICON_PX, ICON_PX), ink_color);
        let sb = StatusBar {
            panels: vec![Panel {
                text: label.into(),
                width: 153,
                pad: TEXT_PAD,
                icon: Some(icon),
            }],
            grip: false,
        };
        let r = Rect::new(PANEL_X, 643, 153, HEIGHT);
        let mut c = Canvas::new(900, 700);
        c.fill_rect(c.bounds(), cs.window);
        draw(&mut c, &text, &classic_paint(cs, 1), &sb, r, 11);
        let p = layout(&sb, r, TextUnits::ONE)[0];
        assert_eq!(p.x, PANEL_X);

        let icon_cols = (p.x..p.right())
            .filter(|&x| (p.y..p.bottom()).any(|y| c.get(x, y) == ink_color.argb()))
            .collect::<Vec<_>>();
        let icon_rows = (p.y..p.bottom())
            .filter(|&y| (p.x..p.right()).any(|x| c.get(x, y) == ink_color.argb()))
            .collect::<Vec<_>>();
        assert_eq!(icon_cols.first().copied(), Some(ICON_X));
        assert_eq!(icon_cols.len(), ICON_PX as usize);
        assert_eq!(icon_rows.first().copied(), Some(ICON_TOP));
        assert_eq!(icon_rows.len(), ICON_PX as usize);

        let ink = |canvas: &Canvas, b: Rect| {
            (b.x..b.right())
                .find(|&x| (b.y..b.bottom()).any(|y| canvas.get(x, y) == cs.button_text.argb()))
        };
        let mut scratch = Canvas::new(200, HEIGHT as u32);
        scratch.fill_rect(scratch.bounds(), cs.button_face);
        let origin = Rect::new(0, 0, 200, HEIGHT);
        text.draw(
            &mut scratch,
            origin,
            label,
            false,
            cs.button_text,
            Align::Left,
            11,
        );
        let bearing = ink(&scratch, origin).expect("the label inks something");
        let first = ink(&c, p).expect("the panel's text inks something");
        assert_eq!(first, TEXT_X + bearing);
    }

    fn explorer_bar(grip: bool) -> StatusBar {
        let pane = |width| Panel {
            text: String::new(),
            width,
            pad: EXPLORER_TEXT_PAD,
            icon: None,
        };
        StatusBar {
            panels: vec![pane(0), pane(76), pane(153)],
            grip,
        }
    }

    fn blue_bar(v: crate::blue::Variant, sb: &StatusBar, w: i32) -> Canvas {
        let theme = crate::interfaces::blue_theme(v);
        let text = TextRenderer::load(&FontConfig::default());
        let h = height(&theme.controls());
        let mut c = Canvas::new(w as u32, h as u32);
        draw(&mut c, &text, &theme.paint(), sb, Rect::new(0, 0, w, h), 11);
        c
    }

    /// On a 23 px Blue bar a pane's 16 px icon fills rows 4 to 19 in every
    /// variant; its text keeps centring in the pane's 18 rows from row 4.
    #[test]
    fn a_blue_panes_icon_starts_on_the_content_top() {
        let ink = Color::rgb(0, 0, 255);
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.fill_rect(Rect::new(0, 0, ICON_PX, ICON_PX), ink);
        for v in crate::blue::Variant::ALL {
            let mut sb = explorer_bar(false);
            sb.panels[2].icon = Some(icon.clone());
            let c = blue_bar(v, &sb, 400);
            let rows = (0..23)
                .filter(|&y| (0..400).any(|x| c.get(x, y) == ink.argb()))
                .collect::<Vec<_>>();
            assert_eq!(rows.first(), Some(&4), "{v:?}: the icon's top row");
            assert_eq!(rows.len(), ICON_PX as usize, "{v:?}: the icon's rows");
        }
    }

    #[test]
    fn a_blue_bar_is_23_px_tall() {
        let blue = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        assert_eq!(height(&blue.controls()), 23);
        assert_eq!(height(&Theme::default().controls()), HEIGHT);
    }

    /// A navy, olive green or silver pane is its divider, a shadow line and a
    /// white one down rows 4 to 20, ending 3 px before the next pane. The last
    /// pane's divider sits at the bar's end, and not under the grip.
    #[test]
    fn a_blue_panes_divider_sits_before_the_next_pane() {
        let luma = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
        let r = Rect::new(0, 0, 400, 23);
        let lay = layout(&explorer_bar(true), r, TextUnits::ONE);
        use crate::blue::Variant;
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let c = blue_bar(v, &explorer_bar(true), 400);
            for next in [lay[1].x, lay[2].x] {
                for y in 4..=20 {
                    assert_ne!(
                        c.get(next - 5, y),
                        c.get(next - 7, y),
                        "{v:?}: shadow at {y}"
                    );
                    assert_eq!(c.get(next - 4, y), 0xFFFF_FFFF, "{v:?}: white at {y}");
                }
                assert!(
                    luma(c.get(next - 5, 9)) < luma(c.get(next - 7, 9)),
                    "{v:?}: the shadow darkens the bar"
                );
                assert_eq!(c.get(next - 4, 3), c.get(next - 7, 3), "{v:?}: over it");
                assert_eq!(c.get(next - 4, 21), c.get(next - 7, 21), "{v:?}: under it");
            }
            let open = blue_bar(v, &explorer_bar(false), 400);
            assert!(
                luma(open.get(398, 9)) < luma(open.get(396, 9)),
                "{v:?}: the last divider"
            );
            assert_eq!(open.get(399, 9), 0xFFFF_FFFF, "{v:?}: the last divider");
            assert_ne!(c.get(399, 9), 0xFFFF_FFFF, "{v:?}: none under the grip");
        }
    }

    #[test]
    fn a_blue_grip_sits_on_the_bar() {
        let v = crate::blue::Variant::Blue;
        let with = blue_bar(v, &explorer_bar(true), 400);
        let bare = blue_bar(v, &explorer_bar(false), 400);
        let grey = v.scheme().size_box.normal.inner.argb();
        assert_eq!(with.get(396, 11), grey, "the top dot");
        assert_eq!(with.get(388, 19), grey, "the lower-left dot");
        assert_eq!(with.get(398, 21), 0xFFFF_FFFF, "the corner dot's white");
        assert_eq!(
            with.get(388, 11),
            bare.get(388, 11),
            "the bar shows between"
        );
        assert_eq!(
            with.get(399, 22),
            bare.get(399, 22),
            "the corner stays the bar's"
        );
    }

    #[test]
    fn a_blue_bar_doubles_at_paint_scale_2() {
        let text = TextRenderer::load(&FontConfig::default());
        for grip in [true, false] {
            assert!(
                crate::look::styled::fixture::blue_doubles_at_paint_scale_2(400, 23, |c, p| {
                    draw(
                        c,
                        &text,
                        p,
                        &explorer_bar(grip),
                        Rect::new(0, 0, 400, 23),
                        11,
                    );
                }),
                "grip {grip}"
            );
        }
    }

    /// Writes target/statusbar-preview.png for a visual check.
    #[test]
    #[ignore]
    fn render_statusbar() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.fill_rect(Rect::new(2, 2, 12, 12), Color::rgb(80, 193, 255));
        let sb = StatusBar {
            panels: vec![
                Panel {
                    text: "4 objects".into(),
                    width: 0,
                    pad: TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: String::new(),
                    width: 76,
                    pad: TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: "My Computer".into(),
                    width: 153,
                    pad: TEXT_PAD,
                    icon: Some(icon),
                },
            ],
            grip: true,
        };
        let r = Rect::new(0, 0, 792, HEIGHT);
        let mut c = Canvas::new(792, HEIGHT as u32);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &sb,
            r,
            theme.fonts.size_px,
        );
        crate::raster::write_preview(&c, "statusbar-preview.png");
    }
}
