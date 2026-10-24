use super::*;

#[test]
fn identity_tells_same_named_files_apart() {
    let mut a = file_info();
    a.path = "/one/x.txt".into();
    let mut b = file_info();
    b.path = "/two/x.txt".into();
    let a = Dialog::properties(&a, "text-x-generic", (0, 0));
    let b = Dialog::properties(&b, "text-x-generic", (0, 0));
    assert_eq!(a.title, b.title);
    assert_ne!(a.identity(), b.identity());
    assert_eq!(
        Dialog::run(&history()).identity(),
        Dialog::run(&history()).identity()
    );
}

#[test]
fn the_wheel_scrolls_a_form_dialog_s_open_combo() {
    use reprisekit::controls::form::{Control, Form, Item};
    let items = ["a", "b", "c", "d", "e", "f"];
    let form = Form::new(vec![Item::combo("c", 10, 10, 120, &items, 3)]);
    let fd = FormDialog::plain((200, 100), form, FormKind::ChangeIcon);
    let mut d = Dialog::new("Form", Style::Form(Box::new(fd)), Placement::Center);
    assert_eq!(d.wheel(None, 3), Outcome::Nothing, "nothing to scroll");
    let page = &mut d.form_mut().unwrap().pages_mut()[0];
    let Control::Combo(c) = &mut page.items[0].control else {
        unreachable!()
    };
    c.toggle();
    assert_eq!(d.wheel(None, 3), Outcome::Redraw);
    let Control::Combo(c) = &d.form().unwrap().pages()[0].items[0].control else {
        unreachable!()
    };
    assert_eq!(c.top, 3);
}

#[test]
fn advanced_attributes_hands_its_boxes_to_the_sheet() {
    use reprisekit::controls::filesheet::ADVANCED;
    let (on, off) = (CheckState::On, CheckState::Off);
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut info = file_info();
    info.robots = Some([true, true]);
    let mut d = Dialog::properties(&info, "text-x-generic", (0, 0));
    assert_eq!(
        press_control(&mut d, ADVANCED),
        Outcome::AdvancedAttributes {
            subject: Subject::File,
            robots: Some([on, on]),
        }
    );
    let mut adv =
        Dialog::advanced_attributes(Subject::File, Some([on, on]), &text, px, TextUnits::ONE);
    adv.key(Key::Space);
    assert_eq!(adv.key(Key::Enter), Outcome::AdvancedChosen([off, on]));
    assert!(d.advanced_chosen([off, on]));
    let apply = |d: &Dialog| {
        matches!(
            sheet_form(d)
                .item(reprisekit::controls::propsheet::APPLY)
                .map(|i| &i.control),
            Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
        )
    };
    assert!(apply(&d), "Apply lights");
    assert_eq!(
        press_control(&mut d, reprisekit::controls::propsheet::OK),
        Outcome::Properties {
            path: info.path.clone(),
            name: None,
            attrs: reprisefs::props::AttrMoves {
                backup: Some(false),
                ..Default::default()
            },
            hidden: None,
            folder_icon: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            close: true,
            recurse: None,
        }
    );
    d.sheet_applied(&written(&d.clone().key(Key::Enter)));
    assert!(!apply(&d), "written, Apply greys");
}

#[test]
fn advanced_attributes_frames_the_shared_box() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let d = Dialog::advanced_attributes(Subject::File, None, &text, px, TextUnits::ONE);
    assert_eq!(d.title, "Advanced Attributes");
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (384, 287));
    assert_eq!(l.icon, Some(Rect::new(14, 33, 32, 32)));
    let folder = Dialog::advanced_attributes(Subject::Folder, None, &text, px, TextUnits::ONE);
    assert_eq!(
        DialogLayout::compute(&theme, &text, &folder).size,
        (384, 309)
    );
    assert_eq!(
        d.clone().click(l.hit(&d, 30, 110, &text, px)),
        Outcome::Nothing
    );
    assert_eq!(d.clone().key(Key::Enter), Outcome::Close);
    let mut ok = d.clone();
    let hit = l.hit(&ok, 250, 260, &text, px);
    assert!(
        matches!(hit, Some(DialogHit::Control(_))),
        "OK is a control"
    );
    ok.click(hit);
    assert_eq!(ok.release(None), Outcome::Close);
}

fn written(o: &Outcome) -> Written {
    let Outcome::Properties {
        path,
        name,
        attrs,
        hidden,
        summary,
        folder_icon,
        launcher,
        ..
    } = o.clone()
    else {
        panic!("a Properties write, not {o:?}");
    };
    Written {
        path: name.map_or(path.clone(), |n| path.with_file_name(n)),
        attrs,
        hidden,
        summary,
        folder_icon,
        launcher,
        reread: None,
    }
}

fn sheet_form_mut(d: &mut Dialog) -> &mut reprisekit::controls::form::Form {
    d.form_mut()
        .and_then(|f| f.open_page_mut())
        .expect("a sheet page")
}

fn set_name(d: &mut Dialog, name: &str) {
    use reprisekit::controls::form::Control;
    if let Some(Control::Edit { edit, .. }) = sheet_form_mut(d)
        .item_mut(reprisekit::controls::filesheet::NAME)
        .map(|i| &mut i.control)
    {
        *edit = TextEdit::new(name, false);
    }
}

fn press_control(d: &mut Dialog, id: &str) -> Outcome {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let r = sheet_form(d).rect(id).expect(id);
    let l = DialogLayout::compute(&theme, &text, d);
    let hit = l.hit(d, r.x + 3, r.y + 3, &text, theme.fonts.size_px);
    let out = d.click(hit);
    match d.release(None) {
        Outcome::Nothing => out,
        released => released,
    }
}

#[test]
fn properties_sheet_frames_the_general_page() {
    use reprisekit::controls::filesheet::{ADVANCED, CHANGE, NAME};
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let d = Dialog::properties(&file_info(), "text-x-generic", (600, 500));
    assert_eq!(d.title, "New Text Document Properties");
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (367, 502));
    assert_eq!(l.icon, Some(Rect::new(24, 62, 32, 32)));
    let page = l.page.unwrap();
    assert_eq!((page.x, page.y, page.right()), (9, 49, 358));
    let form = sheet_form(&d);
    assert_eq!(form.rect(NAME).unwrap(), Rect::new(97, 69, 246, 23));
    assert_eq!(
        reprisekit::controls::filesheet::typed_name(form),
        "New Text Document"
    );
    assert_eq!(
        d.clone().key(Key::Enter),
        Outcome::Close,
        "nothing to write"
    );
    let mut d2 = d.clone();
    set_name(&mut d2, "Notes");
    assert_eq!(
        d2.key(Key::Enter),
        Outcome::Properties {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            name: Some("Notes.txt".into()),
            attrs: Default::default(),
            hidden: None,
            folder_icon: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            close: true,
            recurse: None,
        }
    );
    let mut d3 = d.clone();
    assert_eq!(
        press_control(&mut d3, CHANGE),
        Outcome::ChangeDefault("/home/u/Desktop/New Text Document.txt".into())
    );
    assert_eq!(
        press_control(&mut d3, ADVANCED),
        Outcome::AdvancedAttributes {
            subject: Subject::File,
            robots: None,
        }
    );
    let mut folder = file_info();
    folder.is_dir = true;
    folder.opens_with = None;
    folder.contains = Some((2, 1));
    let mut fd = Dialog::properties(&folder, "folder", (0, 0));
    assert_eq!(
        press_control(&mut fd, ADVANCED),
        Outcome::AdvancedAttributes {
            subject: Subject::Folder,
            robots: None,
        },
        "a folder's Advanced... asks for the folder box"
    );
}

#[test]
fn folder_sheet_reads_calculating_until_its_totals_arrive() {
    use reprisekit::controls::form::Control;
    let mut folder = file_info();
    folder.is_dir = true;
    folder.opens_with = None;
    folder.type_name = "File Folder".into();
    let mut d = Dialog::properties(&folder, "folder", (0, 0));
    let values = |d: &Dialog| -> Vec<String> {
        ["v_size", "v_disk", "v_has"]
            .iter()
            .map(|id| match sheet_form(d).item(id).map(|i| &i.control) {
                Some(Control::Label { text, .. }) => text.clone(),
                other => panic!("{other:?}"),
            })
            .collect()
    };
    assert_eq!(values(&d), vec!["Calculating..."; 3]);
    let totals = reprisefs::props::FolderTotals {
        files: 2,
        folders: 1,
        size: 1536,
        on_disk: 8192,
    };
    assert!(d.set_folder_totals(totals));
    assert_eq!(
        values(&d),
        vec![
            reprisefs::props::format_size(1536),
            reprisefs::props::format_size(8192),
            "2 Files, 1 Folder".to_string(),
        ]
    );
}

#[test]
fn the_executable_box_writes_the_execute_bit_and_greys_when_it_cannot() {
    use reprisekit::controls::filesheet::EXECUTABLE;
    use reprisekit::controls::propsheet::APPLY;
    let path: std::path::PathBuf = "/home/u/Desktop/New Text Document.txt".into();
    let exec = |on: bool| reprisefs::props::AttrMoves {
        executable: Some(on),
        ..Default::default()
    };
    let mut d = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
    assert_eq!(press_control(&mut d, EXECUTABLE), Outcome::Redraw);
    assert_eq!(
        press_control(&mut d, APPLY),
        Outcome::Properties {
            path: path.clone(),
            name: None,
            attrs: exec(true),
            hidden: None,
            folder_icon: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            close: false,
            recurse: None,
        }
    );
    d.sheet_applied(&Written {
        path: path.clone(),
        attrs: exec(true),
        ..Default::default()
    });
    press_control(&mut d, EXECUTABLE);
    assert!(
        matches!(press_control(&mut d, APPLY), Outcome::Properties { attrs, .. } if attrs == exec(false)),
        "the written state is the new start"
    );
    let mut theirs = file_info();
    (theirs.executable, theirs.can_set_executable) = (true, false);
    let mut d = Dialog::properties(&theirs, "text-x-generic", (0, 0));
    assert!(reprisekit::controls::filesheet::executable(sheet_form(&d)));
    press_control(&mut d, EXECUTABLE);
    assert!(
        reprisekit::controls::filesheet::executable(sheet_form(&d)),
        "a greyed box takes no click"
    );
}

#[test]
fn attribute_boxes_toggle_and_light_apply() {
    use reprisekit::controls::filesheet::{HIDDEN, READ_ONLY};
    use reprisekit::controls::propsheet::{APPLY, OK};
    let apply_on = |d: &Dialog| {
        matches!(
            sheet_form(d).item(APPLY).map(|i| &i.control),
            Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
        )
    };
    let mut d = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
    assert!(!apply_on(&d), "Apply starts greyed");
    assert_eq!(press_control(&mut d, READ_ONLY), Outcome::Redraw);
    assert!(apply_on(&d), "Apply is lit");
    assert_eq!(
        press_control(&mut d, APPLY),
        Outcome::Properties {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            name: None,
            attrs: reprisefs::props::AttrMoves {
                read_only: Some(true),
                ..Default::default()
            },
            hidden: None,
            folder_icon: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            close: false,
            recurse: None,
        }
    );
    d.sheet_applied(&Written {
        path: "/home/u/Desktop/New Text Document.txt".into(),
        attrs: reprisefs::props::AttrMoves {
            read_only: Some(true),
            ..Default::default()
        },
        ..Default::default()
    });
    assert!(!apply_on(&d));
    press_control(&mut d, HIDDEN);
    assert_eq!(
        press_control(&mut d, OK),
        Outcome::Properties {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            name: Some(".New Text Document.txt".into()),
            attrs: Default::default(),
            hidden: Some(true),
            folder_icon: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            close: true,
            recurse: None,
        }
    );
    // A file its folder's .hidden lists reads hidden and keeps its
    // name: clearing the box writes Hidden alone, ticking it again
    // writes nothing.
    let mut listed = file_info();
    (listed.hidden, listed.listed) = (true, true);
    let mut d = Dialog::properties(&listed, "text-x-generic", (0, 0));
    assert!(reprisekit::controls::filesheet::hidden(sheet_form(&d)));
    press_control(&mut d, HIDDEN);
    assert_eq!(
        press_control(&mut d, APPLY),
        Outcome::Properties {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            name: None,
            attrs: Default::default(),
            hidden: Some(false),
            folder_icon: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            close: false,
            recurse: None,
        }
    );
    press_control(&mut d, HIDDEN);
    assert_eq!(press_control(&mut d, OK), Outcome::Close);
    let mut folder = file_info();
    folder.is_dir = true;
    folder.opens_with = None;
    folder.contains = Some((2, 1));
    let mut fd = Dialog::properties(&folder, "folder", (0, 0));
    let changes = |d: &Dialog| match &d.form().unwrap().kind {
        FormKind::Properties(b) => props_changes(sheet_form(d), b),
        _ => unreachable!(),
    };
    assert_eq!(changes(&fd), (None, Default::default(), None));
    press_control(&mut fd, READ_ONLY);
    assert_eq!(changes(&fd).1.read_only, Some(false));
    assert!(apply_on(&fd));
    press_control(&mut fd, READ_ONLY);
    assert_eq!(changes(&fd).1.read_only, Some(true));
    press_control(&mut fd, READ_ONLY);
    assert_eq!(changes(&fd).1.read_only, None, "back where it started");
    assert!(!apply_on(&fd));
}

#[test]
fn sheet_keeps_spaces_around_an_unedited_name() {
    let mut info = file_info();
    info.name = " notes ".into();
    info.hidden_ext = String::new();
    info.path = "/home/u/Desktop/ notes ".into();
    let d = Dialog::properties(&info, "text-x-generic", (0, 0));
    assert_eq!(d.clone().key(Key::Enter), Outcome::Close, "OK only closes");
    let mut d = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
    set_name(&mut d, "   ");
    assert_eq!(d.key(Key::Enter), Outcome::Close, "a blank name is ignored");
}

#[test]
fn properties_sheet_draws_the_opens_with_icon() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut info = file_info();
    info.read_only = true;
    let d = Dialog::properties(&info, "text-x-generic", (0, 0));
    let l = DialogLayout::compute(&theme, &text, &d);
    let mut app_icon = Canvas::new(16, 16);
    app_icon.fill_rect(Rect::new(0, 0, 16, 16), theme.colors.hilight);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[],
        &[Some(&app_icon)],
        None,
        &d,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let cs = &theme.colors;
    let ir = sheet_form(&d).rect("opens_icon").unwrap();
    assert_eq!(ir, Rect::new(98, 142, 16, 16));
    assert_eq!(c.get(ir.x, ir.y), cs.hilight.argb());
    assert_eq!(c.get(ir.right() - 1, ir.bottom() - 1), cs.hilight.argb());
    assert_eq!(c.get(ir.x - 1, ir.y), cs.button_face.argb());
    assert_eq!(c.get(ir.right(), ir.bottom()), cs.button_face.argb());
    let ro = sheet_form(&d)
        .rect(reprisekit::controls::filesheet::READ_ONLY)
        .unwrap();
    assert_eq!(c.get(ro.x + 6, ro.y + 8), cs.button_text.argb());
}

#[test]
fn a_folders_hidden_asks_how_far_it_reaches() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let dir = std::path::Path::new("/d/Reports");
    let confirm = |read_only| AttrConfirm {
        write: AttrWrite::One {
            path: dir.into(),
            name: Some(".Reports".into()),
            hidden: Some(true),
            folder_icon: None,
        },
        attrs: reprisefs::props::AttrMoves {
            read_only,
            ..Default::default()
        },
        close: true,
    };
    let verb = |read_only| {
        let d = Dialog::confirm_attributes(
            confirm(read_only),
            &text,
            theme.fonts.size_px,
            TextUnits::ONE,
        );
        match &d
            .form()
            .unwrap()
            .open_page()
            .unwrap()
            .item("attr_verb")
            .unwrap()
            .control
        {
            reprisekit::controls::form::Control::Label { text, .. } => text.clone(),
            c => panic!("{c:?}"),
        }
    };
    assert_eq!(verb(None), "hide");
    assert_eq!(verb(Some(true)), "set read-only, hide");
    assert_eq!(
        confirm(None).answered(true),
        Outcome::Properties {
            path: dir.into(),
            name: Some(".Reports".into()),
            attrs: Default::default(),
            hidden: Some(true),
            folder_icon: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            close: true,
            recurse: Some(true),
        }
    );
}

#[test]
fn attribute_confirm_lays_out_its_template() {
    use reprisekit::controls::filesheet::{ATTR_ONLY, ATTR_TREE};
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut d = attr_confirm();
    assert_eq!(d.title, "Confirm Attribute Changes");
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (392, 240));
    let form = d.form().unwrap().open_page().unwrap();
    let at = |id: &str| form.rect(id).unwrap();
    assert_eq!((at("attr_intro").x, at("attr_intro").y), (14, 33));
    assert_eq!((at("attr_verb").x, at("attr_verb").y), (59, 59));
    assert!(matches!(
        &form.item("attr_verb").unwrap().control,
        reprisekit::controls::form::Control::Label { text, .. } if text == "set read-only"
    ));
    assert_eq!((at("attr_q0").x, at("attr_q0").y), (14, 98));
    assert_eq!(at("attr_q1").y, 111, "the question wraps into two lines");
    assert_eq!((at(ATTR_ONLY).x, at(ATTR_ONLY).y), (57, 138));
    assert_eq!(at(ATTR_TREE).y, 161);
    assert_eq!(at("attr_ok"), Rect::new(222, 202, BUTTON_W, BUTTON_H));
    assert_eq!(at("attr_cancel").x, 303);
    let only = form.index_of(ATTR_ONLY);
    let hit = l.hit(&d, 80, 141, &text, px);
    assert!(matches!(hit, Some(DialogHit::Control(h)) if Some(h.item) == only));
    assert_eq!(d.click(hit), Outcome::Redraw);
    assert_eq!(d.release(hit), Outcome::Redraw);
    assert_eq!(
        d.key(Key::Enter),
        Outcome::Properties {
            path: "/home/u/Desktop/New Folder".into(),
            name: None,
            attrs: reprisefs::props::AttrMoves {
                read_only: Some(true),
                ..Default::default()
            },
            hidden: None,
            folder_icon: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            close: false,
            recurse: Some(false),
        }
    );
    assert_eq!(d.key(Key::Down), Outcome::Redraw);
    assert!(matches!(
        d.key(Key::Enter),
        Outcome::Properties {
            recurse: Some(true),
            ..
        }
    ));
    assert_eq!(d.key(Key::Escape), Outcome::Close);
}

/// Two files on the desktop, the second read-only, and a folder not
/// walked yet.
#[test]
fn change_icon_gives_a_folder_its_own_icon() {
    use reprisekit::controls::filesheet::CHANGE_ICON;
    use reprisekit::controls::iconlist::IconList;
    let mut folder = file_info();
    folder.is_dir = true;
    folder.opens_with = None;
    folder.path = "/home/u/Desktop/Tunes".into();
    folder.name = "Tunes".into();
    folder.hidden_ext = String::new();
    let mut d = Dialog::properties(&folder, "folder", (0, 0));
    d.cycle_page(false);
    d.cycle_page(false);
    assert_eq!(
        press_control(&mut d, CHANGE_ICON),
        Outcome::ChangeIcon {
            name: "Tunes".into(),
            current: None,
            launcher: false,
        }
    );
    let names = ["folder", "folder-music"].map(String::from).to_vec();
    let mut list = IconList::new(names, vec![None, None], None);
    list.pick("folder");
    let mut ci = Dialog::change_icon("Tunes", "theme", list, false);
    assert_eq!(ci.title, "Change Icon for Tunes Folder");
    ci.key(Key::Down);
    assert_eq!(
        ci.key(Key::Enter),
        Outcome::IconChosen(Some("folder-music".into()))
    );
    assert!(d.icon_chosen(Some("folder-music".into())));
    assert_eq!(
        page_pictures(&d)[0],
        ("cu_icon", "folder-music".to_string(), 32)
    );
    assert_eq!(
        press_control(&mut d, reprisekit::controls::propsheet::OK),
        Outcome::Properties {
            path: folder.path.clone(),
            name: None,
            attrs: Default::default(),
            hidden: None,
            summary: Vec::new(),
            launcher: Vec::new(),
            folder_icon: Some(Some("folder-music".into())),
            close: true,
            recurse: None,
        }
    );
}

#[test]
fn a_shortcuts_header_icon_wears_the_arrow() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(0, 0, 32, 32), Color::rgb(255, 0, 0));
    let render = |d: &Dialog| {
        let l = DialogLayout::compute(&theme, &text, d);
        let r = l.icon.expect("the header icon");
        let c = draw(
            &theme,
            &text,
            &l,
            Some(&icon),
            None,
            &[],
            &[],
            None,
            d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        (c.get(r.x, r.bottom() - 1), c.get(r.x, r.bottom() - 12))
    };
    let black = Color::rgb(0, 0, 0).argb();
    let red = Color::rgb(255, 0, 0).argb();
    let mut info = file_info();
    info.name = "Shell".into();
    info.hidden_ext = String::new();
    info.path = "/home/u/Desktop/shell.desktop".into();
    info.is_shortcut = true;
    info.launcher = Some(reprisefs::launcher::Launcher {
        target: "/bin/sh".into(),
        ..Default::default()
    });
    let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
    assert_eq!(render(&d), (black, red), "on Shortcut");
    d.form_mut().unwrap().sheet_mut().unwrap().open_tab(0);
    assert_eq!(render(&d), (black, red), "on General");
    let plain = Dialog::properties(&file_info(), "text-plain", (0, 0));
    assert_eq!(render(&plain), (red, red));
}

#[test]
fn a_launchers_shortcut_tab_takes_its_keys_to_ok() {
    use reprisefs::launcher::{Key as LKey, Launcher};
    use reprisekit::controls::filesheet::{CHANGE_ICON, FIND_TARGET, SHORTCUT_IDS};
    use reprisekit::controls::form::Control;
    let mut info = file_info();
    info.name = "Shell".into();
    info.hidden_ext = String::new();
    info.path = "/home/u/Desktop/shell.desktop".into();
    info.is_shortcut = true;
    info.opens_with = None;
    info.launcher = Some(Launcher {
        target: "/bin/sh -l".into(),
        path: String::new(),
        comment: "A shell".into(),
        icon: Some("utilities-terminal".into()),
        location: "bin".into(),
        ..Default::default()
    });
    let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
    let f = d.form().unwrap();
    let sheet = f.sheet().unwrap();
    assert_eq!(sheet.tabs.labels, ["General", "Shortcut", "Compatibility"]);
    assert_eq!(sheet.tabs.open, 1);
    assert_eq!(f.size, (367, 502));
    assert!(matches!(
        sheet_form(&d).item("sc_where").map(|i| &i.control),
        Some(Control::Label { text, .. }) if text == "bin"
    ));

    sheet_form_mut(&mut d).focus_id(SHORTCUT_IDS[2]);
    d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
    d.key(Key::Char('!'));
    assert!(matches!(
        sheet_form(&d)
            .item(reprisekit::controls::propsheet::APPLY)
            .map(|i| &i.control),
        Some(Control::Button { enabled: true, .. })
    ));
    assert_eq!(
        press_control(&mut d, CHANGE_ICON),
        Outcome::ChangeIcon {
            name: "shell.desktop".into(),
            current: Some("utilities-terminal".into()),
            launcher: true,
        }
    );
    let list = reprisekit::controls::iconlist::IconList::new(Vec::new(), Vec::new(), None);
    let ci = Dialog::change_icon("shell.desktop", "theme", list, true);
    assert_eq!(ci.title, "Change Icon");
    assert!(d.icon_chosen(Some("terminal".into())));
    assert_eq!(d.icon.as_deref(), Some("terminal"));
    assert_eq!(
        press_control(&mut d, FIND_TARGET),
        Outcome::FindTarget("/bin/sh".into())
    );
    assert_eq!(
        press_control(&mut d, reprisekit::controls::propsheet::OK),
        Outcome::Properties {
            path: info.path.clone(),
            name: None,
            attrs: Default::default(),
            hidden: None,
            summary: Vec::new(),
            folder_icon: None,
            launcher: vec![
                (LKey::Comment, "A shell!".to_string()),
                (LKey::Icon, "terminal".to_string())
            ],
            close: true,
            recurse: None,
        }
    );
}

#[test]
fn a_symlinks_shortcut_tab_takes_its_target_to_ok() {
    use reprisefs::launcher::Key as LKey;
    use reprisekit::controls::filesheet::{FIND_TARGET, SHORTCUT_IDS};
    use reprisekit::controls::form::Control;
    let dir = std::env::temp_dir().join(format!("reprise-symlink-sheet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let real = std::fs::canonicalize(&dir).unwrap();
    std::fs::create_dir(real.join("Documents")).unwrap();
    let link = real.join("Docs");
    std::os::unix::fs::symlink("Documents", &link).unwrap();
    let info = reprisefs::props::glance(&link, "Docs", &[]).unwrap();
    let mut d = Dialog::properties(&info, "folder", (0, 0));
    let sheet = d.form_mut().unwrap().sheet_mut().unwrap();
    assert_eq!(sheet.tabs.labels, ["General", "Shortcut"]);
    assert_eq!(sheet.tabs.open, 1);
    assert!(matches!(
        sheet.pages[0].item("v_desc").map(|i| &i.control),
        Some(Control::Label { text, .. }) if text == "Documents"
    ));
    assert!(matches!(
        sheet_form(&d).item(SHORTCUT_IDS[1]).map(|i| &i.control),
        Some(Control::Edit { enabled: false, .. })
    ));
    assert_eq!(
        press_control(&mut d, FIND_TARGET),
        Outcome::FindTarget(real.join("Documents"))
    );
    sheet_form_mut(&mut d).focus_id(SHORTCUT_IDS[0]);
    d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
    d.key(Key::Char('2'));
    assert_eq!(
        press_control(&mut d, reprisekit::controls::propsheet::OK),
        Outcome::Properties {
            path: link.clone(),
            name: None,
            attrs: Default::default(),
            hidden: None,
            summary: Vec::new(),
            folder_icon: None,
            launcher: vec![(LKey::Link, "Documents2".to_string())],
            close: true,
            recurse: None,
        }
    );
    assert!(matches!(
        press_control(&mut d, FIND_TARGET),
        Outcome::Notice(_)
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_untrusted_launchers_sheet_is_a_files() {
    use reprisekit::controls::form::Control;
    let dir = std::env::temp_dir().join(format!("reprise-untrusted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("term.desktop");
    let app = "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\n";
    std::fs::write(&path, app).unwrap();
    let db = reprisefs::apps::MimeDb::empty();
    let info = reprisefs::props::glance_in(&path, "term.desktop", &[], &db).unwrap();
    let mut d = Dialog::properties(&info, "text-x-generic", (0, 0));
    let sheet = d.form_mut().unwrap().sheet_mut().unwrap();
    assert_eq!(sheet.tabs.labels, ["General", "Summary"]);
    assert_eq!(sheet.tabs.open, 0);
    let general = &sheet.pages[0];
    assert!(matches!(
        general.item("v_type").map(|i| &i.control),
        Some(Control::Label { text, .. }) if text == "DESKTOP File"
    ));
    assert!(general.item("v_desc").is_none());
    assert!(matches!(
        general.item(reprisekit::controls::filesheet::NAME).map(|i| &i.control),
        Some(Control::Edit { edit, .. }) if edit.text == "term.desktop"
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_symlink_to_a_document_shows_opens_with() {
    use reprisekit::controls::form::Control;
    let dir = std::env::temp_dir().join(format!("reprise-doc-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("song.rtest"), "x").unwrap();
    let link = dir.join("Song");
    std::os::unix::fs::symlink("song.rtest", &link).unwrap();
    let db = reprisefs::apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
    let player = reprisefs::apps::AppEntry {
        name: "Player".into(),
        exec_line: "player %f".into(),
        terminal: false,
        id: "player.desktop".into(),
        icon: None,
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: vec!["application/x-reprise-test".into()],
        shown: true,
    };
    let info = reprisefs::props::glance_in(&link, "Song", &[player], &db).unwrap();
    let mut d = Dialog::properties(&info, "text-x-generic", (0, 0));
    let sheet = d.form_mut().unwrap().sheet_mut().unwrap();
    assert_eq!(sheet.tabs.labels, ["General", "Shortcut"]);
    let general = &sheet.pages[0];
    assert_eq!(
        general.item("v_opens").map(|i| &i.control),
        Some(&Control::Lines(vec!["Player".into()]))
    );
    assert!(general.item("v_desc").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_symlinks_apply_shows_where_it_points_now() {
    use reprisekit::controls::filesheet::SHORTCUT_IDS;
    use reprisekit::controls::form::Control;
    use reprisekit::controls::propsheet::APPLY;
    let dir = std::env::temp_dir().join(format!("reprise-relink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let real = std::fs::canonicalize(&dir).unwrap();
    std::fs::create_dir_all(real.join("a/Documents")).unwrap();
    std::fs::create_dir_all(real.join("b")).unwrap();
    std::fs::write(real.join("b/song.rtest"), "").unwrap();
    let link = real.join("Docs");
    std::os::unix::fs::symlink("a/Documents", &link).unwrap();
    let db = reprisefs::apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
    let apps = [reprisefs::apps::AppEntry {
        name: "Player".into(),
        exec_line: "player %f".into(),
        terminal: false,
        id: "player.desktop".into(),
        icon: None,
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: vec!["application/x-reprise-test".into()],
        shown: true,
    }];
    let info = reprisefs::props::glance_in(&link, "Docs", &apps, &db).unwrap();
    let mut d = Dialog::properties(&info, "folder", (0, 0));
    let label = |d: &Dialog, page: usize, id: &str| {
        let form = &d.form().unwrap().pages()[page];
        match form.item(id).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => Some(text.clone()),
            Some(Control::Lines(lines)) => Some(lines.join(" ")),
            _ => None,
        }
    };
    let open = |d: &Dialog| {
        let s = d.form().unwrap().sheet().unwrap();
        s.tabs.labels[s.tabs.open].clone()
    };
    assert_eq!(label(&d, 1, "sc_type").as_deref(), Some("File Folder"));
    assert_eq!(label(&d, 0, "v_desc").as_deref(), Some("Documents"));
    sheet_form_mut(&mut d).focus_id(SHORTCUT_IDS[0]);
    d.key(Key::Edit(crate::textedit::EditKey::SelectAll));
    for c in "b/song.rtest".chars() {
        d.key(Key::Char(c));
    }
    let asked = press_control(&mut d, APPLY);
    let Outcome::Properties { launcher, .. } = &asked else {
        panic!("Apply asks for the write, not {asked:?}");
    };
    reprisefs::launcher::write_keys(&link, launcher).unwrap();
    let name = d.sheet_name().unwrap().to_string();
    assert_eq!(name, "Docs", "read back under the name it opened under");
    let now = reprisefs::props::glance_in(&link, &name, &apps, &db).unwrap();
    d.sheet_reread(&now, "text-x-generic");
    assert_eq!(d.icon.as_deref(), Some("text-x-generic"));
    assert_eq!(d.title, "Docs Properties");
    assert_eq!(open(&d), "Shortcut");
    assert_eq!(label(&d, 1, "sc_type").as_deref(), Some("RTEST File"));
    assert_eq!(label(&d, 1, "sc_where").as_deref(), Some("b"));
    assert_eq!(label(&d, 0, "v_opens").as_deref(), Some("Player"));
    assert_eq!(label(&d, 0, "v_desc"), None);
    assert!(
        !matches!(
            sheet_form(&d).item(APPLY).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        ),
        "nothing left to write"
    );
    d.form_mut().unwrap().sheet_mut().unwrap().open_tab(0);
    let back = [(reprisefs::launcher::Key::Link, "a/Documents".to_string())];
    reprisefs::launcher::write_keys(&link, &back).unwrap();
    let now = reprisefs::props::glance_in(&link, &name, &apps, &db).unwrap();
    d.sheet_reread(&now, "folder");
    assert_eq!(open(&d), "General");
    assert_eq!(d.icon.as_deref(), Some("folder"));
    assert_eq!(label(&d, 0, "v_desc").as_deref(), Some("Documents"));
    assert_eq!(label(&d, 0, "v_opens"), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_web_links_web_document_tab_takes_its_url_to_ok() {
    use reprisefs::launcher::Key as LKey;
    use reprisekit::controls::filesheet::SHORTCUT_IDS;
    let dir = std::env::temp_dir().join(format!("reprise-web-sheet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ex.desktop");
    let _ = std::fs::remove_file(&path);
    let text = reprisefs::apps::link_text("Example", "https://example.org/").unwrap();
    reprisefs::apps::write_launcher(&path, &text).unwrap();
    let info = reprisefs::props::glance(&path, "Example", &[]).unwrap();
    let mut d = Dialog::properties(&info, "text-html", (0, 0));
    let sheet = d.form_mut().unwrap().sheet_mut().unwrap();
    assert_eq!(sheet.tabs.labels, ["General", "Web Document"]);
    assert_eq!(sheet.tabs.open, 1);
    sheet_form_mut(&mut d).focus_id(SHORTCUT_IDS[0]);
    d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
    d.key(Key::Char('x'));
    assert_eq!(
        press_control(&mut d, reprisekit::controls::propsheet::OK),
        Outcome::Properties {
            path: path.clone(),
            name: None,
            attrs: Default::default(),
            hidden: None,
            summary: Vec::new(),
            folder_icon: None,
            launcher: vec![(LKey::Url, "https://example.org/x".to_string())],
            close: true,
            recurse: None,
        }
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_read_only_launchers_sheet_starts_from_its_general_box() {
    use reprisefs::launcher::Launcher;
    use reprisekit::controls::filesheet::READ_ONLY;
    use reprisekit::controls::propsheet::OK;
    let mut info = file_info();
    info.name = "Shell".into();
    info.hidden_ext = String::new();
    info.path = "/home/u/Desktop/shell.desktop".into();
    info.is_shortcut = true;
    info.read_only = true;
    info.launcher = Some(Launcher {
        target: "/bin/sh".into(),
        path: String::new(),
        comment: String::new(),
        icon: None,
        ..Default::default()
    });
    let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
    assert_eq!(
        d.clone().key(Key::Enter),
        Outcome::Close,
        "nothing moved, nothing to write"
    );
    d.form_mut().unwrap().sheet_mut().unwrap().open_tab(0);
    press_control(&mut d, READ_ONLY);
    let Outcome::Properties { attrs, .. } = press_control(&mut d, OK) else {
        panic!("OK asks for the write");
    };
    assert_eq!(attrs.read_only, Some(false));
}

/// Apply's write settles only what it wrote: a Comment typed while it
/// ran, and a field whose write failed, keep Apply lit, and OK then
/// writes them.
#[test]
fn apply_settles_only_the_fields_it_wrote() {
    use reprisefs::launcher::{Key as LKey, Launcher};
    use reprisekit::controls::filesheet::SHORTCUT_IDS;
    use reprisekit::controls::form::Control;
    use reprisekit::controls::propsheet::{APPLY, OK};
    let mut info = file_info();
    info.name = "Shell".into();
    info.hidden_ext = String::new();
    info.path = "/home/u/Desktop/shell.desktop".into();
    info.is_shortcut = true;
    info.launcher = Some(Launcher {
        target: "/bin/sh".into(),
        path: String::new(),
        comment: "A shell".into(),
        icon: None,
        ..Default::default()
    });
    let apply_lit = |d: &Dialog| {
        matches!(
            sheet_form(d).item(APPLY).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )
    };
    let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
    let type_in = |d: &mut Dialog, id: &str, c: char| {
        sheet_form_mut(d).focus_id(id);
        d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        d.key(Key::Char(c));
    };
    type_in(&mut d, SHORTCUT_IDS[2], '!');
    let asked = press_control(&mut d, APPLY);
    type_in(&mut d, SHORTCUT_IDS[2], '?');
    d.sheet_applied(&written(&asked));
    assert!(apply_lit(&d), "the Comment typed since is still to write");
    let Outcome::Properties { launcher, .. } = press_control(&mut d, OK) else {
        panic!("OK asks for the write");
    };
    assert_eq!(launcher, [(LKey::Comment, "A shell!?".to_string())]);

    let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
    type_in(&mut d, SHORTCUT_IDS[0], 'x');
    press_control(&mut d, APPLY);
    d.sheet_applied(&Written {
        path: info.path.clone(),
        ..Default::default()
    });
    assert!(apply_lit(&d), "a failed write stays a change");
}

#[test]
fn a_files_summary_takes_its_fields_to_ok() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut info = file_info();
    info.summary = Some(["Report", "", "", "", "", ""].map(String::from));
    let mut d = Dialog::properties(&info, "text-x-generic", (0, 0));
    let labels = |d: &Dialog| d.form().unwrap().sheet().unwrap().tabs.labels.clone();
    assert_eq!(labels(&d), ["General", "Summary"]);
    d.cycle_page(false);
    d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
    d.key(Key::Char('s'));
    assert!(matches!(
        sheet_form(&d)
            .item(reprisekit::controls::propsheet::APPLY)
            .map(|i| &i.control),
        Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
    ));
    assert_eq!(
        press_control(&mut d, reprisekit::controls::propsheet::OK),
        Outcome::Properties {
            path: info.path.clone(),
            name: None,
            attrs: Default::default(),
            hidden: None,
            folder_icon: None,
            summary: vec![(0, "Reports".to_string())],
            launcher: Vec::new(),
            close: true,
            recurse: None,
        }
    );

    let docs = reprisefs::props::glance_set(&[file_info(), file_info()]);
    let d = Dialog::set_properties(&docs, "a", (0, 0));
    assert_eq!(labels(&d), ["General", "Summary"]);
    assert_eq!(DialogLayout::compute(&theme, &text, &d).size, (367, 502));
    let mut link = file_info();
    link.is_shortcut = true;
    let links = reprisefs::props::glance_set(&[link.clone(), link]);
    let d = Dialog::set_properties(&links, "a", (0, 0));
    assert_eq!(labels(&d), ["General", "Compatibility"]);
    assert_eq!(
        DialogLayout::compute(&theme, &text, &d).size,
        (367, 437 + 24)
    );
}

#[test]
fn a_folder_and_a_shortcut_set_get_their_tabs() {
    use reprisekit::controls::filesheet::PAGE_ICONS;
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut folder = file_info();
    folder.is_dir = true;
    folder.opens_with = None;
    let mut d = Dialog::properties(&folder, "folder", (0, 0));
    let labels = |d: &Dialog| d.form().unwrap().sheet().unwrap().tabs.labels.clone();
    assert_eq!(labels(&d), ["General", "Sharing", "Customize"]);
    assert!(page_pictures(&d).is_empty(), "General has none");
    d.cycle_page(false);
    assert_eq!(
        page_pictures(&d),
        PAGE_ICONS.map(|(id, n)| (id, n.to_string(), 32))
    );
    d.cycle_page(false);
    assert_eq!(
        page_pictures(&d),
        [
            ("cu_icon", "folder".to_string(), 32),
            ("cu_preview", "folder".to_string(), 96)
        ],
        "Customize shows the standard folder"
    );
    assert_eq!(DialogLayout::compute(&theme, &text, &d).size.1, 424 + 24);

    let mut link = file_info();
    link.is_shortcut = true;
    link.path = "/home/u/Desktop/app.desktop".into();
    let set = reprisefs::props::glance_set(&[file_info(), link]);
    let d = Dialog::set_properties(&set, "New Text Document", (0, 0));
    assert_eq!(labels(&d), ["General", "Compatibility", "Summary"]);
    assert_eq!(DialogLayout::compute(&theme, &text, &d).size, (367, 502));
}

#[test]
fn a_set_sheet_frames_the_set_page_and_adds_up_the_walk() {
    use reprisekit::controls::filesheet::{ADVANCED, COUNT};
    use reprisekit::controls::form::Control;
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut d = Dialog::set_properties(&a_set(), "New Text Document", (0, 0));
    assert_eq!(d.title, "New Text Document, ... Properties");
    assert_eq!(DialogLayout::compute(&theme, &text, &d).size, (367, 502));
    let label = |d: &Dialog, id: &str| match sheet_form(d).item(id).map(|i| &i.control) {
        Some(Control::Label { text, .. }) => text.clone(),
        other => panic!("{id}: {other:?}"),
    };
    assert_eq!(label(&d, COUNT), "Calculating...");
    assert_eq!(label(&d, "v_type"), "Multiple Types");
    assert_eq!(label(&d, "v_loc"), "All in /home/u/Desktop");
    assert!(d.set_folder_totals(reprisefs::props::FolderTotals {
        files: 3,
        folders: 1,
        size: 1536,
        on_disk: 8192,
    }));
    assert_eq!(label(&d, COUNT), "5 Files, 2 Folders");
    assert_eq!(label(&d, "v_size"), reprisefs::props::format_size(1536));
    assert_eq!(d.clone().key(Key::Enter), Outcome::Close);
    assert_eq!(
        press_control(&mut d, ADVANCED),
        Outcome::AdvancedAttributes {
            subject: Subject::Set { has_dir: true },
            robots: None,
        }
    );
}

#[test]
fn a_set_sheet_writes_what_moved_across_the_set() {
    use reprisekit::controls::filesheet::{HIDDEN, READ_ONLY};
    use reprisekit::controls::propsheet::{APPLY, OK};
    let apply_on = |d: &Dialog| {
        matches!(
            sheet_form(d).item(APPLY).map(|i| &i.control),
            Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
        )
    };
    let set = a_set();
    let mut d = Dialog::set_properties(&set, "New Text Document", (0, 0));
    assert_eq!(
        reprisekit::controls::filesheet::read_only(sheet_form(&d)),
        CheckState::Mixed
    );
    press_control(&mut d, READ_ONLY);
    assert!(apply_on(&d));
    assert_eq!(
        press_control(&mut d, APPLY),
        Outcome::SetProperties {
            paths: set.paths.clone(),
            attrs: reprisefs::props::AttrMoves {
                read_only: Some(false),
                ..Default::default()
            },
            hidden: None,
            folder_icon: None,
            close: false,
            recurse: None,
        }
    );
    d.set_applied(set.paths.clone());
    assert!(!apply_on(&d), "nothing left to write");
    press_control(&mut d, HIDDEN);
    assert_eq!(
        press_control(&mut d, OK),
        Outcome::SetProperties {
            paths: set.paths.clone(),
            attrs: Default::default(),
            hidden: Some(true),
            folder_icon: None,
            close: true,
            recurse: None,
        }
    );
}

#[test]
fn a_set_attribute_confirm_answers_for_the_set() {
    use reprisekit::controls::filesheet::{ATTR_TREE, SET_ATTR_LABELS};
    use reprisekit::controls::form::Control;
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let paths = a_set().paths;
    let mut d = Dialog::confirm_attributes(
        AttrConfirm {
            write: AttrWrite::Set {
                paths: paths.clone(),
                hidden: Some(true),
                folder_icon: None,
            },
            attrs: reprisefs::props::AttrMoves {
                read_only: Some(true),
                ..Default::default()
            },
            close: true,
        },
        &text,
        theme.fonts.size_px,
        TextUnits::ONE,
    );
    assert!(matches!(
        &d.form().unwrap().open_page().unwrap().item(ATTR_TREE).unwrap().control,
        Control::Radio { label, .. } if label == SET_ATTR_LABELS[1]
    ));
    assert_eq!(
        d.key(Key::Enter),
        Outcome::SetProperties {
            paths,
            attrs: reprisefs::props::AttrMoves {
                read_only: Some(true),
                ..Default::default()
            },
            hidden: Some(true),
            folder_icon: None,
            close: true,
            recurse: Some(true),
        }
    );
}
