use super::*;

fn forget_enabled(app: &SettingsApp) -> bool {
    match &app.sheet.pages[Page::Privacy.index()]
        .item(pages::FORGET)
        .unwrap()
        .control
    {
        form::Control::Button { enabled, .. } => *enabled,
        _ => panic!("Forget is a button"),
    }
}

fn stored(apps: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    apps.iter()
        .map(|(app, v)| (app.to_string(), v.iter().map(|s| s.to_string()).collect()))
        .collect()
}

#[test]
fn permission_answers_list_the_prompts_then_each_tables_and_skip_the_ones_still_to_ask() {
    let agent = std::collections::BTreeMap::from([("org.example.Weather".to_string(), false)]);
    let portal = vec![
        (
            Kind::Location,
            stored(&[
                ("org.example.City", &["CITY", "1"]),
                ("org.example.Empty", &[]),
                ("org.gnome.Maps", &["EXACT", "1234"]),
                ("org.gnome.Weather", &["NONE", "0"]),
            ]),
        ),
        (
            Kind::Background,
            stored(&[
                ("org.example.Asked", &["ask"]),
                ("org.example.Sync", &["yes"]),
                ("org.example.Idle", &["no"]),
            ]),
        ),
        (Kind::Screenshot, stored(&[("org.example.Shot", &["yes"])])),
        (
            Kind::Wallpaper,
            stored(&[("org.example.Paint", &["no"]), ("org.example.Blank", &[])]),
        ),
        (
            Kind::InputCapture,
            stored(&[("org.example.Share", &["yes"])]),
        ),
    ];
    let got: Vec<(String, Kind, bool, bool)> = permission_answers(agent, portal)
        .into_iter()
        .map(|a| (a.app, a.kind, a.allow, a.portal))
        .collect();
    let want = [
        ("org.example.Weather", Kind::Location, false, false),
        ("org.example.City", Kind::Location, true, true),
        ("org.example.Empty", Kind::Location, false, true),
        ("org.gnome.Maps", Kind::Location, true, true),
        ("org.gnome.Weather", Kind::Location, false, true),
        ("org.example.Sync", Kind::Background, true, true),
        ("org.example.Idle", Kind::Background, false, true),
        ("org.example.Shot", Kind::Screenshot, true, true),
        ("org.example.Paint", Kind::Wallpaper, false, true),
        ("org.example.Share", Kind::InputCapture, true, true),
    ]
    .map(|(app, k, allow, portal)| (app.to_string(), k, allow, portal));
    assert_eq!(got, want);
}

#[test]
fn each_permission_is_kept_in_the_store_table_and_id_of_its_portal() {
    let tables: Vec<&str> = Kind::ALL.iter().map(|k| k.table()).collect();
    assert_eq!(
        tables,
        [
            "location",
            "background",
            "screenshot",
            "wallpaper",
            "input-capture"
        ]
    );
}

#[test]
fn a_row_names_its_program_by_desktop_entry_then_app_id() {
    use crate::permissions::program_name;
    assert_eq!(program_name("org.gnome.Maps", Some("Maps")), "Maps");
    assert_eq!(program_name("org.example.Gone", None), "org.example.Gone");
    assert_eq!(program_name("", None), "Unidentified programs");
}

/// Show the Privacy page and wait for the permission read it starts.
fn read_permissions_on_privacy(app: &mut SettingsApp) {
    app.sheet.tabs.open = Page::Privacy.index();
    app.ensure_permissions();
    let found = app
        .reading_permissions
        .take()
        .expect("a read under way")
        .recv()
        .unwrap();
    app.show_permissions(found);
}

fn cells(app: &SettingsApp) -> Vec<Vec<String>> {
    app.permissions
        .rows
        .iter()
        .map(|r| r.cells.clone())
        .collect()
}

#[test]
fn the_permission_list_is_read_off_the_loop_once_the_privacy_page_shows() {
    let dir = tmp("location-lazy");
    let file = dir.join("location.toml");
    std::fs::write(&file, "[apps]\n\"org.example.NoSuchMaps\" = true\n").unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"), "us".into());
    app.location_file = file;
    assert!(app.permissions.rows.is_empty(), "opening reads no answers");
    app.ensure_permissions();
    assert!(
        app.reading_permissions.is_none(),
        "no read while another page shows"
    );

    app.sheet.tabs.open = Page::Privacy.index();
    app.ensure_permissions();
    assert!(
        app.reading_permissions.is_some(),
        "the Privacy page starts the read"
    );
    assert_eq!(cells(&app), [["Loading...", "", ""].map(String::from)]);
    app.permissions.selection.set_only(Some(0));
    app.sync_apply();
    assert!(!forget_enabled(&app), "Forget stays greyed while loading");

    let found = app.reading_permissions.take().unwrap().recv().unwrap();
    app.show_permissions(found);
    assert_eq!(
        cells(&app),
        [["org.example.NoSuchMaps", "Location", "Allowed"].map(String::from)]
    );
    assert_eq!(
        app.permissions.selection.focus, None,
        "the loading row's pick is dropped"
    );
    app.ensure_permissions();
    assert!(app.reading_permissions.is_none(), "the list is read once");
}

#[test]
fn forget_drops_a_location_answer_from_the_file() {
    let dir = tmp("location");
    let file = dir.join("location.toml");
    std::fs::write(
        &file,
        "[apps]\n\"org.example.NoSuchMaps\" = true\n\"org.example.NoSuchWeather\" = false\n",
    )
    .unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"), "us".into());
    app.location_file = file.clone();
    read_permissions_on_privacy(&mut app);
    assert_eq!(
        cells(&app),
        [
            ["org.example.NoSuchMaps", "Location", "Allowed"],
            ["org.example.NoSuchWeather", "Location", "Denied"],
        ]
        .map(|r| r.map(String::from))
    );
    assert!(!forget_enabled(&app), "Forget greys with no row picked");

    app.permissions.selection.set_only(Some(0));
    app.sync_apply();
    assert!(forget_enabled(&app));
    app.forget_permission();
    assert_eq!(
        repriseconf::location::load_from(&file),
        std::collections::BTreeMap::from([("org.example.NoSuchWeather".to_string(), false)])
    );
    assert_eq!(app.permissions.rows.len(), 1);
    assert_eq!(
        app.permissions.rows[0].cells[0],
        "org.example.NoSuchWeather"
    );
    assert!(
        crate::permissions::FORGOTTEN.take().is_empty(),
        "the store is left alone"
    );
    assert!(!app.dirty(), "Forget leaves nothing for Apply");
}

#[test]
fn forget_deletes_the_picked_answer_from_its_table_in_the_store() {
    let mut app = app("forget-permission");
    app.open_page(Page::Privacy, None);
    app.permissions_due = false;
    let answer = |app: &str, kind| crate::permissions::PermissionAnswer {
        app: app.into(),
        kind,
        allow: true,
        portal: true,
    };
    app.show_permissions(crate::permissions::PermissionList {
        answers: vec![
            answer("org.example.Shot", Kind::Screenshot),
            answer("org.example.Share", Kind::InputCapture),
        ],
        rows: vec![
            ("Shot".into(), Kind::Screenshot.label(), true),
            ("Share".into(), Kind::InputCapture.label(), true),
        ],
        icons: vec![None, None],
    });
    app.permissions.selection.set_only(Some(1));
    app.sync_apply();
    app.forget_permission();
    assert_eq!(
        crate::permissions::FORGOTTEN.take(),
        [["input-capture", "input-capture", "org.example.Share"].map(String::from)]
    );
    assert_eq!(
        cells(&app),
        [["Shot", "Screenshots", "Allowed"].map(String::from)]
    );
    assert!(!forget_enabled(&app), "nothing is picked after a Forget");
}

#[test]
fn forget_saved_layouts_drops_the_file_and_greys_the_button() {
    let mut app = app("forget-layouts");
    let mut ctx = test_ctx();
    app.display.layouts = tmp("forget-layouts").join("displays.toml");
    std::fs::write(&app.display.layouts, "outputs_seen = \"0\"\n").unwrap();
    app.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1")]),
        &mut ctx,
    );
    let forget = |app: &SettingsApp| match app.sheet.pages[Page::Display.index()]
        .item(displaypage::FORGET)
        .map(|it| &it.control)
    {
        Some(form::Control::Button { enabled, .. }) => *enabled,
        other => panic!("Forget is a button: {other:?}"),
    };
    assert!(forget(&app), "lit while layouts are saved");
    app.forget_layouts();
    assert!(!app.display.layouts.exists());
    assert!(!forget(&app), "greyed once forgotten");
}

#[test]
fn the_clipboard_access_list_reads_the_installed_apps_and_ticks_into_trusted_apps() {
    let dir = tmp("clip-access");
    let install = dir.join("flatpak");
    let active = install.join("app/org.example.Pass/current/active");
    std::fs::create_dir_all(&active).unwrap();
    std::fs::write(
        active.join("metadata"),
        "[Application]\nname=org.example.Pass\n\n[Context]\nsockets=x11;wayland;\n",
    )
    .unwrap();
    let exports = install.join("exports/share/applications");
    std::fs::create_dir_all(&exports).unwrap();
    std::fs::write(
        exports.join("org.example.Pass.desktop"),
        "[Desktop Entry]\nType=Application\nName=Pass\nExec=pass\n",
    )
    .unwrap();
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        "[clipboard]\ntrusted_apps = [\"org.example.Gone\"]\n",
    )
    .unwrap();
    let mut app = SettingsApp::open(config, "us".into());
    app.flatpak_installs = vec![install];
    let list = |app: &SettingsApp| match app.sheet.pages[Page::Privacy.index()]
        .item(crate::clipaccess::LIST)
        .map(|i| &i.control)
    {
        Some(form::Control::ListBox { list, .. }) => (list.items.clone(), list.checks.clone()),
        _ => panic!("the list is on the Privacy page"),
    };
    app.ensure_clip_apps();
    assert!(
        app.clip_reading.is_none(),
        "no read while another page shows"
    );

    app.sheet.tabs.open = Page::Privacy.index();
    app.ensure_clip_apps();
    assert_eq!(list(&app).0, ["Loading..."]);
    let found = app
        .clip_reading
        .take()
        .expect("a read under way")
        .recv()
        .unwrap();
    app.show_clip_apps(found);
    app.ensure_clip_apps();
    assert_eq!(
        list(&app),
        (
            vec!["Pass".to_string(), "org.example.Gone".to_string()],
            vec![false, true]
        )
    );

    if let Some(form::Control::ListBox { list, .. }) = app.sheet.pages[Page::Privacy.index()]
        .item_mut(crate::clipaccess::LIST)
        .map(|i| &mut i.control)
    {
        list.toggle(0);
        list.toggle(1);
    }
    let changed = form::Event::Changed(crate::clipaccess::LIST);
    app.sheet_event(propsheet::Event::Page(Page::Privacy.index(), changed));
    assert_eq!(app.want.clipboard_trusted, ["org.example.Pass"]);
    assert!(app.dirty(), "Apply writes it");
    app.ensure_clip_apps();
    assert_eq!(
        list(&app),
        (
            vec!["Pass".to_string(), "org.example.Gone".to_string()],
            vec![true, false]
        ),
        "an unticked id that isn't installed stays until Apply"
    );
}
