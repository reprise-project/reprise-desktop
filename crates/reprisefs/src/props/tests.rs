use super::*;

#[test]
fn a_stopped_walk_gives_up() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a"), "1234").expect("write");
    let go = AtomicBool::new(false);
    assert_eq!(
        folder_totals_unless(dir.path(), &go).map(|t| (t.files, t.size)),
        Some((1, 4))
    );
    let stop = AtomicBool::new(true);
    assert_eq!(folder_totals_unless(dir.path(), &stop), None);
}

#[test]
fn short_sizes_drop_the_byte_count() {
    assert_eq!(short_size(21_365_735_424), "19.8 GB");
    assert_eq!(short_size(1260), "1.23 KB");
    assert_eq!(short_size(512), "512 bytes");
}

#[test]
fn sizes_and_counts_read_as_byte_counts() {
    assert_eq!(format_size(0), "0 bytes");
    assert_eq!(format_size(1023), "1023 bytes");
    assert_eq!(format_size(1260), "1.23 KB (1,260 bytes)");
    assert_eq!(format_size(12_800), "12.5 KB (12,800 bytes)");
    assert_eq!(format_size(512_000), "500 KB (512,000 bytes)");
    assert_eq!(format_size(1_572_864), "1.50 MB (1,572,864 bytes)");
    assert_eq!(format_size(3 << 30), "3.00 GB (3,221,225,472 bytes)");
    assert_eq!(format_size(6_500), "6.34 KB (6,500 bytes)");
    assert_eq!(format_size(150_000), "146 KB (150,000 bytes)");
    assert_eq!(format_size(1_234_567), "1.17 MB (1,234,567 bytes)");
    assert_eq!(format_size(12_345_678), "11.7 MB (12,345,678 bytes)");
    assert_eq!(format_size(13_738_771), "13.1 MB (13,738,771 bytes)");
    assert_eq!(format_contains(1, 0), "1 File, 0 Folders");
    assert_eq!(format_contains(1200, 1), "1,200 Files, 1 Folder");
}

#[test]
fn hidden_renames_a_name_that_is_not_utf8_by_its_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let t = tempfile::tempdir().unwrap();
    let plain = t.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
    let dotted = t.path().join(std::ffi::OsStr::from_bytes(b".caf\xe9.txt"));
    std::fs::write(&plain, "x").unwrap();
    let w = write_set(vec![plain.clone()], AttrMoves::default(), Some(true), false);
    assert_eq!(w.now, std::slice::from_ref(&dotted));
    assert!(dotted.exists());
    let w = write_set(
        vec![dotted.clone()],
        AttrMoves::default(),
        Some(false),
        false,
    );
    assert_eq!(w.now, std::slice::from_ref(&plain));
    assert!(plain.exists());
}

#[test]
fn hiding_adds_one_dot_and_showing_takes_it() {
    assert_eq!(hidden_name("notes.txt".as_ref(), true), ".notes.txt");
    assert_eq!(hidden_name(".notes.txt".as_ref(), true), ".notes.txt");
    assert_eq!(hidden_name(".notes.txt".as_ref(), false), "notes.txt");
    assert_eq!(hidden_name("notes.txt".as_ref(), false), "notes.txt");
}

#[test]
fn thousands_groups_by_threes_from_the_right() {
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(999), "999");
    assert_eq!(thousands(1_000), "1,000");
    assert_eq!(thousands(361_672), "361,672");
    assert_eq!(thousands(2_097_152), "2,097,152");
    assert_eq!(thousands(1_234_567_890), "1,234,567,890");
}

#[test]
fn type_names_and_defaults_come_from_mime_data() {
    assert_eq!(
        comment_of("<mime-type>\n  <comment>Plain text document</comment>\n  <comment xml:lang=\"uk\">x</comment>\n").as_deref(),
        Some("Plain text document")
    );
    assert_eq!(generic_type_name(Path::new("/x/a.tar.gz")), "GZ File");
    assert_eq!(generic_type_name(Path::new("/x/Makefile")), "File");
    let list = "[Added Associations]\ntext/plain=nvim.desktop;\n[Default Applications]\ntext/plain=micro.desktop;vim.desktop\nimage/png=\n";
    assert_eq!(
        default_in_list(list, "text/plain").as_deref(),
        Some("micro.desktop")
    );
    assert_eq!(default_in_list(list, "image/png"), None);
    assert_eq!(default_in_list(list, "text/html"), None);
    assert_eq!(verbs(Some("text/plain"), true), (true, true));
    assert_eq!(verbs(Some("text/x-rust"), false), (false, true));
    assert_eq!(verbs(Some("image/png"), true), (true, false));
    assert_eq!(verbs(Some("application/pdf"), true), (true, false));
    assert_eq!(verbs(Some("application/zip"), true), (false, false));
    assert_eq!(verbs(None, true), (false, false));
    let list = "[Added Associations]\ntext/plain=nvim.desktop;\n[Default Applications]\ntext/plain=micro.desktop;vim.desktop\nimage/png=eog.desktop\n";
    let out = with_default(list, "text/plain", "vim.desktop");
    assert_eq!(
        default_in_list(&out, "text/plain").as_deref(),
        Some("vim.desktop")
    );
    assert_eq!(
        default_in_list(&out, "image/png").as_deref(),
        Some("eog.desktop")
    );
    assert_eq!(
        out.matches("text/plain=").count(),
        2,
        "the association stays"
    );
    let out = with_default(list, "text/html", "firefox.desktop");
    assert_eq!(
        default_in_list(&out, "text/html").as_deref(),
        Some("firefox.desktop")
    );
    let out = with_default("", "text/html", "firefox.desktop");
    assert!(out.starts_with("[Default Applications]\ntext/html=firefox.desktop\n"));
}

#[test]
fn edits_keep_the_rest_of_the_list() {
    let list = "# mine\n\
                [Added Associations]\n\
                text/plain=nvim.desktop;\n\
                \n\
                [Default Applications]\n\
                image/png=eog.desktop\n\
                text/plain=micro.desktop\n\
                \n\
                [X-Unknown Section]\n\
                some=thing\n\
                [Removed Associations]\n\
                text/plain=gedit.desktop;\n";
    let edits = |e: &[(&str, Option<&str>)]| -> Vec<(String, Option<String>)> {
        e.iter()
            .map(|(m, id)| (m.to_string(), id.map(String::from)))
            .collect()
    };
    let set = with_edits(
        list,
        &edits(&[
            ("text/plain", Some("zed.desktop")),
            ("x-scheme-handler/https", Some("firefox.desktop")),
        ]),
    );
    assert_eq!(
        set,
        list.replace("text/plain=micro.desktop", "text/plain=zed.desktop")
            .replace(
                "\n\n[X-Unknown",
                "\nx-scheme-handler/https=firefox.desktop\n\n[X-Unknown"
            ),
        "replaced in place, the new type added at the section's end"
    );
    let dropped = with_edits(
        &set,
        &edits(&[
            ("x-scheme-handler/https", None),
            ("text/plain", Some("micro.desktop")),
        ]),
    );
    assert_eq!(dropped, list, "the round trip gives the file back");
    let restored = without_default(list, "text/plain");
    assert_eq!(default_in_list(&restored, "text/plain"), None);
    assert_eq!(
        restored
            .lines()
            .filter(|l| l.starts_with("text/plain="))
            .count(),
        2,
        "the added and removed associations stay"
    );
    assert_eq!(without_default("", "text/plain"), "");
}

/// A caller that knows the path is a file gets its program without a stat, which a file on a
/// mount that does not answer would hold: even a folder by that name opens as a file.
#[test]
fn open_file_asks_the_file_system_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let viewer = AppEntry {
        exec_line: "viewer %F".into(),
        ..app("viewer.desktop", &["image/svg+xml"])
    };
    let open = |d| open_file(dir.path(), d, "foot");
    assert_eq!(
        open(Some(&viewer)),
        Opening {
            argv: vec!["viewer".into(), dir.path().as_os_str().to_owned()],
            app: Some("viewer.desktop".into()),
        },
        "in the program's app scope"
    );
    assert_eq!(open(None).argv[0], "xdg-open");
    assert_eq!(open(None).app, None);
}

#[test]
fn open_in_looks_up_the_types_default_program_and_runs_as_it() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("song.rtest");
    let other = dir.path().join("song.rtother");
    let db = apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
    let player = AppEntry {
        exec_line: "player %f".into(),
        ..app("player.desktop", &["application/x-reprise-test"])
    };
    let open = |p: &Path| open_in(p, &db, std::slice::from_ref(&player), "foot");
    assert_eq!(
        open(&song),
        Opening {
            argv: vec!["player".into(), song.clone().into_os_string()],
            app: Some("player.desktop".into()),
        }
    );
    assert_eq!(
        open(&other),
        Opening {
            argv: vec!["xdg-open".into(), other.into_os_string()],
            app: None,
        }
    );
    let folder = open(dir.path());
    assert_eq!(folder.argv[0], "reprise-explorer");
    assert_eq!(folder.app, None);
}

fn app(id: &str, mimes: &[&str]) -> AppEntry {
    AppEntry {
        name: id.into(),
        exec_line: String::new(),
        terminal: false,
        id: id.into(),
        icon: None,
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: mimes.iter().map(|m| m.to_string()).collect(),
        shown: true,
    }
}

fn pick<'a>(lists: &[&str], tree: &MimeTree, mime: &str, apps: &'a [AppEntry]) -> Option<&'a str> {
    let lists: Vec<String> = lists.iter().map(|s| s.to_string()).collect();
    default_in_lists(&lists, tree, mime, apps).map(|i| apps[i].id.as_str())
}

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

#[test]
fn a_type_s_own_programs_come_before_a_parent_s_default() {
    let tree = MimeTree::parse(
        ["application/x-pdf application/pdf\n"],
        [format!(
            "image/svg+xml application/xml\n\
             application/xml text/plain\n\
             {DOCX} application/zip\n\
             application/epub+zip application/zip\n\
             application/x-foo application/zip\n"
        )
        .as_str()],
    );
    let apps = vec![
        app("notepad.desktop", &["text/plain"]),
        app("viewer.desktop", &["image/svg+xml"]),
        app("ark.desktop", &["application/zip"]),
        app("writer.desktop", &[DOCX]),
        app("reader.desktop", &["application/epub+zip"]),
        app("inkscape.desktop", &[]),
        app("evince.desktop", &["application/x-pdf"]),
    ];
    let system =
        "[Default Applications]\napplication/xml=notepad.desktop\ntext/plain=notepad.desktop\n";
    let user = "[Default Applications]\napplication/zip=ark.desktop\n";
    let lists = [user, system];
    let at = |mime| pick(&lists, &tree, mime, &apps);
    assert_eq!(at("image/svg+xml"), Some("viewer.desktop"));
    assert_eq!(at("application/xml"), Some("notepad.desktop"));
    assert_eq!(at(DOCX), Some("writer.desktop"));
    assert_eq!(at("application/epub+zip"), Some("reader.desktop"));
    assert_eq!(
        at("application/x-foo"),
        Some("ark.desktop"),
        "a type with no program of its own takes its parent's default"
    );
    assert_eq!(
        at("application/pdf"),
        Some("evince.desktop"),
        "a program declaring an alias handles the type"
    );
    let added = "[Added Associations]\nimage/svg+xml=gone.desktop;inkscape.desktop;\n";
    assert_eq!(
        pick(&[added, user, system], &tree, "image/svg+xml", &apps),
        Some("inkscape.desktop"),
        "an added association comes before a declared type"
    );
    let by_alias = "[Default Applications]\napplication/x-pdf=notepad.desktop\n";
    assert_eq!(
        pick(&[by_alias], &tree, "application/pdf", &apps),
        Some("notepad.desktop"),
        "a default set under an alias beats the type's programs"
    );
}

#[test]
fn a_type_with_no_program_of_its_own_takes_its_parent_s_default() {
    let tree = MimeTree::parse([], ["text/x-python application/x-executable\n"]);
    let apps = vec![
        app("idle.desktop", &["text/x-python"]),
        app("mousepad.desktop", &["text/plain"]),
        app("zed.desktop", &[]),
    ];
    let user = "[Default Applications]\ntext/plain=zed.desktop\n";
    let gone = "[Default Applications]\ntext/x-python=gone.desktop\n";
    let system = "[Default Applications]\ntext/x-python=mousepad.desktop\n";
    assert_eq!(
        pick(&[user], &tree, "text/x-python", &apps[1..]),
        Some("zed.desktop"),
        "with no program of its own, the text/plain default"
    );
    assert_eq!(
        pick(&[user], &tree, "text/x-python", &apps),
        Some("idle.desktop"),
        "a program declaring the type beats the text/plain default"
    );
    assert_eq!(
        pick(&[gone, user, system], &tree, "text/x-python", &apps),
        Some("mousepad.desktop"),
        "the type's own default wins, skipping an id that is not installed"
    );
    assert_eq!(
        pick(&[], &tree, "text/x-python", &apps[1..]),
        Some("mousepad.desktop"),
        "without a default, a text/plain program"
    );
    assert_eq!(pick(&[user], &tree, "image/png", &apps), None);
}

#[test]
fn inspect_reads_a_file_and_a_folder() {
    let base = std::env::temp_dir().join(format!("reprise-props-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let sub = base.join("Reports").join("2026");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(base.join("Reports").join("a.txt"), "hello").unwrap();
    std::fs::write(sub.join("b.txt"), "hello world").unwrap();
    let notes = base.join(".notes.txt");
    std::fs::write(&notes, "12345").unwrap();
    let f = inspect(&notes, ".notes.txt", &[]).unwrap();
    assert!(!f.is_dir && f.hidden && !f.listed && !f.read_only);
    std::fs::write(base.join(".hidden"), "Reports\n").unwrap();
    let listed = glance(&base.join("Reports"), "Reports", &[]).unwrap();
    assert!(listed.hidden && listed.listed);
    std::fs::remove_file(base.join(".hidden")).unwrap();
    assert_eq!(f.size, 5);
    assert!(f.on_disk >= 5 || f.on_disk == 0);
    assert_eq!(f.location, base.to_string_lossy());
    assert!(f.contains.is_none() && f.modified > 0);
    let d = inspect(&base.join("Reports"), "Reports", &[]).unwrap();
    assert!(d.is_dir);
    assert_eq!(d.type_name, "File Folder");
    assert_eq!(d.contains, Some((2, 1)));
    assert_eq!(d.size, 16);
    assert!(d.opens_with.is_none());
    let link = base.join("Shortcut to Reports");
    std::os::unix::fs::symlink(base.join("Reports"), &link).unwrap();
    let l = inspect(&link, "Shortcut to Reports", &[]).unwrap();
    assert_eq!(l.type_name, "Shortcut");
    std::fs::remove_dir_all(&base).unwrap();
    assert!(!l.is_dir && l.contains.is_none());
    assert_eq!(l.size, base.join("Reports").as_os_str().len() as u64);
    assert_eq!(l.description.as_deref(), Some("Reports"));
    let sc = l.launcher.expect("a symlink's keys");
    assert_eq!(sc.kind, crate::launcher::Kind::Symlink);
    assert_eq!(sc.target_type, "File Folder");
}

#[test]
fn a_web_link_is_an_internet_shortcut() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ex.desktop");
    crate::apps::write_launcher(
        &path,
        &crate::apps::link_text("Ex", "https://example.org/").unwrap(),
    )
    .unwrap();
    let f = glance(&path, "Ex", &[]).unwrap();
    assert_eq!(f.type_name, "Internet Shortcut");
    assert_eq!(f.description.as_deref(), Some("Ex"));
    assert!(f.is_shortcut && f.opens_with.is_none());
    let l = f.launcher.expect("a web link's keys");
    assert_eq!(l.kind, crate::launcher::Kind::Web);
    assert_eq!(l.fields()[0], "https://example.org/");
}

#[test]
fn an_untrusted_launcher_is_the_plain_file_it_lists_as() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let app = "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\n";
    let plain = dir.path().join("plain.desktop");
    std::fs::write(&plain, app).unwrap();
    let trusted = dir.path().join("app.desktop");
    crate::apps::write_launcher(&trusted, app).unwrap();
    let db = MimeDb::empty();
    let listed = |p: &Path| crate::listing::item_at(p, &db).kind;
    for mode in [0o644, 0o755] {
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(mode)).unwrap();
        let f = glance_in(&plain, "plain.desktop", &[], &db).unwrap();
        assert!(!f.is_shortcut, "mode {mode:o}");
        assert_eq!((f.launcher, f.description), (None, None));
        assert_eq!(f.type_name, listed(&plain));
        assert_ne!(f.type_name, crate::listing::SHORTCUT);
    }
    let f = glance_in(&trusted, "Terminal", &[], &db).unwrap();
    assert!(f.is_shortcut && f.launcher.is_some());
    assert_eq!(f.type_name, listed(&trusted));
}

#[test]
fn a_set_adds_up_its_items_and_names_what_they_share() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    std::fs::write(base.join("a.txt"), "12345678").unwrap();
    std::fs::write(base.join("b.txt"), "1234").unwrap();
    std::fs::create_dir_all(base.join("Reports/old")).unwrap();
    std::fs::write(base.join("Reports/x.txt"), "123").unwrap();
    let a = inspect(&base.join("a.txt"), "a", &[]).unwrap();
    let b = inspect(&base.join("b.txt"), "b", &[]).unwrap();
    let files = glance_set(&[a.clone(), b.clone()]);
    assert_eq!((files.files, files.folders, files.size), (2, 0, 12));
    assert!(!files.has_dir && !files.walking);
    assert_eq!(files.count_text(), "2 Files, 0 Folders");
    assert_eq!(files.type_text(), format!("All of type {}", a.type_name));
    let loc = base.to_string_lossy();
    assert_eq!(files.location_text(), format!("All in {loc}"));
    assert_eq!((files.read_only, files.hidden), (Some(false), Some(false)));

    let reports = glance(&base.join("Reports"), "Reports", &[]).unwrap();
    let mut mixed = glance_set(&[a.clone(), reports.clone()]);
    assert!(mixed.has_dir && mixed.walking);
    assert_eq!((mixed.files, mixed.folders), (1, 1));
    mixed.add_totals(folder_totals(&base.join("Reports")));
    assert!(!mixed.walking);
    assert_eq!(mixed.count_text(), "2 Files, 2 Folders");
    assert_eq!(mixed.size, 11);
    assert_eq!(mixed.type_text(), "Multiple Types");
    let walked = inspect(&base.join("Reports"), "Reports", &[]).unwrap();
    assert_eq!(
        glance_set(&[a.clone(), walked]),
        mixed,
        "inspect walks up front"
    );

    let x = inspect(&base.join("Reports/x.txt"), "x", &[]).unwrap();
    assert_eq!(
        glance_set(&[a.clone(), x]).location_text(),
        "Various Folders"
    );

    set_read_only(&base.join("b.txt"), true).unwrap();
    let b = glance(&base.join("b.txt"), "b", &[]).unwrap();
    assert_eq!(glance_set(&[a.clone(), b.clone()]).read_only, None);

    if a.robots.is_some() {
        assert_eq!(
            glance_set(&[a.clone(), b.clone()]).robots,
            Some([Some(true), Some(true)])
        );
        crate::meta::set_robots(&base.join("b.txt"), crate::meta::BACKUP, false).unwrap();
        let b = glance(&base.join("b.txt"), "b", &[]).unwrap();
        assert_eq!(glance_set(&[a, b]).robots, Some([None, Some(true)]));
    }
}

#[test]
fn attribute_changes_read_as_one_line() {
    let moves = AttrMoves {
        read_only: Some(true),
        backup: None,
        index: Some(false),
        ..Default::default()
    };
    assert_eq!(moves.verbs(Some(true)), "set read-only, hide, unset index");
    assert!(AttrMoves::default().is_empty());
}

#[test]
fn glance_in_types_the_file_from_the_given_tables() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("song.rtest");
    std::fs::write(&file, "x").unwrap();
    let db = apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
    let player = AppEntry {
        name: "Player".into(),
        exec_line: "player %f".into(),
        terminal: false,
        id: "player.desktop".into(),
        icon: None,
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: vec!["application/x-reprise-test".into()],
        shown: true,
    };
    let f = glance_in(&file, "song", &[player], &db).unwrap();
    assert_eq!(f.opens_with.as_deref(), Some("Player"));
    assert_eq!(f.type_name, "RTEST File");
}

#[test]
fn a_symlink_to_a_document_shows_what_opens_its_target() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("song.rtest"), "x").unwrap();
    let link = dir.path().join("Song");
    std::os::unix::fs::symlink("song.rtest", &link).unwrap();
    let db = apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
    let player = AppEntry {
        name: "Player".into(),
        exec_line: "player %f".into(),
        terminal: false,
        id: "player.desktop".into(),
        icon: Some("audio-player".into()),
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: vec!["application/x-reprise-test".into()],
        shown: true,
    };
    let f = glance_in(&link, "Song", &[player], &db).unwrap();
    assert_eq!(f.type_name, "Shortcut");
    assert_eq!(f.opens_with.as_deref(), Some("Player"));
    assert_eq!(f.opens_with_icon.as_deref(), Some("audio-player"));
    assert_eq!(f.description, None);
    let l = f.launcher.expect("a symlink's keys");
    assert_eq!(l.kind, crate::launcher::Kind::Symlink);
    assert_eq!(l.target_type, "RTEST File");
    let bare = glance_in(&link, "Song", &[], &db).unwrap();
    assert_eq!(bare.opens_with, None);
    assert_eq!(bare.description.as_deref(), Some("song.rtest"));
}

/// Open runs an executable script, so its sheet names the script instead of the editor its type
/// opens in; the same script without the execute bit opens there.
#[test]
fn an_executable_script_opens_with_nothing_and_is_described_by_its_name() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("hi.sh");
    std::fs::write(&f, "#!/bin/sh\n").unwrap();
    let db = apps::MimeDb::parse(["50:application/x-shellscript:*.sh\n"]);
    let editor = AppEntry {
        name: "Editor".into(),
        exec_line: "editor %f".into(),
        terminal: false,
        id: "editor.desktop".into(),
        icon: Some("text-editor".into()),
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: vec!["application/x-shellscript".into()],
        shown: true,
    };
    let apps = [editor];
    std::fs::set_permissions(&f, PermissionsExt::from_mode(0o644)).unwrap();
    let plain = glance_in(&f, "hi", &apps, &db).unwrap();
    assert_eq!(plain.opens_with.as_deref(), Some("Editor"));
    assert_eq!(plain.description, None);
    std::fs::set_permissions(&f, PermissionsExt::from_mode(0o755)).unwrap();
    let program = glance_in(&f, "hi", &apps, &db).unwrap();
    assert_eq!(program.opens_with, None);
    assert_eq!(program.opens_with_icon, None);
    assert_eq!(program.description.as_deref(), Some("hi"));
}

#[test]
fn read_only_clears_and_restores_the_write_bits() {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("reprise-attrs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let f = base.join("a.txt");
    std::fs::write(&f, "x").unwrap();
    std::fs::set_permissions(&f, PermissionsExt::from_mode(0o664)).unwrap();
    set_read_only(&f, true).unwrap();
    let mode = || std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(), 0o444);
    assert!(inspect(&f, "a", &[]).unwrap().read_only);
    set_read_only(&f, false).unwrap();
    assert_eq!(mode(), 0o644, "only the owner's write bit comes back");
    assert!(!inspect(&f, "a", &[]).unwrap().read_only);
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn executable_sets_execute_where_a_class_can_read_and_clears_all() {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("reprise-exec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let f = base.join("hi.sh");
    std::fs::write(&f, "#!/bin/sh\n").unwrap();
    let mode = || std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
    for (from, want) in [
        (0o644, 0o755),
        (0o640, 0o750),
        (0o600, 0o700),
        (0o604, 0o705),
    ] {
        std::fs::set_permissions(&f, PermissionsExt::from_mode(from)).unwrap();
        let info = inspect(&f, "hi", &[]).unwrap();
        assert!(!info.executable && info.can_set_executable, "{from:o}");
        set_executable(&f, true).unwrap();
        assert_eq!(mode(), want, "{from:o} made executable");
        assert!(inspect(&f, "hi", &[]).unwrap().executable);
        set_executable(&f, false).unwrap();
        assert_eq!(mode(), from, "{want:o} made not executable");
    }
    let moves = AttrMoves {
        executable: Some(true),
        ..Default::default()
    };
    write_item(&f, &moves, None, false).unwrap();
    assert_eq!(mode(), 0o705, "write_item writes the Executable move");
    let folder = inspect(&base, "base", &[]).unwrap();
    assert!(!folder.executable && !folder.can_set_executable, "a folder");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_executable_box_greys_off_the_users_files_and_on_mounts_that_cannot_take_it() {
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    let mine = Mount::default();
    assert!(can_set_executable(me, mine));
    if me != 0 {
        assert!(!can_set_executable(me + 1, mine), "another user's file");
    }
    for mount in [
        Mount {
            read_only: true,
            ..mine
        },
        Mount {
            noexec: true,
            ..mine
        },
        Mount {
            fixed_modes: true,
            ..mine
        },
    ] {
        assert!(!can_set_executable(me, mount), "{mount:?}");
    }
    assert!(
        !Mount::of(&std::env::temp_dir()).fixed_modes,
        "the temp folder keeps modes"
    );
}

#[test]
fn a_fuseblk_mount_is_found_by_its_device_with_its_source_and_mount_point() {
    let info = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
40 22 7:0 / /run/media/u/My\\040Disk rw,relatime shared:20 - fuseblk /dev/loop0 rw,user_id=0
42 22 0:54 / /srv/ssh rw,relatime shared:22 - fuse.sshfs h:/ rw,user_id=1000,group_id=1000
";
    assert_eq!(
        fuseblk_mount(info, 7, 0),
        Some(("/dev/loop0".into(), "/run/media/u/My Disk".into()))
    );
    assert_eq!(fuseblk_mount(info, 0, 54), None, "sshfs");
    assert_eq!(fuseblk_mount(info, 259, 2), None, "ext4");
    assert_eq!(fuseblk_mount(info, 0, 99), None, "a device not listed");
}

#[test]
fn an_ntfs3g_mount_keeps_modes_only_when_asked_for_permissions_or_acl() {
    let (src, dst) = ("/dev/sdb1", "/srv/disk");
    for (args, keeps) in [
        (&["ntfs-3g", "-o", "uid=1000,gid=1000", src, dst][..], false),
        (&["ntfs-3g", "-o", "rw,permissions", src, dst][..], true),
        (&["mount.ntfs", src, dst, "-o", "acl"][..], true),
        (&["ntfs-3g", "-opermissions", src, dst][..], true),
        (
            &["ntfs-3g", "-o", "permissions", src, "/srv/other"][..],
            false,
        ),
        (
            &["ntfs-3g", "-o", "permissions", "/dev/sdc1", dst][..],
            false,
        ),
    ] {
        assert_eq!(keeps_modes(args, src, dst), keeps, "{args:?}");
    }
}

#[test]
fn attr_verbs_list_read_only_then_hidden() {
    assert_eq!(attr_verbs(Some(true), None), "set read-only");
    assert_eq!(attr_verbs(None, Some(true)), "hide");
    assert_eq!(attr_verbs(None, Some(false)), "unhide");
    assert_eq!(attr_verbs(Some(false), Some(true)), "unset read-only, hide");
}

#[test]
fn the_hidden_tree_lists_names_and_never_renames() {
    let base = tempfile::tempdir().unwrap();
    let tree = base.path().join("tree");
    let outside = base.path().join("outside");
    let deep = tree.join("sub").join("deep");
    let git = tree.join(".git");
    for d in [&deep, &git, &outside] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(tree.join("a.txt"), "x").unwrap();
    std::fs::write(tree.join(".config"), "x").unwrap();
    std::fs::write(deep.join("b.txt"), "x").unwrap();
    std::fs::write(git.join("HEAD"), "x").unwrap();
    std::fs::write(outside.join("d.txt"), "x").unwrap();
    std::fs::write(tree.join("sub").join(".hidden"), "keep me\n").unwrap();
    std::os::unix::fs::symlink(&outside, tree.join("link")).unwrap();
    let list = |d: &Path| std::fs::read_to_string(d.join(".hidden")).ok();

    set_hidden_tree(&tree, true).unwrap();
    assert_eq!(list(&tree).as_deref(), Some("a.txt\nlink\nsub\n"));
    assert_eq!(list(&tree.join("sub")).as_deref(), Some("keep me\ndeep\n"));
    assert_eq!(list(&deep).as_deref(), Some("b.txt\n"));
    assert_eq!(list(&git), None, "a dotted folder is not entered");
    assert_eq!(list(&outside), None, "a linked folder is not entered");
    assert!(crate::hidden::is_hidden(&deep.join("b.txt")));
    assert!(
        !crate::hidden::is_hidden(&tree),
        "the folder is left to its rename"
    );
    set_hidden_tree(&tree, true).unwrap();
    assert_eq!(
        list(&deep).as_deref(),
        Some("b.txt\n"),
        "hiding twice lists once"
    );

    set_hidden_tree(&tree, false).unwrap();
    assert_eq!(list(&tree), None);
    assert_eq!(list(&deep), None);
    assert_eq!(list(&tree.join("sub")).as_deref(), Some("keep me\n"));
    for p in [tree.join(".config"), git.join("HEAD"), deep.join("b.txt")] {
        assert!(p.exists(), "{} keeps its name", p.display());
    }
    assert_eq!(std::fs::read_dir(&git).unwrap().count(), 1);

    let file = base.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    set_hidden_tree(&file, true).unwrap();
    assert_eq!(list(base.path()), None, "a file has nothing under it");
}

#[test]
fn read_only_and_hidden_land_together_down_a_tree() {
    let base = tempfile::tempdir().unwrap();
    let tree = base.path().join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("sub").join("a.txt"), "x").unwrap();
    let list = |d: &Path| std::fs::read_to_string(d.join(".hidden")).ok();
    let lock = AttrMoves {
        read_only: Some(true),
        ..Default::default()
    };
    let written = write_item(&tree, &lock, Some(true), true);
    let listed = (list(&tree), list(&tree.join("sub")));
    let locked = std::fs::metadata(&tree).unwrap().permissions().readonly();
    let again = write_item(&tree, &AttrMoves::default(), Some(false), true);
    set_read_only_tree(&tree, false).unwrap();
    written.unwrap();
    assert_eq!(listed.0.as_deref(), Some("sub\n"));
    assert_eq!(listed.1.as_deref(), Some("a.txt\n"));
    assert!(locked, "the folder is read-only after");
    assert!(again.is_err(), "a read-only folder's list is not written");
}

#[test]
fn the_read_only_tree_leaves_symlinked_files_and_folders_alone() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let tree = base.path().join("tree");
    let outside = base.path().join("outside");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(tree.join("sub").join("a.txt"), "x").unwrap();
    std::fs::write(outside.join("b.txt"), "x").unwrap();
    std::os::unix::fs::symlink(outside.join("b.txt"), tree.join("b-link")).unwrap();
    std::os::unix::fs::symlink(&outside, tree.join("sub").join("outside-link")).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    for p in [&tree, &tree.join("sub"), &outside] {
        std::fs::set_permissions(p, PermissionsExt::from_mode(0o755)).unwrap();
    }
    for p in [tree.join("sub").join("a.txt"), outside.join("b.txt")] {
        std::fs::set_permissions(&p, PermissionsExt::from_mode(0o644)).unwrap();
    }
    set_read_only_tree(&tree, true).unwrap();
    assert_eq!(mode(&tree), 0o555);
    assert_eq!(mode(&tree.join("sub")), 0o555);
    assert_eq!(mode(&tree.join("sub").join("a.txt")), 0o444);
    assert_eq!(mode(&outside), 0o755, "a linked folder keeps its mode");
    assert_eq!(
        mode(&outside.join("b.txt")),
        0o644,
        "a linked file keeps its mode"
    );
    set_read_only_tree(&tree, false).unwrap();
    assert_eq!(mode(&tree.join("sub")), 0o755);
    assert_eq!(mode(&tree.join("sub").join("a.txt")), 0o644);
}

#[test]
fn times_name_today_and_carry_seconds() {
    let now = now_secs();
    let today = format_time(now, crate::locale::is_12h());
    assert!(today.starts_with("Today, "), "{today}");
    assert_eq!(today.matches(':').count(), 2, "{today}");
    let old = format_time(now - 10 * 86_400, crate::locale::is_12h());
    assert!(!old.starts_with("Today"), "{old}");
}

#[test]
fn the_day_distance_counts_across_months_and_years() {
    let day = |y: i32, m: i32, d: i32| {
        civil_day(&libc::tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 12,
            tm_mday: d,
            tm_mon: m - 1,
            tm_year: y - 1900,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: std::ptr::null(),
        })
    };
    assert_eq!(day(2027, 1, 1) - day(2026, 12, 31), 1, "new year");
    assert_eq!(day(2026, 10, 1) - day(2026, 9, 30), 1, "new month");
    assert_eq!(day(2024, 3, 1) - day(2024, 2, 29), 1, "leap day");
    assert_eq!(day(2026, 1, 1) - day(2025, 1, 1), 365);
    assert_eq!(day(1970, 1, 1), 0, "the epoch");
}

#[test]
fn the_task_panes_date_names_yesterday_and_drops_the_seconds() {
    let now = now_secs();
    let is_12h = crate::locale::is_12h();
    let today = format_pane_time(now, is_12h);
    assert!(today.starts_with("Today, "), "{today}");
    assert_eq!(today.matches(':').count(), 1, "{today}");
    let old = format_pane_time(now - 10 * 86_400, is_12h);
    assert!(
        !old.starts_with("Today") && !old.starts_with("Yesterday"),
        "{old}"
    );
    // A day back is the day before, except within an hour of
    // midnight, where a DST shift can leave it on the same date.
    let hour = crate::locale::localtime(now).tm_hour;
    if (1..23).contains(&hour) {
        let yesterday = format_pane_time(now - 86_400, is_12h);
        assert!(yesterday.starts_with("Yesterday, "), "{yesterday}");
    }
}
