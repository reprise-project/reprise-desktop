use crate::look::Paint;
use crate::look::styled::state;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::text::{Align, TextRenderer};

pub fn draw_back(st: &Style, c: &mut Canvas, r: Rect, s: i32) -> bool {
    if st.part(Part::TaskPane).is_none() {
        return false;
    }
    st.draw(c, r, Part::TaskPane, State::Normal, s);
    true
}

#[expect(clippy::too_many_arguments)]
pub fn draw_header(
    st: &Style,
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    title_r: Rect,
    title: &str,
    open: bool,
    ptr: Interaction,
    px: u32,
) -> bool {
    let (button, mark) = if open {
        (Part::TaskCollapse, Part::TaskCollapseMark)
    } else {
        (Part::TaskExpand, Part::TaskExpandMark)
    };
    let (Some(_), Some(img)) = (st.part(Part::TaskGroupHeader), st.part(button)) else {
        return false;
    };
    let s = p.s;
    let state = state(true, ptr);
    st.draw(c, r, Part::TaskGroupHeader, State::Normal, s);
    // An atlas painted at scale holds `scale` pixels per design pixel.
    let frame = img.frame(state);
    let (w, h) = (frame.width / img.scale * s, frame.height / img.scale * s);
    if r.height == h {
        st.draw(c, Rect::new(r.right() - w, r.y, w, h), button, state, s);
    } else if st.part(mark).is_some() {
        // A band grown with the text moves the button down by half its growth, drawn without
        // the band's 100% rows behind it.
        let y = r.y + (r.height - h) / 2;
        st.draw(c, Rect::new(r.right() - w, y, w, h), mark, state, s);
    }
    let lit = if state == State::Normal {
        State::Normal
    } else {
        State::Hot
    };
    let ink = st
        .text_color(Part::TaskGroupHeader, lit)
        .unwrap_or(p.cs.button_text);
    text.draw(c, title_r, title, true, ink, Align::Left, px);
    true
}

pub fn draw_body(st: &Style, c: &mut Canvas, r: Rect, s: i32) -> bool {
    if st.part(Part::TaskGroupBody).is_none() {
        return false;
    }
    st.draw(c, r, Part::TaskGroupBody, State::Normal, s);
    true
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::controls::taskpane::{self, Row, Section, TaskPane};
    use crate::interface::Interface;
    use crate::interfaces::{blue_theme, theme_for};
    use crate::look::widgets::Interaction;
    use crate::look::widgets::taskpane::{draw_back, draw_body, draw_header, link_ink};
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, State};
    use crate::text::TextRenderer;
    use crate::theme::Color;
    use crate::theme::FontConfig;

    const BAND_W: i32 = 185;

    const SENTINEL: Color = Color::rgb(1, 2, 3);

    fn px(c: Color) -> u32 {
        0xFF00_0000 | u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b)
    }

    fn band_h(v: Variant) -> i32 {
        if v == Variant::Blue { 25 } else { 23 }
    }

    #[test]
    fn a_blue_pane_shades_down_from_its_first_stop_to_its_last() {
        for v in Variant::ALL {
            let theme = blue_theme(v);
            let p = theme.paint();
            let mut c = Canvas::new(10, 456);
            draw_back(&mut c, &p, Rect::new(0, 0, 10, 456));
            let down = v.scheme().task_pane.normal.down;
            assert_eq!(c.get(4, 0), px(down[0].1), "{v:?} top row");
            assert_eq!(
                c.get(4, 455),
                px(down[down.len() - 1].1),
                "{v:?} bottom row"
            );
        }
    }

    fn header(v: Variant, ptr: Interaction, open: bool, s: i32) -> Canvas {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = blue_theme(v).scaled(s);
        let p = theme.paint();
        let band = Rect::new(0, 0, BAND_W, band_h(v)).scaled(s);
        let mut c = Canvas::new(band.width as u32, band.height as u32);
        c.fill_rect(c.bounds(), SENTINEL);
        draw_header(&mut c, &text, &p, band, band, "", open, ptr, 11 * s as u32);
        c
    }

    #[test]
    fn a_blue_title_band_lights_its_fold_button() {
        for v in Variant::ALL {
            let s = v.scheme();
            let rest = px(s.task_collapse.normal.mark.unwrap());
            let lit = px(s.task_collapse.hot.mark.unwrap());
            let apex = (BAND_W - 24 + 9, 7);
            let c = header(v, Interaction::Idle, true, 1);
            assert_eq!(c.get(apex.0, apex.1), rest, "{v:?} glyph at rest");
            let hot = header(v, Interaction::Hot, true, 1);
            assert_eq!(hot.get(apex.0, apex.1), lit, "{v:?} glyph lit");
            assert!(
                hot == header(v, Interaction::Pressed, true, 1),
                "{v:?} held draws lit"
            );
            let shut = header(v, Interaction::Idle, false, 1);
            let unfold = px(s.task_expand.normal.mark.unwrap());
            assert_eq!(shut.get(BAND_W - 24 + 6, 8), unfold, "{v:?} unfold glyph");
            assert_ne!(shut.get(apex.0, apex.1), unfold, "{v:?} no apex folded");
        }
    }

    #[test]
    fn a_flat_title_band_rounds_its_corners_and_sits_its_button_on_a_disc() {
        for v in [
            Variant::Blue,
            Variant::Olive,
            Variant::Silver,
            Variant::Steel,
        ] {
            let c = header(v, Interaction::Idle, true, 1);
            for (x, y) in [(0, 0), (BAND_W - 1, 0)] {
                assert_eq!(c.get(x, y), px(SENTINEL), "{v:?} corner {x},{y}");
            }
            assert_ne!(c.get(4, 0), px(SENTINEL), "{v:?} top row");
            assert_eq!(c.get(BAND_W - 24 + 6, 7), 0xFFFF_FFFF, "{v:?} disc");
        }
        let c = header(Variant::Blue, Interaction::Idle, true, 1);
        assert_eq!(c.get(10, 12), 0xFFFF_FFFF, "navy's band starts white");
    }

    #[test]
    fn a_blue_title_band_at_scale_2_is_drawn_again() {
        for v in Variant::ALL {
            let one = header(v, Interaction::Idle, true, 1);
            let two = header(v, Interaction::Idle, true, 2);
            assert_eq!((two.width, two.height), (2 * one.width, 2 * one.height));
            let mark = px(v.scheme().task_collapse.normal.mark.unwrap());
            let x0 = 2 * (BAND_W - 24);
            let inked = (x0..two.width as i32)
                .flat_map(|x| (0..two.height as i32).map(move |y| (x, y)))
                .filter(|&(x, y)| two.get(x, y) == mark)
                .count();
            assert!(inked > 0, "{v:?} solid ink at 2");
            let doubled = (0..one.height as i32).all(|y| {
                (0..one.width as i32).all(|x| {
                    let p = one.get(x, y);
                    [(0, 0), (1, 0), (0, 1), (1, 1)]
                        .iter()
                        .all(|&(dx, dy)| two.get(2 * x + dx, 2 * y + dy) == p)
                })
            });
            assert!(!doubled, "{v:?} is not the scale-1 band doubled");
        }
    }

    #[test]
    fn a_blue_fold_button_at_scale_2_keeps_its_own_width() {
        for v in Variant::ALL {
            let two = header(v, Interaction::Idle, true, 2);
            let theme = blue_theme(v).scaled(2);
            let st = theme.look.style().expect("Blue has a style");
            let mut band = Canvas::new(two.width, two.height);
            band.fill_rect(band.bounds(), SENTINEL);
            let r = Rect::new(0, 0, BAND_W, band_h(v)).scaled(2);
            st.draw(&mut band, r, Part::TaskGroupHeader, State::Normal, 2);
            let x0 = 2 * (BAND_W - 24);
            for y in 0..two.height as i32 {
                for x in 0..x0 {
                    assert_eq!(two.get(x, y), band.get(x, y), "{v:?} ({x}, {y})");
                }
            }
            assert!(
                (x0..two.width as i32).any(|x| two.get(x, 14) != band.get(x, 14)),
                "{v:?} the button draws over the band's end"
            );
        }
    }

    #[test]
    fn a_blue_body_is_flat_inside_its_edge() {
        for v in Variant::ALL {
            let b = v.scheme().task_group_body.normal;
            let (fill, edge) = (px(b.across[b.across.len() / 2].1), px(b.rim));
            let theme = blue_theme(v);
            let p = theme.paint();
            let mut c = Canvas::new(40, 20);
            draw_body(&mut c, &p, Rect::new(0, 0, 40, 20));
            assert_eq!(c.get(0, 5), edge, "{v:?} left edge");
            assert_eq!(c.get(39, 5), edge, "{v:?} right edge");
            assert_eq!(c.get(20, 19), edge, "{v:?} bottom edge");
            assert_eq!(c.get(20, 0), fill, "{v:?} no top edge");
            assert_eq!(c.get(20, 10), fill, "{v:?} fill");
        }
    }

    #[test]
    fn blue_titles_and_links_take_their_inks_lit_lighter() {
        for v in Variant::ALL {
            let i = v.scheme().inks;
            let theme = blue_theme(v);
            let p = theme.paint();
            let st = p.look.style().expect("Blue has a style");
            for (state, want) in [
                (State::Normal, i.task_title),
                (State::Hot, i.task_title_hot),
            ] {
                assert_eq!(
                    st.text_color(Part::TaskGroupHeader, state),
                    Some(want),
                    "{v:?} {state:?} title"
                );
            }
            assert_eq!(link_ink(&p, true, false), i.task_link, "{v:?}");
            assert_eq!(link_ink(&p, true, true), i.task_link_hot, "{v:?}");
            assert_eq!(link_ink(&p, false, true), p.cs.gray_text, "{v:?} dead link");
            let light_titles = !matches!(
                v,
                Variant::Blue | Variant::Olive | Variant::Silver | Variant::Steel
            );
            assert_eq!(i.task_title != i.task_link, light_titles, "{v:?}");
        }
        let classic_theme = theme_for(Interface::Classic);
        let classic = classic_theme.paint();
        assert_eq!(link_ink(&classic, true, true), classic.cs.window_text);
    }

    #[test]
    fn a_blue_pane_lays_its_groups_out_at_its_own_sizes() {
        let text = TextRenderer::load(&FontConfig::default());
        let tp = TaskPane {
            sections: vec![Section::links("Tasks", vec![Row::new("Go", None)])],
            ..Default::default()
        };
        for v in Variant::ALL {
            let h = band_h(v);
            let theme = blue_theme(v);
            let r = Rect::new(0, 0, 200, 300);
            let lay = taskpane::layout(&tp, &text, 11, r, &theme.look.controls());
            assert_eq!(lay.headers[0], Rect::new(13, 12, 185, h), "{v:?}");
            assert_eq!(lay.bodies[0].y, 12 + h, "{v:?}");
            assert_eq!(lay.back, Rect::new(1, 0, 199, 299), "{v:?}");
        }
    }
}
