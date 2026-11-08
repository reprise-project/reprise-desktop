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

#[test]
fn location_answers_list_the_prompts_then_the_portals() {
    let agent = std::collections::BTreeMap::from([("org.example.Weather".to_string(), false)]);
    let portal = vec![("org.gnome.Maps".to_string(), true)];
    let got: Vec<(String, bool, bool)> = location_answers(agent, portal)
        .into_iter()
        .map(|a| (a.id, a.allow, a.portal))
        .collect();
    assert_eq!(
        got,
        [
            ("org.example.Weather".to_string(), false, false),
            ("org.gnome.Maps".to_string(), true, true),
        ]
    );
}

/// Show the Privacy page and wait for the location read it starts.
fn read_locations_on_privacy(app: &mut SettingsApp) {
    app.sheet.tabs.open = Page::Privacy.index();
    app.ensure_locations();
    let found = app
        .locating
        .take()
        .expect("a read under way")
        .recv()
        .unwrap();
    app.show_locations(found);
}

#[test]
fn the_location_list_is_read_off_the_loop_once_the_privacy_page_shows() {
    let dir = tmp("location-lazy");
    let file = dir.join("location.toml");
    std::fs::write(&file, "[apps]\n\"org.example.NoSuchMaps\" = true\n").unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"), "us".into());
    app.location_file = file;
    assert!(app.locations.rows.is_empty(), "opening reads no answers");
    app.ensure_locations();
    assert!(app.locating.is_none(), "no read while another page shows");

    app.sheet.tabs.open = Page::Privacy.index();
    app.ensure_locations();
    assert!(app.locating.is_some(), "the Privacy page starts the read");
    let cells: Vec<Vec<String>> = app.locations.rows.iter().map(|r| r.cells.clone()).collect();
    assert_eq!(cells, [vec!["Loading...".to_string(), String::new()]]);
    app.locations.selection.set_only(Some(0));
    app.sync_apply();
    assert!(!forget_enabled(&app), "Forget stays greyed while loading");

    let found = app.locating.take().unwrap().recv().unwrap();
    app.show_locations(found);
    assert_eq!(app.locations.rows.len(), 1);
    assert_eq!(
        app.locations.rows[0].cells,
        ["org.example.NoSuchMaps", "Allowed"]
    );
    assert_eq!(
        app.locations.selection.focus, None,
        "the loading row's pick is dropped"
    );
    app.ensure_locations();
    assert!(app.locating.is_none(), "the list is read once");
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
    read_locations_on_privacy(&mut app);
    let cells: Vec<Vec<String>> = app.locations.rows.iter().map(|r| r.cells.clone()).collect();
    assert_eq!(
        cells,
        [
            ["org.example.NoSuchMaps", "Allowed"],
            ["org.example.NoSuchWeather", "Denied"],
        ]
        .map(|r| r.map(String::from).to_vec())
    );
    assert!(!forget_enabled(&app), "Forget greys with no row picked");

    app.locations.selection.set_only(Some(0));
    app.sync_apply();
    assert!(forget_enabled(&app));
    app.forget_location();
    assert_eq!(
        repriseconf::location::load_from(&file),
        std::collections::BTreeMap::from([("org.example.NoSuchWeather".to_string(), false)])
    );
    assert_eq!(app.locations.rows.len(), 1);
    assert_eq!(app.locations.rows[0].cells[0], "org.example.NoSuchWeather");
    assert!(!app.dirty(), "Forget leaves nothing for Apply");
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
