use super::*;

/// A label wider than its column in the font drawn moves the field after it over, which
/// keeps its right edge; hits and `rect` follow the draw. Text that fits moves nothing.
#[test]
fn a_label_wider_than_its_column_moves_the_field_after_it() {
    let theme = Theme::default();
    let text = TextRenderer::stub(11, |_| 8);
    let form = |label: &str| {
        Form::new(vec![
            Item::label_at("l", 10, 14, 40, label),
            Item::edit("e", Rect::new(54, 10, 100, 20), "", true),
        ])
    };
    let mut c = Canvas::new(200, 40);
    let fits = form("Name");
    draw(&mut c, &text, &theme.paint(), &fits, false, 11);
    assert_eq!(fits.rect("e"), Some(Rect::new(54, 10, 100, 20)));

    let wide = form("&Long name:");
    draw(&mut c, &text, &theme.paint(), &wide, false, 11);
    assert_eq!(
        wide.rect("l"),
        Some(Rect::new(10, 14, 73, 13)),
        "nine 8 px glyphs and the last one's ink"
    );
    assert_eq!(wide.rect("e"), Some(Rect::new(87, 10, 67, 20)));
    assert_eq!(wide.hit(60, 20, &text, 11).map(|h| h.item), None);
    assert_eq!(wide.hit(100, 20, &text, 11).map(|h| h.item), Some(1));
    assert!(wide.overflows(&text, 11).is_empty());
    assert_eq!(fits.overflows(&text, 11), Vec::new());
}

#[test]
fn a_greyed_note_is_drawn_grey_and_rewraps() {
    if !crate::text::test_tahoma() {
        return;
    }
    let text = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let live = Item::lines_at("n", 10, 10, 60, &["one two three four"]);
    let note = live.clone().enabled(false);
    assert!(matches!(note.control, Control::GreyedLines(_)));
    assert_eq!(note.clone().enabled(true), live);
    let form = Form::new(vec![note]);
    let mut c = Canvas::new(200, 60);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &classic_paint(cs, 1), &form, false, PX);
    let count = |colour: Color| c.pixels.iter().filter(|&&p| p == colour.argb()).count();
    assert!(count(cs.gray_text) > 20);
    assert_eq!(count(cs.button_text), 0);
    assert!(
        matches!(form.overflows(&text, PX)[..], [("n", by)] if by >= LINE_H),
        "the note rewraps onto a row it was not built with"
    );
}

#[test]
fn a_greyed_note_draws_its_ampersand_like_the_live_note() {
    let text = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let ink = |item: Item| {
        let mut c = Canvas::new(120, 30);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &Form::new(vec![item]),
            false,
            PX,
        );
        let cols: Vec<i32> = (0..c.width as i32)
            .filter(|&x| (0..c.height as i32).any(|y| c.get(x, y) != cs.button_face.argb()))
            .collect();
        (cols.first().copied(), cols.last().copied())
    };
    let live = Item::lines_at("n", 4, 4, 110, &["R&D notes"]);
    assert_eq!(ink(live.clone().enabled(false)), {
        let (l, r) = ink(live);
        (l, r.map(|r| r + 1))
    });
}

/// Blue lifts a wrapped check box label's second line onto the box's row.
#[test]
fn a_wrapped_check_box_label_is_not_cut_by_its_own_second_line() {
    let theme = blue();
    let text = TextRenderer::stub(11, |_| 6);
    let form = Form::new(vec![
        Item::group("g", Rect::new(0, 0, 300, 80), "Group"),
        Item::check("c", 10, 20, "Do not move files to the bin.").wraps(),
        Item::label("c2", Rect::new(10, 33, 250, 13), "Remove them at once").check_wrap(),
    ]);
    form.drawn_with(&theme.paint());
    assert_eq!(form.overflows(&text, 11), Vec::new());
}

/// In a view, the fit moves a field over first and a rewrapped note above it then moves it
/// down; `overflows` still reports the note's extra row.
#[test]
fn a_fitted_field_under_a_rewrapped_note_moves_over_and_down() {
    let theme = Theme::default();
    let text = TextRenderer::stub(11, |_| 8);
    let mut f = Form::new(vec![
        Item::lines(
            "note",
            Rect::new(10, 4, 100, LINE_H),
            &["aaaaaaaa bbbbbbbb"],
        ),
        Item::label_at("l", 10, 24, 40, "&Long name:"),
        Item::edit("e", Rect::new(54, 20, 100, 20), "", true),
    ]);
    f.view = Some(View {
        rect: Rect::new(0, 0, 200, 60),
        keep: &[],
    });
    let mut c = Canvas::new(200, 60);
    draw(&mut c, &text, &theme.paint(), &f, false, 11);
    assert_eq!(f.rect("note"), Some(Rect::new(10, 4, 100, 2 * LINE_H)));
    assert_eq!(f.rect("e"), Some(Rect::new(87, 20 + LINE_H, 67, 20)));
    assert_eq!(f.overflows(&text, 11), vec![("note", LINE_H)]);
}

/// A note line with a word wider than its rect runs over by the word's excess, though it
/// wraps to no more rows.
#[test]
fn a_note_word_wider_than_its_rect_breaks_inside_its_rect() {
    let text = TextRenderer::stub(11, |_| 8);
    let body = ["aaaaaaaaaaaaaa b".to_string()];
    assert_eq!(*note_lines(&text, &body, 100, 11), ["aaaaaaaaaaaa", "aa b"]);
    let f = Form::new(vec![Item::lines(
        "note",
        Rect::new(10, 4, 100, 2 * LINE_H),
        &[&body[0]],
    )]);
    f.drawn_with(&Theme::default().paint());
    assert!(f.overflows(&text, 11).is_empty());
}

/// A form set in the second dialog font draws its items in it; the
/// same form in the UI font draws the UI font's.
#[test]
fn a_form_draws_in_its_dialog_font() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let stub = TextRenderer::stub(11, |ch| if ch == 'E' { 7 } else { 8 });
    let text = TextRenderer::load(&FontConfig::default()).with_shell_dlg(stub);
    let ink = |font: DialogFont| -> Vec<(i32, i32)> {
        let form = Form::new(vec![Item::label_at("l", 10, 10, 60, "EN")]).set_in(font);
        let mut c = Canvas::new(80, 30);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &form, false, 11);
        (0..80)
            .flat_map(|x| (0..30).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
            .collect()
    };
    let pens: Vec<i32> = ink(DialogFont::ShellDlg).iter().map(|p| p.0).collect();
    assert_eq!(pens, vec![10, 17]);
    assert!(
        ink(DialogFont::Ui).len() > 2,
        "the UI font inks whole glyphs"
    );
}

/// A group box's label, and a label in a form that lays labels out at their ink, ink from
/// the same column in the second dialog font as in the UI font. In the second font the line's
/// break opens 7 px in, in the UI font 8.
#[test]
fn labels_at_their_ink_start_on_the_same_column_in_either_dialog_font() {
    let path = "/usr/share/wine/fonts/sserife.fon";
    if !crate::text::test_tahoma() {
        return;
    }
    if !std::path::Path::new(path).exists() {
        crate::pins::skip!(face path);
    }
    let theme = Theme::default();
    let cs = &theme.colors;
    let tahoma = TextRenderer::load(&FontConfig::default());
    let second = TextRenderer::load(&FontConfig {
        shell_dlg: Some(path.into()),
        ..FontConfig::default()
    });
    let frame = Rect::new(20, 30, 200, 60);
    let items = || {
        vec![
            Item::group("g", frame, "Sound playback"),
            Item::label_at("l", 7, 100, 60, "Fi&nd what:"),
        ]
    };
    let draw_with = |text: &TextRenderer, form: Form| {
        let mut c = Canvas::new(240, 120);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, text, &classic_paint(cs, 1), &form, false, 11);
        c
    };
    // The first column with label ink in rows `ys`.
    let first_ink = |c: &Canvas, ys: std::ops::Range<i32>| {
        (0..240).find(|&x| ys.clone().any(|y| c.get(x, y) == cs.button_text.argb()))
    };
    // The first column of the frame's top line that is not its shadow.
    let gap = |c: &Canvas| {
        (frame.x..frame.right()).find(|&x| c.get(x, frame.y) != cs.button_shadow.argb())
    };
    let at_ink = || {
        Form::new(items())
            .set_in(DialogFont::ShellDlg)
            .labels_at_ink()
    };
    for (name, text, break_at) in [("second", &second, 27), ("Tahoma", &tahoma, 28)] {
        let c = draw_with(text, at_ink());
        assert_eq!(first_ink(&c, 22..38), Some(30), "{name}: the group label");
        assert_eq!(gap(&c), Some(break_at), "{name}: the break");
        assert_eq!(first_ink(&c, 95..115), Some(7), "{name}: the label");
    }
    let at_pen = Form::new(items()).set_in(DialogFont::ShellDlg);
    let c = draw_with(&second, at_pen);
    assert_eq!(first_ink(&c, 95..115), Some(8), "a label at its pen");
}

/// A check box's label starts 5 px past the 13 px box: "Read-only"
/// inks its first column 18 px right of the box's left edge, and a
/// three-state box draws its indeterminate state with the checkerboard.
#[test]
fn a_check_boxs_label_starts_five_px_past_the_box() {
    let text = TextRenderer::load(&FontConfig::default());
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = Theme::default();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let mut form = Form::new(vec![Item::check("ro", 10, 10, "&Read-only")]);
    let mut c = Canvas::new(100, 40);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &classic_paint(cs, 1), &form, false, px);
    let ink = (24..60).find(|&x| (8..26).any(|y| c.get(x, y) == cs.button_text.argb()));
    assert_eq!(ink, Some(28));
    if let Some(Control::Check { mixed, .. }) = form.item_mut("ro").map(|i| &mut i.control) {
        *mixed = Some(true);
    }
    draw(&mut c, &text, &classic_paint(cs, 1), &form, false, px);
    assert_eq!(c.get(12, 12), cs.window.argb());
    assert_eq!(c.get(13, 12), cs.button_face.argb(), "the checkerboard");
    assert_eq!(form.check_state("ro"), Some(CheckState::Mixed));
}

/// A form hit-tests its radios at the sizes of the look it last drew
/// in: after a Blue draw, the 13 px circle's first column, a pixel
/// left of the item, and the label 15 px in both hit the radio; before
/// any draw, Classic's 12 px circle at the item and 17 px offset leave
/// both columns empty.
#[test]
fn a_form_hits_its_radios_at_its_last_draws_sizes() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let f = radio_form();
    let up = f.index_of("up");
    assert_eq!(f.hit(9, 12, &tr(), PX).map(|h| h.item), None);
    assert_eq!(f.hit(25, 12, &tr(), PX).map(|h| h.item), None);
    let mut c = Canvas::new(120, 120);
    draw(&mut c, &tr(), &theme.paint(), &f, false, PX);
    assert_eq!(
        f.hit(9, 12, &tr(), PX).map(|h| h.item),
        up,
        "the circle's first column"
    );
    assert_eq!(
        f.hit(25, 12, &tr(), PX).map(|h| h.item),
        up,
        "the label 15 px in"
    );
}

/// `access` and `hit` follow the look of the form's latest draw: a
/// Blue draw sets the labels 15 px (radio) and 16 px (check box) in,
/// and a later Classic draw puts them back at 17 px and 18 px.
#[test]
fn a_forms_access_and_hit_follow_its_latest_draw() {
    use crate::access::{ROOT, Tree};
    let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let classic = Theme::default();
    let text = tr();
    let f = Form::new(vec![
        Item::radio("up", 10, 10, "&Up", "dir"),
        Item::check("case", 10, 40, "Match &case"),
    ]);
    let (up, case) = (f.index_of("up"), f.index_of("case"));
    // The right edge of the radio's and the check box's nodes, whose
    // bounds run from the box to the end of the label.
    let right_edges = || {
        let mut tree = Tree::window("Find", (200, 80));
        f.access(&mut tree, ROOT, &text, PX);
        let x1 = |role, name| {
            let id = tree.find(role, name).expect("the node");
            tree.get(id).unwrap().bounds().expect("bounds").x1
        };
        (
            x1(Role::RadioButton, "Up"),
            x1(Role::CheckBox, "Match case"),
        )
    };
    let hit = |x, y| f.hit(x, y, &text, PX).map(|h| h.item);
    let mut c = Canvas::new(200, 80);
    draw(&mut c, &text, &classic.paint(), &f, false, PX);
    let (radio, check) = right_edges();
    draw(&mut c, &text, &blue.paint(), &f, false, PX);
    assert_eq!(
        right_edges(),
        (radio - 2.0, check - 2.0),
        "Blue labels end 2 px further left"
    );
    assert_eq!(hit(25, 12), up, "the radio label 15 px in");
    assert_eq!(hit(26, 42), case, "the check box label 16 px in");
    draw(&mut c, &text, &classic.paint(), &f, false, PX);
    assert_eq!(right_edges(), (radio, check), "Classic's edges again");
    assert_eq!(hit(25, 12), None, "nothing 15 px in");
    assert_eq!(hit(27, 12), up, "the radio label 17 px in");
    assert_eq!(hit(27, 42), None, "nothing 17 px into the check box");
    assert_eq!(hit(28, 42), case, "the check box label 18 px in");
}

#[test]
fn lines_wider_than_their_rect_wrap_inside_it() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let lines = [
        "Shift five times turns StickyKeys on",
        "or off, right Shift held down",
    ];
    let w = text.measure(lines[0], false, 11) - 20;
    let f = Form::new(vec![Item::new(
        "note",
        Rect::new(10, 10, w, 2 * LINE_H),
        Control::Lines(lines.iter().map(|l| l.to_string()).collect()),
    )]);
    let mut c = Canvas::new(400, 100);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &classic_paint(cs, 1), &f, false, 11);
    let ink = cs.button_text.argb();
    let inked =
        |x0: i32, x1: i32, y0: i32, y1: i32| (y0..y1).any(|y| (x0..x1).any(|x| c.get(x, y) == ink));
    assert!(!inked(10 + w, 400, 0, 100), "no ink right of the rect");
    assert!(
        inked(10, 10 + w, 10 + 2 * LINE_H, 10 + 3 * LINE_H),
        "the overflow wraps onto a third line"
    );
}

/// In a form with a view, a note rewrapped onto a third line grows a line: what lies under it
/// moves down a line, a group around it grows and a kept button stays. Without a view
/// nothing moves.
#[test]
fn a_rewrapped_note_in_a_view_pushes_what_lies_under_it() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let lines = [
        "Shift five times turns StickyKeys on",
        "or off, right Shift held down",
    ];
    let w = text.measure(lines[0], false, 11) - 20;
    let items = || {
        vec![
            Item::new(
                "group",
                Rect::new(4, 4, w + 12, 4 * LINE_H),
                Control::Group("G".into()),
            ),
            Item::lines("note", Rect::new(10, 20, w, 2 * LINE_H), &lines),
            Item::check("beside", w + 20, 20, "B"),
            Item::check("below", 10, 20 + 2 * LINE_H, "Below"),
            Item::button("ok", Rect::new(10, 100, 75, 23), "OK", true, true),
        ]
    };
    let mut f = Form::new(items());
    f.view = Some(View {
        rect: Rect::new(0, 0, 400, 90),
        keep: &["ok"],
    });
    let mut c = Canvas::new(400, 140);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &f,
        false,
        11,
    );
    assert_eq!(f.rect("note").unwrap().height, 3 * LINE_H);
    assert_eq!(f.rect("group").unwrap().height, 5 * LINE_H);
    assert_eq!(f.rect("beside").unwrap().y, 20);
    assert_eq!(f.rect("below").unwrap().y, 20 + 3 * LINE_H);
    assert_eq!(f.rect("ok").unwrap().y, 100);

    let plain = Form::new(items());
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &plain,
        false,
        11,
    );
    assert_eq!(plain.rect("below").unwrap().y, 20 + 2 * LINE_H);
}
