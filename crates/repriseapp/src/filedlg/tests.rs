use super::*;
use reprisekit::controls::filedlg::CLIENT;

/// Folder `id` as [`Host::list`] lists it, read on the test's thread.
fn listed(
    folders: &mut FolderTable,
    id: FolderId,
    icons: &mut IconCache,
    folders_only: bool,
    keep: impl Fn(&Item) -> bool,
) -> Folder {
    let path = folders.path(id).expect("the id resolves").to_path_buf();
    let r = read(path, picks(folders_only, None, keep), icons.scale(), 16);
    listing(folders, id, r, icons).folder
}

fn host_in(t: &Path) -> Host {
    let mut h = Host::default();
    h.folder = Some(h.folders.id(t));
    h
}

fn tmp(tag: &str) -> PathBuf {
    let t = std::env::temp_dir().join(format!("repriseapp-host-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(t.join("sub")).unwrap();
    std::fs::write(t.join("a.txt"), "").unwrap();
    t
}

fn key(sym: u32, ctrl: bool, utf8: Option<&str>) -> Event {
    let mods = crate::Mods {
        ctrl,
        ..Default::default()
    };
    Event::Key(crate::KeyInput {
        sym,
        utf8: utf8.map(str::to_string),
        mods,
        edit: reprisekit::textedit::key_for(sym, ctrl, false, utf8),
    })
}

#[test]
fn the_next_dialog_opens_in_the_view_and_sort_the_last_one_left() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("state").join("filedialog.toml");
    let fields = || Fields {
        title: "Open".into(),
        accept_label: "Open".into(),
        filters: vec![],
        mode: DlgMode::Open,
        multiple: false,
    };
    let host = || Host {
        memory: Some(Memory {
            file: file.clone(),
            app: "test.app".into(),
        }),
        ..Host::default()
    };
    let theme = Theme::default();
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    let metrics = Metrics::default();

    let mut h = host();
    let mut dlg = h.open(t.path(), fields(), theme.text);
    assert_eq!(dlg.remembered(), Remembered::default(), "no file yet");
    let r = Rect::new(0, 0, CLIENT.0, CLIENT.1);
    dlg.click(r, Some(Hit::ToolBar(3)), Mods::default());
    dlg.click(r, Some(Hit::ViewsItem(4)), Mods::default());
    for _ in 0..2 {
        dlg.click(
            r,
            Some(Hit::Shell(shellview::Hit::Header(3))),
            Mods::default(),
        );
        dlg.release();
    }
    h.event(&mut dlg, &Event::Blink, &text, &theme, CLIENT, &metrics);
    let details = Remembered {
        mode: Mode::Details,
        sort: shellview::Field::Modified,
        ascending: false,
    };
    assert_eq!(dlg.remembered(), details);

    let dlg = host().open(t.path(), fields(), theme.text);
    assert_eq!(dlg.remembered(), details, "a Save As opened after");

    std::fs::write(&file, [0xff, 0xfe, 0]).unwrap();
    let dlg = host().open(t.path(), fields(), theme.text);
    assert_eq!(dlg.remembered(), Remembered::default(), "a bad file");
}

fn open_fields() -> Fields {
    Fields {
        title: "Open".into(),
        accept_label: "Open".into(),
        filters: vec![],
        mode: DlgMode::Open,
        multiple: false,
    }
}

fn host_of(file: &Path, app: &str) -> Host {
    Host {
        memory: Some(Memory {
            file: file.to_path_buf(),
            app: app.into(),
        }),
        ..Host::default()
    }
}

fn shown(h: &Host) -> PathBuf {
    let id = h.folder.expect("a folder");
    h.folders.path(id).expect("its path").to_path_buf()
}

#[test]
fn each_app_opens_in_its_own_last_folder_and_view() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("filedialog.toml");
    let (docs, music) = (t.path().join("docs"), t.path().join("music"));
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::create_dir_all(&music).unwrap();
    std::fs::write(docs.join("a.txt"), "").unwrap();
    std::fs::write(music.join("b.ogg"), "").unwrap();
    let u = Theme::default().text;

    let mut editor = host_of(&file, "org.example.Editor");
    editor.open_last(t.path(), open_fields(), u);
    editor.folder = Some(editor.folders.id(&docs));
    assert_eq!(
        editor.resolve("a.txt"),
        Ok(Resolved::File(docs.join("a.txt")))
    );
    let mut dlg = editor.open_last(t.path(), open_fields(), u);
    dlg.restore(Remembered {
        mode: Mode::Icons,
        ..Remembered::default()
    });
    editor.remember(&dlg);

    let mut player = host_of(&file, "org.example.Player");
    player.open_last(t.path(), open_fields(), u);
    player.folder = Some(player.folders.id(&music));
    assert_eq!(
        player.resolve("b.ogg"),
        Ok(Resolved::File(music.join("b.ogg")))
    );

    let mut editor = host_of(&file, "org.example.Editor");
    let dlg = editor.open_last(t.path(), open_fields(), u);
    assert_eq!(shown(&editor), docs, "the editor's last folder");
    assert_eq!(dlg.remembered().mode, Mode::Icons, "and its view");
    let mut player = host_of(&file, "org.example.Player");
    let dlg = player.open_last(t.path(), open_fields(), u);
    assert_eq!(shown(&player), music, "the player's last folder");
    assert_eq!(dlg.remembered(), Remembered::default(), "and its own view");
    let mut fresh = host_of(&file, "org.example.New");
    fresh.open_last(t.path(), open_fields(), u);
    assert_eq!(
        shown(&fresh),
        t.path(),
        "an app with nothing kept opens at the fallback"
    );
}

#[test]
fn a_start_folder_the_caller_names_wins_over_the_last_one() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("filedialog.toml");
    let (docs, own) = (t.path().join("docs"), t.path().join("own"));
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::create_dir_all(&own).unwrap();
    let u = Theme::default().text;
    let mut h = host_of(&file, "test.app");
    h.keep_folder(&docs);
    h.open(&own, open_fields(), u);
    assert_eq!(shown(&h), own);
    h.open_last(&own, open_fields(), u);
    assert_eq!(shown(&h), docs);
    std::fs::remove_dir(&docs).unwrap();
    h.open_last(&own, open_fields(), u);
    assert_eq!(shown(&h), own, "a last folder since removed falls back");
}

#[test]
fn a_text_size_change_lays_the_dialog_out_for_the_new_size() {
    let mut h = Host::default();
    let base = Theme::default();
    let mut dlg = h.icons.dialog(Fields {
        title: String::new(),
        accept_label: String::new(),
        filters: vec![],
        mode: DlgMode::Open,
        multiple: false,
    });
    dlg.set_text(base.text);
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    let metrics = Metrics::default();
    let grown = base.with_form_text(200);
    let step = h.event(&mut dlg, &Event::Resized, &text, &grown, CLIENT, &metrics);
    assert!(matches!(step, Step::Redraw));
    let big = grown.text.size(CLIENT);
    assert_eq!(
        dlg.client(),
        big,
        "the old window size does not hold it back"
    );
    h.event(&mut dlg, &Event::Resized, &text, &grown, big, &metrics);
    assert_eq!(dlg.client(), big, "the window's resize to it stands");

    h.event(&mut dlg, &Event::Resized, &text, &base, big, &metrics);
    assert_eq!(dlg.client(), CLIENT, "at 100% it takes its own size again");
    let wide = (CLIENT.0 + 300, CLIENT.1 + 200);
    h.event(&mut dlg, &Event::Resized, &text, &base, wide, &metrics);
    h.event(&mut dlg, &Event::Resized, &text, &grown, wide, &metrics);
    assert_eq!(
        dlg.client(),
        (wide.0.max(big.0), wide.1.max(big.1)),
        "a dialog sized by hand keeps its size, no smaller than its minimum"
    );
}

#[test]
fn thumb_buttons_step_back_and_forward() {
    use reprisekit::controls::filedlg::{Crumb, Event as E, Folder};
    let listing = |id| Folder {
        id,
        parent: None,
        crumbs: vec![Crumb {
            id,
            label: "Folder".into(),
            depth: 0,
            icon: None,
        }],
        entries: vec![],
    };
    let mut h = Host::default();
    let mut dlg = h.icons.dialog(Fields {
        title: String::new(),
        accept_label: String::new(),
        filters: vec![],
        mode: DlgMode::Open,
        multiple: false,
    });
    dlg.set_listing(listing(1));
    dlg.set_listing(listing(2));
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    let thumb = |button| {
        Event::Pointer(PointerInput::Press {
            x: 0,
            y: 0,
            button,
            time: 0,
            mods: crate::Mods::default(),
        })
    };
    assert!(matches!(
        h.event(
            &mut dlg,
            &thumb(0x113),
            &text,
            &Theme::default(),
            CLIENT,
            &Metrics::default()
        ),
        Step::Out(E::NeedListing(1))
    ));
    dlg.set_listing(listing(1));
    assert!(matches!(
        h.event(
            &mut dlg,
            &thumb(0x114),
            &text,
            &Theme::default(),
            CLIENT,
            &Metrics::default()
        ),
        Step::Out(E::NeedListing(2))
    ));
}

#[test]
fn ctrl_space_toggles_a_file_into_a_multiple_pick() {
    use reprisekit::controls::filedlg::{Crumb, Folder, Hit};
    use reprisekit::controls::shellview;
    let mut dlg = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["All Files".into()],
            mode: DlgMode::Open,
            multiple: true,
        },
        vec![],
    );
    let file = |name: &str| Entry {
        name: name.into(),
        ..Default::default()
    };
    dlg.set_listing(Folder {
        id: 1,
        parent: None,
        crumbs: vec![Crumb {
            id: 1,
            label: "Folder".into(),
            depth: 0,
            icon: None,
        }],
        entries: vec![file("a.txt"), file("b.txt")],
    });
    let (w, h) = dlg.client();
    let r = Rect::new(0, 0, w, h);
    dlg.click(
        r,
        Some(Hit::Shell(shellview::Hit::Entry(0))),
        Mods::default(),
    );
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    let mut host = Host::default();
    host.event(
        &mut dlg,
        &key(keysym::DOWN, true, None),
        &text,
        &Theme::default(),
        CLIENT,
        &Metrics::default(),
    );
    assert_eq!(dlg.file_name(), "a.txt", "Ctrl+Down moves the focus alone");
    host.event(
        &mut dlg,
        &key(keysym::SPACE, true, Some(" ")),
        &text,
        &Theme::default(),
        CLIENT,
        &Metrics::default(),
    );
    assert_eq!(dlg.file_name(), "\"b.txt\" \"a.txt\"");
}

#[test]
fn resolve_a_plain_name_in_the_open_folder() {
    let t = tmp("plain");
    assert_eq!(
        host_in(&t).resolve("new.txt"),
        Ok(Resolved::File(t.join("new.txt")))
    );
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn resolve_a_folder_name_opens_it() {
    let t = tmp("folder");
    let mut h = host_in(&t);
    match h.resolve("sub") {
        Ok(Resolved::Folder(id, p)) => {
            assert_eq!(p, t.join("sub"));
            assert_eq!(h.folders.path(id), Some(t.join("sub").as_path()));
        }
        r => panic!("expected a folder, got {r:?}"),
    }
    assert!(matches!(h.resolve("."), Ok(Resolved::Folder(_, p)) if p == t));
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn resolve_relative_parent_and_absolute_paths() {
    let t = tmp("rel");
    let mut h = host_in(&t.join("sub"));
    assert_eq!(h.resolve("../a.txt"), Ok(Resolved::File(t.join("a.txt"))));
    assert_eq!(
        h.resolve("../sub/x.txt"),
        Ok(Resolved::File(t.join("sub/x.txt")))
    );
    let abs = t.join("a.txt");
    assert_eq!(
        h.resolve(abs.to_str().unwrap()),
        Ok(Resolved::File(abs.clone()))
    );
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn resolve_expands_the_home_folder() {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
    let t = tmp("home");
    let mut h = host_in(&t);
    assert!(matches!(h.resolve("~"), Ok(Resolved::Folder(_, p)) if p == home));
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn a_typed_path_that_does_not_answer_is_checked_once() {
    let t = tmp("dead");
    let mut h = host_in(&t);
    let (answer, gate) = std::sync::mpsc::channel::<()>();
    h.probing = Some((
        t.join("gone"),
        Job::spawn(None, move || {
            gate.recv().ok();
            (true, true)
        }),
    ));
    let start = std::time::Instant::now();
    assert_eq!(
        h.resolve("gone"),
        Err(format!("{} is not responding.", t.join("gone").display()))
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    answer.send(()).unwrap();
    assert!(
        matches!(h.resolve("gone"), Ok(Resolved::Folder(_, p)) if p == t.join("gone")),
        "the second Enter takes the first probe's answer"
    );
    assert!(h.probing.is_none());
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn resolve_refuses_a_missing_parent() {
    let t = tmp("missing");
    assert!(host_in(&t).resolve("nope/x.txt").is_err());
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn resolve_in_recent_documents() {
    let t = tmp("recent");
    let mut h = host_in(Path::new(reprisefs::places::RECENT));
    h.alias("a.txt".into(), t.join("a.txt").into_os_string());
    assert_eq!(h.resolve("a.txt"), Ok(Resolved::File(t.join("a.txt"))));
    assert!(h.resolve("new.txt").is_err());
    assert_eq!(
        h.resolve(&t.join("b.txt").to_string_lossy()),
        Ok(Resolved::File(t.join("b.txt"))),
        "a full path still works"
    );
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn a_places_tile_gives_its_folder() {
    let mut h = Host::default();
    let id = h.place(5).expect("Computer has a folder");
    assert_eq!(h.folders.path(id), Some(Path::new("/")));
}

#[test]
fn a_listed_name_that_is_not_utf8_resolves_to_its_real_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let t = tmp("latin1");
    let raw = t.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
    std::fs::write(&raw, "").unwrap();
    let mut h = host_in(&t);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let id = h.folder.unwrap();
    let mut dlg = h.icons.dialog(Fields {
        title: String::new(),
        accept_label: String::new(),
        filters: vec![],
        mode: DlgMode::Open,
        multiple: false,
    });
    h.list(&mut dlg, id, &mut icons, &|_| true);
    let shown = dlg
        .entries()
        .iter()
        .find(|e| e.name.starts_with("caf"))
        .expect("listed")
        .name
        .clone();
    assert_eq!(h.resolve(&shown), Ok(Resolved::File(raw.clone())));
    h.alias(
        "caf?.pdf".into(),
        std::ffi::OsString::from_vec(b"caf\xe9.pdf".to_vec()),
    );
    assert_eq!(
        h.resolve("caf?.pdf"),
        Ok(Resolved::File(
            t.join(std::ffi::OsStr::from_bytes(b"caf\xe9.pdf"))
        ))
    );
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn a_dangling_symlink_is_taken() {
    let t = tmp("link");
    std::os::unix::fs::symlink(t.join("gone"), t.join("dangling")).unwrap();
    assert!(taken(&t.join("dangling")));
    assert!(!taken(&t.join("free.txt")));
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn a_filter_keeps_folders_and_folder_mode_keeps_only_folders() {
    let t = tmp("filter");
    let mut folders = FolderTable::default();
    let id = folders.id(&t);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let names = |f: Folder| f.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    let none = |_: &reprisefs::listing::Item| false;
    let f = listed(&mut folders, id, &mut icons, false, none);
    assert_eq!(
        names(f),
        vec!["sub"],
        "a filter that keeps nothing still keeps folders"
    );
    let all = |_: &reprisefs::listing::Item| true;
    let f = listed(&mut folders, id, &mut icons, true, all);
    assert_eq!(names(f), vec!["sub"], "folder mode lists folders only");
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn the_picked_type_filters_the_listing() {
    let t = tmp("types");
    std::fs::write(t.join("bash"), "").unwrap();
    std::fs::write(t.join("b.conf"), "").unwrap();
    let mut h = host_in(&t);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = h.icons.dialog(Fields {
        title: "Open".into(),
        accept_label: "Open".into(),
        filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
        mode: DlgMode::Open,
        multiple: false,
    });
    let names = |d: &FileDlg| {
        let mut n: Vec<String> = d.entries().iter().map(|e| e.name.clone()).collect();
        n.sort();
        n
    };
    let keep = type_keep(&dlg);
    h.ensure_listed(&mut dlg, &mut icons, &keep);
    assert_eq!(names(&dlg), ["a.txt", "sub"]);
    dlg.set_filter(1);
    let id = h.folder.unwrap();
    let keep = type_keep(&dlg);
    h.list(&mut dlg, id, &mut icons, &keep);
    assert_eq!(names(&dlg), ["a.txt", "b.conf", "bash", "sub"]);
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn a_metadata_change_in_the_folder_shown_reads_it_again() {
    let t = tmp("gvfs");
    let mut h = host_in(&t);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = h.icons.dialog(Fields {
        title: "Open".into(),
        accept_label: "Open".into(),
        filters: vec![],
        mode: DlgMode::Open,
        multiple: false,
    });
    h.ensure_listed(&mut dlg, &mut icons, &|_: &Item| true);
    let none = BTreeSet::new();
    h.read_again(
        &none,
        &BTreeSet::from([PathBuf::from("/srv/elsewhere/sub")]),
    );
    assert!(h.reading.is_none(), "a change in another folder is left");
    h.read_again(&none, &BTreeSet::from([t.join("sub")]));
    assert!(
        h.reading.is_some(),
        "a change to a subfolder reads the folder again"
    );
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn a_typed_pattern_lists_through_it() {
    use reprisekit::controls::filedlg::Event as E;
    let t = tmp("wildcard");
    std::fs::write(t.join("bash"), "").unwrap();
    std::fs::write(t.join("sub/x.conf"), "").unwrap();
    std::fs::write(t.join("sub/y.txt"), "").unwrap();
    let mut h = host_in(&t);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = h.icons.dialog(Fields {
        title: "Open".into(),
        accept_label: "Open".into(),
        filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
        mode: DlgMode::Open,
        multiple: false,
    });
    let keep = type_keep(&dlg);
    h.ensure_listed(&mut dlg, &mut icons, &keep);
    let names = |d: &FileDlg| {
        let mut n: Vec<String> = d.entries().iter().map(|e| e.name.clone()).collect();
        n.sort();
        n
    };
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    let accept = |h: &mut Host, dlg: &mut FileDlg, typed: &str| {
        dlg.set_file_name(typed);
        h.event(
            dlg,
            &key(keysym::RETURN, false, None),
            &text,
            &Theme::default(),
            CLIENT,
            &Metrics::default(),
        )
    };
    let Step::Out(E::NeedListing(id)) = accept(&mut h, &mut dlg, "*") else {
        panic!("a bare pattern relists the open folder");
    };
    assert_eq!(Some(id), h.folder);
    h.list(&mut dlg, id, &mut icons, &keep);
    assert_eq!(names(&dlg), ["a.txt", "bash", "sub"], "* shows every file");
    let Step::Out(E::NeedListing(id)) =
        accept(&mut h, &mut dlg, &format!("{}/sub/*.conf", t.display()))
    else {
        panic!("a folder part lists that folder");
    };
    h.list(&mut dlg, id, &mut icons, &keep);
    assert_eq!(names(&dlg), ["x.conf"]);
    assert_eq!(dlg.file_name(), "*.conf");
    match accept(&mut h, &mut dlg, "nowhere/*.txt") {
        Step::Out(E::Accept(n)) => assert_eq!(n, ["nowhere/*.txt"]),
        s => panic!("expected Accept, got {s:?}"),
    }
    assert_eq!(dlg.file_name(), "nowhere/*.txt", "the field as typed");
    assert_eq!(dlg.wildcard(), Some("*.conf"), "the filter stays");
    assert_eq!(names(&dlg), ["x.conf"], "the listing stays");
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn place_icons_land_on_the_tiles_in_order() {
    let one = || Some(Canvas::new(1, 1));
    let icons = Icons {
        places: [one(), None, one(), None, one(), None],
        ..Icons::default()
    };
    let places = icons.places();
    let got: Vec<bool> = places.iter().map(|p| p.icon.is_some()).collect();
    assert_eq!(got, [true, false, true, false, true, false]);
}

#[test]
fn a_listed_subfolder_carries_a_resolvable_id_and_hidden_entries_are_left_out() {
    let t = std::env::temp_dir().join(format!("repriseapp-filedlg-{}", std::process::id()));
    std::fs::create_dir_all(t.join("sub")).unwrap();
    std::fs::write(t.join("a.txt"), "").unwrap();
    std::fs::write(t.join(".hidden"), "").unwrap();
    let mut folders = FolderTable::default();
    let id = folders.id(&t);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let f = listed(&mut folders, id, &mut icons, false, |_| true);
    let mut names: Vec<_> = f.entries.iter().map(|e| e.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["a.txt", "sub"]);
    let sub = f.entries.iter().find(|e| e.is_dir).expect("sub");
    assert_eq!(folders.path(sub.id), Some(t.join("sub").as_path()));
    assert_eq!(f.parent.and_then(|p| folders.path(p)), t.parent());
    let mut h = Host::default();
    let mut dlg = open_dlg(&mut h);
    h.list(&mut dlg, 99, &mut icons, &|_: &Item| true);
    assert!(
        dlg.folder().is_none(),
        "an id that does not resolve lists nothing"
    );
    std::fs::remove_dir_all(&t).ok();
}

#[test]
fn the_wheel_scrolls_the_view_under_the_pointer() {
    use reprisekit::controls::filedlg::{Crumb, Folder, Hit};
    use reprisekit::controls::shellview::{self, Entry};
    use reprisekit::pointer::WHEEL_ROWS;
    let mut dlg = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["All Files".into()],
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![],
    );
    dlg.set_listing(Folder {
        id: 1,
        parent: None,
        crumbs: vec![Crumb {
            id: 1,
            label: "Folder".into(),
            depth: 0,
            icon: None,
        }],
        entries: (0..60)
            .map(|i| Entry {
                name: format!("f{i:02}.txt"),
                ..Default::default()
            })
            .collect(),
    });
    let (w, h) = dlg.client();
    let r = Rect::new(0, 0, w, h);
    dlg.click(r, Some(Hit::ToolBar(3)), Mods::default());
    dlg.click(r, Some(Hit::ViewsItem(4)), Mods::default());
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    let (x, y) = (0..h)
        .step_by(4)
        .flat_map(|y| (0..w).step_by(4).map(move |x| (x, y)))
        .find(|&(x, y)| {
            matches!(
                dlg.hit(&text, 11, r, x, y),
                Some(Hit::Shell(shellview::Hit::Entry(_)))
            )
        })
        .expect("an entry is on screen");
    let wheel = |x, y, rows| Event::Pointer(PointerInput::Scroll { x, y, rows });
    let mut host = Host::default();
    assert!(matches!(
        host.event(
            &mut dlg,
            &wheel(x, y, -WHEEL_ROWS),
            &text,
            &Theme::default(),
            CLIENT,
            &Metrics::default()
        ),
        Step::None
    ));
    assert!(matches!(
        host.event(
            &mut dlg,
            &wheel(x, y, WHEEL_ROWS),
            &text,
            &Theme::default(),
            CLIENT,
            &Metrics::default()
        ),
        Step::Redraw
    ));
    assert!(matches!(
        host.event(
            &mut dlg,
            &wheel(1, 1, WHEEL_ROWS),
            &text,
            &Theme::default(),
            CLIENT,
            &Metrics::default()
        ),
        Step::None
    ));
}

const FOLDER: u32 = 0xff20_40c0;
const TEXT: u32 = 0xffc0_4020;
const PICTURE: u32 = 0xff20_a040;
const PIXELS: u32 = 0xffe0_c000;

fn png(path: &Path, w: u32, h: u32, argb: u32) {
    let [a, r, g, b] = argb.to_be_bytes();
    image::RgbaImage::from_pixel(w, h, image::Rgba([r, g, b, a]))
        .save(path)
        .unwrap();
}

/// An icon theme drawing "folder", "text-x-generic" and "image-x-generic" in flat colours at
/// 16, 32 and 48 px.
fn flat_icons(t: &Path) -> IconCache {
    let root = t.join("icons");
    let theme = root.join("flat");
    let dirs = ["16x16/places", "32x32/places", "48x48/places"];
    let mut index = format!("[Icon Theme]\nName=Flat\nDirectories={}\n", dirs.join(","));
    for (d, px) in dirs.iter().zip([16, 32, 48]) {
        index += &format!("\n[{d}]\nSize={px}\nType=Fixed\n");
        std::fs::create_dir_all(theme.join(d)).unwrap();
        for (name, argb) in [
            ("folder", FOLDER),
            ("text-x-generic", TEXT),
            ("image-x-generic", PICTURE),
            ("computer", FOLDER),
            ("go-up", FOLDER),
        ] {
            png(&theme.join(d).join(format!("{name}.png")), px, px, argb);
        }
    }
    std::fs::write(theme.join("index.theme"), index).unwrap();
    IconCache::new(reprisekit::icontheme::IconTheme::load("flat", &[root]))
}

/// `t` holds a folder, a 2 KB text file and a 40x30 picture.
fn pictures_folder() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("list");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("notes.txt"), [b'x'; 2048]).unwrap();
    png(&dir.join("pic.png"), 40, 30, PIXELS);
    t
}

fn open_dlg(h: &mut Host) -> FileDlg {
    h.icons.dialog(Fields {
        title: "Open".into(),
        accept_label: "Open".into(),
        filters: vec!["All Files".into()],
        mode: DlgMode::Open,
        multiple: false,
    })
}

fn entry<'a>(dlg: &'a FileDlg, name: &str) -> &'a Entry {
    dlg.entries().iter().find(|e| e.name == name).expect(name)
}

fn colour(icon: &Option<std::rc::Rc<Canvas>>) -> Option<(u32, u32)> {
    icon.as_ref()
        .map(|c| (c.width, c.get(c.width as i32 / 2, c.height as i32 / 2)))
}

/// Picks `mode` from Views the way a click does.
fn view(h: &mut Host, dlg: &mut FileDlg, mode: Mode) {
    let i = [
        Mode::Thumbnails,
        Mode::Tiles,
        Mode::Icons,
        Mode::List,
        Mode::Details,
    ]
    .iter()
    .position(|m| *m == mode)
    .unwrap();
    let (w, ht) = dlg.client();
    let r = Rect::new(0, 0, w, ht);
    dlg.click(r, Some(Hit::ToolBar(3)), Mods::default());
    dlg.click(r, Some(Hit::ViewsItem(i)), Mods::default());
    h.ask_thumbs(dlg);
}

/// Takes thumbnails and icon files until some are in.
fn settle_thumbs(h: &mut Host, dlg: &mut FileDlg, icons: &mut IconCache) {
    for _ in 0..500 {
        if h.take_thumbs(dlg, icons) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("nothing arrived");
}

fn thumbs_without_cache(h: &mut Host) {
    h.thumbs = Some(Thumbs::new(
        std::sync::Arc::new(crate::decode::Decoders::default_set()),
        None,
        crate::Waker::noop(),
    ));
}

#[test]
fn a_listing_gives_entries_their_icons_at_every_size_and_tiles_lines() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&t.path().join("list"));
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_| true);
    let sizes = |e: &Entry| [&e.icon, &e.icon32, &e.icon48].map(colour);
    let folder = entry(&dlg, "sub");
    assert_eq!(
        sizes(folder),
        [Some((16, FOLDER)), Some((32, FOLDER)), Some((48, FOLDER))]
    );
    assert!(folder.detail.is_empty(), "a folder's tile names it alone");
    let notes = entry(&dlg, "notes.txt");
    assert_eq!(
        sizes(notes),
        [Some((16, TEXT)), Some((32, TEXT)), Some((48, TEXT))]
    );
    assert_eq!(notes.detail, [notes.kind.clone(), "2 KB".into()]);
    let pic = entry(&dlg, "pic.png");
    assert_eq!(
        sizes(pic),
        [
            Some((16, PICTURE)),
            Some((32, PICTURE)),
            Some((48, PICTURE))
        ]
    );
    assert!(pic.thumb.is_none(), "no thumbnail before it is decoded");
    assert_eq!(pic.detail[0], pic.kind);
}

/// A new icon theme gives the listed entries its icons without listing the folder again,
/// each named anew: a folder whose own icon the old theme lacked wears it now.
#[test]
fn a_new_icon_theme_gives_the_listed_entries_its_icons() {
    let t = pictures_folder();
    let dir = t.path().join("list");
    std::fs::create_dir(dir.join("music")).unwrap();
    reprisefs::meta::set_folder_icon(&dir.join("music"), Some("folder-music")).unwrap();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&dir);
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_| true);
    let wake = crate::Waker::noop();
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    let sizes = |dlg: &FileDlg, name: &str| {
        let e = entry(dlg, name);
        [&e.icon, &e.icon32, &e.icon48].map(colour)
    };
    assert_eq!(sizes(&dlg, "sub")[0], Some((16, FOLDER)));
    assert_eq!(
        sizes(&dlg, "music")[0],
        Some((16, FOLDER)),
        "flat has no folder-music"
    );
    view(&mut h, &mut dlg, Mode::List);
    let shown = dlg.entries().to_vec();

    let other = t.path().join("other");
    let theme = other.join("dark");
    let dirs = ["16x16/places", "32x32/places", "48x48/places"];
    let mut index = format!("[Icon Theme]\nName=Dark\nDirectories={}\n", dirs.join(","));
    for (d, px) in dirs.iter().zip([16, 32, 48]) {
        index += &format!("\n[{d}]\nSize={px}\nType=Fixed\n");
        std::fs::create_dir_all(theme.join(d)).unwrap();
        png(&theme.join(d).join("folder.png"), px, px, PICTURE);
        png(&theme.join(d).join("folder-music.png"), px, px, TEXT);
    }
    std::fs::write(theme.join("index.theme"), index).unwrap();
    let mut dark = IconCache::new(reprisekit::icontheme::IconTheme::load("dark", &[other]));
    h.follow_scale(&mut dlg, 1, &mut dark, &wake);
    settle_thumbs(&mut h, &mut dlg, &mut dark);
    let each = |c| [Some((16, c)), Some((32, c)), Some((48, c))];
    assert_eq!(sizes(&dlg, "sub"), each(PICTURE));
    assert_eq!(sizes(&dlg, "music"), each(TEXT));
    let names = |e: &[Entry]| e.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(dlg.entries()), names(&shown), "the same entries");
    assert_eq!(dlg.view(), Mode::List, "the view stays");
    assert!(h.reading.is_none(), "no listing to show");
}

#[test]
fn tiles_and_thumbnails_show_a_pictures_thumbnail_and_size() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&t.path().join("list"));
    thumbs_without_cache(&mut h);
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_| true);
    assert!(!h.pictures.asked, "List decodes nothing");
    view(&mut h, &mut dlg, Mode::Tiles);
    assert!(h.pictures.asked);
    settle_thumbs(&mut h, &mut dlg, &mut icons);
    let pic = entry(&dlg, "pic.png");
    let thumb = pic.thumb.as_ref().expect("the picture's thumbnail");
    assert_eq!((thumb.width, thumb.height), (40, 30), "never upscaled");
    assert_eq!(thumb.get(20, 15), PIXELS);
    assert_eq!(pic.detail, ["40 x 30".to_string(), pic.kind.clone()]);
    assert!(entry(&dlg, "notes.txt").thumb.is_none());
}

#[test]
fn a_new_draw_scale_replaces_the_thumbnails_made_at_the_old_one() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&t.path().join("list"));
    thumbs_without_cache(&mut h);
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_| true);
    view(&mut h, &mut dlg, Mode::Thumbnails);
    settle_thumbs(&mut h, &mut dlg, &mut icons);
    let wake = crate::Waker::noop();
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    assert!(h.thumbs.as_mut().unwrap().drain().is_empty(), "same scale");

    icons.set_scale(2);
    h.follow_scale(&mut dlg, 2, &mut icons, &wake);
    let kept = entry(&dlg, "pic.png").thumb.as_ref().expect("still shown");
    assert_eq!(kept.scale(), 1);
    settle_thumbs(&mut h, &mut dlg, &mut icons);
    let thumb = entry(&dlg, "pic.png").thumb.as_ref().expect("the new one");
    assert_eq!((thumb.scale(), thumb.device_size()), (2, (80, 60)));
}

#[test]
fn a_new_draw_scale_fetches_the_places_and_toolbar_icons_again() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&t.path().join("list"));
    h.icons.prime(&mut icons);
    let mut dlg = open_dlg(&mut h);
    let wake = crate::Waker::noop();
    let fresh = |icons: &mut IconCache| {
        let mut f = Host::default();
        f.icons.prime(icons);
        let scales =
            [&f.icons.places[5], &f.icons.toolbar[1]].map(|c| c.as_ref().map(Canvas::scale));
        (open_dlg(&mut f), scales)
    };

    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    icons.set_scale(2);
    h.icons.prime(&mut icons);
    h.follow_scale(&mut dlg, 2, &mut icons, &wake);
    let (want, scales) = fresh(&mut icons);
    assert_eq!(scales, [Some(2), Some(2)], "Computer's tile and Up");
    assert!(dlg == want, "the dialog wears the scale 2 icons");

    icons.set_scale(1);
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    assert!(dlg == fresh(&mut icons).0, "and the scale 1 ones again");
}

/// At 200% text the places tiles wear 64 px icons and the toolbar 32 px ones, Views too
/// (the theme's grown or the drawn one); back at 100% they are 32 and 16 px again.
#[test]
fn a_text_size_change_fetches_the_places_and_toolbar_icons_grown() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&t.path().join("list"));
    h.icons.prime(&mut icons);
    let mut dlg = open_dlg(&mut h);
    let wake = crate::Waker::noop();
    let widths = |h: &Host| {
        [&h.icons.places[5], &h.icons.toolbar[1], &h.icons.toolbar[3]]
            .map(|c| c.as_ref().map(|c| c.width))
    };
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    assert_eq!(widths(&h), [Some(32), Some(16), Some(16)], "at 100%");

    dlg.set_text(TextUnits::new(22, 11));
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    assert_eq!(widths(&h), [Some(64), Some(32), Some(32)], "at 200%");
    let mut grown = open_dlg(&mut h);
    grown.set_text(TextUnits::new(22, 11));
    assert!(dlg == grown, "the dialog wears the grown icons");

    dlg.set_text(TextUnits::ONE);
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    assert_eq!(widths(&h), [Some(32), Some(16), Some(16)], "at 100% again");
}

/// The Look in field wears the folder icon whether the folder lists before or after the
/// dialog first draws, grown to 32 px at 200% text.
#[test]
fn the_look_in_field_wears_a_folder_icon() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let wake = crate::Waker::noop();
    let worn = |h: &Host, dlg: &FileDlg| {
        let mut want = dlg.clone();
        want.set_crumb_icons(vec![h.icons.folder.clone()]);
        *dlg == want
    };
    let mut h = host_in(&t.path().join("list"));
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_| true);
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    let folder = h.icons.folder.as_ref().map(|c| (c.width, c.get(8, 8)));
    assert_eq!(folder, Some((16, FOLDER)));
    assert!(worn(&h, &dlg), "listed before the first draw");

    h.list(&mut dlg, h.folder.unwrap(), &mut icons, &|_| true);
    assert!(worn(&h, &dlg), "listed again after it");

    dlg.set_text(TextUnits::new(22, 11));
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    assert_eq!(h.icons.folder.as_ref().map(|c| c.width), Some(32));
    assert!(worn(&h, &dlg), "grown at 200%");
}

#[test]
fn a_new_icon_theme_gives_the_places_and_toolbar_its_icons() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&t.path().join("list"));
    h.icons.prime(&mut icons);
    let mut dlg = open_dlg(&mut h);
    let wake = crate::Waker::noop();
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);

    let other = t.path().join("other");
    let theme = other.join("dark");
    let dir = theme.join("32x32/places");
    std::fs::create_dir_all(&dir).unwrap();
    png(&dir.join("computer.png"), 32, 32, TEXT);
    let index = "[Icon Theme]\nName=Dark\nDirectories=32x32/places\n\n[32x32/places]\nSize=32\nType=Fixed\n";
    std::fs::write(theme.join("index.theme"), index).unwrap();
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("dark", &[other]));
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);

    let mut fresh = Host::default();
    fresh.icons.prime(&mut icons);
    let computer = fresh.icons.places[5].as_ref().expect("the dark Computer");
    assert_eq!(
        computer.get(16, 16),
        TEXT,
        "Computer's tile from the dark theme"
    );
    assert!(fresh.icons.toolbar[1].is_none(), "the dark theme has no Up");
    assert!(
        dlg == open_dlg(&mut fresh),
        "the open dialog wears the dark theme's icons"
    );
}

/// Each size's canvas scale, device width and middle colour.
fn icon_sizes(e: &Entry) -> [Option<(i32, u32, u32)>; 3] {
    [&e.icon, &e.icon32, &e.icon48].map(|c| {
        c.as_ref().map(|c| {
            let mid = c.get(c.width as i32 / 2, c.height as i32 / 2);
            (c.scale(), c.device_size().0, mid)
        })
    })
}

#[test]
fn a_new_draw_scale_swaps_the_list_icons_without_listing_again() {
    const OWN: u32 = 0xff80_20a0;
    let t = pictures_folder();
    let dir = t.path().join("list");
    let own = t.path().join("own.png");
    png(&own, 96, 96, OWN);
    let directory = format!("[Desktop Entry]\nIcon={}\n", own.display());
    std::fs::write(dir.join("sub/.directory"), directory).unwrap();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&dir);
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_| true);
    let listing = h.pictures.generation;
    let wake = crate::Waker::noop();
    assert_eq!(
        icon_sizes(entry(&dlg, "notes.txt")),
        [
            Some((1, 16, TEXT)),
            Some((1, 32, TEXT)),
            Some((1, 48, TEXT))
        ]
    );

    icons.set_scale(2);
    h.follow_scale(&mut dlg, 2, &mut icons, &wake);
    assert_eq!(
        icon_sizes(entry(&dlg, "notes.txt")),
        [
            Some((2, 32, TEXT)),
            Some((2, 64, TEXT)),
            Some((2, 96, TEXT))
        ]
    );
    assert_eq!(
        icon_sizes(entry(&dlg, "sub"))[0],
        Some((1, 16, OWN)),
        "an icon file keeps its old canvas until its thread decodes it"
    );
    settle_thumbs(&mut h, &mut dlg, &mut icons);
    assert_eq!(
        icon_sizes(entry(&dlg, "sub")),
        [Some((2, 32, OWN)), Some((2, 64, OWN)), Some((2, 96, OWN))]
    );
    assert_eq!(
        h.pictures.generation, listing,
        "the folder is not listed again"
    );

    icons.set_scale(1);
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    assert_eq!(
        icon_sizes(entry(&dlg, "sub")),
        [Some((1, 16, OWN)), Some((1, 32, OWN)), Some((1, 48, OWN))],
        "the cache still holds the icon file at scale 1"
    );

    std::fs::remove_file(&own).unwrap();
    icons.set_scale(2);
    h.follow_scale(&mut dlg, 2, &mut icons, &wake);
    assert_eq!(
        icon_sizes(entry(&dlg, "sub")),
        [Some((2, 32, OWN)), Some((2, 64, OWN)), Some((2, 96, OWN))],
        "the cache kept the icon file the thread decoded at scale 2"
    );
}

/// At 125% and 150% text List and Details' icons are fetched at their grown column, 20 and
/// 25 px, without listing the folder again: theme icons at once, icon files once their thread
/// decodes them. A listing made there wears them too.
#[test]
fn a_text_size_change_fetches_the_list_icons_at_the_grown_column() {
    const OWN: u32 = 0xff80_20a0;
    let t = pictures_folder();
    let dir = t.path().join("list");
    let own = t.path().join("own.png");
    png(&own, 96, 96, OWN);
    let directory = format!("[Desktop Entry]\nIcon={}\n", own.display());
    std::fs::write(dir.join("sub/.directory"), directory).unwrap();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&dir);
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_| true);
    let wake = crate::Waker::noop();
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    let listing = h.pictures.generation;
    let list = |dlg: &FileDlg, name: &str| icon_sizes(entry(dlg, name))[0];
    assert_eq!(list(&dlg, "notes.txt"), Some((1, 16, TEXT)));

    for (u, side) in [(TextUnits::new(14, 11), 20), (TextUnits::new(17, 11), 25)] {
        dlg.set_text(u);
        h.follow_scale(&mut dlg, 1, &mut icons, &wake);
        assert_eq!(list(&dlg, "notes.txt"), Some((1, side, TEXT)));
        settle_thumbs(&mut h, &mut dlg, &mut icons);
        assert_eq!(list(&dlg, "sub"), Some((1, side, OWN)), "the icon file");
        assert_eq!(icon_sizes(entry(&dlg, "sub"))[1], Some((1, 32, OWN)));
    }
    assert_eq!(
        h.pictures.generation, listing,
        "the folder is not listed again"
    );

    h.list(&mut dlg, h.folder.unwrap(), &mut icons, &|_| true);
    assert_eq!(
        list(&dlg, "notes.txt"),
        Some((1, 25, TEXT)),
        "listed at 150%"
    );
    assert_eq!(list(&dlg, "sub"), Some((1, 25, OWN)));

    dlg.set_text(TextUnits::ONE);
    h.follow_scale(&mut dlg, 1, &mut icons, &wake);
    assert_eq!(list(&dlg, "notes.txt"), Some((1, 16, TEXT)));
}

/// A folder still being read leaves the dialog as it was and its loop free; the listing
/// lands once the read answers, wearing the folder icon and icon file its thread read though
/// both are gone by then.
#[test]
fn a_late_listing_lands_with_the_icons_its_thread_read() {
    const OWN: u32 = 0xff80_20a0;
    let t = pictures_folder();
    let dir = t.path().join("list");
    let own = t.path().join("own.png");
    png(&own, 96, 96, OWN);
    let directory = format!("[Desktop Entry]\nIcon={}\n", own.display());
    std::fs::write(dir.join("sub/.directory"), directory).unwrap();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&dir);
    let mut dlg = open_dlg(&mut h);
    let id = h.folder.unwrap();
    let read = read(dir.clone(), |_: &Item| true, icons.scale(), 16);
    std::fs::remove_file(dir.join("sub/.directory")).unwrap();
    std::fs::remove_file(&own).unwrap();
    let (answer, gate) = std::sync::mpsc::channel::<()>();
    h.reading = Some((
        ReadKey::of(&dlg, id, icons.scale()),
        Job::spawn(None, move || {
            gate.recv().ok();
            read
        }),
    ));

    h.ensure_listed(&mut dlg, &mut icons, &|_: &Item| true);
    assert!(dlg.folder().is_none(), "the dialog waits for the read");
    assert!(!h.take_thumbs(&mut dlg, &mut icons), "nothing has landed");

    answer.send(()).unwrap();
    settle_thumbs(&mut h, &mut dlg, &mut icons);
    assert_eq!(dlg.folder(), Some(id));
    assert_eq!(
        icon_sizes(entry(&dlg, "sub")),
        [Some((1, 16, OWN)), Some((1, 32, OWN)), Some((1, 48, OWN))]
    );
    assert_eq!(icon_sizes(entry(&dlg, "notes.txt"))[0], Some((1, 16, TEXT)));
}

#[test]
fn a_relist_of_a_folder_being_read_waits_on_that_read() {
    let t = tmp("join");
    let mut h = host_in(&t);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = open_dlg(&mut h);
    let id = h.folder.unwrap();
    let in_flight = |h: &mut Host, key: ReadKey| {
        let before = read(t.clone(), |_: &Item| true, 1, 16);
        h.reading = Some((key, Job::spawn(None, move || before)));
    };
    let names =
        |dlg: &FileDlg| -> Vec<String> { dlg.entries().iter().map(|e| e.name.clone()).collect() };
    in_flight(&mut h, ReadKey::of(&dlg, id, icons.scale()));
    std::fs::write(t.join("late.txt"), "").unwrap();

    h.list(&mut dlg, id, &mut icons, &|_| true);
    assert_eq!(names(&dlg), ["sub", "a.txt"], "the read in flight answers");
    assert!(h.reading.is_none());

    in_flight(&mut h, ReadKey::of(&dlg, id, icons.scale() + 1));
    h.list(&mut dlg, id, &mut icons, &|_| true);
    assert_eq!(
        names(&dlg),
        ["sub", "a.txt", "late.txt"],
        "another scale reads again"
    );
    std::fs::remove_dir_all(&t).ok();
}

/// A program with a file dialog in window `DLG` that does nothing for its thumbnails.
struct Program {
    host: Host,
    dlg: FileDlg,
}

const DLG: crate::WinId = crate::WinId::named("open");

impl crate::App for Program {
    fn title(&self) -> String {
        String::new()
    }
    fn app_id(&self) -> String {
        String::new()
    }
    fn initial_size(&self) -> (i32, i32) {
        CLIENT
    }
    fn event(&mut self, _: crate::WinId, _: Event, _: &mut crate::Ctx) -> crate::Reply {
        crate::Reply::default()
    }
    fn draw(&mut self, _: crate::WinId, _: &mut Canvas, _: &crate::Ctx) {}
}

impl crate::Extended for Program {
    fn drop_file_dialog(&mut self, win: crate::WinId) -> Option<crate::fielddrop::FileDrop<'_>> {
        (win == DLG).then_some(crate::fielddrop::FileDrop {
            dlg: &mut self.dlg,
            host: &mut self.host,
        })
    }
}

#[test]
fn every_programs_file_dialog_shows_thumbnails_without_wiring() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let mut host = host_in(&t.path().join("list"));
    let dlg = open_dlg(&mut host);
    let mut app = Program { host, dlg };
    thumbs_on(&mut app, crate::MAIN, &crate::Waker::noop());
    assert!(
        app.host.thumbs.is_none(),
        "a window without a dialog starts nothing"
    );
    thumbs_on(&mut app, DLG, &crate::Waker::noop());
    assert!(
        app.host.thumbs.is_some(),
        "the dialog's window starts the worker"
    );
    thumbs_without_cache(&mut app.host);
    thumbs_on(&mut app, DLG, &crate::Waker::noop());
    let Program { host, dlg } = &mut app;
    host.ensure_listed(dlg, &mut icons, &|_| true);
    view(host, dlg, Mode::Thumbnails);
    let mut redrawn = Vec::new();
    for _ in 0..500 {
        redrawn = take_thumbs(&mut app, [crate::MAIN, DLG], &mut icons);
        if !redrawn.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(redrawn, [DLG], "the dialog's window redraws");
    assert!(entry(&app.dlg, "pic.png").thumb.is_some());
    assert!(
        take_thumbs(&mut app, [DLG], &mut icons).is_empty(),
        "nothing new to show"
    );
}

#[test]
fn a_changed_cache_setting_restarts_the_worker_at_the_next_dialog() {
    use repriseconf::ThumbnailCache;
    let off = ThumbnailCache {
        enabled: false,
        ..ThumbnailCache::DEFAULT
    };
    let mut h = Host::default();
    h.follow_cache(off);
    thumbs_without_cache(&mut h);
    h.follow_cache(off);
    assert!(h.thumbs.is_some(), "the same setting keeps the worker");
    h.follow_cache(ThumbnailCache::DEFAULT);
    assert!(h.thumbs.is_none(), "a new setting stops it");
    h.thumbs_on(&crate::Waker::noop());
    assert!(h.thumbs.is_some());
    assert_eq!(h.cache, Some(ThumbnailCache::DEFAULT));
}

/// Where a colour lands in the drawn dialog: its bounding box.
fn found(c: &Canvas, argb: u32) -> Option<Rect> {
    let mut hit: Option<(i32, i32, i32, i32)> = None;
    for y in 0..c.height as i32 {
        for x in 0..c.width as i32 {
            if c.get(x, y) == argb {
                let b = hit.get_or_insert((x, y, x, y));
                *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
    }
    hit.map(|(l, t, r, b)| Rect::new(l, t, r - l + 1, b - t + 1))
}

fn drawn(dlg: &FileDlg) -> Canvas {
    let (w, h) = dlg.client();
    let mut c = Canvas::new(w as u32, h as u32);
    let theme = reprisekit::theme::Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    dlg.draw(
        &mut c,
        &text,
        &theme.paint(),
        Rect::new(0, 0, w, h),
        theme.fonts.size_px,
    );
    c
}

#[test]
fn the_dialog_draws_tiles_icons_and_thumbnails_pictures() {
    let t = pictures_folder();
    let mut icons = flat_icons(t.path());
    let mut h = host_in(&t.path().join("list"));
    thumbs_without_cache(&mut h);
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_| true);
    view(&mut h, &mut dlg, Mode::Tiles);
    let tiles = drawn(&dlg);
    for argb in [FOLDER, TEXT, PICTURE] {
        let r = found(&tiles, argb).expect("a 48 px icon");
        assert_eq!((r.width, r.height), (48, 48));
    }
    let mut bare = dlg.clone();
    for e in bare.entries_mut() {
        e.detail.clear();
    }
    assert_ne!(drawn(&bare), tiles, "the kind and size lines are drawn");
    settle_thumbs(&mut h, &mut dlg, &mut icons);
    assert!(
        found(&drawn(&dlg), PICTURE).is_some(),
        "Tiles keeps the icon"
    );
    view(&mut h, &mut dlg, Mode::Thumbnails);
    let thumbnails = drawn(&dlg);
    let pic = found(&thumbnails, PIXELS).expect("the picture");
    assert_eq!((pic.width, pic.height), (40, 30), "at its own size");
    assert!(
        found(&thumbnails, PICTURE).is_none(),
        "the picture stands for its icon"
    );
    let r = found(&thumbnails, TEXT).expect("a 32 px icon");
    assert_eq!((r.width, r.height), (32, 32));
}

/// Calls [`Host::take_thumbs`] until `done` holds of the dialog, as the program's loop does on
/// each wake.
fn until(h: &mut Host, dlg: &mut FileDlg, icons: &mut IconCache, done: impl Fn(&FileDlg) -> bool) {
    for _ in 0..1000 {
        h.take_thumbs(dlg, icons);
        if done(dlg) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the dialog never caught up with the folder");
}

fn has(dlg: &FileDlg, name: &str) -> bool {
    dlg.entries().iter().any(|e| e.name == name)
}

#[test]
fn a_folder_made_or_removed_in_the_folder_shown_lists_without_a_refresh() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("sub")).unwrap();
    std::fs::write(t.path().join("a.txt"), "").unwrap();
    let mut h = host_in(t.path());
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = open_dlg(&mut h);
    h.thumbs_on(&crate::Waker::noop());
    h.ensure_listed(&mut dlg, &mut icons, &|_: &Item| true);
    assert!(dlg.select_named("sub"));

    std::fs::create_dir(t.path().join("made")).unwrap();
    until(&mut h, &mut dlg, &mut icons, |d| has(d, "made"));
    let picked: Vec<&str> = dlg.selected().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(picked, ["sub"], "the selection stays on its entry");

    std::fs::remove_file(t.path().join("a.txt")).unwrap();
    until(&mut h, &mut dlg, &mut icons, |d| !has(d, "a.txt"));
    std::fs::rename(t.path().join("made"), t.path().join("renamed")).unwrap();
    until(&mut h, &mut dlg, &mut icons, |d| {
        has(d, "renamed") && !has(d, "made")
    });
}

#[test]
fn the_folder_shown_renamed_lists_under_its_new_name() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("old");
    std::fs::create_dir_all(dir.join("inside")).unwrap();
    let mut h = host_in(&dir);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = open_dlg(&mut h);
    h.thumbs_on(&crate::Waker::noop());
    h.ensure_listed(&mut dlg, &mut icons, &|_: &Item| true);

    let old = dlg.folder();
    let new = t.path().join("new");
    std::fs::rename(&dir, &new).unwrap();
    until(&mut h, &mut dlg, &mut icons, |d| d.folder() != old);
    let shown = h.folders.path(dlg.folder().unwrap()).unwrap().to_path_buf();
    assert_eq!(shown, new);
    assert!(has(&dlg, "inside"));
}

fn press(sym: u32, ctrl: bool, shift: bool, utf8: Option<&str>) -> Event {
    Event::Key(crate::KeyInput {
        sym,
        utf8: utf8.map(str::to_string),
        mods: crate::Mods {
            ctrl,
            shift,
            alt: false,
        },
        edit: reprisekit::textedit::key_for(sym, ctrl, shift, utf8),
    })
}

/// [`Host::event`] with the test's renderer and theme, in a client its opening size.
fn send(h: &mut Host, dlg: &mut FileDlg, ev: &Event) -> Step {
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    let theme = Theme::default();
    h.event(dlg, ev, &text, &theme, CLIENT, &theme.metrics)
}

/// The Recycle Bins and `[recycle_bin]` settings a test deletes under: the home trash in `t`, no
/// other drive, `settings` in a config of its own. One test holds them at a time.
struct FakeBins(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

fn fake_bins(t: &Path, settings: &str) -> FakeBins {
    static HELD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    let config = t.join("config.toml");
    std::fs::write(&config, format!("[recycle_bin]\n{settings}")).unwrap();
    reprisefs::bincap::set_config_path(Some(config));
    reprisefs::trash::set_mount_table(Some(PathBuf::from("/dev/null")));
    reprisefs::trash::set_home_trash(Some(t.join("trash")));
    FakeBins(held)
}

impl Drop for FakeBins {
    fn drop(&mut self) {
        reprisefs::bincap::set_config_path(None);
        reprisefs::trash::set_home_trash(None);
        reprisefs::trash::set_mount_table(None);
    }
}

/// Takes the host's work off the loop until `done` holds.
fn settle(h: &mut Host, dlg: &mut FileDlg, icons: &mut IconCache, done: impl Fn(&Host) -> bool) {
    for _ in 0..1000 {
        h.take_thumbs(dlg, icons);
        if done(h) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the host never got there");
}

/// The box's title and text, once it is up. A box a step off the loop puts up tells the
/// runtime to look at the dialog's window again.
fn asked(h: &mut Host, dlg: &mut FileDlg, icons: &mut IconCache) -> (String, String) {
    for _ in 0..1000 {
        if h.asking().is_some() {
            break;
        }
        let woke = h.take_thumbs(dlg, icons);
        if h.asking().is_some() {
            assert!(
                woke,
                "the step that put the box up asks for the window again"
            );
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let m = h.asking().expect("a box");
    (m.title.clone(), m.fields.text.clone())
}

/// A press on the list's background, which takes the keyboard there.
fn focus_list(h: &mut Host, dlg: &mut FileDlg) {
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    dlg.draw(
        &mut Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32),
        &text,
        &Theme::default().paint(),
        Rect::new(0, 0, CLIENT.0, CLIENT.1),
        11,
    );
    let press = Event::Pointer(PointerInput::Press {
        x: 300,
        y: 300,
        button: crate::BTN_LEFT,
        time: 0,
        mods: crate::Mods::default(),
    });
    send(h, dlg, &press);
}

/// A key on the box over the dialog.
fn answer(h: &mut Host, sym: u32) -> Outcome {
    box_key(h, press(sym, false, false, None))
}

/// Alt+Y on the box over the dialog: Yes, whichever button Enter presses.
fn answer_yes(h: &mut Host) -> Outcome {
    let Event::Key(mut k) = press('y' as u32, false, false, Some("y")) else {
        unreachable!()
    };
    k.mods.alt = true;
    box_key(h, Event::Key(k))
}

fn box_key(h: &mut Host, ev: Event) -> Outcome {
    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    h.box_event(&ev, &text, &Theme::default())
}

#[test]
fn new_folder_rename_and_delete_work_on_the_folder_shown() {
    use reprisekit::textedit::keysym;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("docs");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "").unwrap();
    let _bins = fake_bins(t.path(), "");
    let mut h = host_in(&dir);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = open_dlg(&mut h);
    h.thumbs_on(&crate::Waker::noop());
    h.ensure_listed(&mut dlg, &mut icons, &|_: &Item| true);

    send(&mut h, &mut dlg, &press('N' as u32, true, true, Some("N")));
    assert!(
        dir.join("New Folder").is_dir(),
        "Ctrl+Shift+N makes the folder"
    );
    until(&mut h, &mut dlg, &mut icons, |d| {
        d.focused_edit().is_some_and(|e| e.text == "New Folder")
    });
    for ch in ["P", "a", "p", "e", "r", "s"] {
        send(
            &mut h,
            &mut dlg,
            &press(ch.chars().next().unwrap() as u32, false, false, Some(ch)),
        );
    }
    send(&mut h, &mut dlg, &press(keysym::RETURN, false, false, None));
    assert!(dir.join("Papers").is_dir(), "Enter renames it");
    until(&mut h, &mut dlg, &mut icons, |d| {
        d.selected().iter().map(|e| e.name.as_str()).eq(["Papers"])
    });

    send(&mut h, &mut dlg, &press(keysym::DELETE, false, false, None));
    assert_eq!(
        asked(&mut h, &mut dlg, &mut icons),
        (
            "Confirm Folder Delete".to_string(),
            "Are you sure you want to remove the folder 'Papers' and move all its contents to \
             the Recycle Bin?"
                .to_string()
        )
    );
    assert!(matches!(
        answer(&mut h, keysym::RETURN),
        Outcome::Closed(Some(0))
    ));
    settle(&mut h, &mut dlg, &mut icons, |_| {
        !dir.join("Papers").exists()
    });
    assert!(
        t.path().join("trash/files/Papers").is_dir(),
        "Yes moves it to the Recycle Bin"
    );
    until(&mut h, &mut dlg, &mut icons, |d| !has(d, "Papers"));

    assert!(dlg.select_named("a.txt"));
    send(&mut h, &mut dlg, &press(keysym::F2, false, false, None));
    send(&mut h, &mut dlg, &press(keysym::END, false, false, None));
    send(
        &mut h,
        &mut dlg,
        &press('/' as u32, false, false, Some("/")),
    );
    send(&mut h, &mut dlg, &press(keysym::RETURN, false, false, None));
    assert_eq!(
        dlg.focused_edit().map(|e| e.text.as_str()),
        Some("a.txt/"),
        "a refused name comes back in the box"
    );
    let (title, text) = asked(&mut h, &mut dlg, &mut icons);
    assert_eq!(title, "Rename");
    assert!(text.starts_with("A file name cannot contain"), "{text}");
    assert!(dir.join("a.txt").is_file());
}

#[test]
fn shift_delete_asks_then_deletes_for_good_and_a_failure_says_why() {
    use reprisekit::textedit::keysym;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("docs");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    let _bins = fake_bins(t.path(), "");
    let mut h = host_in(&dir);
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = open_dlg(&mut h);
    h.thumbs_on(&crate::Waker::noop());
    h.ensure_listed(&mut dlg, &mut icons, &|_: &Item| true);
    let shift_delete = press(keysym::DELETE, false, true, None);
    focus_list(&mut h, &mut dlg);

    assert!(dlg.select_named("a.txt"));
    send(&mut h, &mut dlg, &shift_delete);
    assert_eq!(
        asked(&mut h, &mut dlg, &mut icons),
        (
            "Confirm File Delete".to_string(),
            "Are you sure you want to delete 'a.txt'?".to_string()
        ),
        "Shift+Delete asks to delete for good"
    );
    assert_eq!(answer(&mut h, keysym::ESCAPE), Outcome::Closed(None));
    assert!(h.asking().is_none());
    assert!(dir.join("a.txt").is_file(), "No keeps it");

    send(&mut h, &mut dlg, &shift_delete);
    asked(&mut h, &mut dlg, &mut icons);
    assert_eq!(answer(&mut h, keysym::RETURN), Outcome::Closed(Some(1)));
    assert!(dir.join("a.txt").is_file(), "Enter presses No and keeps it");

    send(&mut h, &mut dlg, &shift_delete);
    asked(&mut h, &mut dlg, &mut icons);
    answer_yes(&mut h);
    settle(&mut h, &mut dlg, &mut icons, |_| {
        !dir.join("a.txt").exists()
    });
    assert!(
        !t.path().join("trash/files/a.txt").exists(),
        "Yes deletes it for good"
    );

    assert!(dlg.select_named("b.txt"));
    send(&mut h, &mut dlg, &shift_delete);
    asked(&mut h, &mut dlg, &mut icons);
    std::fs::remove_file(dir.join("b.txt")).unwrap();
    answer_yes(&mut h);
    let (title, text) = asked(&mut h, &mut dlg, &mut icons);
    assert_eq!(title, "Error Deleting File or Folder");
    assert!(text.starts_with("Cannot delete b.txt: "), "{text}");
}

#[test]
fn delete_asks_before_deleting_for_good_what_the_recycle_bin_cannot_take() {
    use reprisekit::textedit::keysym;
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("big.iso"), "big").unwrap();
    std::fs::write(t.path().join("now.txt"), "now").unwrap();
    let _bins = fake_bins(t.path(), "confirm = false\nmax_percent = 0\n");
    let mut h = host_in(t.path());
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = open_dlg(&mut h);
    h.thumbs_on(&crate::Waker::noop());
    h.ensure_listed(&mut dlg, &mut icons, &|_: &Item| true);
    let delete = press(keysym::DELETE, false, false, None);
    focus_list(&mut h, &mut dlg);

    assert!(dlg.select_named("big.iso"));
    send(&mut h, &mut dlg, &delete);
    assert_eq!(
        asked(&mut h, &mut dlg, &mut icons).1,
        "'big.iso' is too big for the Recycle Bin. Do you want to delete it permanently?"
    );
    answer(&mut h, keysym::ESCAPE);
    assert!(t.path().join("big.iso").is_file(), "Escape keeps it");
    send(&mut h, &mut dlg, &delete);
    asked(&mut h, &mut dlg, &mut icons);
    answer(&mut h, keysym::RETURN);
    settle(&mut h, &mut dlg, &mut icons, |h| h.deleting.is_none());
    assert!(
        t.path().join("big.iso").is_file(),
        "Enter presses No and keeps it"
    );
    send(&mut h, &mut dlg, &delete);
    asked(&mut h, &mut dlg, &mut icons);
    answer_yes(&mut h);
    settle(&mut h, &mut dlg, &mut icons, |_| {
        !t.path().join("big.iso").exists()
    });
    assert!(!t.path().join("trash/files/big.iso").exists());

    std::fs::write(
        t.path().join("config.toml"),
        "[recycle_bin]\nremove_immediately = true\n",
    )
    .unwrap();
    assert!(dlg.select_named("now.txt"));
    send(&mut h, &mut dlg, &delete);
    assert_eq!(
        asked(&mut h, &mut dlg, &mut icons).1,
        "Are you sure you want to delete 'now.txt'?",
        "a bin set to delete at once asks to delete for good"
    );
}

#[test]
fn f5_and_ctrl_r_read_the_folder_again_and_a_right_press_opens_the_list_menu() {
    use reprisekit::textedit::keysym;
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("a.txt"), "").unwrap();
    let mut h = host_in(t.path());
    let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
    let mut dlg = open_dlg(&mut h);
    h.ensure_listed(&mut dlg, &mut icons, &|_: &Item| true);
    std::fs::write(t.path().join("b.txt"), "").unwrap();
    for key in [
        press(keysym::F5, false, false, None),
        press('r' as u32, true, false, Some("r")),
    ] {
        send(&mut h, &mut dlg, &key);
        assert!(h.reading.is_some(), "{key:?} reads the folder again");
        for _ in 0..1000 {
            h.take_thumbs(&mut dlg, &mut icons);
            if h.reading.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(has(&dlg, "b.txt"));
    }

    let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
    dlg.draw(
        &mut Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32),
        &text,
        &Theme::default().paint(),
        Rect::new(0, 0, CLIENT.0, CLIENT.1),
        11,
    );
    let (x, y) = (300, 300);
    let right = Event::Pointer(PointerInput::Press {
        x,
        y,
        button: crate::BTN_RIGHT,
        time: 0,
        mods: crate::Mods::default(),
    });
    assert!(matches!(send(&mut h, &mut dlg, &right), Step::Redraw));
    let popped = reprisekit::controls::filedlg::Popped {
        dlg: &mut dlg,
        text: std::rc::Rc::new(text),
        px: 11,
    };
    use reprisekit::controls::hosted::Hosted;
    assert_eq!(popped.spots().len(), 1, "the background's menu is up");
}
