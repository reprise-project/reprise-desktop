use super::*;

fn name_field(d: &FileDlg, t: &TextRenderer, px: u32) -> Rect {
    d.layout(t, px, R, &d.drawn.0.get()).filename
}

#[test]
fn text_dropped_on_file_name_lands_at_the_drop_caret_and_takes_the_focus() {
    let (t, px) = (text(), Theme::default().fonts.size_px);
    let mut d = sample();
    d.set_file_name("ab.txt");
    d.focus = Focus::Shell;
    let f = name_field(&d, &t, px);
    let x = f.x + widgets::FIELD_PAD + t.measure("ab", false, px);
    assert!(
        !d.drag_over(&t, px, R, f.x, f.y - 20),
        "the view takes no text"
    );
    assert!(d.drag_over(&t, px, R, x, f.y + 5));
    assert!(d.drop_text(&t, px, R, &crate::textedit::DropLine::Text("c".into())));
    assert_eq!(d.file_name(), "abc.txt");
    assert_eq!(d.filename.selected_text(), "c");
    assert_eq!(d.focus, Focus::FileName);
    assert!(
        !d.drop_text(&t, px, R, &crate::textedit::DropLine::Text("x".into())),
        "the drop ended the drag"
    );
    assert_eq!(d.typed, "abc.txt", "the dropped text counts as typed");
}

#[test]
fn dropped_files_replace_file_name_quoted_only_where_several_are_picked() {
    let (t, px) = (text(), Theme::default().fonts.size_px);
    let names = ["a.txt".to_string(), "/tmp/my b.txt".to_string()];
    let mut one = sample();
    one.set_file_name("old");
    let f = name_field(&one, &t, px);
    assert!(!one.drop_names(&names), "no drag rests on the field");
    assert!(one.drag_over(&t, px, R, f.x + 10, f.y + 5));
    assert!(one.drop_names(&names));
    assert_eq!(one.file_name(), "a.txt");
    assert_eq!(one.filename.selected_text(), "a.txt");
    let mut many = multi(DlgMode::Open, true);
    assert!(many.drag_over(&t, px, R, f.x + 10, f.y + 5));
    assert!(many.drop_names(&names));
    assert_eq!(many.file_name(), r#""a.txt" "/tmp/my b.txt""#);
    assert_eq!(
        split_names(many.file_name()),
        names,
        "Accept takes them back apart"
    );
}

#[test]
fn a_drag_resting_at_file_names_edge_scrolls_it_and_draws_a_drop_caret() {
    let (t, theme) = (text(), Theme::default());
    let px = theme.fonts.size_px;
    let mut d = sample();
    d.set_file_name(&"long name ".repeat(12));
    let f = name_field(&d, &t, px);
    let (y, left) = (f.y + 5, f.x + 2);
    assert!(d.drag_over(&t, px, R, f.x + 40, y));
    assert_eq!(d.drag_scroll(&t, px, R), None, "the middle rests still");
    let end = d.name_drop.unwrap().1;
    assert!(end > 0, "the caret at the end scrolled the name");
    assert!(d.drag_over(&t, px, R, left, y));
    assert!(d.drag_tick(&t, px, R));
    assert_eq!(d.name_drop, Some((left, end - 1)));
    let shot = |d: &FileDlg| {
        let mut c = Canvas::new(R.width as u32, R.height as u32);
        d.draw(&mut c, &t, &classic_paint(&theme.colors, 1), R, px);
        c
    };
    let dropping = shot(&d);
    assert!(d.drag_leave());
    assert!(!d.drag_tick(&t, px, R), "no drag, no step");
    assert_ne!(dropping.pixels, shot(&d).pixels);
}
