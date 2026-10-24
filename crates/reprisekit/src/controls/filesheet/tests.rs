use super::summary::SUM_COMMENTS;
use super::*;
use crate::controls::form::Key;
use crate::look::classic_paint;
use crate::text::TextRenderer;
use crate::theme::{FontConfig, TextUnits, Theme};

fn text() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

fn px() -> u32 {
    Theme::default().fonts.size_px
}

#[test]
fn a_sets_picture_is_32_px_and_doubles_at_scale_2() {
    let one = set_picture(1);
    let two = set_picture(2);
    assert_eq!((one.width, one.height), (32, 32));
    assert_eq!((two.width, two.height), (64, 64));
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(two.get(2 * x + 1, 2 * y + 1), one.get(x, y), "({x}, {y})");
        }
    }
    assert_eq!(one.get(0, 0) >> 24, 0, "a clear corner");
    assert_eq!(one.get(16, 20) >> 24, 255, "the pages");
}

/// The Summary page's labels fit their column in DejaVu Sans, which is wider than Tahoma:
/// the fields move right of "Comments:".
#[test]
fn the_summary_labels_fit_in_a_wider_face() {
    let family = "DejaVu Sans";
    let text = TextRenderer::load(&FontConfig {
        family: vec![family.into()],
        ..FontConfig::default()
    });
    if !text.has_face(family) {
        return;
    }
    let page = summary_page(None);
    page.drawn_with(&Theme::default().paint());
    assert_eq!(page.overflows(&text, px()), Vec::new());
    let comments = page.rect(SUMMARY_IDS[5]).unwrap();
    assert!(comments.x > SUM_COMMENTS.x, "{comments:?}");
    assert_eq!(comments.right(), SUM_COMMENTS.right());
}

#[test]
fn a_long_date_wraps_inside_its_row() {
    let family = "DejaVu Sans";
    let text = TextRenderer::load(&FontConfig {
        family: vec![family.into()],
        ..FontConfig::default()
    });
    if !text.has_face(family) {
        return;
    }
    let date = "Wednesday, September 30, 2026, 12:00:00 PM";
    let mut f = facts(false);
    f.modified = date.into();
    let (page, _) = general_page(&rows(&f), "a", None, None);
    page.drawn_with(&Theme::default().paint());
    let r = page.rect("v_mod").unwrap();
    assert!(text.measure(date, false, px()) > r.width);
    assert_eq!(page.overflows(&text, px()), Vec::new());
    assert_eq!(page.rect("l_acc").unwrap().y, r.y + ROW_PITCH);
}

#[test]
fn a_long_program_name_wraps_beside_change() {
    let name = "Compressed (zipped) Folders";
    for family in ["Tahoma", "DejaVu Sans"] {
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            ..FontConfig::default()
        });
        if !text.has_face(family) {
            continue;
        }
        let mut f = facts(false);
        f.opens_with = Some(name.into());
        let (page, _) = general_page(&rows(&f), "a", None, None);
        page.drawn_with(&Theme::default().paint());
        let r = page.rect("v_opens").unwrap();
        assert!(text.measure(name, false, px()) > r.width, "{family}");
        assert_eq!(page.overflows(&text, px()), Vec::new(), "{family}");
        assert_eq!(page.rect(CHANGE), Some(Rect::new(234, 118, 105, 23)));
        assert_eq!(page.rect("sep1").unwrap().y, r.y + ROW_PITCH);
    }
}

fn facts(is_dir: bool) -> Facts {
    Facts {
        is_dir,
        type_name: if is_dir {
            "File Folder"
        } else {
            "Text Document"
        }
        .into(),
        opens_with: (!is_dir).then(|| "Notepad".to_string()),
        description: None,
        location: "/home/user/Desktop".into(),
        size: "8 bytes (8 bytes)".into(),
        on_disk: "4.00 KB (4,096 bytes)".into(),
        contains: is_dir.then(|| "1 Files, 1 Folders".to_string()),
        created: Some("Monday, September 14, 2026, 3:43:34 PM".into()),
        modified: "Monday, September 14, 2026, 3:43:53 PM".into(),
        accessed: "Sunday, September 20, 2026, 12:24:12 PM".into(),
        read_only: false,
        executable: false,
        can_set_executable: !is_dir,
        hidden: false,
    }
}

fn page(is_dir: bool) -> (Form, i32) {
    general_page(&rows(&facts(is_dir)), "New Folder", None, None)
}

fn label_of<'a>(page: &'a Form, id: &str) -> &'a str {
    match &page.item(id).expect(id).control {
        Control::Label { text, .. } => text,
        other => panic!("{id} is a label, not {other:?}"),
    }
}

#[test]
fn a_reader_calls_the_name_field_name() {
    let (page, _) = general_page(&rows(&facts(false)), "notes.txt", None, None);
    let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
    let mut t = crate::access::Tree::window("", (400, 400));
    page.access(&mut t, crate::access::ROOT, &text, 11);
    let (_, field) = t
        .nodes()
        .find(|(_, n)| n.role() == crate::access::accesskit::Role::TextInput)
        .expect("the name field");
    assert_eq!(field.label(), Some(NAME_FIELD));
}

#[test]
fn an_ampersand_in_a_location_or_bin_name_shows_and_is_no_access_key() {
    let f = Facts {
        location: "/srv/R&D".into(),
        ..facts(false)
    };
    let (page, _) = general_page(&rows(&f), "R&D notes.txt", None, None);
    let bin = bin_page(&bin_facts(), "R&D notes.txt", None);
    for (label, want) in [
        (label_of(&page, "v_loc"), "/srv/R&D"),
        (label_of(&bin, NAME), "R&D notes.txt"),
    ] {
        assert_eq!(crate::text::split_mnemonic(label), (want.to_string(), None));
    }
}

#[test]
fn an_ampersand_on_a_shortcut_or_web_page_shows_and_is_no_access_key() {
    let link = shortcut_page(
        &ShortcutFacts {
            name: "R&D".into(),
            target_type: "Q&A Document".into(),
            target_location: "Q&A".into(),
            ..shortcut_facts()
        },
        None,
    );
    let web = web_page(
        &WebFacts {
            name: "R&D Wiki".into(),
            url: "http://www.example.com/".into(),
        },
        None,
    );
    for (label, want) in [
        (label_of(&link, "sc_name"), "R&D"),
        (label_of(&link, "sc_type"), "Q&A Document"),
        (label_of(&link, "sc_where"), "Q&A"),
        (label_of(&web, "wd_name"), "R&D Wiki"),
    ] {
        assert_eq!(crate::text::split_mnemonic(label), (want.to_string(), None));
    }
}

#[test]
fn a_files_page_lands_on_its_grid() {
    let (page, height) = page(false);
    let rect = |id: &str| page.item(id).expect(id).rect;
    assert_eq!(rect(NAME), Rect::new(94, 48, 246, 23));
    assert_eq!(rect("icon"), Rect::new(21, 41, 32, 32));
    let seps = ["sep0", "sep1", "sep2", "sep3"].map(&rect);
    assert_eq!(seps.map(|r| r.y), [82, 147, 238, 329]);
    assert!(seps.iter().all(|r| (r.x, r.right()) == (21, 339)));
    assert_eq!(rect("l_type"), Rect::new(21, 95, 76, 13));
    assert_eq!(rect("v_type"), Rect::new(97, 95, 242, 13));
    assert_eq!(rect("l_opens").y, 121);
    assert_eq!(rect("opens_icon"), Rect::new(95, 121, 16, 16));
    assert_eq!(rect("v_opens").x, 120);
    assert_eq!(rect(CHANGE), Rect::new(234, 118, 105, 23));
    assert_eq!(
        ["l_loc", "l_size", "l_disk"].map(|id| rect(id).y),
        [160, 186, 212]
    );
    assert_eq!(
        ["l_made", "l_mod", "l_acc"].map(|id| rect(id).y),
        [251, 277, 303]
    );
    assert_eq!(rect("l_attr").y, 342);
    assert_eq!(rect(READ_ONLY), Rect::new(94, 343, 13, 13));
    assert_eq!(rect(HIDDEN), Rect::new(174, 343, 13, 13));
    assert_eq!(rect(ADVANCED), Rect::new(265, 339, 75, 23));
    assert_eq!(height, 478);
}

#[test]
fn a_folders_page_lands_on_its_grid() {
    let (page, height) = page(true);
    let rect = |id: &str| page.item(id).expect(id).rect;
    assert_eq!(rect(NAME), Rect::new(94, 48, 246, 23));
    assert_eq!(
        ["sep0", "sep1", "sep2"].map(|id| rect(id).y),
        [82, 225, 264]
    );
    for (i, id) in ["l_type", "l_loc", "l_size", "l_disk", "l_has"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(rect(id), Rect::new(21, 95 + 26 * i as i32, 73, 13), "{id}");
    }
    assert_eq!(rect("v_type").x, 94);
    assert_eq!(rect("l_made").y, 238);
    assert_eq!(rect("l_attr").y, 277);
    assert_eq!(rect(READ_ONLY), Rect::new(94, 278, 13, 13));
    assert_eq!(rect(HIDDEN), Rect::new(94, 301, 13, 13));
    assert_eq!(rect(ADVANCED), Rect::new(234, 274, 105, 23));
    assert_eq!(height, 424);
}

fn set_facts(has_dir: bool) -> SetFacts {
    SetFacts {
        has_dir,
        count: "2 Files, 2 Folders".into(),
        type_text: "Multiple Types".into(),
        location: "All in /home/user/Desktop".into(),
        size: "148 bytes (148 bytes)".into(),
        on_disk: "8.00 KB (8,192 bytes)".into(),
        read_only: CheckState::Off,
        hidden: CheckState::Mixed,
    }
}

fn set_page(has_dir: bool) -> (Form, i32) {
    general_page(&set_rows(&set_facts(has_dir)), "", None, None)
}

#[test]
fn a_sets_page_lands_on_its_grid() {
    for (has_dir, value_x, attr) in [(false, 94, "Attributes:"), (true, 97, "Attributes")] {
        let (page, height) = set_page(has_dir);
        let rect = |id: &str| page.item(id).expect(id).rect;
        assert!(page.item(NAME).is_none(), "no name field");
        assert_eq!(rect("icon"), Rect::new(21, 41, 32, 32));
        assert_eq!((rect(COUNT).x, rect(COUNT).y), (94, 48));
        assert_eq!(["sep0", "sep1"].map(|id| rect(id).y), [82, 199]);
        assert!(page.item("sep2").is_none());
        for (i, id) in ["l_type", "l_loc", "l_size", "l_disk"]
            .into_iter()
            .enumerate()
        {
            assert_eq!((rect(id).x, rect(id).y), (21, 95 + 26 * i as i32), "{id}");
        }
        assert_eq!(rect("v_type").x, value_x, "has_dir {has_dir}");
        assert_eq!(rect("l_attr").y, 212);
        assert!(matches!(
            &page.item("l_attr").unwrap().control,
            Control::Label { text, .. } if text == attr
        ));
        assert_eq!(rect(READ_ONLY), Rect::new(94, 213, 13, 13));
        assert_eq!(rect(HIDDEN), Rect::new(94, 236, 13, 13));
        assert_eq!(rect(ADVANCED), Rect::new(265, 209, 75, 23));
        assert_eq!(
            height, 424,
            "a folder page's; a Summary tab makes the sheet 478"
        );
    }
}

#[test]
fn folder_column_pages_are_set_in_the_second_dialog_font() {
    let page_font = |rows: Vec<Row>| general_page(&rows, "x", None, None).0.font;
    assert_eq!(page_font(rows(&facts(true))), DialogFont::ShellDlg);
    assert_eq!(page_font(rows(&facts(false))), DialogFont::Ui);
    assert_eq!(set_page(false).0.font, DialogFont::ShellDlg);
    assert_eq!(set_page(true).0.font, DialogFont::Ui);
}

#[test]
fn a_sets_boxes_have_three_states_only_where_its_items_differ() {
    let (mut page, _) = set_page(true);
    assert_eq!(read_only(&page), CheckState::Off);
    assert_eq!(hidden_state(&page), CheckState::Mixed);
    page.focus_id(READ_ONLY);
    page.key(Key::Space);
    page.key(Key::Space);
    assert_eq!(read_only(&page), CheckState::Off, "Read-only flips");
    page.focus_id(HIDDEN);
    let mut seen = Vec::new();
    for _ in 0..3 {
        page.key(Key::Space);
        seen.push(hidden_state(&page));
    }
    assert_eq!(seen, [CheckState::Off, CheckState::On, CheckState::Mixed]);
}

#[test]
fn a_sets_page_inks_on_its_grid() {
    let t = text();
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = Theme::default();
    let cs = &theme.colors;
    let (page, height) = set_page(true);
    let mut c = Canvas::new(SHEET_W as u32, height as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    crate::controls::form::draw(&mut c, &t, &classic_paint(cs, 1), &page, false, px());
    let ink = |id: &str| {
        let r = page.item(id).unwrap().rect;
        let hits: Vec<(i32, i32)> = (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
            .collect();
        (
            hits.iter().map(|p| p.0).min().unwrap(),
            hits.iter().map(|p| p.1).min().unwrap(),
        )
    };
    let got = [COUNT, "l_type", "v_type", "v_loc", "v_disk", "l_attr"].map(|id| (id, ink(id)));
    let want = [
        (COUNT, (94, 50)),
        ("l_type", (21, 98)),
        ("v_type", (97, 97)),
        ("v_loc", (97, 123)),
        ("v_disk", (97, 175)),
        ("l_attr", (21, 214)),
    ];
    assert_eq!(got, want);
}

#[test]
fn a_sets_boxes_speak_of_the_selected_items() {
    let (t, px) = (text(), px());
    let line = |f: &Form, id: &str| match &f.item(id).expect(id).control {
        Control::Label { text, .. } | Control::Radio { label: text, .. } => text.clone(),
        other => panic!("{other:?}"),
    };
    for (has_dir, first) in [
        (false, "Choose the options you want for this file."),
        (true, "Choose the settings you want for the selected items"),
    ] {
        let set = Subject::Set { has_dir };
        let adv = advanced_form(set, None, None, &t, px, TextUnits::ONE);
        assert_eq!(line(&adv, "adv_l0"), first);
        assert_eq!(advanced_size(set), advanced_size(Subject::single(has_dir)));
        assert_eq!(adv.item("adv_l1").is_some(), has_dir);
    }
    let set = Subject::Set { has_dir: true };
    let f = attr_form("set read-only", set, &t, px, TextUnits::ONE);
    assert_eq!(line(&f, ATTR_ONLY), SET_ATTR_LABELS[0]);
    assert_eq!(line(&f, ATTR_TREE), SET_ATTR_LABELS[1]);
    assert!(line(&f, "attr_q0").contains("the selected items only"));
    let folder = attr_form("set read-only", Subject::Folder, &t, px, TextUnits::ONE);
    assert_eq!(line(&folder, ATTR_ONLY), ATTR_LABELS[0]);
}

#[test]
fn the_attribute_radios_keep_their_whole_labels_at_200() {
    let t = text();
    let theme = Theme::default().with_text_size(200);
    let px = theme.fonts.size_px;
    let mut form = attr_form("set read-only", Subject::Folder, &t, px, theme.text);
    let (w, h) = (theme.text.len(ATTR_SIZE.0), theme.text.len(ATTR_SIZE.1));
    let mut c = Canvas::new(w as u32, h as u32);
    crate::controls::form::draw(&mut c, &t, &theme.paint(), &form, false, px);
    for id in [ATTR_ONLY, ATTR_TREE] {
        let r = form.rect(id).unwrap();
        form.motion(r.x + 80, r.y + r.height / 2, None);
        assert!(!form.tip_tick(), "{id} draws its whole label");
    }
}

#[test]
fn the_archive_and_index_boxes_are_live_over_what_was_read() {
    let (t, px) = (text(), px());
    let read = robots_states(Some([Some(true), None])).unwrap();
    assert_eq!(read, [CheckState::On, CheckState::Mixed]);
    let mut f = advanced_form(
        Subject::Set { has_dir: true },
        None,
        Some(read),
        &t,
        px,
        TextUnits::ONE,
    );
    let enabled = |f: &Form, id: &str| {
        matches!(
            f.item(id).map(|i| &i.control),
            Some(Control::Check { enabled: true, .. })
        )
    };
    assert!(enabled(&f, ADV_ARCHIVE) && enabled(&f, ADV_INDEX));
    assert!(!enabled(&f, "adv_compress") && !enabled(&f, "adv_encrypt"));
    assert_eq!(f.focus().map(|i| f.items[i].id), Some(ADV_ARCHIVE));
    assert_eq!(robots_chosen(&f), read);
    f.key(Key::Space);
    assert_eq!(robots_moves(read, robots_chosen(&f)), [Some(false), None]);
    f.focus_id(ADV_INDEX);
    f.key(Key::Space);
    assert_eq!(
        robots_moves(read, robots_chosen(&f)),
        [Some(false), Some(false)]
    );
    let greyed = advanced_form(Subject::File, None, None, &t, px, TextUnits::ONE);
    assert!(!enabled(&greyed, ADV_ARCHIVE));
    assert_eq!(greyed.check_state(ADV_ARCHIVE), Some(CheckState::Off));
}

#[test]
#[ignore]
fn render_set_sheet() {
    for (name, theme) in [
        (
            "filesheet-set.png",
            Theme {
                access_keys: true,
                ..Theme::default()
            },
        ),
        (
            "filesheet-set-blue.png",
            crate::theme::Theme {
                access_keys: true,
                ..crate::interfaces::theme_for(crate::interface::Interface::Blue)
            },
        ),
    ] {
        let t = text();
        let (page, height) = set_page(true);
        let sheet =
            crate::controls::propsheet::PropSheet::new(&["General"], vec![page], (SHEET_W, height));
        let mut c = Canvas::new(SHEET_W as u32, height as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &sheet, true, px());
        crate::raster::write_preview(&c, name);
    }
}

#[test]
fn a_page_opens_on_read_only() {
    let focused = |f: &Form| f.focus().map(|i| f.items[i].id);
    for (mut p, _) in [page(false), page(true), set_page(false)] {
        assert_eq!(focused(&p), Some(READ_ONLY));
        if p.item(NAME).is_some() {
            let back = if p.item(CHANGE).is_some() { 2 } else { 1 };
            for _ in 0..back {
                p.key(Key::Tab { back: true });
            }
            assert_eq!(focused(&p), Some(NAME));
            let Some(Control::Edit { edit, .. }) = p.item(NAME).map(|i| &i.control) else {
                panic!("the name field");
            };
            assert_eq!(edit.selected_text(), "New Folder");
        }
    }
}

#[test]
fn each_kind_of_sheet_gets_its_tabs() {
    let set = |document, shortcut, folder| {
        sheet_tabs(Over::Set {
            document,
            shortcut,
            folder,
        })
    };
    assert_eq!(sheet_tabs(Over::File { shortcut: false }), [Tab::Summary]);
    assert_eq!(sheet_tabs(Over::File { shortcut: true }), []);
    assert_eq!(
        sheet_tabs(Over::Launcher),
        [Tab::Shortcut, Tab::Compatibility]
    );
    assert_eq!(sheet_tabs(Over::Folder), [Tab::Sharing, Tab::Customize]);
    assert_eq!(set(true, false, false), [Tab::Summary]);
    assert_eq!(set(true, false, true), [Tab::Summary]);
    assert_eq!(set(false, true, false), [Tab::Compatibility]);
    assert_eq!(set(true, true, false), [Tab::Compatibility, Tab::NoSummary]);
    assert_eq!(set(false, false, true), [Tab::Customize]);
}

#[test]
fn the_customize_page_lands_on_its_grid() {
    let p = customize_page();
    let rect = |id: &str| p.item(id).expect(id).rect;
    assert_eq!(
        ["cu_g0", "cu_g1", "cu_g2"].map(rect),
        [
            Rect::new(21, 47, 319, 90),
            Rect::new(21, 151, 319, 134),
            Rect::new(21, 299, 319, 78)
        ]
    );
    assert_eq!(rect("cu_template"), Rect::new(31, 80, 299, 21));
    assert_eq!(rect("cu_all"), Rect::new(31, 113, 13, 13));
    assert_eq!(rect(CUSTOM_PREVIEW), Rect::new(225, 181, 96, 96));
    assert_eq!(rect("cu_choose"), Rect::new(31, 222, 98, 23));
    assert_eq!(rect("cu_restore"), Rect::new(31, 251, 98, 23));
    assert_eq!(rect(CHANGE_ICON), Rect::new(31, 345, 98, 23));
    assert_eq!(rect(CUSTOM_ICON), Rect::new(251, 343, 32, 32));
    assert_eq!(p.focus().map(|i| p.items[i].id), Some(CHANGE_ICON));
}

#[test]
fn the_change_icon_box_picks_an_icon() {
    use crate::controls::iconlist::IconList;
    let names: Vec<String> = FOLDER_ICON_NAMES.iter().map(|n| n.to_string()).collect();
    let n = names.len();
    let mut list = IconList::new(names, vec![None; n], None);
    list.pick("folder-music");
    let mut f = change_icon_form("hicolor", list, true);
    let rect = |f: &Form, id: &str| f.item(id).expect(id).rect;
    assert_eq!(rect(&f, "ci_file"), Rect::new(11, 29, 185, 21));
    assert_eq!(rect(&f, CHANGE_ICON_LIST), Rect::new(11, 77, 267, 198));
    assert_eq!(rect(&f, CHANGE_ICON_OK), Rect::new(122, 292, 75, 23));
    assert_eq!(change_icon_picked(&f).as_deref(), Some("folder-music"));
    assert_eq!(f.focus().map(|i| f.items[i].id), Some(CHANGE_ICON_LIST));
    assert_eq!(
        f.key(Key::Down),
        crate::controls::form::Event::Changed(CHANGE_ICON_LIST)
    );
    assert_eq!(change_icon_picked(&f).as_deref(), Some("folder-pictures"));
    if let Some(Control::IconList(l)) = f.item_mut(CHANGE_ICON_LIST).map(|i| &mut i.control) {
        l.left = 2;
    }
    change_icon_restore(&mut f);
    assert_eq!(
        change_icon_picked(&f).as_deref(),
        Some(STANDARD_FOLDER_ICON)
    );
    let Some(Control::IconList(l)) = f.item(CHANGE_ICON_LIST).map(|i| &i.control) else {
        unreachable!()
    };
    assert_eq!(l.left, 0, "the picked column shows");
}

#[test]
fn the_summary_page_lands_on_its_grid_and_reports_its_moves() {
    let values: [String; 6] = ["Report", "", "Ann", "", "q3", "first\nsecond"].map(String::from);
    let mut p = summary_page(Some(&values));
    let rect = |p: &Form, id: &str| p.item(id).expect(id).rect;
    assert_eq!(
        SUMMARY_IDS[..5]
            .iter()
            .map(|id| rect(&p, id).y)
            .collect::<Vec<_>>(),
        [41, 72, 103, 152, 183]
    );
    assert!(SUMMARY_IDS[..5].iter().all(|id| (
        rect(&p, id).x,
        rect(&p, id).width,
        rect(&p, id).height
    ) == (81, 263, 21)));
    assert_eq!(rect(&p, "sum_l0"), Rect::new(22, 44, 59, 13));
    assert_eq!(rect(&p, "sum_sep"), Rect::new(22, 137, 318, 2));
    assert_eq!(rect(&p, SUMMARY_IDS[5]), Rect::new(81, 214, 263, 80));
    assert_eq!(rect(&p, SUMMARY_ADVANCED), Rect::new(244, 305, 99, 23));
    assert!(matches!(
        p.item(SUMMARY_ADVANCED).map(|i| &i.control),
        Some(Control::Button { enabled: false, .. })
    ));
    assert_eq!(summary_values(&p), values);
    assert_eq!(p.focus().map(|i| p.items[i].id), Some(SUMMARY_IDS[0]));
    p.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
    p.key(Key::Char('!'));
    assert_eq!(summary_moves(&p, &values), [(0, "Report!".to_string())]);

    let greyed = summary_page(None);
    assert_eq!(greyed.focus(), None, "nothing on it is live");
    assert_eq!(summary_values(&greyed), <[String; 6]>::default());
}

fn shortcut_facts() -> ShortcutFacts {
    ShortcutFacts {
        name: "Firefox".into(),
        target_type: "Application".into(),
        target_location: "bin".into(),
        symlink: false,
        fields: [
            "firefox %u".into(),
            "/home/user".into(),
            "Browse the web".into(),
        ],
    }
}

fn symlink_facts() -> ShortcutFacts {
    ShortcutFacts {
        name: "Folder Link".into(),
        target_type: "File Folder".into(),
        target_location: "user".into(),
        symlink: true,
        fields: ["/home/user/Documents".into(), String::new(), String::new()],
    }
}

fn web_facts() -> WebFacts {
    WebFacts {
        name: "Web Page".into(),
        url: "http://www.example.com/".into(),
    }
}

#[test]
fn a_shortcuts_page_has_its_description_in_opens_withs_place() {
    let link = Facts {
        type_name: "Shortcut".into(),
        opens_with: None,
        description: Some("Documents".into()),
        ..facts(false)
    };
    let (page, height) = general_page(&rows(&link), "Folder Link", None, None);
    let rect = |id: &str| page.item(id).expect(id).rect;
    assert_eq!(rect("l_desc"), Rect::new(21, 121, 76, 13));
    assert_eq!(rect("v_desc"), Rect::new(97, 121, 242, 13));
    assert!(page.item(CHANGE).is_none());
    assert_eq!(rect("l_loc").y, 160);
    assert_eq!(height, 478);
    let opens = Facts {
        description: Some("unused".into()),
        ..facts(false)
    };
    let (page, _) = general_page(&rows(&opens), "a", None, None);
    assert!(page.item("l_desc").is_none(), "a program shows instead");
}

#[test]
fn symlink_and_web_link_sheets_open_on_their_own_tab() {
    assert_eq!(sheet_tabs(Over::Symlink { runs: false }), [Tab::Shortcut]);
    assert_eq!(
        sheet_tabs(Over::Symlink { runs: true }),
        [Tab::Shortcut, Tab::Compatibility]
    );
    assert_eq!(sheet_tabs(Over::WebLink), [Tab::WebDocument]);
    let (general, height) = page(false);
    let s = sheet(general, height, &sheet_tabs(Over::WebLink), |_| {
        web_page(&web_facts(), None)
    });
    assert_eq!(s.tabs.labels, ["General", "Web Document"]);
    assert_eq!(s.size, (SHEET_W, SHORTCUT_SHEET_H));
    assert_eq!(s.tabs.open, 1);
    assert!(shortcut_of(&s.pages).is_some_and(|p| std::ptr::eq(p, s.open())));
    let (general, height) = page(false);
    let s = sheet(
        general,
        height,
        &sheet_tabs(Over::Symlink { runs: false }),
        |_| shortcut_page(&symlink_facts(), None),
    );
    assert_eq!(s.tabs.labels, ["General", "Shortcut"]);
    assert_eq!(s.tabs.open, 1);
}

#[test]
fn a_symlinks_shortcut_page_has_its_target_as_its_one_field() {
    let facts = symlink_facts();
    let mut p = shortcut_page(&facts, None);
    let text = |id: &str| match p.item(id).map(|i| &i.control) {
        Some(Control::Label { text, .. }) => text.clone(),
        _ => panic!("{id} is no label"),
    };
    assert_eq!(text("sc_type"), "File Folder");
    assert_eq!(text("sc_where"), "user");
    let live = |p: &Form, id: &str| match p.item(id).map(|i| &i.control) {
        Some(
            Control::Edit { enabled, .. }
            | Control::Button { enabled, .. }
            | Control::Label { enabled, .. },
        ) => *enabled,
        _ => panic!("{id}"),
    };
    for id in [
        SHORTCUT_IDS[1],
        SHORTCUT_IDS[2],
        "sc_start_l",
        "sc_comment_l",
        CHANGE_ICON,
    ] {
        assert!(!live(&p, id), "{id} is greyed");
    }
    assert!(live(&p, SHORTCUT_IDS[0]) && live(&p, FIND_TARGET));
    assert!(live(&p, "sc_target_l"));
    assert_eq!(p.focus().map(|i| p.items[i].id), Some(SHORTCUT_IDS[0]));
    p.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
    p.key(Key::Char('s'));
    assert_eq!(
        shortcut_moves(&p, &facts.fields),
        [(0, "/home/user/Documentss".to_string())]
    );
    let launcher = shortcut_page(&shortcut_facts(), None);
    assert!(live(&launcher, SHORTCUT_IDS[1]) && live(&launcher, CHANGE_ICON));
}

#[test]
fn the_web_page_lands_on_its_grid_and_reports_its_url() {
    let facts = web_facts();
    let mut p = web_page(&facts, None);
    let rect = |p: &Form, id: &str| p.item(id).expect(id).rect;
    assert_eq!(rect(&p, "icon"), Rect::new(21, 41, 32, 32));
    assert_eq!((rect(&p, "wd_name").x, rect(&p, "wd_name").y), (64, 49));
    assert_eq!(rect(&p, "wd_sep0"), Rect::new(21, 85, 318, 2));
    assert_eq!(rect(&p, "wd_sep1"), Rect::new(21, 184, 318, 2));
    assert_eq!(rect(&p, SHORTCUT_IDS[0]), Rect::new(96, 98, 245, 23));
    assert_eq!(rect(&p, "wd_key"), Rect::new(96, 126, 245, 23));
    assert_eq!(rect(&p, "wd_url_l"), Rect::new(21, 103, 75, 13));
    assert_eq!(rect(&p, "wd_key_l").y, 131);
    assert_eq!(rect(&p, "wd_visits_l"), Rect::new(21, 160, 75, 13));
    assert_eq!(rect(&p, "wd_visits").x, 96);
    assert_eq!(rect(&p, CHANGE_ICON), Rect::new(250, 194, 90, 23));
    assert!(matches!(
        p.item("wd_key").map(|i| &i.control),
        Some(Control::HotKey(h)) if !h.enabled
    ));
    assert!(matches!(
        p.item("wd_key_l").map(|i| &i.control),
        Some(Control::Label { enabled: false, .. })
    ));
    assert_eq!(p.focus().map(|i| p.items[i].id), Some(SHORTCUT_IDS[0]));
    assert_eq!(
        p.focused_edit().and_then(|e| e.copy_text()).as_deref(),
        Some("http://www.example.com/")
    );
    let opened = [facts.url.clone(), String::new(), String::new()];
    assert_eq!(shortcut_values(&p), opened);
    p.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
    p.key(Key::Char('a'));
    assert_eq!(
        shortcut_moves(&p, &opened),
        [(0, "http://www.example.com/a".to_string())]
    );
}

#[test]
fn the_web_page_draws_on_its_grid() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let (general, height) = page(false);
    let s = sheet(general, height, &sheet_tabs(Over::WebLink), |_| {
        web_page(&web_facts(), None)
    });
    let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    crate::controls::propsheet::draw(&mut c, &text(), &classic_paint(cs, 1), &s, true, px());
    let at = |x, y| c.get(x, y);
    for y in [85, 184] {
        assert!(
            (21..338).all(|x| at(x, y) == cs.button_shadow.argb()),
            "shadow row {y}"
        );
        assert!(
            (21..338).all(|x| at(x, y + 1) == cs.button_hilight.argb()),
            "light row {y}"
        );
    }
    assert_eq!(at(150, 98), cs.button_shadow.argb(), "URL's frame top");
    assert_eq!(at(150, 99), cs.button_dk_shadow.argb());
    assert_eq!(at(96, 110), cs.button_shadow.argb(), "URL's frame left");
    assert_eq!(at(150, 120), cs.button_hilight.argb(), "URL's frame bottom");
    assert_eq!(at(340, 110), cs.button_hilight.argb(), "URL's frame right");
    assert_eq!(at(260, 194), cs.button_hilight.argb(), "Change Icon's top");
    if !crate::text::test_tahoma() {
        return;
    }
    let ink = cs.button_text.argb();
    let first = |x0: i32, x1: i32, y0: i32, y1: i32| {
        let top = (y0..y1).find(|&y| (x0..x1).any(|x| at(x, y) == ink));
        let left = (x0..x1).find(|&x| (y0..y1).any(|y| at(x, y) == ink));
        (left.expect("ink"), top.expect("ink"))
    };
    assert_eq!(first(15, 60, 100, 116), (21, 106), "URL:");
    assert_eq!(first(15, 60, 158, 175), (21, 163), "Visits:");
    assert_eq!(first(55, 70, 45, 70), (64, 52), "the name's capital");
    assert_eq!(first(90, 160, 158, 175).0, 96, "Unknown");
}

#[test]
fn a_launchers_sheet_opens_on_its_shortcut_tab() {
    let (general, height) = page(false);
    let s = sheet(general, height, &sheet_tabs(Over::Launcher), |_| {
        shortcut_page(&shortcut_facts(), None)
    });
    assert_eq!(s.size, (SHEET_W, SHORTCUT_SHEET_H));
    assert_eq!(s.pages.len(), 3);
    assert_eq!(s.tabs.open, 1);
    assert!(shortcut_of(&s.pages).is_some_and(|p| std::ptr::eq(p, s.open())));
}

#[test]
fn the_shortcut_page_lands_on_its_grid_and_reports_its_moves() {
    let facts = shortcut_facts();
    let mut p = shortcut_page(&facts, None);
    let rect = |p: &Form, id: &str| p.item(id).expect(id).rect;
    assert_eq!(rect(&p, "icon"), Rect::new(21, 42, 32, 32));
    assert_eq!(rect(&p, "sc_name").x, 87);
    assert_eq!(rect(&p, "sc_name").y, 49);
    assert_eq!(rect(&p, "sc_sep0"), Rect::new(21, 80, 318, 2));
    assert_eq!(rect(&p, "sc_sep1"), Rect::new(21, 173, 319, 2));
    assert_eq!(rect(&p, "sc_type_l"), Rect::new(21, 92, 79, 13));
    assert_eq!(rect(&p, "sc_where"), Rect::new(100, 118, 240, 13));
    let fields = [
        SHORTCUT_IDS[0],
        SHORTCUT_IDS[1],
        "sc_key",
        "sc_run",
        SHORTCUT_IDS[2],
    ];
    assert_eq!(fields.map(|id| rect(&p, id).y), [142, 186, 217, 248, 277]);
    assert!(
        fields
            .iter()
            .all(|id| (rect(&p, id).x, rect(&p, id).width) == (100, 240))
    );
    assert_eq!(rect(&p, "sc_run").height, 21);
    assert_eq!(rect(&p, "sc_comment_l"), Rect::new(21, 279, 79, 13));
    assert_eq!(
        [FIND_TARGET, CHANGE_ICON, "sc_advanced"].map(|id| rect(&p, id)),
        [
            Rect::new(58, 306, 90, 23),
            Rect::new(154, 306, 90, 23),
            Rect::new(250, 306, 90, 23)
        ]
    );
    let greyed = |id: &str| match p.item(id).map(|i| &i.control) {
        Some(Control::HotKey(h)) => !h.enabled,
        Some(Control::Combo(c)) => !c.enabled,
        Some(Control::Button { enabled, .. }) => !enabled,
        _ => false,
    };
    assert!(greyed("sc_key") && greyed("sc_run") && greyed("sc_advanced"));
    assert!(!greyed(FIND_TARGET) && !greyed(CHANGE_ICON));
    assert_eq!(p.focus().map(|i| p.items[i].id), Some(SHORTCUT_IDS[0]));
    assert_eq!(
        p.focused_edit().and_then(|e| e.copy_text()).as_deref(),
        Some("firefox %u")
    );
    assert_eq!(shortcut_values(&p), facts.fields);
    p.focus_id(SHORTCUT_IDS[2]);
    p.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
    p.key(Key::Char('!'));
    assert_eq!(
        shortcut_moves(&p, &facts.fields),
        [(2, "Browse the web!".to_string())]
    );
}

#[test]
fn the_shortcut_page_draws_on_its_grid() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let (general, height) = page(false);
    let s = sheet(general, height, &sheet_tabs(Over::Launcher), |_| {
        shortcut_page(&shortcut_facts(), None)
    });
    let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    crate::controls::propsheet::draw(&mut c, &text(), &classic_paint(cs, 1), &s, true, px());
    let at = |x, y| c.get(x, y);
    for y in [80, 173] {
        assert!(
            (21..338).all(|x| at(x, y) == cs.button_shadow.argb()),
            "shadow row {y}"
        );
        assert!(
            (21..338).all(|x| at(x, y + 1) == cs.button_hilight.argb()),
            "light row {y}"
        );
    }
    // A label wider than its column under another face moves the fields over.
    let target = s.open().rect(SHORTCUT_IDS[0]).unwrap();
    let (mid_x, mid_y) = (target.x + target.width / 4, target.y + 8);
    assert_eq!(
        at(mid_x, target.y),
        cs.button_shadow.argb(),
        "Target's frame top"
    );
    assert_eq!(at(mid_x, target.y + 1), cs.button_dk_shadow.argb());
    assert_eq!(
        at(target.x, mid_y),
        cs.button_shadow.argb(),
        "Target's frame left"
    );
    assert_eq!(
        at(mid_x, target.bottom() - 1),
        cs.button_hilight.argb(),
        "Target's frame bottom"
    );
    assert_eq!(
        at(target.right() - 1, mid_y),
        cs.button_hilight.argb(),
        "Target's frame right"
    );
    assert_eq!(
        at(100, 306),
        cs.button_hilight.argb(),
        "Find Target's top edge"
    );
    if !crate::text::test_tahoma() {
        return;
    }
    assert_eq!(target, Rect::new(100, 142, 240, 23), "Target's field");
    let ink_top = |x0: i32, x1: i32, y0: i32| {
        (y0..y0 + 13)
            .find(|&y| (x0..x1).any(|x| at(x, y) == cs.button_text.argb()))
            .expect("ink")
    };
    assert_eq!(ink_top(21, 27, 92), 95, "Target type:");
    assert_eq!(ink_top(21, 27, 144), 147, "Target:");
    assert_eq!(ink_top(87, 92, 49), 52, "the name");
    let name_left = (80..100).find(|&x| (49..62).any(|y| at(x, y) == cs.button_text.argb()));
    assert_eq!(name_left, Some(87), "the name's first ink");
}

#[test]
fn a_launchers_change_icon_box_has_no_restore() {
    use crate::controls::iconlist::IconList;
    let list = IconList::new(vec!["firefox".into()], vec![None], None);
    let mut list = list;
    list.pick("firefox");
    let f = change_icon_form("hicolor", list, false);
    assert!(f.item(CHANGE_ICON_RESTORE).is_none());
    assert_eq!(
        f.item(CHANGE_ICON_OK).unwrap().rect,
        Rect::new(122, 292, 75, 23)
    );
    assert_eq!(change_icon_picked(&f).as_deref(), Some("firefox"));
}

#[test]
#[ignore]
fn render_shortcut_page() {
    let t = text();
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let (general, height) = page(false);
    let s = sheet(general, height, &sheet_tabs(Over::Launcher), |_| {
        shortcut_page(&shortcut_facts(), None)
    });
    let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &s, true, px());
    crate::raster::write_preview(&c, "filesheet-shortcut.png");
}

#[test]
#[ignore]
fn render_link_pages() {
    let t = text();
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let pages: [(Over, Form, &str); 2] = [
        (
            Over::Symlink { runs: false },
            shortcut_page(&symlink_facts(), None),
            "filesheet-symlink.png",
        ),
        (
            Over::WebLink,
            web_page(&web_facts(), None),
            "filesheet-web.png",
        ),
    ];
    for (over, own, file) in pages {
        let (general, height) = page(false);
        let mut own = Some(own);
        let s = sheet(general, height, &sheet_tabs(over), |_| {
            own.take().expect("one own page")
        });
        let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &s, true, px());
        crate::raster::write_preview(&c, file);
    }
}

#[test]
#[ignore]
fn render_summary_page() {
    let t = text();
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let (general, _) = page(false);
    let values: [String; 6] = ["Report", "", "Ann", "", "q3", "first\nsecond"].map(String::from);
    let mut s = sheet(general, 478, &[Tab::Summary], |_| {
        summary_page(Some(&values))
    });
    s.open_tab(1);
    let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &s, true, px());
    crate::raster::write_preview(&c, "filesheet-summary.png");
}

#[test]
fn a_sheet_is_as_tall_as_its_tallest_page() {
    let (general, _) = page(true);
    let s = sheet(general.clone(), 424, &[Tab::Sharing], |_| Form::new(vec![]));
    assert_eq!(s.size, (SHEET_W, 424));
    assert_eq!(s.pages.len(), 2);
    let s = sheet(general.clone(), 424, &[Tab::Compatibility], |_| {
        Form::new(vec![])
    });
    assert_eq!(s.size.1, 437);
    let s = sheet(general, 424, &[Tab::Compatibility, Tab::NoSummary], |_| {
        Form::new(vec![])
    });
    assert_eq!(s.size.1, 478);
}

#[test]
fn the_sharing_page_lands_on_its_grid() {
    let p = sharing_page(None, None);
    let rect = |id: &str| p.item(id).expect(id).rect;
    assert_eq!(rect("sh_g0"), Rect::new(21, 43, 318, 108));
    assert_eq!(rect("sh_g1"), Rect::new(21, 161, 318, 150));
    assert_eq!(rect("sh_local"), Rect::new(31, 54, 32, 32));
    assert_eq!(rect("sh_network"), Rect::new(31, 171, 32, 32));
    assert_eq!(
        ["sh_a0", "sh_b0", "sh_c0", "sh_d0"].map(|id| (rect(id).y, rect(id).height)),
        [(53, 26), (92, 26), (170, 39), (237, 26)]
    );
    assert_eq!(rect("sh_private"), Rect::new(70, 129, 13, 13));
    assert_eq!(p.focus(), None, "nothing on it is live");
    lines_fit(&p);
}

#[test]
fn the_compatibility_page_lands_on_its_grid() {
    let p = compat_page();
    let rect = |id: &str| p.item(id).expect(id).rect;
    assert_eq!(
        ["cp_g0", "cp_g1", "cp_g2"].map(rect),
        [
            Rect::new(21, 103, 315, 73),
            Rect::new(21, 197, 315, 92),
            Rect::new(21, 311, 315, 40)
        ]
    );
    assert_eq!(rect("cp_mode"), Rect::new(37, 124, 13, 13));
    assert_eq!((rect("cp_list").x, rect("cp_list").y), (37, 144));
    assert_eq!(
        ["cp_256", "cp_640", "cp_themes", "cp_text"].map(|id| rect(id).y),
        [216, 241, 265, 327]
    );
    assert_eq!(p.focus(), None, "nothing on it is live");
    lines_fit(&p);
}

fn lines_fit(p: &Form) {
    let t = text();
    if !crate::text::test_tahoma() {
        return;
    }
    for it in &p.items {
        let lines = match &it.control {
            Control::Label { text: s, .. } => std::slice::from_ref(s),
            Control::Lines(l) | Control::GreyedLines(l) => l.as_slice(),
            _ => continue,
        };
        for s in lines {
            let w = t.measure(s, false, px());
            assert!(
                w <= it.rect.width,
                "{} is {w} px in {}",
                it.id,
                it.rect.width
            );
        }
    }
}

#[test]
#[ignore]
fn render_tab_pages() {
    let t = text();
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let (general, _) = page(true);
    let mut s = sheet(general, 424, &[Tab::Sharing, Tab::Compatibility], |_| {
        Form::new(vec![])
    });
    for (i, name) in [(1, "filesheet-sharing.png"), (2, "filesheet-compat.png")] {
        s.open_tab(i);
        let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &s, true, px());
        crate::raster::write_preview(&c, name);
    }
}

#[test]
#[ignore]
fn render_file_sheet() {
    let t = text();
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let (page, height) = page(false);
    let sheet =
        crate::controls::propsheet::PropSheet::new(&["General"], vec![page], (SHEET_W, height));
    let mut c = Canvas::new(SHEET_W as u32, height as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &sheet, true, px());
    crate::raster::write_preview(&c, "filesheet-file.png");
}

#[test]
fn a_folders_read_only_box_has_three_states() {
    let (mut folder, _) = page(true);
    assert!(matches!(
        folder.item(HIDDEN).map(|i| &i.control),
        Some(Control::Check { enabled: true, .. })
    ));
    assert!(matches!(
        folder.item(ADVANCED).map(|i| &i.control),
        Some(Control::Button { enabled: true, .. })
    ));
    assert_eq!(read_only(&folder), CheckState::Mixed);
    folder.focus_id(READ_ONLY);
    let mut seen = Vec::new();
    for _ in 0..3 {
        folder.key(Key::Space);
        seen.push(read_only(&folder));
    }
    assert_eq!(seen, [CheckState::Off, CheckState::On, CheckState::Mixed]);
    let (mut file, _) = page(false);
    assert_eq!(read_only(&file), CheckState::Off);
    file.focus_id(READ_ONLY);
    file.key(Key::Space);
    file.key(Key::Space);
    assert_eq!(read_only(&file), CheckState::Off, "a file's box flips");
}

fn exec_page(is_dir: bool, executable: bool, can_set_executable: bool) -> Form {
    let f = Facts {
        executable,
        can_set_executable,
        ..facts(is_dir)
    };
    general_page(&rows(&f), "hi.sh", None, None).0
}

fn exec_enabled(page: &Form) -> bool {
    matches!(
        page.item(EXECUTABLE).map(|i| &i.control),
        Some(Control::Check { enabled: true, .. })
    )
}

#[test]
fn the_executable_box_shows_the_bit_and_greys_where_it_cannot_change() {
    let mut file = exec_page(false, false, true);
    assert!(!executable(&file) && exec_enabled(&file));
    file.focus_id(EXECUTABLE);
    file.key(Key::Space);
    assert!(executable(&file), "a click ticks it");
    file.key(Key::Space);
    assert!(!executable(&file), "two states only");
    let theirs = exec_page(false, true, false);
    assert!(
        executable(&theirs) && !exec_enabled(&theirs),
        "greyed, ticked"
    );
    let folder = exec_page(true, true, true);
    assert!(
        !executable(&folder) && !exec_enabled(&folder),
        "a folder's is greyed"
    );
    assert!(
        set_page(false).0.item(EXECUTABLE).is_none(),
        "a set has none"
    );
    let rect = |p: &Form| p.item(EXECUTABLE).unwrap().rect;
    assert_eq!(rect(&file), Rect::new(94, 366, 13, 13), "under Read-only");
    assert_eq!(rect(&folder), Rect::new(94, 324, 13, 13), "under Hidden");
}

fn fnv_in(c: &Canvas, r: Rect) -> u64 {
    (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, (x, y)| {
            c.get(x, y).to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
}

#[test]
fn the_executable_box_draws_in_each_look() {
    use crate::interface::Interface;
    let t = text();
    if !crate::text::test_tahoma() {
        return;
    }
    let mut got = Vec::new();
    for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::theme::Theme {
            access_keys: true,
            ..crate::interfaces::theme_for(look)
        };
        for enabled in [true, false] {
            let page = exec_page(false, true, enabled);
            let height = 478;
            let mut c = Canvas::new(SHEET_W as u32, height as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            crate::controls::form::draw(&mut c, &t, &theme.paint(), &page, false, px());
            let r = page.item(EXECUTABLE).unwrap().rect;
            let area = Rect::new(r.x - 2, r.y - 2, 90, r.height + 4);
            got.push((look, enabled, format!("{:016x}", fnv_in(&c, area))));
        }
    }
    for pair in got.chunks(2) {
        assert_ne!(pair[0].2, pair[1].2, "{:?}: greyed differs", pair[0].0);
    }
    let want = [
        (Interface::Classic, true, "14484dec4f17638c"),
        (Interface::Classic, false, "bce9ff417d9f497c"),
        (Interface::Blue, true, "d0b4098443f797c2"),
        (Interface::Blue, false, "612c64f3812bf888"),
        (Interface::Glass, true, "c462e8ee9d2f134e"),
        (Interface::Glass, false, "f908158d64544474"),
    ]
    .map(|(look, enabled, h)| (look, enabled, h.to_string()));
    assert_eq!(got, want);
}

#[test]
fn hidden_stands_for_the_leading_dot() {
    assert_eq!(shown_name(".notes.txt", ".txt", true), "notes");
    assert_eq!(shown_name("notes.txt", ".txt", false), "notes");
    assert_eq!(wanted_name("notes", true, ".txt"), ".notes.txt");
    assert_eq!(wanted_name("notes", false, ""), "notes");
}

#[test]
fn advanced_attributes_land_on_their_grid() {
    let (t, px) = (text(), px());
    for (is_dir, size, lines, groups, checks, details, ok) in [
        (
            false,
            (378, 263),
            vec![22],
            [62, 148],
            [83, 104, 168, 191],
            186,
            229,
        ),
        (
            true,
            (378, 285),
            vec![16, 34, 47],
            [82, 170],
            [103, 126, 191, 213],
            209,
            251,
        ),
    ] {
        let subject = Subject::single(is_dir);
        assert_eq!(advanced_size(subject), size);
        let f = advanced_form(subject, None, None, &t, px, TextUnits::ONE);
        let rect = |id: &str| f.item(id).expect(id).rect;
        assert_eq!(rect("icon"), Rect::new(11, 12, 32, 32));
        let ys: Vec<i32> = ["adv_l0", "adv_l1", "adv_l2"]
            .iter()
            .filter_map(|id| f.item(id).map(|i| i.rect.y))
            .collect();
        assert_eq!(ys, lines, "dir {is_dir}");
        assert_eq!(rect("adv_g0"), Rect::new(11, groups[0], 357, 70));
        assert_eq!(rect("adv_g1").y, groups[1]);
        assert_eq!((rect("adv_t0").x, rect("adv_t0").y), (20, groups[0] - 5));
        for (id, y) in ["adv_archive", "adv_index", "adv_compress", "adv_encrypt"]
            .into_iter()
            .zip(checks)
        {
            assert_eq!(rect(id), Rect::new(21, y, 13, 13), "{id}");
            assert!(matches!(
                f.item(id).map(|i| &i.control),
                Some(Control::Check {
                    enabled: false,
                    checked: false,
                    ..
                })
            ));
        }
        assert_eq!(rect("adv_details"), Rect::new(285, details, 75, 23));
        assert_eq!(rect(ADV_OK), Rect::new(213, ok, 75, 23));
        assert_eq!(rect(ADV_CANCEL), Rect::new(294, ok, 75, 23));
        assert_eq!(f.default_button().map(|i| f.items[i].id), Some(ADV_OK));
    }
}

#[test]
fn an_advanced_group_title_breaks_its_top_line() {
    let t = text();
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = Theme::default();
    let cs = &theme.colors;
    let f = advanced_form(Subject::File, None, None, &t, px(), TextUnits::ONE);
    let (w, h) = advanced_size(Subject::File);
    let mut c = Canvas::new(w as u32, h as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    crate::controls::form::draw(&mut c, &t, &classic_paint(cs, 1), &f, false, px());
    let g = f.item("adv_g0").unwrap().rect;
    let tw = t.measure("Archive and Index attributes", false, px());
    assert_eq!(c.get(g.x + 6, g.y), cs.button_shadow.argb(), "line before");
    assert_eq!(c.get(g.x + 7, g.y), cs.button_face.argb(), "the break");
    let back = g.x + 9 + tw + 2;
    assert_eq!(c.get(back - 1, g.y), cs.button_face.argb(), "break end");
    assert_eq!(c.get(back, g.y), cs.button_shadow.argb(), "line after");
    let ink =
        (g.x..g.x + 20).find(|&x| (g.y - 5..g.y + 8).any(|y| c.get(x, y) == cs.button_text.argb()));
    assert_eq!(ink, Some(g.x + 9), "the title's first ink");
}

fn bin_facts() -> BinFacts {
    BinFacts {
        type_name: "Rich Text Document".into(),
        origin: "Desktop".into(),
        size: "4.00 KB".into(),
        deleted: Some("Monday, September 14, 2026, 3:43:39 PM".into()),
        created: Some("Monday, September 14, 2026, 3:43:34 PM".into()),
        read_only: false,
        hidden: false,
    }
}

#[test]
fn a_bin_items_page_lands_on_its_grid() {
    let page = bin_page(&bin_facts(), "New Rich Text Document", None);
    let rect = |id: &str| page.item(id).expect(id).rect;
    assert_eq!(rect("icon"), Rect::new(21, 40, 32, 32));
    assert_eq!((rect(NAME).x, rect(NAME).y), (63, 47));
    assert!(matches!(
        page.item(NAME).map(|i| &i.control),
        Some(Control::Label { .. })
    ));
    let seps = ["sep0", "sep1", "sep2"].map(&rect);
    assert_eq!(seps.map(|r| r.y), [81, 176, 244]);
    assert!(seps.iter().all(|r| (r.x, r.right()) == (21, 339)));
    for (id, y) in [("l_type", 94), ("l_origin", 120), ("l_size", 146)] {
        assert_eq!((rect(id).x, rect(id).y), (21, y), "{id}");
    }
    assert_eq!(rect("v_type"), Rect::new(85, 94, 254, 26));
    assert_eq!(rect("v_origin"), Rect::new(85, 120, 254, 26));
    for (id, y) in [("l_deleted", 189), ("l_made", 215)] {
        assert_eq!((rect(id).x, rect(id).y), (21, y), "{id}");
    }
    assert_eq!(rect("v_deleted"), Rect::new(115, 189, 224, 26));
    assert_eq!(rect("l_attr").y, 257);
    for (id, x, y) in [
        (READ_ONLY, 112, 258),
        (HIDDEN, 204, 258),
        ("archive", 112, 277),
        ("compress", 112, 297),
        ("encrypt", 112, 316),
    ] {
        assert_eq!(rect(id), Rect::new(x, y, 13, 13), "{id}");
        assert!(
            matches!(
                page.item(id).map(|i| &i.control),
                Some(Control::Check { enabled: false, .. })
            ),
            "{id} is greyed"
        );
    }
    assert_eq!(rect(RESTORE), Rect::new(265, 344, 75, 23));
    assert_eq!((SHEET_W, BIN_SHEET_H), (361, 418));
}

#[test]
fn a_bin_item_without_a_date_drops_the_row() {
    let facts = BinFacts {
        deleted: None,
        ..bin_facts()
    };
    let page = bin_page(&facts, "x", None);
    assert!(page.item("l_deleted").is_none());
    assert_eq!(page.item("l_made").map(|i| i.rect.y), Some(189));
}

#[test]
fn a_bin_items_page_inks_on_its_grid() {
    let t = text();
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = Theme::default();
    let cs = &theme.colors;
    let page = bin_page(&bin_facts(), "New Rich Text Document", None);
    let mut c = Canvas::new(SHEET_W as u32, BIN_SHEET_H as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    crate::controls::form::draw(&mut c, &t, &classic_paint(cs, 1), &page, false, px());
    let ink = |r: Rect| {
        let hits: Vec<(i32, i32)> = (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
            .collect();
        (
            hits.iter().map(|p| p.0).min(),
            hits.iter().map(|p| p.1).min(),
        )
    };
    let at = |id: &str| page.item(id).unwrap().rect;
    let got: Vec<_> = [
        NAME,
        "l_type",
        "v_type",
        "l_origin",
        "v_origin",
        "l_size",
        "v_size",
        "l_deleted",
        "v_deleted",
        "l_made",
        "v_made",
        "l_attr",
    ]
    .into_iter()
    .map(|id| (id, ink(at(id))))
    .collect();
    let want = [
        (NAME, (63, 49)),
        ("l_type", (21, 97)),
        ("v_type", (85, 96)),
        ("l_origin", (21, 123)),
        ("v_origin", (85, 122)),
        ("l_size", (21, 149)),
        ("v_size", (85, 149)),
        ("l_deleted", (21, 191)),
        ("v_deleted", (115, 191)),
        ("l_made", (21, 217)),
        ("v_made", (115, 217)),
        ("l_attr", (21, 259)),
    ]
    .map(|(id, (x, y))| (id, (Some(x), Some(y))));
    assert_eq!(got, want);
}

#[test]
fn a_bin_items_page_fits_its_text() {
    let date = "Wednesday, September 30, 2026, 12:00:00 PM";
    let facts = BinFacts {
        type_name: "Encapsulated PostScript image with a TIFF preview".into(),
        origin: "Quarterly reports for the northern region (final drafts, 2026)".into(),
        size: "1.23 MB".into(),
        deleted: Some(date.into()),
        created: Some(date.into()),
        ..bin_facts()
    };
    let unbroken = BinFacts {
        origin: "Quarterly_reports_for_the_northern_region-final_drafts".into(),
        ..facts.clone()
    };
    let over = crate::interfaces::form_overflows(|_, _, _| {
        [
            ("bin item", &facts),
            ("bin item, origin without spaces", &unbroken),
        ]
        .map(|(name, facts)| {
            let page = bin_page(facts, "Quarterly report 2026.odt", None);
            let sheet = crate::controls::propsheet::PropSheet::new(
                &["General"],
                vec![page],
                (SHEET_W, BIN_SHEET_H),
            );
            (name.into(), sheet.pages[0].clone())
        })
        .into()
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

#[test]
#[ignore]
fn render_bin_item_sheet() {
    let theme = Theme::default();
    let t = text();
    let page = bin_page(&bin_facts(), "New Rich Text Document", None);
    let sheet = crate::controls::propsheet::PropSheet::new(
        &["General"],
        vec![page],
        (SHEET_W, BIN_SHEET_H),
    );
    let mut c = Canvas::new(SHEET_W as u32, BIN_SHEET_H as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    crate::controls::propsheet::draw(
        &mut c,
        &t,
        &classic_paint(&theme.colors, 1),
        &sheet,
        true,
        px(),
    );
    crate::raster::write_preview(&c, "filesheet-bin.png");
}

#[test]
fn a_blue_advanced_group_title_inks_like_a_group_box() {
    let t = text();
    if !crate::text::test_tahoma() {
        return;
    }
    let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cs = &blue.colors;
    let f = advanced_form(Subject::File, None, None, &t, px(), TextUnits::ONE);
    let (w, h) = advanced_size(Subject::File);
    let mut c = Canvas::new(w as u32, h as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    crate::controls::form::draw(&mut c, &t, &blue.paint(), &f, false, px());
    let title = f.item("adv_t0").unwrap().rect;
    let count = |ink: u32| {
        (title.y..title.bottom())
            .flat_map(|y| (title.x..title.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .count()
    };
    let label = crate::blue::Variant::Blue.scheme().inks.group_label.argb();
    assert!(count(label) > 0, "the title inks the group box label");
    assert_eq!(count(cs.button_text.argb()), 0, "no black in the title");
}
