use super::*;

#[test]
fn the_command_line_takes_a_place() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let here = Place::Folder(dir.path().to_path_buf());
    let args = |a: &[&std::ffi::OsStr]| parse_args(a.iter().map(|s| s.to_os_string())).place;
    assert_eq!(args(&[]), Place::Home);
    assert_eq!(args(&[dir.path().as_os_str()]), here);
    assert_eq!(args(&["Computer".as_ref()]), Place::Computer);
    assert_eq!(args(&["Recycle Bin".as_ref()]), Place::RecycleBin);
    assert_eq!(args(&["My Computer".as_ref()]), Place::Home);
    assert_eq!(args(&["network".as_ref()]), Place::network());
    let start = parse_args(["SMB://nas/public".into()]);
    assert_eq!(
        start.place,
        Place::Computer,
        "a server address opens Computer"
    );
    assert_eq!(start.connect.as_deref(), Some("smb://nas/public"));
}

#[test]
fn select_opens_the_folder_on_the_entry() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for i in 0..60 {
        std::fs::write(dir.path().join(format!("f{i:02}.txt")), "x").expect("write");
    }
    let last = dir.path().join("f59.txt");
    let args = |a: &[&std::ffi::OsStr]| parse_args(a.iter().map(|s| s.to_os_string()));
    let start = args(&["--select".as_ref(), last.as_os_str()]);
    assert_eq!(start.place, Place::Folder(dir.path().to_path_buf()));
    assert_eq!(start.select, std::slice::from_ref(&last));
    assert_eq!(
        args(&["--select".as_ref(), "f59.txt".as_ref()]),
        Start {
            place: Place::Home,
            dir: None,
            select: Vec::new(),
            alone: None,
            connect: None,
            search: false,
        }
    );

    let mut ctx = test_ctx();
    let mut app = Lazy {
        start,
        inner: None,
        units: TextUnits::ONE,
    };
    let _ = app.event(MAIN, Event::Resized, &mut ctx);
    let ex = app.inner.as_mut().expect("the window");
    let i = ex.index_of(&last).expect("listed");
    assert_eq!(ex.sv.selection.set.iter().copied().collect::<Vec<_>>(), [i]);
    assert_eq!(ex.sv.selection.focus, Some(i));
    let lay = ex.layout(&ctx);
    let sl = ex.view_layout(lay.view, &ctx);
    let cell = sl.cells[i];
    assert!(
        cell.y >= sl.body.y && cell.bottom() <= sl.body.bottom(),
        "{cell:?} inside {:?}",
        sl.body
    );
}

/// A command-line folder whose check has not answered opens the window there at once; a late
/// "missing" sends it Home with nothing to go back to.
#[test]
fn a_start_folder_answering_late_opens_the_window_at_once() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let gone = dir.path().join("gone");
    let mut ctx = test_ctx();
    let start = parse_args([gone.clone().into_os_string()]);
    assert_eq!(start.dir.as_deref(), Some(gone.as_path()));
    let (answer, gate) = std::sync::mpsc::channel::<()>();

    let opened = std::time::Instant::now();
    let mut ex = Explorer::open(
        start,
        move |d| {
            gate.recv_timeout(std::time::Duration::from_secs(20)).ok();
            d.is_dir()
        },
        test_explorer,
        &mut ctx,
    );
    assert!(
        opened.elapsed() < std::time::Duration::from_secs(10),
        "the window opens before the folder answers"
    );
    assert_eq!(ex.place, Place::Folder(gone));

    answer.send(()).expect("send");
    wake_until(&mut ex, &mut ctx, |ex| ex.place == Place::Home);
    assert_eq!(ex.place, Place::Home);
    assert!(!ex.history.can_back());
}

#[test]
fn a_start_folder_that_answers_opens_there_and_selects_its_entry() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "x").expect("write");
    let mut ctx = test_ctx();
    let open = |a: &[&std::ffi::OsStr], ctx: &mut Ctx| {
        let start = parse_args(a.iter().map(|s| s.to_os_string()));
        Explorer::open(start, |d| d.is_dir(), test_explorer, ctx)
    };

    let ex = open(&["--select".as_ref(), file.as_os_str()], &mut ctx);
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
    let i = ex.index_of(&file).expect("listed");
    assert_eq!(ex.sv.selection.set.iter().copied().collect::<Vec<_>>(), [i]);

    let missing = open(&[dir.path().join("gone").as_os_str()], &mut ctx);
    assert_eq!(missing.place, Place::Home);
    assert!(missing.checking.is_none());
}

#[test]
fn search_opens_its_folder_with_the_keyboard_in_the_search_field() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let start = parse_args([
        std::ffi::OsString::from("--search"),
        dir.path().as_os_str().to_os_string(),
    ]);
    let mut ctx = test_ctx();
    let ex = Explorer::open(start, |d| d.is_dir(), test_explorer, &mut ctx);
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
    assert_eq!(ex.focus, Focus::Search);

    let plain = parse_args([dir.path().as_os_str().to_os_string()]);
    let ex = Explorer::open(plain, |d| d.is_dir(), test_explorer, &mut ctx);
    assert_ne!(ex.focus, Focus::Search, "without --search the view has it");
}

#[test]
fn select_takes_every_entry_in_the_first_ones_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    for f in [&a, &b] {
        std::fs::write(f, "x").expect("write");
    }
    let elsewhere = std::env::temp_dir().join("c.txt");
    let args = [
        std::ffi::OsString::from("--select"),
        b.clone().into(),
        a.clone().into(),
        elsewhere.into(),
    ];
    let start = parse_args(args);
    assert_eq!(start.select, [b.clone(), a.clone()]);

    let mut ctx = test_ctx();
    let ex = Explorer::open(start, |d| d.is_dir(), test_explorer, &mut ctx);
    let (ia, ib) = (ex.index_of(&a).expect("a"), ex.index_of(&b).expect("b"));
    let mut picked: Vec<usize> = ex.sv.selection.set.iter().copied().collect();
    picked.sort();
    assert_eq!(picked, [ia.min(ib), ia.max(ib)]);
    assert_eq!(
        ex.sv.selection.focus,
        Some(ib),
        "the first named entry has the focus"
    );
}

#[test]
fn properties_shows_the_sheet_alone_and_ends_with_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "x").expect("write");
    let start = parse_args([
        std::ffi::OsString::from("--properties"),
        file.clone().into(),
    ]);
    assert_eq!(start.alone, Some(Alone::Properties(vec![file.clone()])));
    let mut app = Lazy {
        start,
        inner: None,
        units: TextUnits::ONE,
    };
    assert!(repriseapp::Extended::hidden(&app), "no folder window");

    let mut ctx = test_ctx();
    let _ = app.event(MAIN, Event::Wake, &mut ctx);
    assert!(
        app.children().iter().any(|c| c.id == PROPS),
        "the sheet is up"
    );
    let reply = app.event(PROPS, Event::CloseRequest, &mut ctx);
    assert!(reply.quit, "closing the sheet ends the process");
}

#[test]
fn properties_of_several_paths_shows_the_combined_sheet() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    for f in [&a, &b] {
        std::fs::write(f, "x").expect("write");
    }
    let gone = dir.path().join("gone.txt");
    let sheet_of = |paths: &[&PathBuf]| {
        let args = std::iter::once("--properties".into()).chain(paths.iter().map(|p| p.into()));
        let mut app = Lazy {
            start: parse_args(args.collect::<Vec<std::ffi::OsString>>()),
            inner: None,
            units: TextUnits::ONE,
        };
        let mut ctx = test_ctx();
        let _ = app.event(MAIN, Event::Wake, &mut ctx);
        let props = app.inner.and_then(|ex| ex.props).expect("the sheet is up");
        (props.set.map(|s| s.paths), props.info.path)
    };
    assert_eq!(sheet_of(&[&b, &a]).0, Some(vec![b.clone(), a.clone()]));
    assert_eq!(
        sheet_of(&[&a, &gone]),
        (None, a.clone()),
        "one path that reads gets its own sheet"
    );
}

#[test]
fn bin_properties_shows_recycle_bin_properties_alone_and_ends_with_it() {
    let start = parse_args([std::ffi::OsString::from("--bin-properties")]);
    assert_eq!(start.alone, Some(Alone::BinProperties));
    assert_eq!(start.place, Place::RecycleBin);
    let mut app = Lazy {
        start,
        inner: None,
        units: TextUnits::ONE,
    };
    assert!(repriseapp::Extended::hidden(&app), "no folder window");

    let mut ctx = test_ctx();
    let _ = app.event(MAIN, Event::Wake, &mut ctx);
    let ex = app.inner.as_mut().expect("the explorer");
    wake_until(ex, &mut ctx, |ex| ex.bin_props.is_some());
    let ids: Vec<WinId> = app.children().iter().map(|c| c.id).collect();
    assert_eq!(ids, [crate::bin_props::BIN_PROPS], "the sheet alone");
    let reply = app.event(crate::bin_props::BIN_PROPS, Event::CloseRequest, &mut ctx);
    assert!(reply.quit, "closing the sheet ends the process");
}

#[test]
fn open_with_shows_the_chooser_alone_and_ends_with_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "x").expect("write");
    let start = parse_args([std::ffi::OsString::from("--open-with"), file.clone().into()]);
    assert_eq!(start.alone, Some(Alone::OpenWith(file.clone())));
    let mut app = Lazy {
        start,
        inner: None,
        units: TextUnits::ONE,
    };
    assert!(repriseapp::Extended::hidden(&app), "no folder window");

    let mut ctx = test_ctx();
    let _ = app.event(MAIN, Event::Wake, &mut ctx);
    assert!(
        app.children()
            .iter()
            .any(|c| c.id == crate::dialogs::DIALOG),
        "the chooser is up"
    );
    let reply = app.event(crate::dialogs::DIALOG, Event::CloseRequest, &mut ctx);
    assert!(reply.quit, "closing the chooser ends the process");
}

#[test]
fn a_start_menu_entrys_file_is_named_and_drawn_as_its_entry() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let apps_dir = dir.path().join("applications");
    std::fs::create_dir_all(apps_dir.join("org")).expect("mkdir");
    let calc = apps_dir.join("org/calc.desktop");
    std::fs::write(&calc, "[Desktop Entry]\nName=Calculator\nExec=calc\n").expect("write");
    let entry = |id: &str, name: &str| reprisefs::apps::AppEntry {
        name: name.into(),
        exec_line: "x".into(),
        terminal: false,
        id: id.parse().unwrap(),
        icon: Some("accessories-calculator".into()),
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: Vec::new(),
        shown: true,
        path: None,
    };
    let apps = [
        entry("calc.desktop", "Other"),
        entry("org-calc.desktop", "Calculator"),
    ];
    let dirs = [apps_dir.clone()];
    let found = crate::props_host::menu_entry_in(&calc, &apps, &dirs).map(|e| e.name.as_str());
    assert_eq!(found, Some("Calculator"));
    let elsewhere = dir.path().join("calc.desktop");
    std::fs::write(&elsewhere, "x").expect("write");
    assert!(crate::props_host::menu_entry_in(&elsewhere, &apps, &dirs).is_none());
}
