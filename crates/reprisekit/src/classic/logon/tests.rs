use super::*;
use crate::interface::Interface;
use crate::logon::{Part, PowerAction, text as strings};

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
    let mut over_picture = filled.clone();
    over_picture.over_picture = true;
    vec![
        ("empty", empty),
        ("name filled", filled),
        ("message", message),
        ("busy", busy),
        ("caps lock", caps),
        ("layout badge", badge),
        ("admin shut down", admin(PowerAction::PowerOff, "")),
        ("admin restart filled", admin(PowerAction::Reboot, "anna")),
        ("shutting down", going_down()),
        ("over a picture", over_picture),
    ]
}

/// The administrator's form for `action` with `name` typed and, after it, a password.
fn admin(action: PowerAction, name: &str) -> Logon {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    m.admin(action);
    if !name.is_empty() {
        m.key(crate::logon::LogonKey::Text(name));
        m.key(crate::logon::LogonKey::Enter);
        m.key(crate::logon::LogonKey::Text("hunter2"));
    }
    m
}

fn going_down() -> Logon {
    let mut m = admin(PowerAction::PowerOff, "anna");
    m.going_down();
    m
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
    crate::pins::fonts();
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
fn over_a_picture_only_the_dialog_is_drawn() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    m.over_picture = true;
    for s in [1, 2] {
        let (c, l, theme) = drawn(&m, s);
        let d = l.dialog.expect("a dialog").scaled(s);
        assert_eq!(c.get(0, 0), 0, "@{s}: the picture shows around the dialog");
        assert_eq!(
            c.get(d.x - 1, d.y + d.height / 2),
            0,
            "@{s}: up to its edge"
        );
        assert_eq!(
            c.get(d.x, d.y),
            theme.colors.button_face.argb(),
            "@{s}: the frame"
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

#[test]
fn busy_says_verifying_on_the_first_free_message_line() {
    let mut filled = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    filled.key(crate::logon::LogonKey::Text("hunter2"));
    let mut busy = filled.clone();
    busy.set_busy(true);
    let mut caps = busy.clone();
    caps.caps_lock = true;
    let inked = |m: &Logon| -> Vec<bool> {
        let (c, _, theme) = drawn(m, 1);
        let r = geometry(&theme, OUTPUT).message;
        let h = r.height / MESSAGE_LINES as i32;
        let face = theme.colors.button_face.argb();
        (0..MESSAGE_LINES as i32)
            .map(|i| {
                (r.y + i * h..r.y + (i + 1) * h)
                    .any(|y| (r.x..r.right()).any(|x| c.get(x, y) != face))
            })
            .collect()
    };
    assert_eq!(inked(&filled), [false, false], "idle: no text");
    assert_eq!(inked(&busy), [true, false], "busy: the first line");
    assert_eq!(inked(&caps), [true, true], "under the Caps Lock notice");
}

#[test]
fn a_prompt_label_draws_its_ampersand() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    let asked = "Token & PIN:";
    m.ask(crate::logon::Prompt {
        text: asked.into(),
        echo: false,
    });
    for s in [1, 2] {
        let (c, l, theme) = drawn(&m, s);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px * s as u32 / theme.scale.max(1) as u32;
        let g = geometry(&theme, OUTPUT);
        let row = l.rect(Part::Password).unwrap().scaled(s);
        let r = Rect::new(
            g.label_x * s,
            row.y,
            theme.text.len(LABEL_W) * s,
            row.height,
        );
        let mut want = c.clone();
        want.fill_rect(r, theme.colors.button_face);
        let ink = theme.colors.button_text;
        text.for_output(s)
            .draw(&mut want, r, asked, false, ink, Align::Left, px);
        assert!(c == want, "@{s}: the label reads {asked:?}");
    }
}

fn locked_states() -> Vec<(&'static str, Logon)> {
    let user = crate::logon::LogonUser {
        name: "anna".into(),
        real_name: "Anna Novak".into(),
        picture: None,
        note: None,
    };
    let rest = Logon::locked(Interface::Classic, user.clone(), true, true);
    let mut typed = rest.clone();
    typed.key(crate::logon::LogonKey::Text("hunter2"));
    let mut caps = typed.clone();
    caps.caps_lock = true;
    caps.layout = Some("EN".into());
    let mut failed = typed.clone();
    failed.fail(strings::incorrect());
    let mut busy = typed.clone();
    busy.set_busy(true);
    let mut switch_focused = rest.clone();
    switch_focused.focus = Part::SwitchUser;
    let alone = Logon::locked(Interface::Classic, user.clone(), false, true);
    let powerless = Logon::locked(Interface::Classic, user, true, false);
    vec![
        ("locked", rest),
        ("locked typed", typed),
        ("locked caps lock and badge", caps),
        ("locked failed", failed),
        ("locked busy", busy),
        ("locked switch user focused", switch_focused),
        ("locked without power", powerless),
        ("locked without switch user", alone),
    ]
}

#[test]
fn classic_lock_pins_at_scale_1_and_2() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (name, m) in locked_states() {
        for s in [1, 2] {
            got.push((format!("{name} @{s}"), fnv(&drawn(&m, s).0)));
        }
    }
    crate::pins::check("classic/logon-locked", &got);
}

#[test]
fn the_classic_lock_fixes_the_name_and_starts_the_button_row_with_switch_user() {
    let (_, m) = locked_states().remove(0);
    for s in [1, 2] {
        let (c, l, theme) = drawn(&m, s);
        let name = l.rect(Part::Name).unwrap().scaled(s);
        assert_eq!(
            c.get(name.right() - 4 * s, name.y + name.height / 2),
            theme.colors.button_face.argb(),
            "@{s}: the fixed name keeps the face colour"
        );
        let g = geometry(&theme, OUTPUT);
        let switch = l.rect(Part::SwitchUser).expect("Switch User");
        assert_eq!(switch.x, g.label_x, "Switch User lines up with the labels");
        assert_eq!(switch.y, g.ok.y, "on the button row");
        assert!(switch.right() < g.ok.x, "left of OK");
    }
    let (_, alone) = locked_states().pop().unwrap();
    let l = drawn(&alone, 1).1;
    assert_eq!(
        l.rect(Part::SwitchUser),
        None,
        "no Switch User unless offered"
    );
    assert!(l.rect(Part::Power).is_some(), "Shut Down... stays");
}

#[test]
fn the_classic_lock_without_power_moves_ok_and_cancel_into_shut_downs_place() {
    let mut states = locked_states().into_iter();
    let (_, m) = states.find(|(n, _)| *n == "locked without power").unwrap();
    assert!(!m.offers_power());
    let (_, l, theme) = drawn(&m, 1);
    let g = geometry(&theme, OUTPUT);
    assert_eq!(l.rect(Part::Power), None, "no Shut Down...");
    assert_eq!(l.rect(Part::Cancel), Some(g.power), "Cancel ends the row");
    assert_eq!(l.rect(Part::Submit), Some(g.cancel), "OK left of it");
    assert!(l.rect(Part::SwitchUser).is_some(), "Switch User stays");
}

#[test]
fn the_classic_lock_states_are_distinct() {
    let hashes: Vec<_> = (locked_states().iter())
        .map(|(n, m)| (*n, fnv(&drawn(m, 1).0)))
        .collect();
    for (i, (a, ha)) in hashes.iter().enumerate() {
        for (b, hb) in &hashes[i + 1..] {
            assert_ne!(ha, hb, "{a} and {b} draw the same");
        }
    }
    let login = Logon::new(Interface::Classic, Vec::new(), false, Some("anna"));
    assert_ne!(
        fnv(&drawn(&login, 1).0),
        hashes[0].1,
        "the lock differs from the login dialog"
    );
}
