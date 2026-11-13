use super::*;
use crate::controls::form::Key;
use crate::interface::Interface;
use crate::interfaces;
use crate::raster::Canvas;
use crate::text::TextRenderer;
use crate::theme::Theme;

fn found(fingers: &[Finger]) -> Reader {
    Reader::Found {
        name: "Synaptics Sensors".into(),
        fingers: fingers.to_vec(),
    }
}

fn yours(reader: Reader) -> Facts {
    Facts {
        reader,
        yours: true,
        busy: false,
    }
}

#[test]
fn each_finger_round_trips_through_its_fprintd_name() {
    for f in Finger::ALL {
        assert_eq!(Finger::from_key(f.key()), Some(f));
    }
    assert_eq!(Finger::RightIndex.key(), "right-index-finger");
    assert_eq!(Finger::LeftThumb.key(), "left-thumb");
    assert_eq!(Finger::from_key("any"), None);
    assert_eq!(Finger::RightIndex.label(), "Right index finger");
    assert_eq!(Finger::LeftLittle.hand(), Hand::Left);
}

#[test]
fn another_users_page_shows_only_a_greyed_note() {
    for reader in [
        Reader::Looking,
        Reader::NoService,
        found(&[Finger::RightIndex]),
    ] {
        let s = section(&Facts {
            yours: false,
            ..yours(reader)
        });
        assert_eq!(s.note, Some(NOT_YOURS));
        assert!(!s.add && !s.delete && s.fingers.is_empty());
        let (items, _) = items(&s, 0, 0, W);
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].control, Control::GreyedLines(_)));
    }
}

#[test]
fn without_fprintd_or_a_reader_every_action_greys_and_says_why() {
    for (reader, why) in [
        (Reader::Looking, LOOKING),
        (Reader::NoService, NO_SERVICE),
        (Reader::NoReader, NO_READER),
        (Reader::Silent, SILENT),
    ] {
        let s = section(&yours(reader));
        assert_eq!(s.note, Some(why));
        assert!(!s.add && !s.delete, "{why}");
        assert_eq!(s.reader, None);
    }
}

#[test]
fn with_a_reader_add_is_lit_and_delete_all_needs_a_finger() {
    let s = section(&yours(found(&[])));
    assert!(s.add && !s.delete);
    assert_eq!(s.note, Some(NONE_ADDED));
    assert_eq!(s.reader.as_deref(), Some("Synaptics Sensors"));
    let s = section(&yours(found(&[Finger::RightIndex])));
    assert!(s.add && s.delete);
    assert_eq!(s.note, None);
    let s = section(&yours(found(&Finger::ALL)));
    assert!(!s.add && s.delete);
    assert_eq!(s.note, Some(ALL_ADDED));
}

#[test]
fn a_reader_name_past_one_line_is_cut_short() {
    let long = "x".repeat(NAME_CHARS + 5);
    let s = section(&yours(Reader::Found {
        name: long,
        fingers: Vec::new(),
    }));
    let (items, _) = items(&s, 0, 0, W);
    let form = Form::new(items);
    let shown = match form.item(READER).map(|i| &i.control) {
        Some(Control::Label { text, .. }) => text.clone(),
        _ => String::new(),
    };
    assert_eq!(shown, format!("Reader: {}...", "x".repeat(NAME_CHARS - 3)));
}

#[test]
fn a_change_under_way_greys_every_action() {
    let s = section(&Facts {
        busy: true,
        ..yours(found(&[Finger::RightIndex]))
    });
    assert!(!s.add && !s.delete);
    let (items, _) = items(&s, 0, 0, W);
    let mut form = Form::new(items);
    form.focus_id(LIST);
    form.key(Key::Down);
    follow(&mut form, &s);
    assert!(!lit(&form, DELETE));
}

fn lit(form: &Form, id: &str) -> bool {
    matches!(
        form.item(id).map(|i| &i.control),
        Some(Control::Button { enabled: true, .. })
    )
}

#[test]
fn delete_lights_for_the_picked_row_and_names_its_finger() {
    let s = section(&yours(found(&[Finger::LeftThumb, Finger::RightIndex])));
    let (items, _) = items(&s, 0, 0, W);
    let mut form = Form::new(items);
    follow(&mut form, &s);
    assert!(!lit(&form, DELETE), "nothing picked");
    assert_eq!(action(&Event::Pressed(DELETE), &form, &s), None);
    form.focus_id(LIST);
    form.key(Key::Down);
    form.key(Key::Down);
    follow(&mut form, &s);
    assert!(lit(&form, DELETE));
    assert_eq!(
        action(&Event::Pressed(DELETE), &form, &s),
        Some(Action::Delete(Finger::RightIndex))
    );
    assert_eq!(action(&Event::Pressed(ADD), &form, &s), Some(Action::Add));
    assert_eq!(
        action(&Event::Pressed(CLEAR), &form, &s),
        Some(Action::DeleteAll)
    );
    assert_eq!(action(&Event::Default, &form, &s), None);
}

/// The width of an account sheet's page inside its 16 px margins.
const W: i32 = 366;
/// Room around the page, so every frame draws whole.
const M: i32 = 8;

fn states() -> Vec<(&'static str, Facts)> {
    vec![
        ("looking", yours(Reader::Looking)),
        ("no fprintd", yours(Reader::NoService)),
        ("no reader", yours(Reader::NoReader)),
        ("silent", yours(Reader::Silent)),
        ("none added", yours(found(&[]))),
        (
            "two added",
            yours(found(&[Finger::LeftThumb, Finger::RightIndex])),
        ),
        (
            "all added on a long-named reader",
            yours(Reader::Found {
                name: "Synaptics Sensors Prometheus MIS Touch Fingerprint Reader".into(),
                fingers: Finger::ALL.to_vec(),
            }),
        ),
        (
            "busy",
            Facts {
                busy: true,
                ..yours(found(&[Finger::RightIndex]))
            },
        ),
        (
            "another user",
            Facts {
                yours: false,
                ..yours(found(&[Finger::RightIndex]))
            },
        ),
    ]
}

fn form_of(f: &Facts) -> (Form, i32) {
    let (items, bottom) = items(&section(f), M, M, W);
    (Form::new(items), bottom + M)
}

#[test]
fn every_item_sits_inside_the_page() {
    for (name, f) in states() {
        let (form, bottom) = form_of(&f);
        for it in &form.items {
            let r = it.rect;
            assert!(
                r.x >= M && r.right() <= M + W && r.y >= M && r.bottom() <= bottom - M,
                "{name}: {} at {r:?}",
                it.id
            );
        }
    }
}

#[test]
fn the_page_fits_its_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| {
        states()
            .into_iter()
            .map(|(name, f)| (name.to_string(), form_of(&f).0))
            .collect()
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

pub(crate) fn looks() -> Vec<(String, Theme)> {
    let mut out = vec![
        ("Classic".to_string(), Theme::default()),
        (
            "Blue".to_string(),
            interfaces::blue_theme(crate::blue::Variant::ALL[0]),
        ),
    ];
    for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
        let g = crate::glass::Settings {
            mode,
            ..Default::default()
        };
        let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
        out.push((format!("Glass {mode:?}"), t));
    }
    out
}

pub(crate) fn hash_form(theme: &Theme, form: &Form, (w, h): (i32, i32), s: i32) -> u64 {
    let theme = theme.scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let p = theme.paint();
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), p.cs.button_face);
    crate::controls::form::draw(&mut c, &text, &p, form, true, theme.fonts.size_px);
    fnv(&c)
}

#[test]
fn the_page_pins_its_pixels_in_every_state_and_look() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (look, base) in looks() {
        for (state, f) in states() {
            let (mut form, bottom) = form_of(&f);
            if state == "two added" {
                form.focus_id(LIST);
                form.key(Key::Down);
                follow(&mut form, &section(&f));
            }
            for s in [1, 2] {
                let h = hash_form(&base, &form, (W + 2 * M, bottom), s);
                got.push((format!("{look} {state} {s}"), h));
            }
        }
    }
    crate::pins::check("fingerprints", &got);
}
