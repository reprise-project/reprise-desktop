use super::*;

#[test]
fn a_double_click_on_a_file_type_opens_the_chooser_like_enter() {
    let mut a = app("types-double-click");
    let mut ctx = test_ctx();
    a.open_page(Page::DefaultPrograms, None);
    a.defaults = Some(defaultspage::tests::defaults());
    a.refill_defaults();
    let mime = |a: &SettingsApp| a.chooser.as_ref().map(|c| c.mime.clone());

    press_side_row(&mut a, &mut ctx, 1, 1000);
    assert_eq!(mime(&a), None, "one click only picks the row");
    press_side_row(&mut a, &mut ctx, 1, 1000 + 1100);
    assert_eq!(mime(&a), None, "1100 ms apart is two clicks");
    press_side_row(&mut a, &mut ctx, 1, 1000 + 1400);
    let double = mime(&a).expect("the chooser opens on a double click");

    a.chooser = None;
    plain_key(&mut a, &ctx, keysym::RETURN);
    assert_eq!(mime(&a), Some(double), "Enter opens it for the same type");
}

#[test]
fn picking_a_terminal_keeps_its_arguments_and_typed_arguments_follow_the_program() {
    let mut a = app("terminal-args");
    a.open_page(Page::DefaultPrograms, None);
    a.defaults = Some(defaultspage::tests::defaults());
    a.want.terminal = "foot -e tmux".into();
    a.refill_defaults();
    let page = Page::DefaultPrograms.index();
    a.sheet.pages[page].set_combo(defaultspage::TERMINAL, 1);
    a.defaults_changed(defaultspage::TERMINAL);
    assert_eq!(a.want.terminal, "/usr/bin/kitty -e tmux");
    if let Some(form::Control::Edit { edit, .. }) = a.sheet.pages[page]
        .item_mut(defaultspage::TERMINAL_ARGS)
        .map(|i| &mut i.control)
    {
        *edit = reprisekit::textedit::TextEdit::new("--hold", false);
    }
    a.defaults_changed(defaultspage::TERMINAL_ARGS);
    assert_eq!(a.want.terminal, "/usr/bin/kitty --hold");
    assert_eq!(
        a.sheet.pages[page].combo_selected(defaultspage::TERMINAL),
        Some(1),
        "the program stays picked"
    );
}

#[test]
fn the_add_dialog_opens_at_the_text_size() {
    let mut a = app("add-grown");
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    a.follow_text(&ctx);
    a.open_add(Pick::Autostart);
    let client = a
        .add
        .as_ref()
        .map(|d| d.client())
        .expect("the dialog is open");
    assert_ne!(client, filedlg::CLIENT, "the dialog grows at 200%");
    let kid = a.children().into_iter().find(|k| k.id == ADD_WIN);
    assert_eq!(
        kid.map(|k| k.size),
        Some(client),
        "its window opens at the grown size"
    );
}

#[test]
fn browse_in_the_chooser_picks_a_program_file_into_its_list() {
    let mut a = app("types-browse");
    let mut ctx = test_ctx();
    a.open_page(Page::DefaultPrograms, None);
    a.defaults = Some(defaultspage::tests::defaults());
    a.refill_defaults();
    press_side_row(&mut a, &mut ctx, 1, 1000);
    plain_key(&mut a, &ctx, keysym::RETURN);
    assert!(a.chooser.is_some());
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    let alt_b = Event::Key(key_input('b' as u32, alt, Some("b")));
    a.event(CHOOSER_WIN, alt_b, &mut ctx);
    assert!(
        a.add.is_some() && a.add_for == Pick::Program,
        "Browse... opens the file dialog"
    );
    let kids = a.children();
    assert!(kids.iter().any(|k| k.id == ADD_WIN && k.modal));
    assert!(
        kids.iter().any(|k| k.id == CHOOSER_WIN),
        "the chooser stays up under it"
    );
    a.browsed_program(std::path::Path::new("/opt/view"));
    let picked = a
        .chooser
        .as_ref()
        .and_then(|c| c.ow.chosen())
        .map(|p| p.id.clone());
    assert_eq!(picked.as_deref(), Some("/opt/view"));
}

#[test]
fn a_bind_on_a_none_rows_keys_takes_that_row() {
    let mut a = app("shortcut-none-row");
    let q = (a.want.shortcuts.iter())
        .position(|b| b.keys == "Super+q")
        .unwrap();
    model::forget(&mut a.want, model::Row::Bind(q));
    let n = a.want.shortcuts.len();
    a.open_shortcut(None);
    assert_eq!(
        a.accept_shortcut(model::Shortcut::new("Super+q", "lock", "")),
        q
    );
    assert_eq!(a.want.shortcuts.len(), n);
    assert_eq!(
        a.want.shortcuts[q],
        model::Shortcut::new("Super+q", "lock", "")
    );

    model::forget(&mut a.want, model::Row::Bind(q));
    a.want
        .shortcuts
        .push(model::Shortcut::new("Super+F9", "close", ""));
    a.open_shortcut(Some(n));
    assert_eq!(
        a.accept_shortcut(model::Shortcut::new("Super+q", "close", "")),
        n - 1
    );
    assert_eq!(a.want.shortcuts.len(), n);
    assert!(!a.want.shortcuts.iter().any(|b| b.action == "none"));
}

#[test]
fn a_double_click_on_a_shortcut_edits_it_like_enter() {
    let mut a = app("shortcuts-double-click");
    let mut ctx = test_ctx();
    a.open_page(Page::Keyboard, None);
    press_side_row(&mut a, &mut ctx, 0, 1000);
    assert!(a.shortcut.is_none(), "one click only picks the row");
    press_side_row(&mut a, &mut ctx, 0, 1300);
    assert!(a.shortcut.is_some(), "a double click opens the editor");
    let target = a.shortcut_target.take();
    a.shortcut = None;
    plain_key(&mut a, &ctx, keysym::RETURN);
    assert!(a.shortcut.is_some());
    assert_eq!(a.shortcut_target, target, "Enter edits the same row");
}

#[test]
fn a_program_row_edits_its_keys_and_delete_forgets_it() {
    let dir = tmp("programs");
    std::fs::write(
        dir.join("config.toml"),
        "[[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"talk\"\n\
         description = \"Hold to talk\"\nkeys = \"Pause\"\n\
         [portal]\nshortcuts_denied = [\"org.example.NoSuchNotes\"]\n",
    )
    .unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"), "us".into());
    let n = app.want.shortcuts.len();
    assert_eq!(
        app.shortcuts.rows[n].cells,
        ["Pause", "org.example.NoSuchApp: Hold to talk"]
    );
    app.shortcuts.selection.set_only(Some(n + 1));
    app.edit_shortcut();
    assert!(app.shortcut.is_none(), "a refusal has nothing to edit");

    app.shortcuts.selection.set_only(Some(n));
    app.edit_shortcut();
    let d = app.shortcut.as_mut().expect("the dialog is up");
    assert_eq!(d.title, "Change Shortcut");
    d.event(form::Event::Pressed(shortcutdlg::CLEAR));
    let shortcutdlg::Outcome::Accept(sc) = d.event(form::Event::Pressed(shortcutdlg::OK)) else {
        panic!("no keys is a valid answer");
    };
    assert_eq!(app.accept_shortcut(sc), n);
    assert_eq!(app.want.programs[0].keys, "");
    assert_eq!(app.want.shortcuts.len(), n, "no bind was added");
    assert_eq!(app.shortcuts.rows[n].cells[0], "None");

    app.shortcuts.selection.set_only(Some(n + 1));
    app.remove_shortcut();
    assert!(app.want.denied.is_empty());
    app.shortcuts.selection.set_only(Some(n));
    app.remove_shortcut();
    assert!(app.want.programs.is_empty());
    assert_eq!(app.shortcuts.rows.len(), n);
}

#[test]
fn a_rebound_zoom_key_rewrites_the_magnifier_note() {
    let dir = tmp("zoom-note");
    std::fs::write(dir.join("config.toml"), "").unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"), "us".into());
    let note = |app: &SettingsApp| match &app.sheet.pages[Page::MagnifierFilters.index()]
        .item("mag_note")
        .unwrap()
        .control
    {
        form::Control::Lines(lines) => lines.join(" "),
        _ => panic!("the note is lines"),
    };
    assert!(note(&app).starts_with("Super+= zooms in"), "{}", note(&app));
    let i = (app.want.shortcuts.iter())
        .position(|sc| sc.action == "zoom-in")
        .expect("zoom-in is bound by default");
    app.shortcut_target = Some(model::Row::Bind(i));
    app.accept_shortcut(model::Shortcut::new("Ctrl+Alt+plus", "zoom-in", ""));
    assert!(
        note(&app).starts_with("Ctrl+Alt++ zooms in"),
        "{}",
        note(&app)
    );
}

#[test]
fn a_programs_taken_keys_name_the_program() {
    let dir = tmp("program-taken");
    std::fs::write(
        dir.join("config.toml"),
        "[[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"talk\"\n\
         description = \"Hold to talk\"\nkeys = \"Pause\"\n\
         [[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"mute\"\nkeys = \"F9\"\n",
    )
    .unwrap();
    let app = SettingsApp::open(dir.join("config.toml"), "us".into());
    let taken = app.taken_keys(None);
    let n = app.want.shortcuts.len();
    assert_eq!(taken[n].action, "org.example.NoSuchApp: Hold to talk");
    assert_eq!(
        taken[n + 1].action,
        "org.example.NoSuchApp: mute",
        "no description: the id"
    );
}

#[test]
fn a_link_row_starts_its_program_in_its_app_scope_and_settings_stays() {
    use repriseconf::pages::links;
    use std::ffi::OsString;
    let mut a = app("links");
    let net = links()
        .position(|p| p.id == "reprise-network.desktop")
        .unwrap();
    let bt = links()
        .position(|p| p.id == "reprise-bluetooth.desktop")
        .unwrap();
    a.links = links()
        .map(|p| {
            (p.id == "reprise-network.desktop").then(|| {
                reprisefs::apps::parse_desktop(
                    "[Desktop Entry]\nType=Application\nName=Network Connections\nExec=reprise-network\n",
                    p.id,
                )
                .unwrap()
            })
        })
        .collect();
    a.show_links();
    let r = a.sheet_event(propsheet::Event::Link(net));
    assert_eq!(r.spawn, [vec![OsString::from("reprise-network")]]);
    assert_eq!(r.spawn_app.as_deref(), Some("reprise-network.desktop"));
    assert_eq!(
        registry::open(&a.sheet),
        Page::Appearance,
        "Settings stays on its page"
    );
    let list = a.sheet.list_mut().unwrap();
    assert!(list.links[net].enabled);
    assert_eq!(
        (
            list.links[net].label.as_str(),
            list.links[net].name.as_str()
        ),
        ("Connections", "Network Connections"),
        "a screen reader hears the program's whole name"
    );
    assert!(!list.links[bt].enabled, "not installed: greyed");
    assert!(a.sheet_event(propsheet::Event::Link(bt)).spawn.is_empty());
}

#[test]
fn audio_devices_greys_without_a_shown_sounds_entry() {
    let enabled = |a: &SettingsApp| match a.sheet.pages[Page::Sound.index()]
        .item(pages::AUDIO_DEVICES)
        .map(|it| &it.control)
    {
        Some(form::Control::Button { enabled, .. }) => *enabled,
        other => panic!("Audio Devices... is a button: {other:?}"),
    };
    let mut a = app("audio-devices");
    assert!(a.app_dirs.is_empty());
    assert!(!enabled(&a), "greyed with no entry");
    let dir = tmp("audio-devices-apps");
    std::fs::write(
        dir.join("reprise-sound.desktop"),
        "[Desktop Entry]\nType=Application\nName=Sounds\nExec=reprise-sound\n",
    )
    .unwrap();
    a.app_dirs = vec![dir];
    a.show_links();
    assert!(enabled(&a), "lit once the entry is shown");
}

#[test]
fn open_program_starts_a_shown_entry_and_nothing_for_a_missing_one() {
    use std::ffi::OsString;
    let mut a = app("open-program");
    let dir = tmp("open-program-apps");
    std::fs::write(
        dir.join("reprise-sound.desktop"),
        "[Desktop Entry]\nType=Application\nName=Sounds\nExec=reprise-sound\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("reprise-accounts.desktop"),
        "[Desktop Entry]\nType=Application\nName=User Accounts\nExec=reprise-accounts\n",
    )
    .unwrap();
    a.app_dirs = vec![dir];
    let r = a.open_program("reprise-sound.desktop", &[]);
    assert_eq!(r.spawn, [vec![OsString::from("reprise-sound")]]);
    assert_eq!(r.spawn_app.as_deref(), Some("reprise-sound.desktop"));
    let r = a.open_program("reprise-accounts.desktop", &["--remote-password"]);
    assert_eq!(
        r.spawn,
        [vec![
            OsString::from("reprise-accounts"),
            OsString::from("--remote-password")
        ]]
    );
    assert!(
        a.open_program("reprise-updates.desktop", &[])
            .spawn
            .is_empty()
    );
}
