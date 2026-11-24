use super::*;

#[test]
fn a_config_written_elsewhere_shows_on_the_open_sheet_and_apply_keeps_it() {
    let path = tmp("config-changed").join("config.toml");
    std::fs::write(&path, "").unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    a.want.slow_keys = true;
    std::fs::write(&path, "[accessibility]\nmouse_keys = true\n").unwrap();
    assert!(a.config_changed());
    let pointer = &a.sheet.pages[Page::PointerAlerts.index()];
    assert_eq!(pointer.checked("mouse_keys"), Some(true));
    let speed = &pointer.item("mouse_keys_speed").unwrap().control;
    assert!(
        matches!(speed, form::Control::TrackBar(t) if t.enabled),
        "its controls ungrey"
    );
    let ax = &a.sheet.pages[Page::TextKeys.index()];
    assert_eq!(ax.checked("slow_keys"), Some(true), "a pending check stays");
    assert!(!a.config_changed(), "a file read as before changes nothing");
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("mouse_keys = true") && text.contains("slow_keys = true"),
        "{text}"
    );
}

#[test]
fn a_config_written_elsewhere_keeps_the_selection_of_an_edit_typed_in() {
    let path = tmp("config-caret").join("config.toml");
    std::fs::write(&path, "").unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let id = "task_manager";
    let p = (0..a.sheet.pages.len())
        .find(|&i| a.sheet.pages[i].item(id).is_some())
        .unwrap();
    fn edit<'a>(f: &'a mut form::Form, id: &str) -> &'a mut reprisekit::textedit::TextEdit {
        match &mut f.item_mut(id).unwrap().control {
            form::Control::Edit { edit, .. } => edit,
            c => panic!("{id} is an edit, not {c:?}"),
        }
    }
    a.sheet.pages[p].show_edit_text(id, "htop --tree");
    edit(&mut a.sheet.pages[p], id).set_selection(0, 4);
    a.sheet_event(propsheet::Event::Page(p, form::Event::Changed(id)));
    std::fs::write(&path, "[accessibility]\nmouse_keys = true\n").unwrap();
    assert!(a.config_changed());
    let e = edit(&mut a.sheet.pages[p], id);
    assert_eq!(e.text, "htop --tree");
    assert_eq!(e.selection(), 0..4, "the selection stays");
}

#[test]
fn a_save_that_fails_shows_why_and_does_not_quit() {
    let mut app = app("save-fails");
    std::fs::write(&app.path, "[general\n").expect("write");
    let keys = outputwrite::OutputKeys {
        adaptive_sync: Some(repriseconf::config::VrrPolicy::On),
        ..outputwrite::OutputKeys::default()
    };
    let r = app.display_effects(vec![
        displayflow::Effect::Save("DP-1".into(), keys),
        displayflow::Effect::Quit,
    ]);
    assert!(!r.quit, "the error stays on screen");
    let text = app.error.shown().map(|b| b.fields.text.clone());
    assert_eq!(
        text.as_deref(),
        Some("config.toml does not parse; the display setting was not saved.")
    );
}

#[test]
fn a_missing_config_opens_on_the_defaults_without_an_error() {
    let dir = tmp("missing");
    assert_eq!(read_config(&dir.join("config.toml")), (String::new(), None));
}

#[test]
fn a_config_that_does_not_parse_names_its_line() {
    let path = tmp("broken").join("config.toml");
    std::fs::write(&path, "[general]\nx = \n").expect("write");
    let (text, err) = read_config(&path);
    assert_eq!(text, "[general]\nx = \n");
    let err = err.expect("a parse error for the box");
    assert!(
        err.starts_with("config.toml does not parse at line 2: "),
        "the box names line 2, got {err:?}"
    );
}

#[test]
fn an_unreadable_config_is_reported() {
    // A directory where the file should be fails the read with EISDIR.
    let path = tmp("unreadable").join("config.toml");
    std::fs::create_dir_all(&path).expect("mkdir");
    let (text, err) = read_config(&path);
    assert_eq!(text, "");
    let err = err.expect("a read error for the box");
    assert!(
        err.starts_with("config.toml could not be read: "),
        "the box says the read failed, got {err:?}"
    );
}

/// Restore Defaults on Sound asks with the page's keys the file sets, then takes only those out
/// of `config.toml`; a page whose keys are all at their defaults has it greyed, and a page that
/// writes no key has none.
#[test]
fn restore_defaults_takes_only_the_pages_keys_out_of_the_file() {
    let dir = tmp("restore-defaults");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "# mine\n[sounds]\nbell = false\ntheme = \"oxygen\"\n\n[taskbar]\nauto_hide = true\n",
    )
    .unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let lit = |a: &SettingsApp, p: Page| match a.sheet.pages[p.index()].item(propsheet::RESTORE) {
        Some(it) => matches!(it.control, form::Control::Button { enabled: true, .. }),
        None => panic!("{p:?} has no Restore Defaults"),
    };
    assert!(lit(&a, Page::Sound) && lit(&a, Page::Taskbar));
    let at = a.sheet.pages[Page::Sound.index()]
        .item(propsheet::RESTORE)
        .unwrap()
        .rect;
    let ok = a.sheet.pages[Page::Sound.index()]
        .item(propsheet::OK)
        .unwrap()
        .rect;
    assert_eq!(
        at,
        reprisekit::raster::Rect::new(152, 498, 96, 23),
        "under the page's left edge"
    );
    assert_eq!(at.y, ok.y, "on the button row");
    assert!(!lit(&a, Page::Cursor), "nothing there is off its default");
    assert!(!lit(&a, Page::StartMenu), "the file sets no Start Menu key");
    for p in [Page::Display, Page::Tablet, Page::Backup] {
        assert!(
            a.sheet.pages[p.index()].item(propsheet::RESTORE).is_none(),
            "{p:?}"
        );
    }
    a.want.cursor_size = 48;
    a.sync_apply();
    assert!(lit(&a, Page::Cursor), "an unapplied change counts");

    let keys = a.restorable(Page::Sound);
    assert_eq!(
        apply::restore_question(Page::Sound, &keys),
        "Restore the defaults on the Sound page?\n\
         These settings are taken out of config.toml:\n    sounds.theme\n    sounds.bell"
    );
    a.sheet.tabs.open = Page::Sound.index();
    a.ask_restore();
    assert!(!a.asking.is_empty(), "it asks first");
    a.asking.close();
    a.restore_defaults(Page::Sound);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# mine\n\n[taskbar]\nauto_hide = true\n"
    );
    let def = model::Settings::default();
    assert_eq!(
        (&a.want.sound_theme, a.want.bell),
        (&def.sound_theme, def.bell)
    );
    assert!(!lit(&a, Page::Sound), "greyed once at the defaults");
    assert_eq!(a.want.cursor_size, 48, "the Cursor change is still pending");
    assert!(a.dirty());
}

/// The Start Menu page writes only what sets the menu apart from the default one, and Restore
/// Defaults takes that out, or a whole `[[startmenu]]` list.
#[test]
fn the_start_menu_saves_its_changes_and_restore_defaults_takes_them_out() {
    use crate::startmenu;
    let path = tmp("startmenu-changes").join("config.toml");
    std::fs::write(&path, "").unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let lit =
        |a: &SettingsApp| match a.sheet.pages[Page::StartMenu.index()].item(propsheet::RESTORE) {
            Some(it) => matches!(it.control, form::Control::Button { enabled: true, .. }),
            None => panic!("Start Menu has Restore Defaults"),
        };
    assert!(!lit(&a), "the default menu");
    let help = startmenu::take(&mut a.want.startmenu, &[4]).unwrap();
    assert_eq!(help.label(), Some("&Help and Support"));
    a.sync_apply();
    assert!(lit(&a), "an unapplied change counts");
    assert!(a.apply());
    let text = || std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text(),
        "[[startmenu_changes]]\ndefault = \"help\"\nhidden = true\n"
    );
    assert_eq!(
        apply::restore_question(Page::StartMenu, &a.restorable(Page::StartMenu)),
        "Restore the defaults on the Start Menu page?\n\
         These settings are taken out of config.toml:\n    startmenu_changes"
    );
    a.restore_defaults(Page::StartMenu);
    assert_eq!(text(), "");
    assert_eq!(a.want.startmenu, startmenu::default_menu());
    assert_eq!(a.loaded, a.want, "nothing is left pending");
    assert!(!lit(&a));

    std::fs::write(&path, "[[startmenu]]\nlabel = \"A\"\ncommand = \"a\"\n").unwrap();
    assert!(a.config_changed());
    assert!(lit(&a), "a whole menu in the file");
    assert_eq!(
        a.restorable(Page::StartMenu).keys,
        [startmenu::STARTMENU],
        "the whole list is what goes"
    );
    a.restore_defaults(Page::StartMenu);
    assert_eq!(text(), "");
    assert_eq!(a.want.startmenu, startmenu::default_menu());
}

#[test]
fn apply_writes_back_the_menu_changes_it_cannot_show_as_the_file_has_them() {
    use crate::startmenu;
    let typo = "\
[[startmenu_changes]]
label = \"Tools\"   # mine
after = \"run\"

[[startmenu_changes.items]]
label = \"Edit\"
command = \"gedit\"
tooltip = \"Edits text\"
";
    let unknown = "\
[[startmenu_changes]]
default = \"from-a-later-reprise\"
hidden = true
";
    let path = tmp("startmenu-idle").join("config.toml");
    let text = format!("# My menu.\n{typo}\n{unknown}");
    std::fs::write(&path, &text).unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    assert_eq!(a.want.startmenu, startmenu::default_menu());
    startmenu::shift(&mut a.want.startmenu, &[5], -1).unwrap();
    assert!(a.apply());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!(
            "# My menu.\n[[startmenu_changes]]\ndefault = \"help\"\nafter = \"run\"\n\n{typo}\n{unknown}"
        ),
        "the edit, then the two changes as they were"
    );
}
