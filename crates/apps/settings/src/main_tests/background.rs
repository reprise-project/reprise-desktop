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
