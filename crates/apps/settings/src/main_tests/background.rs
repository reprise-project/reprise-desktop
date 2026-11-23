use super::*;

fn pick_row(a: &mut SettingsApp, i: usize) {
    let page = &mut a.sheet.pages[Page::Background.index()];
    if let Some(form::Control::ListBox { list, .. }) = page
        .item_mut(backgroundpage::WALLPAPER)
        .map(|i| &mut i.control)
    {
        list.select(i, 11);
    }
    a.pick_wallpaper();
}

/// Under Blue the list starts on Blue's own built-in; a built-in
/// picked is written as `reprise:design/variant`, and "(None)" as an
/// empty image, which stands for none rather than Blue's default.
#[test]
fn the_background_list_picks_and_writes_a_builtin() {
    let dir = tmp("builtin");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[appearance]\ninterface = \"blue\"\n").unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let at = backgroundpage::selected(&a.sheet.pages[Page::Background.index()]);
    assert_eq!(
        at.map(backgroundpage::row),
        Some(backgroundpage::Row::Builtin(
            backgroundpage::builtin("reprise:tarn/blue").unwrap()
        ))
    );
    let dusk = backgroundpage::builtins()
        .iter()
        .position(|b| b.to_string() == "ridges/dusk")
        .unwrap();
    assert!(
        a.wallpapers.is_empty() && a.slide_folders().is_empty(),
        "a built-in is neither a picture file nor a slideshow folder"
    );
    pick_row(&mut a, dusk + 1);
    assert_eq!(a.want.wallpaper, "reprise:ridges/dusk");
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("image = \"reprise:ridges/dusk\""), "{text}");
    pick_row(&mut a, 0);
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("image = \"\""), "{text}");
}

/// Use for picks desktop 2: the list shows its picture, a pick is written to its entry alone,
/// and picking the picture for all desktops again drops the entry.
#[test]
fn a_picture_picked_for_one_desktop_is_written_to_its_entry() {
    let dir = tmp("per-desktop");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[workspaces]\ncount = 3\n[wallpaper]\nimage = \"reprise:tarn/blue\"\n",
    )
    .unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let bg = Page::Background.index();
    let use_for = |a: &mut SettingsApp, i: usize| {
        a.sheet.pages[bg].set_combo(backgroundpage::USE_FOR, i);
        a.sheet_event(propsheet::Event::Page(
            bg,
            form::Event::Changed(backgroundpage::USE_FOR),
        ));
    };
    let row_of = |uri: &str| {
        let b = backgroundpage::builtin(uri).unwrap();
        backgroundpage::builtins()
            .iter()
            .position(|x| *x == b)
            .unwrap()
            + 1
    };
    use_for(&mut a, 2);
    assert_eq!(
        backgroundpage::selected(&a.sheet.pages[bg]),
        Some(row_of("reprise:tarn/blue")),
        "a desktop without its own shows the picture for all"
    );
    assert!(
        matches!(
            a.sheet.pages[bg].item(backgroundpage::MODE).map(|i| &i.control),
            Some(form::Control::Combo(c)) if !c.enabled
        ),
        "no slideshow per desktop"
    );
    pick_row(&mut a, row_of("reprise:ridges/dusk"));
    assert_eq!(a.want.wallpaper, "reprise:tarn/blue");
    assert_eq!(a.want.wallpaper_desktops, ["", "reprise:ridges/dusk"]);
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains(r#"desktops = ["", "reprise:ridges/dusk"]"#),
        "{text}"
    );
    use_for(&mut a, 0);
    assert_eq!(
        backgroundpage::selected(&a.sheet.pages[bg]),
        Some(row_of("reprise:tarn/blue"))
    );
    use_for(&mut a, 2);
    pick_row(&mut a, row_of("reprise:tarn/blue"));
    assert!(a.want.wallpaper_desktops.is_empty());
}

#[test]
fn a_slideshow_folder_survives_a_switch_to_picture_and_its_first_picture_reaches_the_monitor() {
    let dir = tmp("slide-folder");
    let (walls, trips) = (dir.join("Walls"), dir.join("Trips"));
    std::fs::create_dir_all(&walls).unwrap();
    std::fs::create_dir_all(&trips).unwrap();
    for f in ["anim.gif", "coast10.jpg", "Coast9.jpg"] {
        std::fs::write(trips.join(f), b"x").unwrap();
    }
    let path = dir.join("config.toml");
    let wall = walls.join("dunes.jpg");
    std::fs::write(&path, format!("[wallpaper]\nimage = {:?}\n", wall)).unwrap();
    let mut a = SettingsApp::open(path, "us".into());
    let state = dir.join("settings.toml");
    a.state_path = Some(state.clone());
    let bg = Page::Background.index();
    let mode = |a: &mut SettingsApp, i: usize| {
        a.sheet.pages[bg].set_combo(backgroundpage::MODE, i);
        a.sheet_event(propsheet::Event::Page(
            bg,
            form::Event::Changed(backgroundpage::MODE),
        ));
    };
    let name = |p: &Path| p.to_string_lossy().into_owned();

    mode(&mut a, 1);
    assert_eq!(a.want.slideshow, name(&walls), "the wallpaper's folder");
    a.slide_folder(trips.clone());
    for _ in 0..500 {
        a.wake();
        if a.listing.is_none() && a.decoding.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        a.picture.as_ref().map(|p| p.path.clone()),
        Some(name(&trips.join("Coast9.jpg"))),
        "the desktop's first slide: no .gif, Coast9 before coast10"
    );

    mode(&mut a, 0);
    assert_eq!(a.want.slideshow, "");
    assert_eq!(winstate::slideshow(&state), name(&trips));
    mode(&mut a, 1);
    assert_eq!(a.want.slideshow, name(&trips), "the folder picked before");
}

#[test]
fn a_picture_added_to_the_slideshow_folder_becomes_the_monitors_first_slide() {
    let dir = tmp("slide-relist");
    let trips = dir.join("Trips");
    std::fs::create_dir_all(&trips).unwrap();
    std::fs::write(trips.join("coast.jpg"), b"x").unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, format!("[wallpaper]\nslideshow = {:?}\n", trips)).unwrap();
    let mut a = SettingsApp::open(path, "us".into());
    let settle = |a: &mut SettingsApp| {
        for _ in 0..500 {
            a.wake();
            if a.listing.is_none() && a.decoding.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        a.picture.as_ref().map(|p| p.path.clone())
    };
    let name = |p: &Path| Some(p.to_string_lossy().into_owned());
    a.want_preview();
    assert_eq!(settle(&mut a), name(&trips.join("coast.jpg")));

    std::fs::write(trips.join("bay.jpg"), b"x").unwrap();
    a.slides_touched.store(true, Ordering::Release);
    assert_eq!(
        settle(&mut a),
        name(&trips.join("bay.jpg")),
        "bay sorts first"
    );
}

/// While the applied slideshow runs, the monitor shows the picture the desktop shows now and
/// follows it; a folder picked shows its first picture until the compositor picks from it.
#[test]
fn the_monitor_follows_the_running_slideshows_picture() {
    let dir = tmp("slide-live");
    let (trips, walks) = (dir.join("Trips"), dir.join("Walks"));
    for (folder, files) in [
        (&trips, ["coast.jpg", "lake.jpg"]),
        (&walks, ["fell.jpg", "moor.jpg"]),
    ] {
        std::fs::create_dir_all(folder).unwrap();
        for f in files {
            std::fs::write(folder.join(f), b"x").unwrap();
        }
    }
    let path = dir.join("config.toml");
    std::fs::write(&path, format!("[wallpaper]\nslideshow = {trips:?}\n")).unwrap();
    let mut a = SettingsApp::open(path, "us".into());
    let state = dir.join("state.toml");
    a.compositor_state = Some(state.clone());
    let slide = |a: &mut SettingsApp, p: &Path| {
        std::fs::write(&state, format!("slideshow_picture = {p:?}\n")).unwrap();
        a.slide_moved.store(true, Ordering::Release);
    };
    let settle = |a: &mut SettingsApp| {
        for _ in 0..500 {
            a.wake();
            if a.listing.is_none() && a.decoding.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        a.picture.as_ref().map(|p| p.path.clone())
    };
    let name = |p: &Path| Some(p.to_string_lossy().into_owned());
    a.want_preview();
    assert_eq!(settle(&mut a), name(&trips.join("coast.jpg")));
    slide(&mut a, &trips.join("lake.jpg"));
    assert_eq!(
        settle(&mut a),
        name(&trips.join("lake.jpg")),
        "the next picture"
    );

    a.slide_folder(walks.clone());
    assert_eq!(
        settle(&mut a),
        name(&walks.join("fell.jpg")),
        "a folder not yet applied"
    );
    assert!(a.apply());
    assert_eq!(settle(&mut a), name(&walks.join("fell.jpg")));
    slide(&mut a, &walks.join("moor.jpg"));
    assert_eq!(settle(&mut a), name(&walks.join("moor.jpg")));
}

/// Two monitors listed: Use for offers them by their Display page names, a picture and a
/// position picked for the second are written to its `[[outputs]]` entry, the monitor shows its
/// own picture, and a pick for all monitors takes the entry's wallpaper keys out again.
#[test]
fn a_picture_picked_for_one_monitor_is_written_to_its_outputs_entry() {
    let dir = tmp("per-monitor");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[wallpaper]\nimage = \"reprise:tarn/blue\"\n\n[[outputs]]\nname = \"HDMI-A-1\"\nscale = 2\n",
    )
    .unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let mut ctx = test_ctx();
    let heads = vec![
        displaypage::test_head("DP-1"),
        displaypage::test_head("HDMI-A-1"),
    ];
    a.event(MAIN, Event::Heads(heads), &mut ctx);
    let bg = Page::Background.index();
    let rows = match a.sheet.pages[bg]
        .item(backgroundpage::USE_FOR)
        .map(|i| &i.control)
    {
        Some(form::Control::Combo(c)) => c.items.iter().map(|i| i.text.clone()).collect::<Vec<_>>(),
        _ => panic!("Use for is a combo"),
    };
    assert_eq!(
        rows[rows.len() - 2..],
        [
            "1. Dell Inc. DELL U2412M on DP-1",
            "2. Dell Inc. DELL U2412M on HDMI-A-1"
        ]
    );
    let use_for = |a: &mut SettingsApp, i: usize| {
        a.sheet.pages[bg].set_combo(backgroundpage::USE_FOR, i);
        a.sheet_event(propsheet::Event::Page(
            bg,
            form::Event::Changed(backgroundpage::USE_FOR),
        ));
    };
    let row_of = |uri: &str| {
        let b = backgroundpage::builtin(uri).unwrap();
        backgroundpage::builtins()
            .iter()
            .position(|x| *x == b)
            .unwrap()
            + 1
    };
    use_for(&mut a, rows.len() - 1);
    pick_row(&mut a, row_of("reprise:fold/sand"));
    a.sheet.pages[bg].set_combo(backgroundpage::POSITION, 4);
    a.sheet_event(propsheet::Event::Page(
        bg,
        form::Event::Changed(backgroundpage::POSITION),
    ));
    assert_eq!(
        a.want.wallpaper, "reprise:tarn/blue",
        "the picture for all stays"
    );
    assert_eq!(a.want.wallpaper_position, "fill");
    assert_eq!(
        a.bg().wallpaper,
        "reprise:fold/sand",
        "the monitor shows its own"
    );
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.ends_with(
            "name = \"HDMI-A-1\"\nscale = 2\nwallpaper = \"reprise:fold/sand\"\nwallpaper_position = \"center\"\n"
        ),
        "{text}"
    );
    let c = repriseconf::config::Config::parse(&text).unwrap();
    assert_eq!(
        c.wallpaper.output_image("HDMI-A-1"),
        Some(std::path::Path::new("reprise:fold/sand"))
    );
    use_for(&mut a, 0);
    pick_row(&mut a, row_of("reprise:ridges/dusk"));
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("fold/sand"),
        "all monitors take the pick: {text}"
    );
    assert!(text.contains("wallpaper_position = \"center\""), "{text}");
    assert!(text.contains("image = \"reprise:ridges/dusk\""), "{text}");
}

/// Under a slideshow, a desktop without a picture of its own shows the slideshow: the monitor
/// shows its first picture, no picture row is picked, and the picture for all picked for the
/// desktop becomes its own.
#[test]
fn a_desktop_without_its_own_picture_shows_the_slideshow() {
    let dir = tmp("desktop-slides");
    let (walls, trips) = (dir.join("Walls"), dir.join("Trips"));
    std::fs::create_dir_all(&walls).unwrap();
    std::fs::create_dir_all(&trips).unwrap();
    std::fs::write(walls.join("dunes.jpg"), b"x").unwrap();
    std::fs::write(trips.join("coast.jpg"), b"x").unwrap();
    let path = dir.join("config.toml");
    let wall = walls.join("dunes.jpg");
    std::fs::write(
        &path,
        format!("[workspaces]\ncount = 2\n[wallpaper]\nimage = {wall:?}\nslideshow = {trips:?}\n"),
    )
    .unwrap();
    let mut a = SettingsApp::open(path, "us".into());
    a.wallpapers = vec![wall.clone()];
    let bg = Page::Background.index();
    let settle = |a: &mut SettingsApp| {
        for _ in 0..500 {
            a.wake();
            if a.listing.is_none() && a.decoding.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        a.picture.as_ref().map(|p| p.path.clone())
    };
    let name = |p: &Path| Some(p.to_string_lossy().into_owned());
    a.want_preview();
    assert_eq!(settle(&mut a), name(&trips.join("coast.jpg")));

    a.sheet.pages[bg].set_combo(backgroundpage::USE_FOR, 2);
    a.sheet_event(propsheet::Event::Page(
        bg,
        form::Event::Changed(backgroundpage::USE_FOR),
    ));
    assert_eq!(
        settle(&mut a),
        name(&trips.join("coast.jpg")),
        "desktop 2 shows the slideshow"
    );
    let rows = backgroundpage::builtins().len() + 2;
    assert_eq!(backgroundpage::selected(&a.sheet.pages[bg]), Some(0));
    pick_row(&mut a, rows - 1);
    assert_eq!(a.want.wallpaper_desktops, ["", &*name(&wall).unwrap()]);
    a.want_preview();
    assert_eq!(settle(&mut a), name(&wall));
}

/// With a slideshow running, a folder picked for all monitors takes the connected monitors' own
/// pictures away, as a picture does; a monitor not connected keeps its own.
#[test]
fn a_slideshow_folder_picked_for_all_monitors_takes_their_own_pictures_away() {
    let dir = tmp("per-monitor-slides");
    let slides = dir.join("Slides");
    std::fs::create_dir_all(&slides).unwrap();
    let path = dir.join("config.toml");
    let own = |name: &str| {
        format!("[[outputs]]\nname = \"{name}\"\nwallpaper = \"reprise:fold/sand\"\n\n")
    };
    std::fs::write(&path, own("DP-1") + &own("HDMI-A-1") + &own("DP-9")).unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let mut ctx = test_ctx();
    let heads = vec![
        displaypage::test_head("DP-1"),
        displaypage::test_head("HDMI-A-1"),
    ];
    a.event(MAIN, Event::Heads(heads), &mut ctx);
    a.slide_folder(slides);
    let kept: Vec<&str> = (a.want.output_wallpapers.iter())
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(kept, ["DP-9"]);
}

/// Restore Defaults on Background lists the connected monitors' own wallpaper keys and takes
/// them out of their `[[outputs]]` entries, dropping an entry left with its name alone; a monitor
/// not connected keeps its own.
#[test]
fn restore_defaults_takes_the_connected_monitors_own_pictures_away() {
    let dir = tmp("restore-monitors");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[[outputs]]\nname = \"DP-1\"\nscale = 2\nwallpaper = \"reprise:fold/sand\"\n\
         wallpaper_position = \"center\"\n\n\
         [[outputs]]\nname = \"HDMI-A-1\"\nwallpaper = \"reprise:fold/sand\"\n\n\
         [[outputs]]\nname = \"DP-9\"\nwallpaper = \"reprise:fold/sand\"\n",
    )
    .unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let mut ctx = test_ctx();
    let heads = vec![
        displaypage::test_head("DP-1"),
        displaypage::test_head("HDMI-A-1"),
    ];
    a.event(MAIN, Event::Heads(heads), &mut ctx);
    let bg = Page::Background;
    let lit = |a: &SettingsApp| match a.sheet.pages[bg.index()].item(propsheet::RESTORE) {
        Some(it) => matches!(it.control, form::Control::Button { enabled: true, .. }),
        None => panic!("Background has Restore Defaults"),
    };
    assert!(lit(&a), "the monitors' own keys are off their defaults");
    assert_eq!(
        apply::restore_question(bg, &a.restorable(bg)),
        "Restore the defaults on the Background page?\n\
         These settings are taken out of config.toml:\n    outputs.wallpaper (DP-1)\n    \
         outputs.wallpaper_position (DP-1)\n    outputs.wallpaper (HDMI-A-1)"
    );
    a.restore_defaults(bg);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[[outputs]]\nname = \"DP-1\"\nscale = 2\n\n\
         [[outputs]]\nname = \"DP-9\"\nwallpaper = \"reprise:fold/sand\"\n"
    );
    let kept: Vec<&str> = (a.want.output_wallpapers.iter())
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(kept, ["DP-9"]);
    assert_eq!(a.loaded, a.want, "nothing is left pending");
    assert!(!lit(&a), "greyed once the connected monitors have none");
}

/// Restore Defaults on Background lists and takes out a key the file sets at its default (Glass's
/// position is stretch), so the page's keys leave the file whatever their values.
#[test]
fn restore_defaults_takes_out_the_pages_keys_the_file_sets_at_their_defaults() {
    let dir = tmp("restore-position");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[appearance]\ninterface = \"glass\"\n\n[wallpaper]\nposition = \"stretch\"\n",
    )
    .unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let bg = Page::Background;
    let lit = |a: &SettingsApp| match a.sheet.pages[bg.index()].item(propsheet::RESTORE) {
        Some(it) => matches!(it.control, form::Control::Button { enabled: true, .. }),
        None => panic!("Background has Restore Defaults"),
    };
    assert_eq!(a.loaded.wallpaper_position, "stretch", "Glass's default");
    assert!(lit(&a), "the file sets a Background key");
    assert_eq!(
        apply::restore_question(bg, &a.restorable(bg)),
        "Restore the defaults on the Background page?\n\
         These settings are taken out of config.toml:\n    wallpaper.position"
    );
    a.restore_defaults(bg);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "[appearance]\ninterface = \"glass\"\n");
    assert_eq!(
        model::load(&text),
        a.loaded,
        "the file reads back as the sheet holds it"
    );
    assert_eq!(a.loaded, a.want, "nothing is left pending");
    assert!(
        !lit(&a),
        "greyed once the file sets none of the page's keys"
    );
}

/// A key written or taken out at its default reads the same, but Restore Defaults follows it.
#[test]
fn restore_defaults_follows_a_key_another_program_sets_at_its_default() {
    let dir = tmp("restore-external");
    let path = dir.join("config.toml");
    let glass = "[appearance]\ninterface = \"glass\"\n";
    std::fs::write(&path, glass).unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let bg = Page::Background;
    let lit = |a: &SettingsApp| match a.sheet.pages[bg.index()].item(propsheet::RESTORE) {
        Some(it) => matches!(it.control, form::Control::Button { enabled: true, .. }),
        None => panic!("Background has Restore Defaults"),
    };
    assert!(!lit(&a), "the file sets no Background key");
    std::fs::write(
        &path,
        format!("{glass}\n[wallpaper]\nposition = \"stretch\"\n"),
    )
    .unwrap();
    assert!(a.config_changed(), "the file sets one more key");
    assert!(lit(&a), "lit once the file sets wallpaper.position");
    std::fs::write(&path, glass).unwrap();
    assert!(a.config_changed(), "the file sets one key fewer");
    assert!(!lit(&a), "greyed once the file drops it");
}
