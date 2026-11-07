use super::*;
use crate::blue::powerpanel::TILE_STATES;
use crate::interface::Interface;
use crate::interfaces::blue_theme;
use crate::logon::{LogonKey, LogonUser, PowerAction, hit};
use crate::style::TextSpec;

const OUTPUT: Rect = Rect::new(0, 0, 1024, 768);

fn fnv(c: &Canvas) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in &c.pixels {
        for b in p.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

fn alpha(p: u32) -> u32 {
    p >> 24
}

fn luma(p: u32) -> u32 {
    let [b, g, r, _] = p.to_le_bytes();
    (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000
}

fn users() -> Vec<LogonUser> {
    [
        ("alice", "Alice Liddell"),
        ("bob", ""),
        ("carol", "Carol Shaw"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, real))| LogonUser {
        name: name.into(),
        real_name: real.into(),
        picture: (i == 0).then(|| userpic::draw(userpic::ALL[5], 96)),
        note: (i == 1).then_some(UserNote::LoggedOn),
    })
    .collect()
}

fn states() -> Vec<(&'static str, Logon)> {
    let rest = Logon::new(Interface::Blue, users(), true, Some("carol"));
    let mut hot = rest.clone();
    hot.lit = Some((Part::Tile(0), Interaction::Hot));
    let mut open = rest.clone();
    open.click(Part::Tile(1));
    let mut message = open.clone();
    message.key(LogonKey::Text("x"));
    message.fail(strings::incorrect());
    message.caps_lock = true;
    message.layout = Some("EN".into());
    let mut busy = open.clone();
    busy.key(LogonKey::Text("hunter2"));
    busy.set_busy(true);
    let typed = Logon::new(Interface::Blue, Vec::new(), false, None);
    vec![
        ("tiles at rest", rest),
        ("tile hot", hot),
        ("tile open empty", open),
        ("tile open with message", message),
        ("busy", busy),
        ("typed", typed),
        ("admin shut down", admin(PowerAction::PowerOff, "")),
        ("admin restart filled", admin(PowerAction::Reboot, "anna")),
        ("shutting down", going_down()),
        ("remote note", remote()),
    ]
}

/// The administrator's form for `action` over the tiles, with `name` typed and a password.
fn admin(action: PowerAction, name: &str) -> Logon {
    let mut m = Logon::new(Interface::Blue, users(), true, Some("carol"));
    m.admin(action);
    if !name.is_empty() {
        m.key(LogonKey::Text(name));
        m.key(LogonKey::Enter);
        m.key(LogonKey::Text("hunter2"));
    }
    m
}

fn going_down() -> Logon {
    let mut m = admin(PowerAction::PowerOff, "anna");
    m.going_down();
    m
}

/// The tiles with the first user connected remotely and the second logged on here.
fn remote() -> Logon {
    let mut m = Logon::new(Interface::Blue, users(), true, Some("carol"));
    m.users[0].note = Some(UserNote::Remote);
    m
}

/// Draws the wordmark in Tahoma Bold, since its own faces are whichever the machine installs.
fn drawn(m: &Logon, v: Variant, s: i32) -> (Canvas, LogonLayout) {
    let theme = blue_theme(v).scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let title = TextRenderer::load(
        &TextSpec {
            family: vec!["Tahoma".into()],
            family_sizes: Vec::new(),
            ..title_text()
        }
        .font(),
    );
    let l = layout(m, &theme, &text, OUTPUT);
    let mut c = Canvas::new((OUTPUT.width * s) as u32, (OUTPUT.height * s) as u32);
    draw_titled(&mut c, m, &l, &theme, text.for_output(s), &title, s);
    (c, l)
}

#[test]
fn blue_logon_pins_per_scheme_at_scale_1_and_2() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for v in Variant::ALL {
        for (name, m) in states() {
            for s in [1, 2] {
                got.push((format!("{v:?} {name} @{s}"), fnv(&drawn(&m, v, s).0)));
            }
        }
    }
    crate::pins::check("blue/logon", &got);
}

#[test]
fn blue_tiles_have_clear_corners_and_soft_edges_at_2() {
    let frame = Color::rgb(0xFF, 0xFF, 0xFF);
    for d in [1, 2] {
        let pic = userpic::draw(userpic::ALL[0], (PICTURE * d) as u32);
        let f = framed(&pic, frame, d);
        let n = FRAMED * d;
        assert_eq!((f.width, f.height), (n as u32, n as u32), "54 design px");
        for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
            assert!(alpha(f.get(x, y)) < 40, "d={d}: corner ({x}, {y}) is clear");
        }
        assert_eq!(f.get(0, n / 2), frame.argb(), "d={d}: the rim");
        assert_eq!(
            alpha(f.get(n / 2, n / 2)),
            255,
            "d={d}: the picture is opaque"
        );

        let all: Vec<Canvas> = TILE_STATES.iter().map(|&st| go(st, d)).collect();
        for (i, a) in all.iter().enumerate() {
            assert!(
                alpha(a.get(0, 0)) < 40,
                "d={d}: the arrow's corner is clear"
            );
            for b in &all[i + 1..] {
                assert_ne!(a.pixels, b.pixels, "d={d}: arrow states differ");
            }
        }
        let mid = GO * d / 2;
        assert_eq!(alpha(all[0].get(mid, 2 * d)), 255, "the arrow is opaque");
        assert!(
            alpha(all[4].get(mid, 2 * d)) < 128,
            "a greyed arrow lets the field through"
        );

        if d == 2 {
            let soft = |c: &Canvas| {
                (0..8)
                    .flat_map(|y| (0..8).map(move |x| (x, y)))
                    .filter(|&(x, y)| (1..255).contains(&alpha(c.get(x, y))))
                    .count()
            };
            assert!(soft(&f) >= 4, "the frame's corner is smooth at 2");
            assert!(soft(&all[0]) >= 4, "the arrow's edge is smooth at 2");
        }
    }
}

#[test]
fn a_tiles_band_is_rounded_on_the_left_and_fades_out_to_the_right() {
    let sc = Variant::Blue.scheme();
    for d in [1, 2] {
        let b = band(sc, 200, TILE_H, d);
        let (w, h) = (200 * d, TILE_H * d);
        assert!(
            alpha(b.get(0, 0)) < 40,
            "d={d}: the top-left corner is clear"
        );
        assert_eq!(
            b.get(0, h / 2),
            sc.welcome_tile_hot.argb(),
            "d={d}: the hot colour at the left"
        );
        assert_eq!(alpha(b.get(w - 1, 0)), 0, "d={d}: the right end is gone");
    }
}

#[test]
fn the_bands_and_the_field_take_each_schemes_colours() {
    for v in Variant::ALL {
        let sc = v.scheme();
        let b = backdrop(sc, 400, 600, 1);
        assert_eq!(b.get(200, 10), sc.welcome_band.argb(), "{v:?}: top band");
        assert_eq!(
            b.get(200, 590),
            sc.welcome_band.argb(),
            "{v:?}: bottom band"
        );
        assert_ne!(
            b.get(200, TOP_H - 1),
            sc.welcome_band.argb(),
            "{v:?}: the top band's lit rule"
        );
        assert!(
            luma(b.get(80, 200)) > luma(b.get(399, 500)),
            "{v:?}: the glow side is brighter than the far side"
        );
    }
}

#[test]
fn blue_hit_finds_tiles_and_turn_off() {
    let mid = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
    let states = states();
    let (_, rest) = &states[0];
    let theme = blue_theme(Variant::Blue);
    let text = TextRenderer::load(&theme.fonts);
    let l = layout(rest, &theme, &text, OUTPUT);
    let split = OUTPUT.width / 2;
    for i in 0..3 {
        let r = l.rect(Part::Tile(i)).expect("each user has a tile");
        assert!(r.x > split, "tile {i} sits right of the middle rule");
        let (x, y) = mid(r);
        assert_eq!(hit(&l, x, y), Some(Part::Tile(i)));
    }
    let power = l.rect(Part::Power).unwrap();
    assert!(power.y >= OUTPUT.bottom() - BOTTOM_H, "in the bottom band");
    let (x, y) = mid(power);
    assert_eq!(hit(&l, x, y), Some(Part::Power));
    assert_eq!(hit(&l, split / 2, OUTPUT.height / 2), None, "the mark side");

    let (_, open) = &states[2];
    let l = layout(open, &theme, &text, OUTPUT);
    let tile = l.rect(Part::Tile(1)).unwrap();
    for part in [Part::Password, Part::Submit] {
        let r = l.rect(part).unwrap();
        assert!(tile.contains(r.x, r.y), "{part:?} opens inside the tile");
        let (x, y) = mid(r);
        assert_eq!(hit(&l, x, y), Some(part));
    }
    assert_eq!(hit(&l, tile.x + 4, tile.y + 4), Some(Part::Tile(1)));
    let below = l.rect(Part::Tile(2)).unwrap();
    assert!(
        below.y >= tile.bottom(),
        "the open tile pushes the next one down"
    );
}

#[test]
fn blue_insets_double_exactly_at_scale_2() {
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    let rim = Color::rgb(0x12, 0x34, 0x56);
    for d in [1, 2] {
        let pic = userpic::draw(userpic::ALL[1], (PICTURE * d) as u32);
        let f = framed(&pic, rim, d);
        let row = FRAMED * d / 2;
        let rim_w = (0..FRAMED * d)
            .take_while(|&x| f.get(x, row) == rim.argb())
            .count() as i32;
        assert_eq!(rim_w, FRAME * d, "d={d}: the picture frame's rim");

        let field = field_box(white, rim, 40, FIELD_H, d);
        let row = FIELD_H * d / 2;
        let line = (0..40 * d)
            .take_while(|&x| field.get(x, row) == rim.argb())
            .count() as i32;
        assert_eq!(line, d, "d={d}: the field's rim");
    }

    let (_, open) = &states()[2];
    let (one, l) = drawn(open, Variant::Blue, 1);
    let (two, _) = drawn(open, Variant::Blue, 2);
    let r = l.rect(Part::Password).unwrap();
    let window = blue_theme(Variant::Blue).colors.window.argb();
    let y = r.y + r.height / 2;
    assert_ne!(one.get(r.x, y), window, "the rim at 1");
    assert_eq!(one.get(r.x + 1, y), window, "the fill one in at 1");
    assert_ne!(two.get(2 * r.x + 1, 2 * y), window, "the rim at 2");
    assert_eq!(two.get(2 * r.x + 2, 2 * y), window, "the fill two in at 2");
}

fn many(n: usize) -> Vec<LogonUser> {
    (0..n)
        .map(|i| LogonUser {
            name: format!("user{i}"),
            real_name: String::new(),
            picture: None,
            note: None,
        })
        .collect()
}

fn tiles_shown(l: &LogonLayout) -> Vec<usize> {
    l.parts
        .iter()
        .filter_map(|&(p, _)| match p {
            Part::Tile(i) => Some(i),
            _ => None,
        })
        .collect()
}

#[test]
fn the_focused_and_the_open_tile_stay_in_view_on_a_short_output() {
    let short = Rect::new(0, 0, 960, 540);
    let theme = blue_theme(Variant::Blue);
    let text = TextRenderer::load(&theme.fonts);
    let mut m = Logon::new(Interface::Blue, many(8), true, None);
    for _ in 0..7 {
        m.key(LogonKey::Down);
    }
    assert_eq!(m.focus, Part::Tile(7));
    let l = layout(&m, &theme, &text, short);
    assert!(l.rect(Part::Tile(7)).is_some(), "the focused last tile");
    m.key(LogonKey::Enter);
    assert_eq!(m.screen, Screen::Tile(7));
    let l = layout(&m, &theme, &text, short);
    assert!(l.rect(Part::Tile(7)).is_some(), "the open last tile");
    assert!(l.rect(Part::Password).is_some(), "the open tile's field");
}

#[test]
fn column_is_how_many_tiles_the_output_shows() {
    let theme = blue_theme(Variant::Blue);
    let text = TextRenderer::load(&theme.fonts);
    for output in [
        Rect::new(0, 0, 960, 540),
        OUTPUT,
        Rect::new(0, 0, 1920, 1080),
    ] {
        let n = column(&theme, output);
        let m = Logon::new(Interface::Blue, many(12), true, None);
        let l = layout(&m, &theme, &text, output);
        assert_eq!(tiles_shown(&l).len(), n, "{output:?}");
    }
    assert!(column(&theme, Rect::new(0, 0, 960, 540)) < crate::logon::COLUMN);
}

#[test]
fn caps_lock_goes_under_the_field_on_both_screens() {
    for m in [states().remove(2).1, states().remove(5).1] {
        let (off, l) = drawn(&m, Variant::Blue, 1);
        let mut caps = m.clone();
        caps.caps_lock = true;
        let (on, _) = drawn(&caps, Variant::Blue, 1);
        let field = l.rect(Part::Password).unwrap();
        let row = field.y..field.bottom();
        let under = field.bottom()..field.bottom() + 2 * LINE;
        let differs = |ys: std::ops::Range<i32>| {
            ys.flat_map(|y| (field.x..OUTPUT.width).map(move |x| (x, y)))
                .any(|(x, y)| off.get(x, y) != on.get(x, y))
        };
        assert!(!differs(row), "{:?}: nothing beside the field", m.screen);
        assert!(differs(under), "{:?}: the notice under it", m.screen);
    }
}

#[test]
fn the_backdrop_is_kept_for_each_output_size() {
    let sc = Variant::Blue.scheme();
    let a = cached_backdrop(sc, 64, 48, 1);
    let b = cached_backdrop(sc, 80, 60, 1);
    assert!(
        Rc::ptr_eq(&a, &cached_backdrop(sc, 64, 48, 1)),
        "the first size"
    );
    assert!(
        Rc::ptr_eq(&b, &cached_backdrop(sc, 80, 60, 1)),
        "the second size"
    );
}
