use super::*;
use crate::art::userpic;
use crate::interface::Interface;
use crate::logon::{LogonKey, LogonUser, Part, UserNote, hit, text as strings};
use crate::pointer::Interaction;
use crate::textedit::Preedit;

const OUTPUT: Rect = Rect::new(0, 0, 1024, 768);

/// FNV-1a over the pixels, for pins.
fn fnv(c: &Canvas) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in &c.pixels {
        for b in p.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

fn users(n: usize) -> Vec<LogonUser> {
    (0..n)
        .map(|i| LogonUser {
            name: format!("user{i}"),
            real_name: if i == 0 {
                "Alice Liddell".into()
            } else {
                String::new()
            },
            picture: (i == 0).then(|| userpic::draw(userpic::ALL[3], 128)),
            note: (i == 1).then_some(UserNote::LoggedOn),
        })
        .collect()
}

fn tile() -> Logon {
    Logon::new(Interface::Glass, users(2), true, Some("user0"))
}

fn states() -> Vec<(&'static str, Logon)> {
    let rest = tile();
    let mut hot = tile();
    hot.key(LogonKey::Text("abc"));
    hot.lit = Some((Part::Submit, Interaction::Hot));
    let mut typed = tile();
    typed.click(Part::SwitchUser);
    typed.click(Part::OtherUser);
    typed.key(LogonKey::Text("carol"));
    let mut tiles = tile();
    tiles.click(Part::SwitchUser);
    let mut message = tile();
    message.key(LogonKey::Text("abc"));
    message.fail(strings::incorrect());
    let mut busy = tile();
    busy.key(LogonKey::Text("abc"));
    busy.set_busy(true);
    let mut tile_hot = tile();
    tile_hot.lit = Some((Part::Tile(0), Interaction::Hot));
    let mut power_hot = tile();
    power_hot.lit = Some((Part::Power, Interaction::Hot));
    vec![
        ("tile", rest),
        ("tile hot", tile_hot),
        ("arrow hot", hot),
        ("typed", typed),
        ("tiles", tiles),
        ("message", message),
        ("busy", busy),
        ("power hot", power_hot),
    ]
}

/// The tile after a wrong password and the error screen's OK.
fn tile_after_ok() -> Logon {
    let mut m = tile();
    m.key(LogonKey::Text("abc"));
    m.fail(strings::incorrect());
    m.click(Part::MessageOk);
    m
}

fn drawn(m: &Logon, s: i32) -> (Canvas, LogonLayout) {
    drawn_with(m, s, None)
}

fn drawn_with(m: &Logon, s: i32, preedit: Option<Preedit>) -> (Canvas, LogonLayout) {
    let theme = crate::interfaces::theme_for(Interface::Glass).scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let l = layout(m, &theme, &text, OUTPUT);
    let mut c = Canvas::new((OUTPUT.width * s) as u32, (OUTPUT.height * s) as u32);
    c.set_preedit(preedit);
    draw(&mut c, m, &l, &theme, text.for_output(s), s);
    (c, l)
}

#[test]
fn glass_logon_pins_at_scale_1_and_2() {
    if !crate::text::test_tahoma() {
        return;
    }
    let mut got = Vec::new();
    for (name, m) in states()
        .into_iter()
        .chain([("tile after ok", tile_after_ok())])
    {
        for s in [1, 2] {
            got.push((format!("{name} @{s}"), fnv(&drawn(&m, s).0)));
        }
    }
    crate::pins::check("glass/logon", &got);
}

#[test]
fn glass_states_are_distinct() {
    let hashes: Vec<(&str, u64)> = states()
        .into_iter()
        .map(|(name, m)| (name, fnv(&drawn(&m, 1).0)))
        .collect();
    for (i, a) in hashes.iter().enumerate() {
        for b in &hashes[i + 1..] {
            assert_ne!(a.1, b.1, "{} and {} draw alike", a.0, b.0);
        }
    }
}

#[test]
fn glass_tile_after_ok_draws_no_notice() {
    assert_eq!(
        fnv(&drawn(&tile_after_ok(), 1).0),
        fnv(&drawn(&tile(), 1).0),
        "after OK the tile draws as it did before the try"
    );
}

#[test]
fn glass_typed_name_field_draws_a_preedit() {
    let (_, typed) = states().into_iter().find(|(n, _)| *n == "typed").unwrap();
    assert_eq!((typed.screen, typed.focus), (Screen::Typed, Part::Name));
    let (plain, l) = drawn(&typed, 1);
    let (composing, _) = drawn_with(&typed, 1, Preedit::new("hao".into(), 2, 2));
    let field = l.rect(Part::Name).unwrap();
    let inside = |c: &Canvas| -> Vec<u32> {
        (field.y..field.bottom())
            .flat_map(|y| (field.x..field.right()).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .collect()
    };
    assert_ne!(
        inside(&plain),
        inside(&composing),
        "the name field shows the preedit"
    );
}

#[test]
fn logon_column_is_glass_fit_for_glass_and_the_default_for_classic() {
    use crate::interfaces::logon::logon_column;
    let theme = crate::interfaces::theme_for(Interface::Glass);
    assert_eq!(
        logon_column(&tile(), &theme, OUTPUT),
        column(&theme, OUTPUT)
    );
    let m = Logon::new(Interface::Classic, users(2), true, None);
    assert_eq!(logon_column(&m, &theme, OUTPUT), crate::logon::COLUMN);
}

#[test]
fn glass_frames_have_partial_alpha_on_curves_at_2() {
    let (c, l) = drawn(&tile(), 2);
    assert_eq!(
        c.get(0, 0),
        0,
        "the wallpaper is the host's: the corner stays clear"
    );
    for part in [Part::Tile(0), Part::Submit, Part::SwitchUser, Part::Power] {
        let r = l
            .rect(part)
            .unwrap_or_else(|| panic!("{part:?} is laid out"))
            .scaled(2);
        let corner: Vec<u32> = (0..12)
            .flat_map(|y| (0..12).map(move |x| (x, y)))
            .map(|(x, y)| c.get(r.x + x, r.y + y) >> 24)
            .collect();
        assert!(
            corner.iter().any(|a| (1..255).contains(a)),
            "{part:?}'s corner curve is anti-aliased at 2: {corner:?}"
        );
        assert_eq!(corner[0], 0, "{part:?}'s outermost corner pixel is clear");
    }
}

#[test]
fn glass_hit_finds_arrow_switch_user_and_power() {
    let m = tile();
    let (_, l) = drawn(&m, 1);
    let centre = |p: Part| {
        let r = l.rect(p).unwrap_or_else(|| panic!("{p:?} is laid out"));
        (r.x + r.width / 2, r.y + r.height / 2)
    };
    for part in [
        Part::Password,
        Part::Submit,
        Part::SwitchUser,
        Part::Power,
        Part::PowerMenu,
    ] {
        let (x, y) = centre(part);
        assert_eq!(hit(&l, x, y), Some(part), "{part:?} at its centre");
    }
    assert_eq!(hit(&l, 2, 2), None, "the wallpaper is nothing");
    let (field, arrow) = (
        l.rect(Part::Password).unwrap(),
        l.rect(Part::Submit).unwrap(),
    );
    assert!(
        arrow.x >= field.right(),
        "the arrow sits right of the field"
    );
    assert!(
        (centre(Part::Submit).1 - centre(Part::Password).1).abs() <= 1,
        "and on its line"
    );
    let switch = l.rect(Part::SwitchUser).unwrap();
    assert!(
        switch.y > field.bottom(),
        "Switch User comes under the field"
    );
    assert!(
        (centre(Part::SwitchUser).0 - OUTPUT.width / 2).abs() <= 1,
        "centred across"
    );
    let (power, menu) = (
        l.rect(Part::Power).unwrap(),
        l.rect(Part::PowerMenu).unwrap(),
    );
    assert_eq!(
        menu.x,
        power.right(),
        "the options arrow joins the power button"
    );
    assert!(
        menu.right() < OUTPUT.right() && power.bottom() < OUTPUT.bottom(),
        "both inside the screen, bottom right"
    );
    assert!(power.x > OUTPUT.width * 3 / 4 && power.y > OUTPUT.height * 3 / 4);

    let mut tiles = tile();
    tiles.click(Part::SwitchUser);
    let (_, l) = drawn(&tiles, 1);
    let (a, b, other) = (
        l.rect(Part::Tile(0)).unwrap(),
        l.rect(Part::Tile(1)).unwrap(),
        l.rect(Part::OtherUser).unwrap(),
    );
    assert!(
        a.right() <= b.x && b.right() <= other.x,
        "one row, in order"
    );
    assert_eq!((a.y, a.y), (b.y, other.y), "on one line");
    assert_eq!(
        hit(&l, b.x + b.width / 2, b.y + b.height / 2),
        Some(Part::Tile(1))
    );
    assert_eq!(l.rect(Part::Password), None, "no field on the tiles");
}
