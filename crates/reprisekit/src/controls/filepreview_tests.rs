use super::*;
use crate::interface::Interface;
use crate::interfaces::theme_for;
use crate::theme::{FontConfig, Theme};

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |b: &[u8]| {
        for &b in b {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(&c.width.to_le_bytes());
    eat(&c.height.to_le_bytes());
    for p in &c.pixels {
        eat(&p.to_le_bytes());
    }
    h
}

fn facts() -> Facts {
    Facts {
        name: "Report & notes.dat".into(),
        kind: "DAT File".into(),
        size: Some("12.0 KB".into()),
        modified: "10/3/2026 4:05 PM".into(),
    }
}

/// A 120 x 80 picture: red to blue across, opaque.
fn picture(scale: i32) -> Canvas {
    let (w, h) = (120 * scale as u32, 80 * scale as u32);
    let px = (0..w * h)
        .map(|i| {
            let x = i % w;
            let r = 255 - x * 255 / w;
            0xFF00_0000 | r << 16 | (x * 255 / w)
        })
        .collect();
    Canvas::from_pixels(w, h, px).at_scale(scale)
}

fn icon() -> Rc<Canvas> {
    let mut c = Canvas::new(ICON_PX as u32, ICON_PX as u32);
    c.fill_rect(
        Rect::new(8, 4, 32, 40),
        crate::theme::Color::rgb(0x30, 0x60, 0xC0),
    );
    Rc::new(c)
}

fn metrics(theme: &Theme) -> Metrics {
    let mut m = Metrics {
        bars_always: false,
        ..Metrics::default()
    };
    m.follow_look(&theme.look.controls());
    m
}

#[test]
fn types_route_to_picture_text_or_a_sniff() {
    assert_eq!(route(Some("image/png")), Route::Picture);
    assert_eq!(route(Some("image/jpeg")), Route::Picture);
    assert_eq!(
        route(Some("image/svg+xml")),
        Route::Text,
        "SVG is text no decoder reads"
    );
    assert_eq!(route(Some("text/plain")), Route::Text);
    assert_eq!(route(Some("text/x-rust")), Route::Text);
    assert_eq!(route(Some("application/json")), Route::Text);
    assert_eq!(route(Some("application/x-shellscript")), Route::Text);
    assert_eq!(route(Some("application/xhtml+xml")), Route::Text);
    assert_eq!(route(Some("application/pdf")), Route::Other);
    assert_eq!(route(Some("application/zip")), Route::Other);
    assert_eq!(route(None), Route::Other);
}

#[test]
fn a_sniff_takes_utf8_without_nul_and_forgives_a_cut_last_character() {
    assert!(looks_text(b"fn main() {}\n"));
    assert!(looks_text("caf\u{e9}".as_bytes()));
    assert!(
        looks_text(&"caf\u{e9}".as_bytes()[..4]),
        "the read cut the é in two"
    );
    assert!(!looks_text(b"PK\x03\x04\0\0"));
    assert!(!looks_text(b"\xff\xfe a"));
    assert!(looks_text(b""));
    assert_eq!(text_of(&"ab\u{e9}".as_bytes()[..3]), "ab");
    assert_eq!(text_of(b"a\xffb"), "a\u{fffd}b");
}

#[test]
fn space_and_escape_close_enter_opens_and_the_arrows_step() {
    assert_eq!(key(keysym::SPACE, false, false), Action::Close);
    assert_eq!(key(keysym::ESCAPE, false, false), Action::Close);
    assert_eq!(key(keysym::RETURN, false, false), Action::Open);
    assert_eq!(key(keysym::KP_ENTER, false, false), Action::Open);
    assert_eq!(key(keysym::LEFT, false, false), Action::Step(-1));
    assert_eq!(key(keysym::UP, false, false), Action::Step(-1));
    assert_eq!(key(keysym::RIGHT, false, false), Action::Step(1));
    assert_eq!(key(keysym::DOWN, false, false), Action::Step(1));
    assert_eq!(key(keysym::HOME, false, false), Action::Step(i64::MIN / 2));
    assert_eq!(key(keysym::END, false, false), Action::Step(i64::MAX / 2));
    assert_eq!(key(keysym::PAGE_DOWN, false, false), Action::Page(1));
    assert_eq!(key(keysym::PAGE_UP, false, false), Action::Page(-1));
    assert_eq!(key(keysym::END, true, false), Action::Page(i64::MAX / 2));
    assert_eq!(key(keysym::LEFT, true, false), Action::None);
    assert_eq!(
        key(keysym::RETURN, false, true),
        Action::None,
        "Alt+Enter is not Open"
    );
    assert_eq!(key('a' as u32, false, false), Action::None);
}

#[test]
fn the_window_fits_its_body_inside_the_work_areas_share() {
    let text = TextRenderer::stub(13, |_| 8);
    let m = Metrics {
        line_h: 13,
        bar: 16,
        ..Metrics::default()
    };
    let mut qv = FilePreview::new(facts(), None);
    assert_eq!(qv.size(&text, &m, (1344, 756)), CARD);

    qv.set_body(Body::Picture(picture(1)));
    assert_eq!(
        qv.size(&text, &m, (1344, 756)),
        MIN_PICTURE,
        "a small one held to the least"
    );
    qv.set_body(Body::Picture(Canvas::new(400, 300)));
    assert_eq!(
        qv.size(&text, &m, (1344, 756)),
        (400 + 2 * PAD, 300 + 2 * PAD)
    );
    qv.set_body(Body::Picture(Canvas::new(2000, 1500)));
    assert_eq!(
        qv.size(&text, &m, (1344, 756)),
        (1344, 756),
        "held to the share"
    );

    qv.set_body(Body::Text(MultiEdit::new("a\nb")));
    let w = TEXT_COLS * 8 + 6 + 16 + 2 * PAD;
    let h = TEXT_ROWS * 13 + 6 + 16 + 2 * PAD;
    assert_eq!(qv.size(&text, &m, (1344, 756)), (w, h));
    assert_eq!(qv.size(&text, &m, (500, 300)), (500, 300));
}

#[test]
fn a_picture_centres_and_shrinks_into_a_smaller_window() {
    let pic = picture(1);
    assert_eq!(
        picture_rect(&pic, Rect::new(0, 0, 200, 120)),
        Rect::new(40, 20, 120, 80)
    );
    assert_eq!(
        picture_rect(&pic, Rect::new(0, 0, 76, 200)),
        Rect::new(8, 80, 60, 40),
        "fitted inside the margin, aspect kept"
    );
}

#[test]
fn text_scrolls_by_wheel_page_and_end_but_a_picture_does_not() {
    let text = TextRenderer::stub(13, |_| 8);
    let m = Metrics {
        line_h: 13,
        bars_always: false,
        ..Metrics::default()
    };
    let r = Rect::new(0, 0, 400, 13 * 10 + 6 + 2 * PAD);
    let doc: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut qv = FilePreview::new(facts(), None);
    qv.set_body(Body::Text(MultiEdit::new(&doc.join("\n"))));
    let rows = qv.text_layout(&text, &m, r).expect("text").rows;

    assert!(qv.scroll(3, &text, &m, r));
    assert_eq!(qv.view.top, 3);
    assert!(qv.page(1, &text, &m, r));
    assert_eq!(qv.view.top, 3 + rows);
    assert!(qv.page(i64::MAX / 2, &text, &m, r));
    assert_eq!(qv.view.top, 100 - rows, "the last line at the bottom");
    assert!(!qv.scroll(1, &text, &m, r));
    assert!(qv.page(i64::MIN / 2, &text, &m, r));
    assert_eq!(qv.view.top, 0);

    qv.set_body(Body::Picture(picture(1)));
    assert!(!qv.scroll(3, &text, &m, r));
}

/// The picture, the text box and the card in each interface at 1 and 2.
#[test]
fn the_preview_is_pinned_per_interface_body_and_scale() {
    if !crate::text::test_tahoma() {
        return;
    }
    let text = TextRenderer::load(&FontConfig::default());
    let doc = "fn main() {\n    println!(\"Reprise\");\n}\n\n// a second paragraph\tand a tab";
    let mut got = Vec::new();
    for (name, i) in [
        ("classic", Interface::Classic),
        ("blue", Interface::Blue),
        ("glass", Interface::Glass),
    ] {
        let mut theme = theme_for(i);
        let m = metrics(&theme);
        for s in [1, 2] {
            theme.look = theme.look.at_scale(s);
            let p = theme.paint();
            let bodies = [
                ("picture", Body::Picture(picture(s))),
                ("text", Body::Text(MultiEdit::new(doc))),
                ("card", Body::Card),
            ];
            for (body_name, body) in bodies {
                let mut qv = FilePreview::new(facts(), Some(icon()));
                qv.set_body(body);
                let (w, h) = qv.size(&text, &m, (320, 160));
                let mut c = Canvas::with_scale(w as u32, h as u32, s);
                let r = Rect::new(0, 0, w, h);
                draw(&mut c, &p, &text, 11, &text, &m, &qv, r);
                assert_eq!(
                    c.get(1, 1) | 0xFF00_0000,
                    p.cs.button_face.argb() | 0xFF00_0000,
                    "{name} {body_name} @{s}: the face behind it"
                );
                got.push((format!("{name} {body_name} @{s}"), fnv(&c)));
            }
        }
    }
    crate::pins::check("filepreview", &got);
    let distinct: std::collections::HashSet<_> = got.iter().map(|g| g.1).collect();
    assert_eq!(distinct.len(), got.len(), "every state draws apart");
}

#[test]
fn the_picture_lands_centred_and_the_card_draws_its_icon() {
    let theme = Theme::default();
    let p = theme.paint();
    let text = TextRenderer::stub(11, |_| 6);
    let m = metrics(&theme);
    let mut qv = FilePreview::new(facts(), Some(icon()));
    qv.set_body(Body::Picture(picture(1)));
    let r = Rect::new(0, 0, 200, 120);
    let mut c = Canvas::new(200, 120);
    draw(&mut c, &p, &text, 11, &text, &m, &qv, r);
    assert_eq!(c.get(40, 20), picture(1).get(0, 0), "the picture's corner");
    assert_eq!(
        c.get(39, 20) | 0xFF00_0000,
        p.cs.button_face.argb() | 0xFF00_0000
    );

    qv.set_body(Body::Card);
    let mut c = Canvas::new(CARD.0 as u32, CARD.1 as u32);
    draw(
        &mut c,
        &p,
        &text,
        11,
        &text,
        &m,
        &qv,
        Rect::new(0, 0, CARD.0, CARD.1),
    );
    let (at, _) = card_layout(Rect::new(0, 0, CARD.0, CARD.1), 4);
    assert_eq!(c.get(at.x + 20, at.y + 20), icon().get(20, 20));
}

#[test]
fn the_card_names_the_file_and_each_fact_to_a_reader() {
    let qv = FilePreview::new(facts(), None);
    let text = TextRenderer::stub(13, |_| 8);
    let mut tree = Tree::window("Report & notes.dat", CARD);
    access(
        &mut tree,
        access::ROOT,
        &qv,
        &text,
        &Metrics::default(),
        Rect::new(0, 0, CARD.0, CARD.1),
    );
    let values: Vec<String> = tree
        .nodes()
        .filter(|(_, n)| n.role() == Role::Label)
        .filter_map(|(_, n)| n.value().map(str::to_string))
        .collect();
    assert_eq!(
        values,
        [
            "Report & notes.dat",
            "Type: DAT File",
            "Size: 12.0 KB",
            "Modified: 10/3/2026 4:05 PM"
        ]
    );
}
