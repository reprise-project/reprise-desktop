use super::*;
use crate::controls::form::{Control, Key};
use crate::interface::Interface;
use crate::interfaces;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

fn hsl(h: i32, s: i32, l: i32) -> Hsl {
    Hsl { h, s, l }
}

#[test]
fn colours_read_as_whole_hue_saturation_and_luminosity_steps() {
    let cases = [
        (Color::rgb(0xFF, 0, 0), hsl(0, 240, 120)),
        (Color::rgb(0xFF, 0xFF, 0), hsl(40, 240, 120)),
        (Color::rgb(0, 0xFF, 0), hsl(80, 240, 120)),
        (Color::rgb(0, 0xFF, 0xFF), hsl(120, 240, 120)),
        (Color::rgb(0, 0, 0xFF), hsl(160, 240, 120)),
        (Color::rgb(0xFF, 0, 0xFF), hsl(200, 240, 120)),
        (Color::rgb(0x80, 0, 0), hsl(0, 240, 60)),
        (Color::rgb(0xFF, 0x80, 0x80), hsl(0, 240, 180)),
        (Color::rgb(0, 0, 0), hsl(GREY_HUE, 0, 0)),
        (Color::rgb(0x80, 0x80, 0x80), hsl(GREY_HUE, 0, 120)),
        (WHITE, hsl(GREY_HUE, 0, 240)),
    ];
    for (c, want) in cases {
        assert_eq!(hsl_of(c), want, "{c:?}");
        assert_eq!(rgb_of(want), c, "{want:?}");
    }
    assert_eq!(hsl_of(Color::rgb(0x80, 0x80, 0xC0)), hsl(160, 81, 151));
    assert_eq!(rgb_of(hsl(160, 81, 151)), Color::rgb(0x81, 0x81, 0xC0));
}

fn off(a: Color, b: Color) -> u8 {
    [(a.r, b.r), (a.g, b.g), (a.b, b.b)]
        .iter()
        .map(|&(x, y)| x.abs_diff(y))
        .max()
        .unwrap()
}

#[test]
fn every_basic_colour_round_trips_within_three_levels() {
    for c in BASIC {
        let back = rgb_of(hsl_of(c));
        assert!(
            off(c, back) <= 3,
            "{c:?} reads as {:?}, back as {back:?}",
            hsl_of(c)
        );
    }
}

#[test]
fn any_colour_comes_back_within_four_levels() {
    for r in (0..=255).step_by(5) {
        for g in (0..=255).step_by(5) {
            for b in (0..=255).step_by(5) {
                let c = Color::rgb(r as u8, g as u8, b as u8);
                let back = rgb_of(hsl_of(c));
                assert!(off(c, back) <= 4, "{c:?} came back as {back:?}");
            }
        }
    }
}

#[test]
fn any_steps_come_back_within_a_few_steps() {
    for h in (0..HLS_MAX).step_by(3) {
        for s in (0..=HLS_MAX).step_by(6) {
            for l in (12..=HLS_MAX - 12).step_by(6) {
                let back = hsl_of(rgb_of(hsl(h, s, l)));
                assert!((back.l - l).abs() <= 1, "{h} {s} {l} came back as {back:?}");
                if s >= 60 && (60..=180).contains(&l) {
                    let dh = (back.h - h)
                        .rem_euclid(HLS_MAX)
                        .min((h - back.h).rem_euclid(HLS_MAX));
                    assert!(
                        dh <= 3 && (back.s - s).abs() <= 4,
                        "{h} {s} {l} came back as {back:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_painted_shades_agree_with_the_steps() {
    for (h, s, l) in [(0, 240, 120), (80, 240, 120), (40, 120, 60), (200, 60, 200)] {
        let a = rgb_of(hsl(h, s, l));
        let b = rgb_at(f64::from(h), f64::from(s), f64::from(l));
        for (x, y) in [(a.r, b.r), (a.g, b.g), (a.b, b.b)] {
            assert!(x.abs_diff(y) <= 2, "{a:?} against {b:?}");
        }
    }
}

fn custom() -> [Color; CUSTOM] {
    [WHITE; CUSTOM]
}

fn drawn_form(dlg: &ColorDlg) -> Form {
    let f = form(dlg);
    f.drawn_with(&Theme::default().paint());
    f
}

#[test]
fn a_basic_colour_opens_picked_and_its_grid_has_the_keyboard() {
    let dlg = ColorDlg::new(BASIC[9], custom());
    assert_eq!(
        dlg.picked(),
        Some(Well {
            grid: Grid::Basic,
            i: 9
        })
    );
    assert_eq!(dlg.grid(), Some(Grid::Basic));
    let mut c = custom();
    c[3] = Color::rgb(1, 2, 3);
    let dlg = ColorDlg::new(Color::rgb(1, 2, 3), c);
    assert_eq!(
        dlg.picked(),
        Some(Well {
            grid: Grid::Custom,
            i: 3
        })
    );
    let dlg = ColorDlg::new(Color::rgb(9, 9, 9), custom());
    assert_eq!(dlg.picked(), None);
    assert_eq!(dlg.cursor(Grid::Basic), 0);
}

#[test]
fn arrows_walk_the_grid_and_stop_at_its_ends() {
    let mut dlg = ColorDlg::new(BASIC[0], custom());
    let mut f = drawn_form(&dlg);
    assert_eq!(f.focus(), None, "the grid has the keyboard");
    let mut key = |dlg: &mut ColorDlg, k| dlg.key(&mut f, k);
    assert_eq!(key(&mut dlg, Key::Left), Some(Outcome::Redraw));
    assert_eq!(dlg.cursor(Grid::Basic), 0, "stops at the first");
    key(
        &mut dlg,
        Key::Edit(EditKey::Right {
            select: false,
            word: false,
        }),
    );
    key(&mut dlg, Key::Down);
    assert_eq!(dlg.cursor(Grid::Basic), 9);
    assert_eq!(
        dlg.color(),
        BASIC[9],
        "the well under the keyboard is picked"
    );
    key(&mut dlg, Key::End);
    assert_eq!(dlg.cursor(Grid::Basic), 47);
    key(&mut dlg, Key::Down);
    key(&mut dlg, Key::Right);
    assert_eq!(dlg.cursor(Grid::Basic), 47, "stops at the last");
    key(&mut dlg, Key::Up);
    assert_eq!(dlg.cursor(Grid::Basic), 39);
    key(&mut dlg, Key::Edit(EditKey::Home { select: false }));
    assert_eq!(dlg.color(), BASIC[0]);
}

#[test]
fn tab_goes_basic_custom_then_the_buttons_and_round_again() {
    let mut dlg = ColorDlg::new(BASIC[0], custom());
    let mut f = drawn_form(&dlg);
    dlg.key(&mut f, Key::Tab { back: false });
    assert_eq!(dlg.grid(), Some(Grid::Custom));
    dlg.key(&mut f, Key::Tab { back: false });
    assert_eq!(dlg.grid(), None);
    assert_eq!(f.focus(), f.index_of(DEFINE));
    dlg.key(&mut f, Key::Tab { back: true });
    assert_eq!(dlg.grid(), Some(Grid::Custom), "Shift+Tab goes back");
    f.focus_id(CANCEL);
    dlg.grid = None;
    dlg.key(&mut f, Key::Tab { back: false });
    assert_eq!(dlg.grid(), Some(Grid::Basic), "past the last button");
    assert_eq!(f.focus(), None);
    dlg.key(&mut f, Key::Tab { back: true });
    assert_eq!(f.focus(), f.index_of(CANCEL));
}

#[test]
fn the_labels_access_keys_reach_their_grids() {
    let mut dlg = ColorDlg::new(BASIC[0], custom());
    let mut f = drawn_form(&dlg);
    f.focus_id(OK);
    dlg.grid = None;
    assert_eq!(dlg.key(&mut f, Key::Access('c')), Some(Outcome::Redraw));
    assert_eq!((dlg.grid(), f.focus()), (Some(Grid::Custom), None));
    assert_eq!(dlg.key(&mut f, Key::Access('B')), Some(Outcome::Redraw));
    assert_eq!(dlg.grid(), Some(Grid::Basic));
    assert_eq!(
        dlg.key(&mut f, Key::Access('d')),
        None,
        "Define is the form's"
    );
}

#[test]
fn define_widens_the_window_and_the_spinners_show_the_colour() {
    let mut dlg = ColorDlg::new(Color::rgb(0x80, 0x80, 0xC0), custom());
    let mut f = drawn_form(&dlg);
    assert_eq!(client(&dlg), CLIENT_NARROW);
    assert!(f.item(HUE).is_none());
    assert_eq!(
        dlg.form_event(&mut f, form::Event::Pressed(DEFINE)),
        Outcome::Redraw
    );
    assert!(dlg.expanded());
    assert_eq!(client(&dlg), CLIENT_WIDE);
    let spins: Vec<_> = [HUE, SAT, LUM, RED, GREEN, BLUE]
        .iter()
        .map(|id| spin_value(&f, id).unwrap())
        .collect();
    assert_eq!(spins, [160, 81, 151, 0x80, 0x80, 0xC0]);
    assert_eq!(f.focus(), f.index_of(HUE));
    assert!(
        matches!(
            f.item(DEFINE).unwrap().control,
            Control::Button { enabled: false, .. }
        ),
        "Define greys once used"
    );
}

fn wide(c: Color) -> (ColorDlg, Form) {
    let mut dlg = ColorDlg::new(c, custom());
    let mut f = drawn_form(&dlg);
    dlg.form_event(&mut f, form::Event::Pressed(DEFINE));
    f.drawn_with(&Theme::default().paint());
    (dlg, f)
}

fn set_spin(f: &mut Form, id: &str, v: i32) {
    if let Some(Control::UpDown(u)) = f.item_mut(id).map(|i| &mut i.control) {
        u.set(v);
    }
}

#[test]
fn a_spinner_moves_the_colour_and_the_other_spinners_follow() {
    let (mut dlg, mut f) = wide(BASIC[8]);
    set_spin(&mut f, LUM, 60);
    dlg.form_event(&mut f, form::Event::Changed(LUM));
    assert_eq!(dlg.color(), Color::rgb(0x80, 0, 0));
    assert_eq!(dlg.picked(), None, "the basic well no longer holds it");
    assert_eq!(spin_value(&f, RED), Some(0x80));
    set_spin(&mut f, BLUE, 0x80);
    dlg.form_event(&mut f, form::Event::Changed(BLUE));
    assert_eq!(dlg.color(), Color::rgb(0x80, 0, 0x80));
    assert_eq!(dlg.hsl(), hsl(200, 240, 60));
    assert_eq!(spin_value(&f, HUE), Some(200));
    // A grey keeps the hue it was given.
    set_spin(&mut f, SAT, 0);
    dlg.form_event(&mut f, form::Event::Changed(SAT));
    assert_eq!(dlg.hsl(), hsl(200, 0, 60));
    assert_eq!(dlg.color(), Color::rgb(0x40, 0x40, 0x40));
}

#[test]
fn ok_answers_the_colour_and_cancel_closes() {
    let (mut dlg, mut f) = wide(BASIC[3]);
    assert_eq!(
        dlg.form_event(&mut f, form::Event::Pressed(OK)),
        Outcome::Ok(BASIC[3])
    );
    assert_eq!(
        dlg.form_event(&mut f, form::Event::Default),
        Outcome::Ok(BASIC[3])
    );
    assert_eq!(
        dlg.form_event(&mut f, form::Event::Pressed(CANCEL)),
        Outcome::Cancel
    );
    assert_eq!(dlg.form_event(&mut f, form::Event::Cancel), Outcome::Cancel);
}

#[test]
fn add_fills_the_picked_custom_well_else_the_next_in_turn() {
    let (mut dlg, mut f) = wide(BASIC[0]);
    dlg.form_event(&mut f, form::Event::Pressed(ADD));
    dlg.pick(Well {
        grid: Grid::Basic,
        i: 1,
    });
    follow(&mut f, &dlg, None);
    dlg.form_event(&mut f, form::Event::Pressed(ADD));
    assert_eq!(&dlg.custom[..3], &[BASIC[0], BASIC[1], WHITE]);
    let w5 = Well {
        grid: Grid::Custom,
        i: 5,
    };
    dlg.pick(w5);
    follow(&mut f, &dlg, None);
    set_spin(&mut f, RED, 7);
    dlg.form_event(&mut f, form::Event::Changed(RED));
    assert_eq!(dlg.picked(), Some(w5), "a custom pick stays while editing");
    dlg.form_event(&mut f, form::Event::Pressed(ADD));
    assert_eq!(dlg.custom[5], Color::rgb(7, 0xFF, 0xFF));
    assert_eq!(dlg.custom[2], WHITE);
}

#[test]
fn hits_find_the_wells_the_field_and_the_bar() {
    let (_, f) = wide(BASIC[0]);
    let lay = layout(&f);
    assert_eq!(lay.cell(Grid::Basic, 0), Rect::new(8, 24, 26, 22));
    assert_eq!(lay.well(Grid::Basic, 0), Rect::new(11, 27, 20, 16));
    let well = |grid, i| Some(Hit::Well(Well { grid, i }));
    assert_eq!(hit(&lay, 8, 24), well(Grid::Basic, 0));
    assert_eq!(
        hit(&lay, 8 + 26 * 7 + 25, 24 + 22 * 5 + 21),
        well(Grid::Basic, 47)
    );
    assert_eq!(hit(&lay, 8 + 26 * 2, 180 + 22), well(Grid::Custom, 10));
    assert_eq!(hit(&lay, 100, 160), None, "between the grids");
    let field = lay.field.unwrap();
    assert_eq!(field, Rect::new(232, 8, 216, 184));
    assert_eq!(hit(&lay, 234, 10), Some(Hit::Field { h: 0, s: 240 }));
    assert_eq!(hit(&lay, 445, 189), Some(Hit::Field { h: 239, s: 0 }));
    assert_eq!(hit(&lay, 232, 100), None, "the frame is not the field");
    assert_eq!(lay.bar, Some(Rect::new(456, 8, 12, 184)));
    assert_eq!(hit(&lay, 460, 10), Some(Hit::Lum(240)));
    assert_eq!(hit(&lay, 475, 189), Some(Hit::Lum(0)), "the arrow's column");
    assert_eq!(hit(&lay, 485, 100), None);
}

#[test]
fn a_drag_on_the_field_carries_on_past_its_edges() {
    let (mut dlg, mut f) = wide(Color::rgb(0x80, 0x80, 0x80));
    assert_eq!(dlg.press(&mut f, 300, 100), Some(Outcome::Redraw));
    let (h, s) = layout(&f).hue_sat_at(300, 100).unwrap();
    assert_eq!(dlg.hsl(), hsl(h, s, 120));
    dlg.motion(&mut f, 0, 0);
    assert_eq!(dlg.hsl(), hsl(0, 240, 120), "clamped to the corner");
    assert_eq!(dlg.color(), Color::rgb(0xFF, 0, 0));
    assert_eq!(spin_value(&f, RED), Some(0xFF));
    assert!(dlg.release());
    assert_eq!(
        dlg.motion(&mut f, 300, 100),
        None,
        "no drag after the release"
    );
    dlg.press(&mut f, 470, 8 + 2 + 45);
    assert_eq!(dlg.hsl().l, 180);
    dlg.motion(&mut f, 0, 500);
    assert_eq!(dlg.hsl(), hsl(0, 240, 0), "the bar drags luminosity only");
}

#[test]
fn a_press_on_a_well_takes_the_keyboard_from_the_form() {
    let (mut dlg, mut f) = wide(BASIC[0]);
    assert!(f.focus().is_some());
    let r = layout(&f).cell(Grid::Custom, 4);
    dlg.press(&mut f, r.x + 1, r.y + 1);
    assert_eq!((dlg.grid(), f.focus()), (Some(Grid::Custom), None));
    assert_eq!(dlg.color(), WHITE);
    assert_eq!(spin_value(&f, LUM), Some(240));
    assert_eq!(dlg.press(&mut f, 120, 275), None, "Cancel is the form's");
}

fn drawn(theme: &Theme, dlg: &ColorDlg, s: i32) -> Canvas {
    let theme = theme.scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let p = theme.paint();
    let mut f = form(dlg);
    if dlg.grid().is_none() {
        f.focus_id(HUE);
    }
    let (w, h) = client(dlg);
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), p.cs.button_face);
    draw_window(&mut c, &text, &p, &f, dlg, theme.fonts.size_px);
    c
}

fn sample() -> ColorDlg {
    let mut c = custom();
    c[0] = Color::rgb(0x12, 0x80, 0x40);
    c[1] = Color::rgb(0xE0, 0x60, 0x20);
    let mut dlg = ColorDlg::new(Color::rgb(0x12, 0x80, 0x40), c);
    dlg.expand();
    dlg.grid = None;
    dlg
}

#[test]
fn the_parts_draw_where_they_hit_at_scale_1() {
    let dlg = sample();
    let theme = Theme::default();
    let c = drawn(&theme, &dlg, 1);
    let lay = layout(&drawn_form(&dlg));
    let cs = &theme.colors;
    let mid = |r: Rect| c.get(r.x + r.width / 2, r.y + r.height / 2);
    assert_eq!(mid(lay.well(Grid::Basic, 8)), 0xFFFF_0000);
    assert_eq!(mid(lay.well(Grid::Custom, 1)), 0xFFE0_6020);
    let ring = lay.well(Grid::Custom, 0).inset(-2);
    assert_eq!(
        c.get(ring.x, ring.y + 5),
        cs.button_text.argb(),
        "the pick's ring"
    );
    let other = lay.well(Grid::Custom, 1).inset(-2);
    assert_ne!(c.get(other.x, other.y + 5), cs.button_text.argb());
    let inside = lay.field.unwrap().inset(2);
    let red = Color::rgb(0xFF, 0, 0);
    let corner = c.get(inside.x, inside.y);
    let corner = Color::rgb((corner >> 16) as u8, (corner >> 8) as u8, corner as u8);
    assert!(off(corner, red) <= 5, "full red top left: {corner:?}");
    let grey = c.get(inside.right() - 1, inside.bottom() - 1);
    assert_eq!(grey, 0xFF80_8080, "grey along the bottom");
    let bar = lay.bar.unwrap().inset(2);
    assert_eq!(c.get(bar.x + 3, bar.y), 0xFFFF_FFFF, "white at the top");
    assert_eq!(c.get(bar.x + 3, bar.bottom() - 1), 0xFF00_0000);
    let y = lay.arrow_at(dlg.hsl().l).unwrap();
    let tip = lay.arrow.unwrap().x + 2;
    assert_eq!(c.get(tip + 6, y), cs.button_text.argb(), "the arrow's base");
    assert_eq!(
        c.get(tip + 6, y - 7),
        cs.button_face.argb(),
        "above the arrow"
    );
    let (x, cy) = lay.cross_at(dlg.hsl().h, dlg.hsl().s).unwrap();
    assert_eq!(c.get(x - 5, cy), 0xFF00_0000, "the crosshair's arm");
    assert_ne!(c.get(x, cy), 0xFF00_0000, "clear at its point");
    assert_eq!(mid(lay.preview.unwrap()), 0xFF12_8040);
}

#[test]
fn at_scale_2_the_field_and_arrow_are_drawn_fresh() {
    let dlg = sample();
    let c = drawn(&Theme::default(), &dlg, 2);
    let lay = layout(&drawn_form(&dlg));
    let inside = lay.field.unwrap().inset(2).scaled(2);
    let row: Vec<u32> = (0..8)
        .map(|x| c.get(inside.x + 100 + x, inside.y + 40))
        .collect();
    let distinct = row.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(distinct >= 4, "neighbouring device pixels differ: {row:x?}");
    let y = lay.arrow_at(dlg.hsl().l).unwrap();
    let tip = lay.arrow.unwrap().x + 2;
    let ink = Theme::default().colors.button_text.argb();
    let face = Theme::default().colors.button_face.argb();
    let column: Vec<u32> = ((y - 6) * 2..(y + 6) * 2)
        .map(|dy| c.get((tip + 3) * 2, dy))
        .collect();
    assert!(
        column.iter().any(|&p| p != ink && p != face),
        "the arrow's slanted edges blend"
    );
}

#[test]
fn the_window_fits_its_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| {
        let mut dlg = ColorDlg::new(BASIC[0], custom());
        let narrow = form(&dlg);
        dlg.expand();
        vec![("Color".into(), narrow), ("Color, wide".into(), form(&dlg))]
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied())
    {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn looks() -> Vec<(String, Theme)> {
    vec![
        ("Classic".to_string(), Theme::default()),
        (
            "Blue".to_string(),
            interfaces::blue_theme(crate::blue::Variant::ALL[0]),
        ),
        ("Glass".to_string(), interfaces::theme_for(Interface::Glass)),
    ]
}

#[test]
fn color_dialog_pins_at_scale_1_and_2() {
    crate::pins::fonts();
    let narrow = ColorDlg::new(BASIC[13], custom());
    let wide = sample();
    let mut got = Vec::new();
    for (name, theme) in looks() {
        for (state, dlg) in [("narrow", &narrow), ("wide", &wide)] {
            for s in [1, 2] {
                got.push((format!("{name} {state} {s}"), fnv(&drawn(&theme, dlg, s))));
            }
        }
    }
    crate::pins::check("colordlg", &got);
}
