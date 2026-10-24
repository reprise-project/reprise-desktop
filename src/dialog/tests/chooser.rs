use super::*;

/// At 200% text the Open With chooser's and the Browse dialog's clients double inside their
/// frames, and a click lands on the grown Cancel.
#[test]
fn open_with_and_browse_grow_with_the_text_size() {
    let theme = Theme::default().with_text_size(200);
    let text = TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    let caption_grow = theme.metrics.caption_height - 18;
    let d = chooser();
    let l = DialogLayout::compute(&theme, &text, &d);
    let (cw, ch) = openwith::CLIENT;
    assert_eq!(l.page.map(|p| (p.width, p.height)), Some((2 * cw, 2 * ch)));
    assert_eq!(l.size, (2 * cw + 6, 2 * ch + 21 + caption_grow + 3));

    let dir = std::env::temp_dir().join(format!("reprise-browse-grown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = Dialog::browse(crate::filepick::FilePicker::run_command(
        &dir,
        crate::filepick::Icons::default(),
        true,
    ));
    d.fit(&theme, &text);
    let l = DialogLayout::compute(&theme, &text, &d);
    let page = l.page.unwrap();
    assert_eq!((page.width, page.height), (1110, 902));
    let c = render(&theme, &text, &l, &d);
    assert_eq!((c.width as i32, c.height as i32), l.size);
    // Cancel: 150x46, 12 px in from the right, 2 px above the Files of type row.
    let (x, y) = (page.right() - 12 - 150 + 10, page.y + 844 - 2 + 10);
    assert_eq!(
        l.hit(&d, x, y, &text, px),
        Some(DialogHit::Browse(
            reprisekit::controls::filedlg::Hit::Cancel
        ))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_inactive_glass_chooser_draws_its_ok_plain() {
    let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let d = chooser();
    let l = DialogLayout::compute(&theme, &text, &d);
    let client = l.page.expect("the chooser's client");
    let ok = d
        .chooser()
        .unwrap()
        .ow
        .layout(&text, px, (client.x, client.y), &l.controls)
        .ok;
    let top = |active| {
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            active,
            None,
        );
        c.get(ok.x + 10, ok.y + 1)
    };
    assert_eq!(top(true), 0xFF3B_76AD, "active, the default frame");
    assert_eq!(top(false), 0xFF6D_6D6D, "inactive, a plain button");
}

#[test]
fn open_with_hosts_the_chooser_in_its_frame_and_answers_for_the_file() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let d = chooser();
    assert_eq!(d.title, "Open With");
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (396, 419));
    assert_eq!(l.page, Some(Rect::new(3, 21, 390, 395)));
    let px = theme.fonts.size_px;
    assert_eq!(
        l.hit(&d, 27 + 40, 102 + 2 + 3 * 18 + 5, &text, px),
        Some(DialogHit::Chooser(openwith::Hit::List(
            grouplist::Hit::Row(3)
        )))
    );
    assert_eq!(
        l.hit(&d, 250, 394, &text, px),
        Some(DialogHit::Chooser(openwith::Hit::Ok))
    );
    let mut d2 = d.clone();
    assert_eq!(
        d2.key(Key::Enter),
        Outcome::OpenWith {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            app: "app1.desktop".into(),
            always: false,
            mime: Some("text/plain".into()),
        }
    );
    let browse = Some(DialogHit::Chooser(openwith::Hit::Browse));
    assert_eq!(d2.click(browse), Outcome::Redraw);
    assert_eq!(d2.release(browse), Outcome::BrowseProgram);
    assert_eq!(d2.key(Key::Escape), Outcome::Close);
    assert_eq!(
        d2.add_program(std::path::Path::new("/usr/bin/nano")),
        Outcome::Redraw
    );
    match d2.key(Key::Enter) {
        Outcome::OpenWith { app, .. } => assert_eq!(app, "/usr/bin/nano"),
        other => panic!("expected OpenWith for /usr/bin/nano, got {other:?}"),
    }
    use std::os::unix::ffi::OsStrExt;
    let odd = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/usr/bin/a\xffb"));
    assert!(matches!(d2.add_program(odd), Outcome::Notice(n) if n.title == "Open With"));
    let mut d3 = d.clone();
    d3.key(Key::Tab { back: false });
    assert_eq!(
        d3.key(Key::Edit(EditKey::Insert(" ".into()))),
        Outcome::Redraw
    );
    assert!(d3.chooser().unwrap().ow.always.on, "Space ticks the box");
    let mut d4 = d.clone();
    assert_eq!(
        d4.key(Key::Edit(EditKey::Insert("o".into()))),
        Outcome::Redraw
    );
    assert_eq!(
        d4.chooser().unwrap().ow.list.selected,
        Some(3),
        "o picks the Other Programs header"
    );
    let mut d5 = d.clone();
    assert_eq!(
        d5.key(Key::Edit(EditKey::Insert("op".into()))),
        Outcome::Nothing
    );
    assert_eq!(
        d5.chooser().unwrap().ow.list.selected,
        Some(1),
        "the pick stays put"
    );
}

#[test]
fn the_wheel_scrolls_the_chooser_list_under_the_pointer() {
    let mut d = chooser();
    let row = Some(DialogHit::Chooser(openwith::Hit::List(
        grouplist::Hit::Row(3),
    )));
    let before = d.chooser().unwrap().ow.clone();
    assert_eq!(d.wheel(row, 3), Outcome::Redraw);
    let ow = &d.chooser().unwrap().ow;
    assert_eq!(ow.list.top, 3);
    assert_eq!(ow.list.selected, before.list.selected, "the pick stays");
    let ok = Some(DialogHit::Chooser(openwith::Hit::Ok));
    assert_eq!(d.wheel(ok, 3), Outcome::Nothing, "off the list");
    assert_eq!(d.wheel(row, -9), Outcome::Redraw);
    assert_eq!(d.chooser().unwrap().ow.list.top, 0);
}

#[test]
fn a_chooser_button_acts_on_a_release_over_it() {
    let ok = Some(DialogHit::Chooser(openwith::Hit::Ok));
    let mut d = chooser();
    assert_eq!(d.click(ok), Outcome::Redraw, "the press only arms OK");
    assert_eq!(
        d.release(ok),
        Outcome::OpenWith {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            app: "app1.desktop".into(),
            always: false,
            mime: Some("text/plain".into()),
        }
    );
    let mut d = chooser();
    d.click(ok);
    assert_eq!(d.release(None), Outcome::Redraw, "off OK it only pops up");
    assert!(!d.chooser().unwrap().ow.holding());
}

#[test]
fn a_double_click_on_a_program_row_opens_and_on_a_header_only_picks() {
    let row = |i| {
        Some(DialogHit::Chooser(openwith::Hit::List(
            grouplist::Hit::Row(i),
        )))
    };
    let mut d = chooser();
    assert_eq!(
        d.double_click(row(2)),
        Outcome::OpenWith {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            app: "app2.desktop".into(),
            always: false,
            mime: Some("text/plain".into()),
        }
    );
    assert_eq!(d.double_click(row(3)), Outcome::Redraw);
    assert_eq!(d.chooser().unwrap().ow.list.selected, Some(3));
}

#[test]
fn the_chooser_opens_on_the_default_program() {
    let others: Vec<_> = (0..12)
        .map(|i| Program {
            id: format!("app{i}.desktop"),
            name: format!("Program {i:02}"),
            icon: None,
        })
        .collect();
    let open = |pick| {
        Dialog::open_with(
            std::path::Path::new("/tmp/a.txt"),
            Some("text/plain".into()),
            vec![],
            others.clone(),
            pick,
            false,
        )
    };
    let d = open(Some("app11.desktop"));
    let ow = &d.chooser().unwrap().ow;
    assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("app11.desktop"));
    assert_eq!(ow.list.top, 13 - openwith::ROWS, "the last row is in view");
    let d = open(Some("gone.desktop"));
    let ow = &d.chooser().unwrap().ow;
    assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("app0.desktop"));
    assert_eq!(ow.list.top, 0);
}

#[test]
fn enter_on_an_empty_chooser_does_nothing() {
    let mut d = Dialog::open_with(
        std::path::Path::new("/x"),
        None,
        vec![],
        vec![],
        None,
        false,
    );
    assert_eq!(d.key(Key::Enter), Outcome::Nothing);
}

#[test]
fn change_default_chooser_only_sets_the_default() {
    let mut d = Dialog::open_with(
        std::path::Path::new("/home/u/Desktop/New Text Document.txt"),
        Some("text/plain".into()),
        vec![Program {
            id: "app1.desktop".into(),
            name: "Micro".into(),
            icon: Some("micro".into()),
        }],
        Vec::new(),
        None,
        true,
    );
    let greyed = Check {
        on: true,
        enabled: false,
    };
    assert_eq!(
        d.chooser().unwrap().ow.always,
        greyed,
        "ticked and greyed from the start"
    );
    assert_eq!(
        d.click(Some(DialogHit::Chooser(openwith::Hit::Always))),
        Outcome::Nothing
    );
    assert_eq!(d.chooser().unwrap().ow.always, greyed, "still ticked");
    assert_eq!(
        d.key(Key::Enter),
        Outcome::SetDefault {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            app: "app1.desktop".into(),
            mime: Some("text/plain".into()),
        }
    );
    assert_eq!(d.key(Key::Access('b')), Outcome::BrowseProgram);
    d.add_program(std::path::Path::new("/opt/tools/viewer"));
    assert_eq!(
        d.key(Key::Enter),
        Outcome::SetDefault {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            app: "/opt/tools/viewer".into(),
            mime: Some("text/plain".into()),
        }
    );
    let mut sheet = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
    assert!(sheet.set_opens_with("Vim".into(), Some("vim".into())));
    assert_eq!(
        sheet_form(&sheet).item("v_opens").map(|i| &i.control),
        Some(&reprisekit::controls::form::Control::Lines(vec![
            "Vim".into()
        ]))
    );
    assert!(matches!(
        &sheet.form().unwrap().kind,
        FormKind::Properties(b) if b.opens_icon.as_deref() == Some("vim")
    ));
    assert_eq!(
        sheet.sheet_path(),
        Some(std::path::Path::new(
            "/home/u/Desktop/New Text Document.txt"
        ))
    );
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let l = DialogLayout::compute(&theme, &text, &d);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let lay =
        d.chooser()
            .unwrap()
            .ow
            .layout(&text, theme.fonts.size_px, (3, 21), &theme.look.controls());
    let (b, lr) = (lay.check, lay.check_label);
    let has = |r: Rect, color: u32| {
        (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| c.get(x, y) == color))
    };
    assert!(has(b.inset(2), cs.button_face.argb()), "greyed face");
    assert!(has(b, cs.button_shadow.argb()), "the mark is shadow grey");
    assert!(!has(b, cs.button_text.argb()), "and never black");
    assert!(has(lr, cs.button_shadow.argb()) && has(lr, cs.button_hilight.argb()));
    assert!(!has(lr, cs.button_text.argb()), "the label is etched");
}

#[test]
fn open_with_chooser_draws_group_list_and_selection() {
    let theme = Theme::default();
    let Some(text) = tahoma() else { return };
    let d = chooser();
    let l = DialogLayout::compute(&theme, &text, &d);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let cs = &theme.colors;
    let lay =
        d.chooser()
            .unwrap()
            .ow
            .layout(&text, theme.fonts.size_px, (3, 21), &theme.look.controls());
    let g = lay.group;
    assert_eq!(c.get(g.x + 200, g.y), cs.button_shadow.argb());
    assert_eq!(c.get(g.x + 200, g.y + 1), cs.button_hilight.argb());
    assert_eq!(c.get(g.x + 12, g.y), cs.button_face.argb());
    assert_eq!(c.get(g.x, g.y + 100), cs.button_shadow.argb());
    let list = lay.list.frame;
    assert_eq!(c.get(list.x, list.y + 50), cs.button_shadow.argb());
    assert_eq!(c.get(list.x + 1, list.y + 50), cs.button_dk_shadow.argb());
    assert_eq!(c.get(list.x + 10, list.y + 50), cs.window.argb());
    let sel_y = list.y + 2 + 18 + 8;
    assert_eq!(c.get(list.x + 45 + 3, sel_y), cs.hilight.argb());
    assert_eq!(c.get(list.x + 300, sel_y - 18), cs.window.argb());
    let cb = lay.check;
    assert_eq!(c.get(cb.x + 6, cb.y + 6), cs.window.argb());
    assert_eq!(c.get(lay.ok.x, lay.ok.y + 10), cs.button_text.argb());
}

#[test]
fn browse_is_a_frame_around_the_file_dialog() {
    let theme = Theme::default();
    let Some(text) = tahoma() else { return };
    let dir = std::env::temp_dir().join(format!("reprise-browse-frame-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = Dialog::browse(crate::filepick::FilePicker::run_command(
        &dir,
        crate::filepick::Icons::default(),
        true,
    ));
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (563, 477));
    assert_eq!(l.page, Some(Rect::new(4, 22, 555, 451)));
    assert!(l.help.is_some());
    let px = theme.fonts.size_px;
    assert_eq!(
        l.hit(&d, 4 + 472 + 30, 22 + 416 + 10, &text, px),
        Some(DialogHit::Browse(
            reprisekit::controls::filedlg::Hit::Cancel
        ))
    );
    let client = l.page.unwrap();
    assert_eq!(
        d.picker_mut()
            .unwrap()
            .click(client, Some(reprisekit::controls::filedlg::Hit::Cancel)),
        Outcome::Close
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_grown_browse_dialog_frames_its_bigger_client() {
    use reprisekit::controls::filedlg::{CLIENT, Hit};
    let (dw, dh) = (168, 55);
    let theme = Theme::default();
    let Some(text) = tahoma() else { return };
    let dir = std::env::temp_dir().join(format!("reprise-browse-grown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = Dialog::browse(crate::filepick::FilePicker::run_command(
        &dir,
        crate::filepick::Icons::default(),
        true,
    ));
    let draw_it = |d: &Dialog, l: &DialogLayout| {
        draw(
            &theme,
            &text,
            l,
            None,
            None,
            &[],
            &[],
            None,
            d,
            false,
            None,
            Cues::NONE,
            true,
            None,
        )
    };
    let small_l = DialogLayout::compute(&theme, &text, &d);
    let small = draw_it(&d, &small_l);
    d.picker_mut()
        .unwrap()
        .dlg
        .set_size((CLIENT.0 + dw, CLIENT.1 + dh));
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (731, 532));
    assert_eq!(l.page, Some(Rect::new(4, 22, 723, 506)));
    assert_eq!(
        l.close,
        small_l.close.map(|r| Rect::new(r.x + dw, r.y, 16, 14))
    );
    let m = &theme.metrics;
    assert_eq!(l.sizing_edges(1, 300, m), Some(Edges::LEFT));
    assert_eq!(l.sizing_edges(729, 300, m), Some(Edges::RIGHT));
    assert_eq!(l.sizing_edges(300, 1, m), Some(Edges::TOP));
    assert_eq!(
        l.sizing_edges(730, 531, m),
        Some(Edges::BOTTOM | Edges::RIGHT)
    );
    assert_eq!(l.sizing_edges(300, 10, m), None, "the caption moves it");
    assert_eq!(l.sizing_edges(300, 300, m), None, "the client is no border");
    let px = theme.fonts.size_px;
    assert_eq!(
        l.hit(&d, 4 + 472 + 30 + dw, 22 + 416 + 10 + dh, &text, px),
        Some(DialogHit::Browse(Hit::Cancel))
    );
    assert_eq!(
        l.hit(&d, 4 + 723 - 4, 22 + 506 - 4, &text, px),
        Some(DialogHit::Browse(Hit::Grip))
    );
    let client = l.page.unwrap();
    assert_eq!(
        d.picker_mut().unwrap().click(client, Some(Hit::Grip)),
        Outcome::Size(Edges::BOTTOM | Edges::RIGHT)
    );
    let big = draw_it(&d, &l);
    for y in 477 - 80..477 {
        for x in 563 - 120..563 {
            assert_eq!(big.get(x + dw, y + dh), small.get(x, y), "at ({x}, {y})");
        }
    }
    for y in 22..82 {
        for x in 0..90 {
            assert_eq!(big.get(x, y), small.get(x, y), "at ({x}, {y})");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_browse_dialogs_column_title_lights_under_the_pointer() {
    use reprisekit::controls::{filedlg, shellview};
    let theme = blue();
    let text = TextRenderer::load(&FontConfig::default());
    let dir = std::env::temp_dir().join(format!("reprise-browse-hover-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = Dialog::browse(crate::filepick::FilePicker::run_command(
        &dir,
        crate::filepick::Icons::default(),
        true,
    ));
    let l = DialogLayout::compute(&theme, &text, &d);
    let client = l.page.expect("the picker's page");
    let mut c = Canvas::new(600, 500);
    let px = theme.fonts.size_px;
    d.picker()
        .unwrap()
        .dlg
        .draw(&mut c, &text, &theme.paint(), client, px);
    let title = Some(DialogHit::Browse(filedlg::Hit::Shell(
        shellview::Hit::Header(0),
    )));
    assert!(d.hover(title, (0, 0), true), "the title lights");
    assert!(d.leave(true), "and goes cold");
    let _ = std::fs::remove_dir_all(&dir);
}
