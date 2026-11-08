use super::*;

#[test]
fn a_config_written_elsewhere_shows_on_the_open_sheet_and_apply_keeps_it() {
    let path = tmp("config-changed").join("config.toml");
    std::fs::write(&path, "").unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    a.want.slow_keys = true;
    std::fs::write(&path, "[accessibility]\nmouse_keys = true\n").unwrap();
    assert!(a.config_changed());
    let ax = &a.sheet.pages[Page::TextKeys.index()];
    assert_eq!(ax.checked("mouse_keys"), Some(true));
    let speed = &ax.item("mouse_keys_speed").unwrap().control;
    assert!(
        matches!(speed, form::Control::TrackBar(t) if t.enabled),
        "its controls ungrey"
    );
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
fn a_save_that_fails_shows_why_and_does_not_quit() {
    let mut app = app("save-fails");
    std::fs::write(&app.path, "[general\n").expect("write");
    let keys = outputwrite::OutputKeys {
        adaptive_sync: Some(true),
        ..outputwrite::OutputKeys::default()
    };
    let r = app.display_effects(vec![
        displayflow::Effect::Save("DP-1".into(), keys),
        displayflow::Effect::Quit,
    ]);
    assert!(!r.quit, "the error stays on screen");
    let text = app.error.as_ref().map(|b| b.fields.text.clone());
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
    assert!(a.asking.is_some(), "it asks first");
    a.asking = None;
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
