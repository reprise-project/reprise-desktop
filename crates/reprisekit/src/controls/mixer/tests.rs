use super::*;
use crate::interface::Interface;
use crate::interfaces::theme_for;

fn line(key: &str, volume: usize, balance: Option<usize>, check: Check) -> Line {
    Line {
        key: key.into(),
        title: key.into(),
        volume,
        balance,
        check,
        checked: false,
        live: true,
    }
}

/// The output device and two programs, the second mono.
fn lines() -> Vec<Line> {
    vec![
        line("master", 14, Some(10), Check::MuteAll),
        line("firefox", 16, Some(5), Check::Mute),
        line("mpv", 8, None, Check::Mute),
    ]
}

fn lay_in(m: &Mixer, theme: &Theme, body: Rect) -> Layout {
    let text = TextRenderer::load(&theme.fonts);
    layout(m, theme, &text, body)
}

fn lay(m: &Mixer) -> Layout {
    let (w, h) = m.size(TextUnits::ONE);
    lay_in(m, &Theme::default(), Rect::new(0, 20, w, h))
}

fn col(l: &Layout, i: usize) -> &ColumnLayout {
    l.columns[i].as_ref().expect("column shown")
}

fn mid(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

#[test]
fn a_readers_focus_moves_to_a_trackbar_or_box_without_moving_it() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let (x, y) = mid(col(&l, 1).volume.bounds);
    let volume = m.columns[1].volume.pos;
    assert_eq!(m.focus_at(&l, x, y), Event::Redraw);
    assert_eq!(m.focus, Focus::Volume(1));
    assert_eq!(m.columns[1].volume.pos, volume, "the thumb stays");
    assert_eq!(m.focus_at(&l, x, y), Event::None, "already there");
    let (x, y) = mid(col(&l, 2).check);
    assert_eq!(m.focus_at(&l, x, y), Event::Redraw);
    assert_eq!(m.focus, Focus::Check(2));
    assert!(!m.columns[2].checked, "the box is not flipped");
    let (x, y) = mid(col(&l, 2).balance.bounds);
    assert_eq!(
        m.focus_at(&l, x, y),
        Event::None,
        "a mono line's balance is greyed"
    );
}

#[test]
fn the_body_fits_its_columns() {
    assert_eq!(
        Mixer::new(&lines(), false).size(TextUnits::ONE),
        (135 + 2 * 105, 225)
    );
    assert_eq!(Mixer::new(&lines(), true).size(TextUnits::ONE), (345, 252));
    let rec = [line("mic", 10, None, Check::Select)];
    assert_eq!(
        Mixer::new(&rec, false).size(TextUnits::ONE),
        (106, 225),
        "never under 106"
    );
    let rec2 = [rec[0].clone(), line("mix", 10, None, Check::Select)];
    assert_eq!(Mixer::new(&rec2, false).size(TextUnits::ONE).0, 210);
    assert_eq!(Mixer::new(&lines()[..1], false).size(TextUnits::ONE).0, 135);
    assert_eq!(Mixer::new(&[], false).size(TextUnits::ONE).0, MIN_W);
}

#[test]
fn the_device_column_stacks_title_balance_volume_and_check() {
    let m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let c = col(&l, 0);
    assert_eq!(c.rect, Rect::new(0, 20, 135, 225));
    assert_eq!(c.title, Rect::new(6, 31, 123, 13));
    assert_eq!(c.underline, Some(Rect::new(6, 51, 121, 2)));
    assert_eq!(c.balance_label.y, 61);
    assert_eq!(c.balance.bounds.y, 79);
    assert_eq!(c.balance.channel, Rect::new(46, 86, 44, 4));
    assert_eq!(c.balance.ticks[BALANCE_CENTRE], 68, "centred under 67.5");
    assert_eq!(
        c.speakers,
        [Rect::new(22, 80, 16, 16), Rect::new(98, 80, 16, 16)]
    );
    assert_eq!(c.volume_label.y, 114);
    assert_eq!(c.volume.bounds, Rect::new(53, 137, 31, 75));
    assert_eq!(c.volume.channel.x, 66);
    assert_eq!(c.check, Rect::new(6, 223, 13, 13));
    assert_eq!(c.separator, None);
    assert_eq!(l.bar, None);
    assert_eq!(l.advanced, None);
}

#[test]
fn the_other_columns_follow_in_slots_and_share_a_title_line() {
    let m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let c = col(&l, 1);
    assert_eq!(c.rect.x, 140);
    assert_eq!(c.title.x, 146);
    assert_eq!(c.underline, None);
    assert_eq!(c.balance.channel.x, 169);
    assert_eq!(c.volume.channel.x, 189);
    assert_eq!(c.volume.bounds.x, 176);
    assert_eq!(c.check.x, 146);
    assert_eq!(
        c.separator,
        Some(Rect::new(137, 22, 2, 223)),
        "the line after the device column runs the body's height"
    );
    assert_eq!(col(&l, 2).rect.x, 245);
    assert_eq!(col(&l, 2).separator, Some(Rect::new(242, 61, 2, 184)));
    assert_eq!(l.title_line, Some(Rect::new(146, 51, 345 - 146, 2)));

    let rec = [
        line("mix", 10, Some(10), Check::Select),
        line("mic", 10, Some(10), Check::Select),
    ];
    let m = Mixer::new(&rec, false);
    let l = lay(&m);
    assert_eq!(col(&l, 0).rect.x, 5, "a slot with no line opens the body");
    assert_eq!(col(&l, 0).separator, None);
    assert_eq!(col(&l, 1).separator, Some(Rect::new(107, 61, 2, 184)));
    assert_eq!(l.title_line.map(|r| r.x), Some(11));
}

#[test]
fn the_volume_ticks_fall_every_tenth_position() {
    let m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let v = &col(&l, 0).volume;
    let ticks: Vec<i32> = (0..STEPS).step_by(10).map(|i| v.ticks[i]).collect();
    assert_eq!(ticks, [206, 196, 185, 174, 164, 153, 142]);
    assert_eq!(v.tick_every, 10);
}

#[test]
fn the_volume_thumb_takes_the_looks_width() {
    let m = Mixer::new(&lines(), false);
    let blue = theme_for(Interface::Blue);
    let thumb = blue.look.controls().vtrack_thumb;
    let (w, h) = m.size(TextUnits::ONE);
    let l = lay_in(&m, &blue, Rect::new(0, 0, w, h));
    let v = &col(&l, 0).volume;
    assert_eq!(v.thumb.width, thumb);
    assert_eq!(v.bounds.width, trackbar::vertical_width(thumb));
    assert_eq!(v.channel.x, 66, "the channel stays on the centre line");
    assert_eq!(
        v.channel.x - v.thumb.x,
        (thumb - v.channel.width) / 2,
        "the channel runs down the thumb's middle"
    );
}

#[test]
fn the_volume_channel_takes_the_height_the_body_leaves() {
    let m = Mixer::new(&lines(), false);
    let (w, h) = Mixer::new(&lines(), true).size(TextUnits::ONE);
    let l = lay_in(&m, &Theme::default(), Rect::new(0, 0, w, h));
    assert_eq!(
        col(&l, 0).volume.bounds.height,
        75 + 27,
        "no Advanced row: 27 px more"
    );
    let mut adv = Mixer::new(&lines(), true);
    let l = lay_in(&adv, &Theme::default(), Rect::new(0, 0, w, h));
    assert_eq!(col(&l, 0).volume.bounds.height, 75);
    assert_eq!(l.advanced.unwrap().bottom(), h - 5);
    adv.advanced = false;
    let l = lay_in(&adv, &Theme::default(), Rect::new(0, 0, w, 100));
    assert_eq!(col(&l, 0).volume.bounds.height, 40, "never under 40");
}

#[test]
fn advanced_controls_put_a_button_under_the_first_column() {
    let m = Mixer::new(&lines(), true);
    let l = lay(&m);
    assert_eq!(l.advanced, Some(Rect::new(6, 244, 83, 23)));
    assert_eq!(hit(&m, &l, 20, 250), Hit::Advanced);
}

#[test]
fn a_narrow_body_scrolls_the_columns() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay_in(&m, &Theme::default(), Rect::new(0, 0, 250, 260));
    assert_eq!(l.shown, 2);
    assert!(l.columns[2].is_none());
    let bar = l.bar.expect("a scrollbar");
    assert_eq!(bar.bottom(), 260);
    assert_eq!(bar.width, 250);
    let fwd = (bar.right() - 4, bar.y + 4);
    assert_eq!(m.press(&l, fwd.0, fwd.1), Event::Redraw);
    assert_eq!(m.first, 1);
    assert_eq!(m.press(&l, fwd.0, fwd.1), Event::Redraw);
    assert_eq!(m.first, 1, "stops with the last column shown");
}

#[test]
fn a_column_scrolled_out_publishes_beyond_the_view_and_a_reveal_scrolls_it_in() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay_in(&m, &Theme::default(), Rect::new(0, 0, 250, 260));
    assert_eq!(l.shown, 2);
    let out = l.beyond[2].as_ref().expect("column 2 placed");
    assert_eq!(out.rect.x, col(&l, 1).rect.right() + SLOT);
    assert!(out.rect.x >= l.view.right(), "right of the view");
    assert!(l.beyond[0].is_none() && l.beyond[1].is_none());
    let mut tree = Tree::window("Volume Control", (250, 260));
    m.access(&l, &mut tree, access::ROOT);
    let slider = tree
        .nodes()
        .filter(|(_, n)| n.role() == Role::Slider && n.bounds().is_some())
        .map(|(id, _)| id)
        .last()
        .expect("column 2's volume slider");
    let mut p = access::Published::default();
    p.update(tree);
    let r = p.reveal(slider).expect("out of view");
    assert_eq!(r.row, 2);
    assert_eq!(m.reveal(&l, r.row), Event::Redraw);
    assert_eq!(m.first, 1);
    assert_eq!(m.focus, Focus::Volume(0), "the focus stays");
    let l = lay_in(&m, &Theme::default(), Rect::new(0, 0, 250, 260));
    let left = l.beyond[0].as_ref().expect("column 0 placed");
    assert_eq!(left.rect.right(), l.view.x, "left of the view");
    assert_eq!(m.reveal(&l, 2), Event::None, "already shown");
    assert_eq!(m.reveal(&l, 0), Event::Redraw);
    assert_eq!(m.first, 0);
}

#[test]
fn a_body_that_grew_scrolls_back_on_the_first_press() {
    let five: Vec<Line> = (0..5)
        .map(|i| line(&format!("l{i}"), 10, Some(10), Check::Mute))
        .collect();
    let mut m = Mixer::new(&five, false);
    m.first = 3;
    let l = lay_in(&m, &Theme::default(), Rect::new(0, 0, 4 * COL_W, 300));
    assert_eq!(l.shown, 4);
    let bar = l.bar.expect("a scrollbar");
    assert_eq!(m.press(&l, bar.x + 4, bar.y + 4), Event::Redraw);
    assert_eq!(m.first, 0, "one press moves what shows");
}

#[test]
fn the_advanced_button_takes_no_key_once_its_column_greys() {
    let mut m = Mixer::new(&lines(), true);
    m.focus = Focus::Advanced;
    let l = lay(&m);
    let mut gone = lines();
    gone[0].live = false;
    m.sync(&l, &gone);
    assert_ne!(
        m.focus,
        Focus::Advanced,
        "the focus leaves the greyed button"
    );
    m.focus = Focus::Advanced;
    assert_ne!(m.key(&l, form::Key::Space), Event::Advanced);
    m.set_advanced(false);
    m.sync(&l, &lines());
    m.focus = Focus::Advanced;
    assert_ne!(
        m.key(&l, form::Key::Enter),
        Event::Advanced,
        "no button shows"
    );
}

#[test]
fn a_mono_line_greys_its_balance_at_the_centre() {
    let m = Mixer::new(&lines(), false);
    assert!(!m.columns[2].balance.enabled);
    assert_eq!(m.columns[2].balance.pos, BALANCE_CENTRE);
    assert!(m.columns[2].volume.enabled);
}

#[test]
fn dragging_the_balance_thumb_moves_it() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let thumb = mid(col(&l, 1).balance.thumb);
    assert_eq!(m.press(&l, thumb.0, thumb.1), Event::Redraw);
    let right = *col(&l, 1).balance.ticks.last().unwrap();
    assert_eq!(
        m.motion(&l, right, thumb.1),
        Event::Balance("firefox".into(), 20)
    );
    assert!(m.dragging());
    let _ = m.release();
    assert!(!m.dragging());
}

#[test]
fn a_press_on_the_volume_channel_steps_it() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let t = &col(&l, 0).volume;
    let above = (t.thumb.x + 5, t.thumb.y - 5);
    assert_eq!(
        m.press(&l, above.0, above.1),
        Event::Volume("master".into(), 15)
    );
    assert_eq!(m.focus, Focus::Volume(0));
}

#[test]
fn the_check_flips_on_release() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let (x, y) = mid(col(&l, 1).check);
    assert_eq!(m.press(&l, x, y), Event::Redraw);
    assert_eq!(m.release(), Event::Check("firefox".into(), true));
}

#[test]
fn select_is_one_of_many() {
    let rec = vec![
        Line {
            checked: true,
            ..line("mic", 10, Some(10), Check::Select)
        },
        line("line-in", 10, Some(10), Check::Select),
    ];
    let mut m = Mixer::new(&rec, false);
    m.focus = Focus::Check(0);
    let l = lay(&m);
    assert_eq!(m.key(&l, form::Key::Space), Event::Redraw, "a pick stays");
    m.focus = Focus::Check(1);
    assert_eq!(
        m.key(&l, form::Key::Space),
        Event::Check("line-in".into(), true)
    );
    assert!(!m.columns[0].checked && m.columns[1].checked);
}

#[test]
fn keys_move_the_focused_trackbars() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay(&m);
    assert_eq!(m.key(&l, form::Key::Up), Event::Volume("master".into(), 15));
    assert_eq!(
        m.key(&l, form::Key::Home),
        Event::Volume("master".into(), 60)
    );
    assert_eq!(
        m.key(&l, form::Key::PageDown),
        Event::Volume("master".into(), 48),
        "a page is a fifth"
    );
    m.focus = Focus::Balance(1);
    assert_eq!(
        m.key(&l, form::Key::Left),
        Event::Balance("firefox".into(), 4)
    );
    assert_eq!(
        m.key(&l, form::Key::End),
        Event::Balance("firefox".into(), 20)
    );
}

#[test]
fn tab_walks_balance_volume_check_and_skips_a_greyed_balance() {
    let mut m = Mixer::new(&lines(), true);
    let l = lay(&m);
    m.focus = Focus::Balance(0);
    let mut seen = vec![m.focus];
    for _ in 0..8 {
        m.key(&l, form::Key::Tab { back: false });
        seen.push(m.focus);
    }
    assert_eq!(
        seen,
        [
            Focus::Balance(0),
            Focus::Volume(0),
            Focus::Check(0),
            Focus::Advanced,
            Focus::Balance(1),
            Focus::Volume(1),
            Focus::Check(1),
            Focus::Volume(2),
            Focus::Check(2),
        ]
    );
}

#[test]
fn the_advanced_button_presses_on_release_and_on_space() {
    let mut m = Mixer::new(&lines(), true);
    let l = lay(&m);
    let (x, y) = mid(l.advanced.unwrap());
    assert_eq!(m.press(&l, x, y), Event::Redraw);
    assert_eq!(m.release(), Event::Advanced);
    assert_eq!(m.key(&l, form::Key::Space), Event::Advanced);
}

#[test]
fn the_wheel_steps_the_column_under_it() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let (x, y) = mid(col(&l, 1).rect);
    assert_eq!(
        m.scroll(&l, x, y, WHEEL_ROWS),
        Event::Volume("firefox".into(), 15)
    );
}

#[test]
fn sync_keeps_the_focus_and_a_held_thumb_by_key() {
    let mut m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let thumb = mid(col(&l, 1).volume.thumb);
    m.press(&l, thumb.0, thumb.1);
    let top = col(&l, 1).volume.ticks[20];
    m.motion(&l, thumb.0, top);
    let mut moved = lines();
    moved.swap(1, 2);
    moved[2].volume = 3;
    let l2 = lay(&m);
    m.sync(&l2, &moved);
    assert_eq!(m.focus, Focus::Volume(2), "the focus follows firefox");
    assert_eq!(m.columns[2].volume.pos, 20, "the held thumb stays put");
    assert!(m.dragging());
    m.sync(&l2, &moved[..2]);
    assert_eq!(
        m.focus,
        Focus::Volume(0),
        "a line that left gives up the focus"
    );
    assert!(!m.dragging());
}

#[test]
fn a_greyed_column_takes_nothing() {
    let mut ls = lines();
    ls[0].live = false;
    let mut m = Mixer::new(&ls, false);
    let l = lay(&m);
    let (x, y) = mid(col(&l, 0).volume.thumb);
    assert_eq!(m.press(&l, x, y), Event::None);
    let (x, y) = mid(col(&l, 0).check);
    m.press(&l, x, y);
    assert_eq!(m.release(), Event::None);
    assert!(!m.columns[0].balance.enabled);
}

#[test]
fn an_empty_mixer_takes_every_key_and_the_pointer() {
    let mut m = Mixer::new(&[], true);
    let l = lay(&m);
    use form::Key as K;
    for k in [
        K::Up,
        K::Down,
        K::Left,
        K::Right,
        K::PageUp,
        K::PageDown,
        K::Home,
        K::End,
        K::Space,
        K::Enter,
        K::Tab { back: false },
        K::Tab { back: true },
    ] {
        assert!(
            matches!(m.key(&l, k.clone()), Event::None | Event::Redraw),
            "{k:?} acts on nothing"
        );
    }
    m.focus = Focus::Advanced;
    assert_eq!(m.key(&l, K::Space), Event::None, "no column, no button");
    let (x, y) = mid(l.body);
    assert_eq!(m.press(&l, x, y), Event::None);
    assert_eq!(m.motion(&l, x, y + 5), Event::None);
    assert_eq!(m.release(), Event::None);
    assert_eq!(m.scroll(&l, x, y, WHEEL_ROWS), Event::None);
    let mut tree = Tree::window("Volume Control", (l.body.width, l.body.height));
    assert_eq!(m.access(&l, &mut tree, access::ROOT), access::ROOT);
    let mut full = Mixer::new(&lines(), true);
    full.focus = Focus::Advanced;
    full.sync(&l, &[]);
    assert_eq!(
        full.focus,
        Focus::Volume(0),
        "the button left with its column"
    );
}

#[test]
fn balance_positions_map_to_minus_one_through_one() {
    assert_eq!(balance_pos(0.0), 10);
    assert_eq!(balance_pos(-1.0), 0);
    assert_eq!(balance_pos(0.5), 15);
    assert_eq!(balance_of(0), -1.0);
    assert_eq!(balance_of(10), 0.0);
    assert_eq!(balance_of(20), 1.0);
}

fn drawn(theme: &Theme, m: &Mixer) -> (Canvas, Layout) {
    let (w, h) = m.size(theme.text);
    let l = lay_in(m, theme, Rect::new(0, 0, w, h));
    let text = TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new(w as u32, h as u32);
    draw(&mut c, theme, &text, m, &l);
    (c, l)
}

fn inked(c: &Canvas, r: Rect, ink: u32) -> usize {
    (r.x..r.right())
        .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
        .filter(|&(x, y)| c.get(x, y) == ink)
        .count()
}

/// At 150% and 200% the columns, their text rows, the channels and the Advanced button grow
/// with the text; the check box, the speakers, the thumbs and the slots keep their size, and
/// every label is inked inside its grown row.
#[test]
fn a_grown_text_size_grows_the_columns_and_keeps_the_glyphs() {
    for (pct, px) in [(150, 17), (200, 22)] {
        let u = TextUnits::new(px, 11);
        let theme = Theme::default().with_text_size(pct);
        assert_eq!(theme.text, u);
        let m = Mixer::new(&lines(), true);
        let (w, h) = m.size(u);
        assert_eq!(w, u.len(DEVICE_W) + 2 * (SLOT + u.len(100)), "{pct}%");
        let body_h = u.len(TRACK_Y) + u.len(TRACK_H) + u.len(CHECK_GAP) + u.len(ROW_H);
        assert_eq!(h, body_h + u.len(BOTTOM) + u.len(ADVANCED_ROW), "{pct}%");
        assert_eq!(min_width(u), u.len(MIN_W), "{pct}%");
        let (c, l) = drawn(&theme, &m);
        let d = col(&l, 0);
        assert_eq!(
            d.title,
            Rect::new(
                u.len(PAD),
                u.len(TITLE_Y),
                u.len(DEVICE_W) - 2 * u.len(PAD),
                u.len(ROW_H)
            ),
            "{pct}%"
        );
        assert_eq!(d.volume.bounds.height, u.len(TRACK_H), "{pct}%");
        assert_eq!(
            d.volume.thumb.width,
            crate::metrics::ControlMetrics::CLASSIC.vtrack_thumb,
            "{pct}%"
        );
        assert_eq!(d.balance.channel.width, u.len(BALANCE_W), "{pct}%");
        assert_eq!(d.speakers[0].width, SPEAKER, "{pct}%");
        assert_eq!((d.check.width, d.check.height), (13, 13), "{pct}%");
        assert_eq!(
            d.check.y - d.check_label.y,
            (u.len(ROW_H) - ROW_H) / 2,
            "{pct}%: the box centred down its row"
        );
        assert_eq!(col(&l, 1).rect.x, u.len(DEVICE_W) + SLOT, "{pct}%");
        let adv = l.advanced.expect("Advanced on");
        assert_eq!(
            (adv.width, adv.height),
            (u.len(ADVANCED_W), u.len(ADVANCED_H)),
            "{pct}%"
        );
        assert_eq!(
            hit(&m, &l, adv.x + 2, adv.bottom() - 2),
            Hit::Advanced,
            "{pct}%"
        );
        assert_eq!(
            hit(
                &m,
                &l,
                d.check_label.right() - 1,
                d.check_label.bottom() - 1
            ),
            Hit::Check(0),
            "{pct}%"
        );
        let ink = theme.colors.button_text.argb();
        for r in [d.title, d.balance_label, d.volume_label, d.check_label] {
            let around = Rect::new(r.x, r.y - 4, r.width, r.height + 8);
            assert!(inked(&c, r, ink) > 0, "{pct}%: {r:?} holds its label");
            assert_eq!(
                inked(&c, around, ink),
                inked(&c, r, ink),
                "{pct}%: {r:?} holds all of it"
            );
        }
    }
}

#[test]
fn draws_the_labels_trackbars_checks_and_separators() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let m = Mixer::new(&lines(), false);
    let (c, l) = drawn(&theme, &m);
    assert_eq!(c.get(1, 5), cs.button_face.argb(), "the body is the face");
    let etched = |x, y| (c.get(x, y), c.get(x, y + 1));
    let line = (cs.button_shadow.argb(), cs.button_hilight.argb());
    assert_eq!(etched(1, 0), line, "the line along the top");
    let u = col(&l, 0).underline.unwrap();
    assert_eq!(etched(u.x, u.y), line, "the device's title line");
    assert_eq!(
        c.get(u.right() - 1, u.y),
        cs.button_hilight.argb(),
        "turning up at its end"
    );
    let t = l.title_line.unwrap();
    assert_eq!(etched(t.x, t.y), line, "the programs' title line");
    assert_eq!(etched(t.right() - 1, t.y), line);
    for i in 0..3 {
        let cl = col(&l, i);
        let text = cs.button_text.argb();
        assert!(inked(&c, cl.title, text) > 0, "column {i} title");
        assert!(inked(&c, cl.volume_label, text) > 0, "column {i} Volume:");
        assert!(
            inked(&c, cl.check_label, text) > 0,
            "column {i} check label"
        );
        assert_eq!(
            c.get(cl.volume.channel.x, cl.volume.channel.y + 2),
            cs.button_shadow.argb(),
            "column {i} volume channel"
        );
        assert_eq!(
            c.get(cl.balance.channel.x + 2, cl.balance.channel.y),
            cs.button_shadow.argb(),
            "column {i} balance channel"
        );
        assert_eq!(
            c.get(cl.check.x + 2, cl.check.y + 2),
            cs.window.argb(),
            "column {i} check box well"
        );
    }
    let s = col(&l, 1).separator.unwrap();
    assert_eq!(c.get(s.x, s.y + 50), cs.button_shadow.argb());
    assert_eq!(c.get(s.x + 1, s.y + 50), cs.button_hilight.argb());
    let mono = col(&l, 2);
    assert_eq!(
        inked(&c, mono.balance_label, cs.button_text.argb()),
        0,
        "a mono line's Balance: is greyed"
    );
    assert!(inked(&c, mono.balance_label, cs.button_shadow.argb()) > 0);
    assert!(
        inked(&c, col(&l, 1).balance_label, cs.button_text.argb()) > 0,
        "a stereo line's Balance: is live"
    );
}

#[test]
fn the_focus_rings_frame_each_part() {
    let m = Mixer::new(&lines(), false);
    let l = lay(&m);
    let c = col(&l, 0);
    assert_eq!(balance_ring(&c.balance), Rect::new(38, 77, 60, 28));
    assert_eq!(volume_ring(&c.volume), Rect::new(48, 129, 45, 91));
    let r = check_ring(c.check_label);
    assert_eq!((r.x, r.y, r.height), (c.check_label.x - 1, 222, 16));
}

#[test]
fn the_focus_ring_shows_once_keys_are_used() {
    let theme = Theme::default();
    let mut m = Mixer::new(&lines(), false);
    m.focus = Focus::Check(1);
    let (c, l) = drawn(&theme, &m);
    let ring = check_ring(col(&l, 1).check_label);
    let ink = theme.colors.button_text.argb();
    let top = |c: &Canvas| {
        (ring.x..ring.right())
            .filter(|&x| c.get(x, ring.y) == ink)
            .count()
    };
    assert_eq!(top(&c), 0);
    m.cues = true;
    let (c, _) = drawn(&theme, &m);
    assert!(top(&c) > 0, "the dotted ring is drawn");
}

/// A 16 px speaker for the tests: its left half dark, its right half
/// light, so a mirrored one shows.
fn speaker() -> Canvas {
    use crate::theme::Color;
    let mut i = Canvas::new(16, 16);
    i.fill_rect(Rect::new(0, 0, 8, 16), Color::rgb(0, 0, 128));
    i.fill_rect(Rect::new(8, 0, 8, 16), Color::rgb(0, 255, 255));
    i
}

#[test]
fn speakers_face_out_from_the_balance_ends_and_fade_with_it() {
    use crate::theme::Color;
    let theme = Theme::default();
    let mut m = Mixer::new(&lines(), false);
    let (c, l) = drawn(&theme, &m);
    let [left, right] = col(&l, 0).speakers;
    let face = theme.colors.button_face.argb();
    assert_eq!(
        inked(&c, left, face) + inked(&c, right, face),
        2 * 16 * 16,
        "no icon, nothing drawn"
    );
    m.speaker = Some(speaker());
    let (c, l) = drawn(&theme, &m);
    let dark = Color::rgb(0, 0, 128).argb();
    let light = Color::rgb(0, 255, 255).argb();
    for i in [0, 1] {
        let [left, right] = col(&l, i).speakers;
        assert_eq!(
            (c.get(left.x, left.y), c.get(left.x + 15, left.y)),
            (light, dark),
            "the left one mirrored"
        );
        assert_eq!(
            (c.get(right.x, right.y), c.get(right.x + 15, right.y)),
            (dark, light)
        );
    }
    let [left, right] = col(&l, 2).speakers;
    for (x, y) in [(left.x, left.y), (right.x + 15, right.y)] {
        assert_ne!(c.get(x, y), dark, "a mono line's speakers fade");
        assert_ne!(c.get(x, y), light, "a mono line's speakers fade");
    }
}

/// Writes target/mixer-preview.png, one body under another: the
/// output device and three programs in Classic, the device's volume
/// at 22 and the rest at the top; the same with Advanced Controls on
/// and the second balance focused; the recording lines with one
/// greyed; then the playback lines in Blue and in Glass.
#[test]
#[ignore]
fn render_mixer() {
    let mut four = vec![
        line("Master Volume", 22, Some(10), Check::MuteAll),
        line("Wave", 60, Some(10), Check::Mute),
        line("SW Synth", 60, Some(10), Check::Mute),
        line("CD Player", 60, Some(10), Check::Mute),
    ];
    four[0].title = "Master Volume".into();
    let mut plain = Mixer::new(&four, false);
    plain.speaker = Some(speaker());
    let mut focused = Mixer::new(&lines(), true);
    focused.focus = Focus::Balance(1);
    focused.cues = true;
    focused.speaker = Some(speaker());
    let mut rec = vec![
        Line {
            checked: true,
            ..line("Microphone", 12, Some(10), Check::Select)
        },
        line("Line In", 6, Some(14), Check::Select),
        line("Monitor", 0, None, Check::Select),
    ];
    rec[2].live = false;
    let cases = [
        (Theme::default(), plain),
        (Theme::default(), focused),
        (Theme::default(), Mixer::new(&rec, false)),
        (theme_for(Interface::Blue), Mixer::new(&lines(), true)),
        (theme_for(Interface::Glass), Mixer::new(&lines(), true)),
    ];
    let w = cases
        .iter()
        .map(|(_, m)| m.size(TextUnits::ONE).0)
        .max()
        .unwrap_or(0);
    let h: i32 = cases.iter().map(|(_, m)| m.size(TextUnits::ONE).1).sum();
    let mut c = Canvas::new(w as u32, h as u32);
    let mut y = 0;
    for (theme, m) in &cases {
        let (mw, mh) = m.size(TextUnits::ONE);
        let text = TextRenderer::load(&theme.fonts);
        let l = layout(m, theme, &text, Rect::new(0, y, mw, mh));
        draw(&mut c, theme, &text, m, &l);
        y += mh;
    }
    crate::raster::write_preview(&c, "mixer-preview.png");
}
