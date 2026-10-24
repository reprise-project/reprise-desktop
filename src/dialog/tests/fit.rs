use super::*;

/// A dialog open when the text size changes lays out as one opened at the new size: the
/// config reload refits every open dialog (`configwatch::retheme`).
#[test]
fn an_open_dialog_lays_out_again_at_a_new_text_size() {
    use reprisekit::controls::filesheet;
    let Some(text) = tahoma() else { return };
    let one = Theme::default();
    let big = Theme::default().with_text_size(200);
    let opened = |theme: &Theme| {
        let (px, u) = (theme.fonts.size_px, theme.text);
        let path = std::path::Path::new("/tmp/a.txt");
        let mut ds = vec![
            Dialog::run(&history()),
            Dialog::confirm_delete("a.txt", path, false, false, false),
            Dialog::advanced_attributes(filesheet::Subject::File, None, &text, px, u),
        ];
        ds.iter_mut().for_each(|d| d.fit(theme, &text));
        ds
    };
    for (mut d, fresh) in opened(&one).into_iter().zip(opened(&big)) {
        d.fit(&big, &text);
        assert_eq!(
            DialogLayout::compute(&big, &text, &d),
            DialogLayout::compute(&big, &text, &fresh),
            "{}",
            d.title
        );
    }
}

fn measured_forms() -> Vec<Measured> {
    use reprisekit::controls::{endsession, locationprompt, replacebox};
    let side = |size: &str| replacebox::Side {
        size: size.into(),
        modified: "modified: Monday, September 14, 2026, 3:43:53 PM".into(),
        icon: None,
    };
    let replace = |subject, many| replacebox::Fields {
        name: "Quarterly report for the board.txt".into(),
        folder: None,
        subject,
        icon: None,
        many,
    };
    vec![
        Measured::Replace(replace(
            replacebox::Subject::File {
                existing: Box::new(side("size: 8 bytes")),
                incoming: Box::new(side("size: 12.4 KB")),
            },
            false,
        )),
        Measured::Replace(replace(replacebox::Subject::Folder, true)),
        Measured::EndSession(endsession::Fields {
            goal: Goal::Restart,
            rows: vec![endsession::Row {
                name: "Text Editor".into(),
                reason: "Unsaved changes".into(),
            }],
            icon: None,
        }),
        Measured::Location(locationprompt::Fields {
            program: "Maps".into(),
            icon: None,
        }),
        Measured::Attributes {
            verb: "Hide".into(),
            subject: Subject::Set { has_dir: true },
        },
        Measured::Advanced {
            subject: Subject::Folder,
            robots: Some([CheckState::On, CheckState::Off]),
        },
    ]
}

fn opened_measured(m: &Measured, theme: &Theme, text: &TextRenderer) -> Dialog {
    let (px, u) = (theme.fonts.size_px, theme.text);
    let form = FormDialog::measured(m.clone(), FormKind::EndSession, text, px, u);
    let mut d = Dialog::new("Measured", Style::Form(Box::new(form)), Placement::Center);
    d.fit(theme, text);
    d
}

#[test]
fn a_measured_dialog_is_built_again_at_a_new_text_size_and_font() {
    let Some(tahoma) = tahoma() else { return };
    let dejavu = TextRenderer::load(&FontConfig {
        family: vec!["DejaVu Sans".into()],
        ..FontConfig::default()
    });
    let one = Theme::default();
    let big = Theme::default().with_text_size(200);
    let mut moves = vec![
        ((&one, &tahoma), (&big, &tahoma)),
        ((&big, &tahoma), (&one, &tahoma)),
    ];
    if dejavu.has_face("DejaVu Sans") {
        moves.push(((&one, &dejavu), (&one, &tahoma)));
    }
    for (i, m) in measured_forms().iter().enumerate() {
        for &((from, from_text), (to, to_text)) in &moves {
            let mut d = opened_measured(m, from, from_text);
            d.fit(to, to_text);
            assert!(d == opened_measured(m, to, to_text), "form {i}");
        }
    }
}

#[test]
fn a_measured_dialog_built_again_keeps_what_its_controls_hold() {
    use reprisekit::controls::form::Control;
    use reprisekit::controls::{endsession, locationprompt};
    let text = TextRenderer::load(&FontConfig::default());
    let big = Theme::default().with_text_size(200);
    for m in measured_forms() {
        let mut d = opened_measured(&m, &Theme::default(), &text);
        let Some(form) = d.form_mut().and_then(|f| f.open_page_mut()) else {
            panic!("a plain form");
        };
        let rows = vec![endsession::Row {
            name: "Mail".into(),
            reason: String::new(),
        }];
        match m {
            Measured::Location(_) => form.set_check(locationprompt::REMEMBER, false),
            Measured::EndSession(_) => assert!(endsession::set_rows(form, &rows)),
            _ => {}
        }
        let last = form.items.iter().rev().find_map(|it| match it.control {
            Control::Button { .. } => Some(it.id),
            _ => None,
        });
        form.focus_id(last.expect("a button"));
        let before = form.clone();
        d.fit(&big, &text);
        let after = d.form().and_then(|f| f.open_page()).unwrap();
        let focused = |f: &reprisekit::controls::form::Form| f.focus().map(|i| f.items[i].id);
        assert_eq!(focused(after), focused(&before));
        for a in &after.items {
            if !matches!(a.control, Control::Label { .. } | Control::Lines(_)) {
                assert_eq!(Some(&a.control), before.item(a.id).map(|b| &b.control));
            }
        }
    }
}

/// The usable area of a 1920x1080 output over a 40 px taskbar.
const AREA_1080P: (i32, i32) = (1920, 1040);

/// The tray's Customize dialog in `theme`'s words over three items, as the tray opens it.
fn tray_dialog(theme: &Theme) -> Dialog {
    use reprisekit::controls::traycustomize::{Behaviour, Entry, Wording};
    let entry = |key: &str, name: &str| Entry {
        key: key.into(),
        name: name.into(),
        detail: format!("{name}: all is well"),
        icon: std::sync::Arc::from([]),
        passive: false,
        behaviour: Behaviour::Auto,
    };
    let w = Wording::of(theme);
    Dialog::tray_customize(TrayCustomize::new(
        w,
        vec![entry("nm-applet", "Network"), entry("volume", "Volume")],
        vec![entry("sticky", "Sticky Keys")],
        (w == Wording::Glass).then_some(false),
        theme.look.controls(),
    ))
}

/// At 100% the tray's Customize page keeps its 1184x602 client on a 1080p output; at 150% it
/// grows with the text, and at 200% it fits the output, its page scrolling above the band.
/// A text size change while it is open lays it out again.
#[test]
fn the_tray_customize_page_grows_and_fits_its_output() {
    let text = TextRenderer::load(&FontConfig::default());
    let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let mut d = tray_dialog(&glass);
    let client = |d: &Dialog| d.tray().unwrap().client();
    d.fit_in(&glass, &text, AREA_1080P);
    assert_eq!(client(&d), (1184, 602));
    let big = glass.with_text_size(150);
    d.fit_in(&big, &text, AREA_1080P);
    assert_eq!(client(&d), big.text.size((1184, 602)));
    let huge = glass.with_text_size(200);
    d.fit_in(&huge, &text, AREA_1080P);
    let l = DialogLayout::compute(&huge, &text, &d);
    assert!(
        l.size.0 <= AREA_1080P.0 && l.size.1 <= AREA_1080P.1,
        "{:?}",
        l.size
    );
    let client = l.page.unwrap();
    let tray = d
        .tray()
        .unwrap()
        .layout(&text, huge.fonts.size_px, (client.x, client.y));
    assert!(tray.page_bar.is_some(), "the page scrolls");
    assert_eq!(tray.band.unwrap().bottom(), client.bottom());
    d.fit_in(&glass, &text, AREA_1080P);
    assert_eq!(d.tray().unwrap().client(), (1184, 602), "back at 100%");
}

/// At 200% on an 800x600 output Glass's Customize page stacks its rows and keeps inside the
/// work area, its buttons in the client.
#[test]
fn the_glass_tray_customize_page_fits_an_800x600_output_at_200() {
    const AREA_600: (i32, i32) = (800, 560);
    let text = TextRenderer::load(&FontConfig::default());
    let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass)
        .with_text_size(200);
    let mut d = tray_dialog(&theme);
    d.fit_in(&theme, &text, AREA_600);
    let l = DialogLayout::compute(&theme, &text, &d);
    assert!(
        l.size.0 <= AREA_600.0 && l.size.1 <= AREA_600.1,
        "{:?}",
        l.size
    );
    let tray = d.tray().unwrap();
    let client = tray.client();
    let parts = tray.layout(&text, theme.fonts.size_px, (0, 0));
    assert!(parts.cancel.right() <= client.0);
    assert!(parts.cancel.bottom() <= client.1);
    assert_eq!(tray.overflows(&text, theme.fonts.size_px), vec![]);
}

/// At 200% on a 1024x768 output the Customize dialog keeps inside the work area with its
/// buttons in the client, under each interface; at 100% Classic's and Blue's room changes
/// nothing (Glass's 1184 px page narrows to the output at any size).
#[test]
fn the_tray_customize_dialog_fits_a_small_output_at_200() {
    use reprisekit::interface::Interface;
    const AREA_768: (i32, i32) = (1024, 708);
    let text = TextRenderer::load(&FontConfig::default());
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = reprisekit::interfaces::theme_for(i);
        let mut d = tray_dialog(&theme);
        d.fit(&theme, &text);
        let whole = DialogLayout::compute(&theme, &text, &d);
        d.fit_in(&theme, &text, AREA_768);
        let fitted = DialogLayout::compute(&theme, &text, &d);
        if i != Interface::Glass {
            assert_eq!(fitted, whole, "{i:?} at 100%");
        }
        let huge = theme.with_text_size(200);
        d.fit_in(&huge, &text, AREA_768);
        let l = DialogLayout::compute(&huge, &text, &d);
        assert!(
            l.size.0 <= AREA_768.0 && l.size.1 <= AREA_768.1,
            "{i:?}: {:?}",
            l.size
        );
        let tray = d.tray().unwrap();
        let client = tray.client();
        let parts = tray.layout(&text, huge.fonts.size_px, (0, 0));
        assert!(parts.cancel.right() <= client.0, "{i:?}");
        assert!(parts.cancel.bottom() <= client.1, "{i:?}");
    }
}

/// Every compositor dialog laid out as a form, under each interface, at each text size.
fn form_dialogs(theme: &Theme, text: &TextRenderer) -> Vec<Dialog> {
    use reprisekit::controls::iconlist::IconList;
    let (px, u) = (theme.fonts.size_px, theme.text);
    let mut folder = file_info();
    folder.is_dir = true;
    folder.opens_with = None;
    folder.name = "Reports".into();
    folder.hidden_ext = String::new();
    folder.path = "/home/u/Desktop/Reports".into();
    folder.contains = Some((12, 3));
    let mut launcher = file_info();
    launcher.name = "Shell".into();
    launcher.hidden_ext = String::new();
    launcher.path = "/home/u/Desktop/shell.desktop".into();
    launcher.is_shortcut = true;
    launcher.launcher = Some(reprisefs::launcher::Launcher {
        target: "/bin/sh".into(),
        ..Default::default()
    });
    let bin = crate::binprops::BinSheet::new(
        reprisefs::bincap::Policy::default(),
        vec![crate::binprops::Drive {
            key: "home".into(),
            label: "Local Disk (/home)".into(),
            size: 21_365_735_424,
        }],
    );
    let confirm = |write| AttrConfirm {
        write,
        attrs: reprisefs::props::AttrMoves {
            read_only: Some(true),
            ..Default::default()
        },
        close: true,
    };
    let one = AttrWrite::One {
        path: "/d/Reports".into(),
        name: Some(".Reports".into()),
        hidden: Some(true),
        folder_icon: None,
    };
    let set = AttrWrite::Set {
        paths: vec!["/d/a".into(), "/d/Reports".into()],
        hidden: Some(true),
        folder_icon: None,
    };
    let icons = IconList::new(
        ["folder", "folder-music"].map(String::from).to_vec(),
        vec![None, None],
        None,
    );
    let mut zip = file_info();
    zip.opens_with = Some("Compressed (zipped) Folders".into());
    let mut ds = vec![
        Dialog::properties(&file_info(), "text-x-generic", (0, 0)),
        Dialog::properties(&zip, "package-x-generic", (0, 0)),
        Dialog::properties(&folder, "folder", (0, 0)),
        Dialog::properties(&launcher, "utilities-terminal", (0, 0)),
        Dialog::set_properties(&a_set(), "Reports", (0, 0)),
        crate::binprops::dialog(bin, (0, 0), "user-trash"),
        Dialog::change_icon("Reports", "theme", icons, false),
        Dialog::advanced_attributes(Subject::File, None, text, px, u),
        Dialog::advanced_attributes(Subject::Folder, None, text, px, u),
        Dialog::advanced_attributes(Subject::Set { has_dir: true }, None, text, px, u),
        Dialog::confirm_attributes(confirm(one), text, px, u),
        Dialog::confirm_attributes(confirm(set), text, px, u),
    ];
    // Dates in a long weekday and month, whatever the locale.
    let date = "Wednesday, September 30, 2026, 12:00:00 PM";
    for d in &mut ds[..4] {
        let page = &mut d.form_mut().unwrap().pages_mut()[0];
        for id in ["v_made", "v_mod", "v_acc"] {
            page.set_text(id, date);
        }
    }
    ds.iter_mut().for_each(|d| d.fit(theme, text));
    ds.extend(
        measured_forms()
            .iter()
            .map(|m| opened_measured(m, theme, text)),
    );
    ds
}

/// Every compositor dialog drawn from a template or as a Welcome panel.
fn template_dialogs() -> Vec<Dialog> {
    let path = std::path::Path::new("/home/u/Desktop/Quarterly report.txt");
    let paths: Vec<std::path::PathBuf> = vec!["/d/a".into(), "/d/b".into()];
    let too_big = reprisefs::bincap::TooBig::new(
        vec!["/d/big.iso".into()],
        vec![Some(reprisefs::bincap::Measured::unstamped(2, 1))],
    );
    let mut ds = vec![
        Dialog::run(&history()),
        Dialog::log_off(),
        Dialog::turn_off(&power()),
        Dialog::confirm_delete("Quarterly report.txt", path, false, false, false),
        Dialog::confirm_delete("Reports", path, true, true, true),
        Dialog::confirm_delete_many(&paths, false, BinLack::None),
        Dialog::confirm_delete_many(&paths, true, BinLack::All),
        Dialog::too_big(std::path::Path::new("/d/big.iso"), false, too_big),
        Dialog::confirm_delete_icons(vec!["computer".into()], paths.clone()),
        Dialog::end_program("Text Editor", 42),
        Dialog::error("Run", "Reprise cannot find 'frobnicate'.".into()),
        Dialog::confirm_empty_bin(3, None),
        Dialog::confirm_empty_bin(1, Some("Quarterly report.txt")),
    ];
    ds.extend(
        [
            crate::desktop::Place::Documents,
            crate::desktop::Place::Computer,
            crate::desktop::Place::Home,
        ]
        .map(|p| Dialog::confirm_hide(p, &p.label())),
    );
    ds.extend(
        [
            crate::keyfilter::Aid::StickyKeys,
            crate::keyfilter::Aid::SlowKeys,
            crate::keyfilter::Aid::MouseKeys,
        ]
        .map(Dialog::key_aid_prompt),
    );
    ds
}

/// Forms through `Form::overflows`; templates by their lines, field label and push buttons.
#[test]
fn every_dialog_fits_its_text_under_every_interface() {
    use reprisekit::interface::Interface;
    let mut over = Vec::new();
    for family in ["Tahoma", "DejaVu Sans"] {
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            ..FontConfig::default()
        });
        if !text.has_face(family) {
            continue;
        }
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 125, 150, 175, 200] {
                let theme = reprisekit::interfaces::theme_for(i).with_text_size(pct);
                let px = theme.fonts.size_px;
                let mut put = |title: &str, what: String| {
                    over.push(format!("{family}, {}, {pct}%, {title}: {what}", i.as_str()))
                };
                for d in form_dialogs(&theme, &text) {
                    for (n, page) in d.form().unwrap().pages().iter().enumerate() {
                        for (id, by) in page.overflows(&text, px) {
                            put(&d.title, format!("page {n}, {id} {by} px over"));
                        }
                    }
                }
                for mut d in template_dialogs() {
                    d.fit(&theme, &text);
                    let l = DialogLayout::compute(&theme, &text, &d);
                    let t = text.dialog(d.font());
                    let width = |s: &str| t.measure(&split_mnemonic(s).0, false, px);
                    for (r, line) in &l.lines {
                        if width(line) > r.width {
                            put(
                                &d.title,
                                format!("{line:?} {} px over", width(line) - r.width),
                            );
                        }
                    }
                    if let (Some(f), Some(r)) = (&d.field, l.field_label)
                        && width(&f.label) > r.width
                    {
                        put(&d.title, format!("{:?} over", f.label));
                    }
                    if matches!(d.style, Style::Welcome) {
                        continue;
                    }
                    for (b, r) in d.buttons.iter().zip(&l.buttons) {
                        if width(&b.label) > r.width - theme.text.len(6) {
                            put(&d.title, format!("{:?} over its button", b.label));
                        }
                    }
                }
                let mut d = tray_dialog(&theme);
                d.fit_in(&theme, &text, AREA_1080P);
                let l = DialogLayout::compute(&theme, &text, &d);
                if l.size.0 > AREA_1080P.0 || l.size.1 > AREA_1080P.1 {
                    put(&d.title, format!("{:?} past a 1080p output", l.size));
                }
                for (part, by) in d.tray().unwrap().overflows(&text, px) {
                    put(&d.title, format!("{part} {by} px over"));
                }
            }
        }
    }
    assert!(over.is_empty(), "{}", over.join("\n"));
}

/// At 150% and 200% Run grows inside its frame: its lines sit a grown line apart and fit
/// their rows, the field holds a grown line, its open list's rows grow, and the buttons sit
/// inside the dialog under the field. 100% is Run as laid out.
#[test]
fn run_grows_with_the_text_size() {
    let Some(text) = tahoma() else { return };
    let one = DialogLayout::compute(&Theme::default(), &text, &Dialog::run(&history()));
    assert_eq!(one.size, (347, 179));
    for pct in [150, 200] {
        let theme = Theme::default().with_text_size(pct);
        let (u, px) = (theme.text, theme.fonts.size_px);
        let mut d = Dialog::run(&history());
        d.fit(&theme, &text);
        let l = DialogLayout::compute(&theme, &text, &d);
        let caption_grow = theme.metrics.caption_height - 18;
        assert_eq!(l.size.0, 2 * 3 + u.len(347 - 6), "{pct}%");
        assert_eq!(l.size.1, 21 + caption_grow + u.len(179 - 24) + 3, "{pct}%");
        for pair in l.lines.windows(2) {
            assert!(pair[0].0.bottom() <= pair[1].0.y, "{pct}%: lines apart");
        }
        for (r, line) in &l.lines {
            assert!(text.measure(line, false, px) <= r.width, "{pct}%: {line:?}");
            assert!(r.height >= u.len(LINE_H) - 1, "{pct}%");
        }
        let field = l.field.unwrap();
        assert!(
            field.height >= px as i32 + 6,
            "{pct}%: the field holds a line"
        );
        assert!(l.lines.last().unwrap().0.bottom() <= field.y);
        let list = l.list.unwrap();
        assert_eq!(list.height, LIST_ROWS as i32 * u.len(13) + 2, "{pct}%");
        for b in &l.buttons {
            assert!(b.y >= field.bottom(), "{pct}%");
            assert!(
                b.right() <= l.size.0 - 3 && b.bottom() <= l.size.1 - 3,
                "{pct}%"
            );
        }
        let c = render(&theme, &text, &l, &d);
        assert_eq!((c.width as i32, c.height as i32), l.size);
    }
}

/// A message box and a form dialog at 150%: the template's client grows by the units, and a
/// form dialog's outer size and its page's controls grow from the client's corner.
#[test]
fn prompts_and_form_dialogs_grow_with_the_text_size() {
    use reprisekit::controls::filesheet;
    let Some(text) = tahoma() else { return };
    let theme = Theme::default().with_text_size(150);
    let u = theme.text;
    let d = Dialog::confirm_delete(
        "a.txt",
        std::path::Path::new("/tmp/a.txt"),
        false,
        false,
        false,
    );
    let l = DialogLayout::compute(&theme, &text, &d);
    let base = DialogLayout::compute(&Theme::default(), &text, &d);
    assert_eq!(l.size.0, 6 + u.len(base.size.0 - 6));
    for b in &l.buttons {
        assert!((u.len(23) - 1..=u.len(23) + 1).contains(&b.height));
        assert!(b.right() <= l.size.0 - 3 && b.bottom() <= l.size.1 - 3);
    }
    let icon = l.icon.unwrap();
    assert_eq!((icon.width, icon.height), (ICON_PX, ICON_PX));

    let mut adv = Dialog::advanced_attributes(
        filesheet::Subject::File,
        None,
        &text,
        theme.fonts.size_px,
        u,
    );
    adv.fit(&theme, &text);
    let l = DialogLayout::compute(&theme, &text, &adv);
    let f = adv.form().unwrap();
    let caption_grow = theme.metrics.caption_height - 18;
    let (w, h) = filesheet::advanced_size(filesheet::Subject::File);
    assert_eq!(l.size, (6 + u.len(w), 21 + caption_grow + u.len(h) + 3));
    let page = f.open_page().unwrap();
    let ok = page.rect(filesheet::ADV_OK).unwrap();
    let design = page.item(filesheet::ADV_OK).unwrap().rect;
    assert_eq!(ok.width, u.len(design.right()) - u.len(design.x));
    assert!(ok.right() <= l.size.0 && ok.bottom() <= l.size.1);
    let c = render(&theme, &text, &l, &adv);
    assert_eq!((c.width as i32, c.height as i32), l.size);
}
