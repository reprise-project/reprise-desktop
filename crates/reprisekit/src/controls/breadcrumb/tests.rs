use super::*;
use crate::interface::Interface;
use crate::interfaces;
use crate::theme::{Color, Theme};

const FIELD: Rect = Rect::new(10, 4, 400, HEIGHT);
const PX: u32 = 11;

/// Every character 6 px wide, so a label's width is 6 per character.
fn stub() -> TextRenderer {
    TextRenderer::stub(PX, |_| 6)
}

fn crumb(label: &str, target: &str) -> Crumb {
    Crumb {
        label: label.to_string(),
        target: target.to_string(),
    }
}

fn home_music() -> Breadcrumb {
    let mut b = Breadcrumb::new();
    let crumbs = vec![
        crumb("Home", "/home/u"),
        crumb("Music", "/home/u/Music"),
        crumb("Albums", "/home/u/Music/Albums"),
    ];
    b.set_place(crumbs, None, "/home/u/Music/Albums");
    b
}

fn cm() -> ControlMetrics {
    glass(crate::glass::Mode::Light)
        .look
        .controls_at_100()
        .grown(TextUnits::default())
}

fn lay(b: &Breadcrumb, field: Rect) -> Layout {
    layout(b, field, &stub(), PX, &cm())
}

fn glass(mode: crate::glass::Mode) -> Theme {
    let g = crate::glass::Settings {
        mode,
        ..Default::default()
    };
    interfaces::theme_with(Interface::Glass, Default::default(), false, &g)
}

fn mid(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

#[test]
fn segments_lay_out_left_to_right_with_a_chevron_after_each() {
    let b = home_music();
    let l = lay(&b, FIELD);
    assert_eq!(l.folded, 0);
    assert_eq!(l.segments.len(), 3);
    let first = l.overflow.right();
    assert_eq!(
        l.segments[0].button.x, first,
        "the first segment follows the lead chevron"
    );
    for (seg, label) in l.segments.iter().zip(["Home", "Music", "Albums"]) {
        assert_eq!(seg.button.width, 6 * label.len() as i32 + 2 * SEG_PAD);
        assert_eq!(
            seg.chevron.x,
            seg.button.right() - 1,
            "chevron on the button's edge"
        );
        assert_eq!(seg.chevron.width, CHEVRON_W);
    }
    for w in l.segments.windows(2) {
        assert_eq!(w[1].button.x, w[0].chevron.right());
    }
    assert_eq!(l.refresh.right(), FIELD.right());
    assert_eq!(
        l.frame.right() - 1,
        l.refresh.x,
        "frame and Refresh share a column"
    );
    assert_eq!(l.history.right() - 1, l.refresh.x);
    assert_eq!(l.empty.x, l.segments[2].chevron.right());
    assert_eq!(l.empty.right(), l.history.x);
}

#[test]
fn narrow_crumbs_fold_leading_segments_into_the_overflow_menu() {
    let b = home_music();
    // The stub's segments with their chevrons take 48, 54 and 60 px; the icon, the lead chevron,
    // the history drop and Refresh take 80.
    assert_eq!(
        lay(
            &b,
            Rect {
                width: 242,
                ..FIELD
            }
        )
        .folded,
        0
    );
    let two = lay(
        &b,
        Rect {
            width: 241,
            ..FIELD
        },
    );
    assert_eq!(two.folded, 1, "Home folds");
    assert_eq!(
        two.segments.iter().map(|s| s.crumb).collect::<Vec<_>>(),
        [1, 2]
    );
    let tiny = lay(
        &b,
        Rect {
            width: 130,
            ..FIELD
        },
    );
    assert_eq!(tiny.folded, 2, "only the last segment shows");
    let last = tiny.segments[0];
    assert_eq!(last.crumb, 2);
    assert!(
        last.chevron.right() <= tiny.history.x,
        "the last segment is cut to fit"
    );
    assert_eq!(
        hit(&b, &tiny, mid(tiny.overflow).0, mid(tiny.overflow).1),
        Hit::Overflow
    );
}

#[test]
fn a_segment_click_goes_to_its_target() {
    let mut b = home_music();
    let l = lay(&b, FIELD);
    let (x, y) = mid(l.segments[1].button);
    let h = hit(&b, &l, x, y);
    assert_eq!(h, Hit::Segment(1));
    assert_eq!(b.press(h, x, &l, &stub(), PX, 1), Event::Redraw);
    assert_eq!(lit(&b, Part::Segment(1)), Some(State::Pressed));
    assert_eq!(b.release(h), Event::Go("/home/u/Music".to_string()));
    b.press(h, x, &l, &stub(), PX, 1);
    assert_eq!(
        b.release(Hit::Empty),
        Event::Redraw,
        "released off the segment"
    );
}

#[test]
fn a_chevron_press_opens_its_menu_and_draws_it_held_until_closed() {
    let mut b = home_music();
    let l = lay(&b, FIELD);
    let (x, y) = mid(l.segments[0].chevron);
    let h = hit(&b, &l, x, y);
    assert_eq!(h, Hit::Chevron(0));
    assert_eq!(b.press(h, x, &l, &stub(), PX, 1), Event::OpenChevron(0));
    assert_eq!(lit(&b, Part::Chevron(0)), Some(State::Pressed));
    assert_eq!(lit(&b, Part::Segment(0)), Some(State::Pressed));
    b.menu_closed();
    assert_eq!(lit(&b, Part::Chevron(0)), None);
}

#[test]
fn a_lit_chevron_greys_its_segment_and_a_lit_segment_lights_its_chevron() {
    let mut b = home_music();
    b.hover(Hit::Chevron(1));
    assert_eq!(lit(&b, Part::Chevron(1)), Some(State::Hot));
    assert_eq!(lit(&b, Part::Segment(1)), Some(State::Checked));
    b.hover(Hit::Segment(1));
    assert_eq!(lit(&b, Part::Segment(1)), Some(State::Hot));
    assert_eq!(lit(&b, Part::Chevron(1)), Some(State::Hot));
    assert_eq!(lit(&b, Part::Segment(0)), None);
    assert!(b.leave());
    assert_eq!(lit(&b, Part::Segment(1)), None);
}

#[test]
fn a_click_on_empty_field_starts_edit_with_text_selected() {
    for spot in [Hit::Empty, Hit::Icon] {
        let mut b = home_music();
        let l = lay(&b, FIELD);
        let (x, y) = match spot {
            Hit::Empty => mid(l.empty),
            _ => (FIELD.x + 4, mid(FIELD).1),
        };
        assert_eq!(hit(&b, &l, x, y), spot);
        b.press(spot, x, &l, &stub(), PX, 1);
        assert!(b.editing(), "{spot:?}");
        let edit = b.edit.combo.edit.as_ref().expect("an editable field");
        assert_eq!(edit.text, "/home/u/Music/Albums");
        assert_eq!(
            edit.selection(),
            0..edit.text.len(),
            "{spot:?}: all selected"
        );
    }
}

#[test]
fn esc_in_edit_returns_to_crumbs_with_the_old_place() {
    let mut b = home_music();
    b.start_edit();
    b.key(EditKey::Insert("/tmp".to_string()));
    assert_eq!(b.edit.text(), "/tmp");
    assert_eq!(b.list_key(&Key::Escape), Some(Event::Redraw));
    assert!(!b.editing());
    assert_eq!(b.edit.text(), "/home/u/Music/Albums");
    assert_eq!(
        b.list_key(&Key::Escape),
        None,
        "Esc outside edit mode is the host's"
    );
}

#[test]
fn focus_loss_ends_edit_mode() {
    let mut b = home_music();
    b.start_edit();
    b.key(EditKey::Insert("/tmp".to_string()));
    assert!(b.focus_lost());
    assert!(!b.editing());
    assert_eq!(b.edit.text(), "/home/u/Music/Albums");
    assert!(!b.focus_lost());
}

#[test]
fn enter_in_edit_goes_and_returns_to_crumbs() {
    let mut b = home_music();
    b.start_edit();
    b.key(EditKey::Insert("/tmp".to_string()));
    assert_eq!(b.list_key(&Key::Enter), Some(Event::Go("/tmp".to_string())));
    assert!(!b.editing());
}

#[test]
fn the_history_drop_opens_the_edit_with_its_list() {
    let mut b = home_music();
    b.set_history(vec![ListItem::new("/tmp"), ListItem::new("/etc")]);
    let l = lay(&b, FIELD);
    let (x, y) = mid(l.history);
    let h = hit(&b, &l, x, y);
    assert_eq!(h, Hit::History);
    assert_eq!(b.press(h, x, &l, &stub(), PX, 1), Event::OpenHistory);
    assert!(b.editing());
    assert!(b.edit.combo.open.is_some(), "the list is open");
    assert_eq!(b.list_key(&Key::Escape), Some(Event::Redraw));
    assert!(b.editing(), "the first Esc closes the list only");
}

#[test]
fn refresh_fires_on_release_over_it() {
    let mut b = home_music();
    let l = lay(&b, FIELD);
    let (x, y) = mid(l.refresh);
    let h = hit(&b, &l, x, y);
    assert_eq!(h, Hit::Refresh);
    b.press(h, x, &l, &stub(), PX, 1);
    assert_eq!(b.release(h), Event::Refresh);
}

#[test]
fn access_names_each_segment_and_its_chevron() {
    let b = home_music();
    let l = lay(&b, FIELD);
    let mut tree = Tree::window("Explorer", (420, 30));
    access(&b, &l, &stub(), PX, &mut tree, access::ROOT);
    let names: Vec<String> = tree
        .nodes()
        .filter(|(_, n)| n.role() == Role::Button)
        .filter_map(|(_, n)| n.label().map(str::to_string))
        .collect();
    for want in [
        "Earlier Locations",
        "Home",
        "Folders in Home",
        "Music",
        "Folders in Music",
        "Albums",
        "Folders in Albums",
        "Previous Locations",
        "Refresh",
    ] {
        assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
    }
}

/// The states the pins draw: the name, then how to bring the breadcrumb there.
fn states() -> Vec<(&'static str, Breadcrumb)> {
    let with = |f: &dyn Fn(&mut Breadcrumb)| {
        let mut b = home_music();
        f(&mut b);
        b
    };
    vec![
        ("rest", home_music()),
        ("field hot", with(&|b| b.field_hot = true)),
        (
            "segment hot",
            with(&|b| {
                b.hover(Hit::Segment(1));
            }),
        ),
        (
            "segment pressed",
            with(&|b| b.pressed = Some(Part::Segment(1))),
        ),
        (
            "chevron hot",
            with(&|b| {
                b.hover(Hit::Chevron(1));
            }),
        ),
        (
            "chevron open",
            with(&|b| b.open_menu = Some(Part::Chevron(1))),
        ),
        (
            "refresh hot",
            with(&|b| {
                b.hover(Hit::Refresh);
            }),
        ),
        (
            "history hot",
            with(&|b| {
                b.hover(Hit::History);
            }),
        ),
        ("editing", with(&Breadcrumb::start_edit)),
    ]
}

fn drawn(theme: &Theme, b: &Breadcrumb, field: Rect, s: i32) -> Canvas {
    let text = TextRenderer::load(&theme.fonts);
    let px1 = theme.fonts.size_px;
    let cm = theme.controls();
    let l = layout(b, field, &text, px1, &cm);
    let scaled = theme.scaled(s);
    let p = scaled.paint();
    let mut c = Canvas::new(
        ((field.right() + 10) * s) as u32,
        ((field.bottom() + 4) * s) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(0x4A, 0x7A, 0xB0));
    let caret = Caret::new(b.editing(), true);
    draw(
        &mut c,
        &text,
        &p,
        b,
        &l,
        b.editing(),
        caret,
        scaled.fonts.size_px,
    );
    c
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

#[test]
fn breadcrumb_states_pin_in_glass_light_and_dark_at_scale_1_and_2() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
        let theme = glass(mode);
        for (state, b) in states() {
            for s in [1, 2] {
                got.push((
                    format!("{mode:?} {state} {s}"),
                    fnv(&drawn(&theme, &b, FIELD, s)),
                ));
            }
        }
        let narrow = Rect {
            width: 150,
            ..FIELD
        };
        for s in [1, 2] {
            got.push((
                format!("{mode:?} overflow {s}"),
                fnv(&drawn(&theme, &home_music(), narrow, s)),
            ));
        }
    }
    crate::pins::check("controls/breadcrumb", &got);
}

#[test]
fn every_pinned_state_draws_differently() {
    crate::pins::fonts();
    let theme = glass(crate::glass::Mode::Light);
    let drawn: Vec<(&str, Vec<u32>)> = states()
        .into_iter()
        .map(|(name, b)| (name, drawn(&theme, &b, FIELD, 1).pixels))
        .collect();
    for (i, (a, pa)) in drawn.iter().enumerate() {
        for (b, pb) in &drawn[i + 1..] {
            assert_ne!(pa, pb, "{a} and {b} draw the same");
        }
    }
}
