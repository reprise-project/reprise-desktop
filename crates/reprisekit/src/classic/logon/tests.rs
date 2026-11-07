use super::*;
use crate::interface::Interface;
use crate::logon::{Part, text as strings};

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

fn states() -> Vec<(&'static str, Logon)> {
    let empty = Logon::new(Interface::Classic, Vec::new(), false, None);
    let mut filled = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    filled.key(crate::logon::LogonKey::Text("hunter2"));
    let mut message = filled.clone();
    message.fail(strings::session_ended_early());
    let mut busy = filled.clone();
    busy.set_busy(true);
    let mut caps = filled.clone();
    caps.caps_lock = true;
    caps.message = Some(strings::incorrect());
    let mut badge = filled.clone();
    badge.layout = Some("EN".into());
    badge.caps_lock = true;
    vec![
        ("empty", empty),
        ("name filled", filled),
        ("message", message),
        ("busy", busy),
        ("caps lock", caps),
        ("layout badge", badge),
    ]
}

fn drawn(m: &Logon, s: i32) -> (Canvas, LogonLayout, Theme) {
    let theme = Theme::default().scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let l = layout(m, &theme, &text, OUTPUT);
    let mut c = Canvas::new((OUTPUT.width * s) as u32, (OUTPUT.height * s) as u32);
    draw(&mut c, m, &l, &theme, text.for_output(s), s);
    (c, l, theme)
}

#[test]
fn classic_logon_pins_at_scale_1_and_2() {
    let mut got = Vec::new();
    for (name, m) in states() {
        for s in [1, 2] {
            got.push((format!("{name} @{s}"), fnv(&drawn(&m, s).0)));
        }
    }
    crate::pins::check("classic/logon", &got);
}

#[test]
fn the_classic_dialog_is_centred_and_ok_wears_the_default_ring() {
    let m = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    for s in [1, 2] {
        let (c, l, theme) = drawn(&m, s);
        let cs = &theme.colors;
        let d = l.dialog.expect("a dialog");
        assert!(
            (d.x - (OUTPUT.width - d.right())).abs() <= 1,
            "@{s}: centred across, {d:?}"
        );
        assert!(
            (d.y - (OUTPUT.height - d.bottom())).abs() <= 1,
            "@{s}: centred down, {d:?}"
        );
        assert_eq!(
            c.get(0, 0),
            cs.background.argb(),
            "@{s}: the desktop colour"
        );
        assert_eq!(
            c.get(d.x * s, d.y * s),
            cs.button_face.argb(),
            "@{s}: the frame"
        );
        let ok = l.rect(Part::Submit).unwrap().scaled(s);
        let cancel = l.rect(Part::Cancel).unwrap().scaled(s);
        let ring = cs.button_text.argb();
        let mid = ok.y + ok.height / 2;
        assert_eq!(c.get(ok.x, mid), ring, "@{s}: OK's ring on the left");
        assert_eq!(c.get(ok.right() - 1, mid), ring, "@{s}: and the right");
        assert_eq!(
            c.get(ok.x + s, mid),
            cs.button_hilight.argb(),
            "@{s}: bevel inside it"
        );
        assert_eq!(
            c.get(cancel.x, cancel.y + cancel.height / 2),
            cs.button_hilight.argb(),
            "@{s}: Cancel has no ring"
        );
        assert!(cancel.x > ok.right(), "@{s}: OK comes first");
        let power = l.rect(Part::Power).unwrap();
        let name = l.rect(Part::Name).unwrap();
        assert_eq!(
            power.right(),
            name.right(),
            "the buttons line up with the fields"
        );
    }
}

#[test]
fn a_focused_button_takes_the_default_ring() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, None);
    m.focus = Part::Cancel;
    let (c, l, theme) = drawn(&m, 1);
    let ring = theme.colors.button_text.argb();
    let at = |p: Part| {
        let r = l.rect(p).unwrap();
        c.get(r.x, r.y + r.height / 2)
    };
    assert_eq!(at(Part::Cancel), ring);
    assert_ne!(at(Part::Submit), ring, "OK gives the ring up");
}

#[test]
fn the_dialog_grows_with_the_text_and_the_badge_stays_at_the_message_end() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, None);
    m.layout = Some("EN".into());
    let text = TextRenderer::load(&Theme::default().fonts);
    let one = layout(&m, &Theme::default(), &text, OUTPUT);
    let big = Theme::default().with_text_size(150);
    let grown = layout(&m, &big, &text, OUTPUT);
    let (a, b) = (one.dialog.unwrap(), grown.dialog.unwrap());
    assert!(b.width > a.width && b.height > a.height, "{a:?} -> {b:?}");
    for l in [&one, &grown] {
        let badge = l.rect(Part::Layout).unwrap();
        let power = l.rect(Part::Power).unwrap();
        assert_eq!(
            badge.right(),
            power.right(),
            "the badge ends where the buttons do"
        );
        assert!(badge.bottom() <= power.y, "above the buttons");
        assert_eq!(badge.width, 16, "the badge keeps the taskbar cell's size");
    }
}
