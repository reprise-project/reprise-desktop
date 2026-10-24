use super::*;

const FIREFOX: &str =
    "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\nIcon=firefox\n";

#[test]
fn a_link_file_round_trips_and_keeps_its_keys_to_itself() {
    let text = link_text("Example\nExec=x", "https://example.org/a%20b").unwrap();
    assert_eq!(
        text,
        "[Desktop Entry]\nType=Link\nName=ExampleExec=x\nIcon=text-html\n\
         URL=https://example.org/a%20b\n"
    );
    assert_eq!(
        parse_link(&text),
        Some(LinkEntry {
            name: "ExampleExec=x".into(),
            url: "https://example.org/a%20b".into(),
            icon: Some("text-html".into()),
        })
    );
    assert_eq!(link_text("x", "https://a.test/\nExec=y"), None);
    assert_eq!(parse_link(FIREFOX), None);
    assert_eq!(parse_link("[Desktop Entry]\nType=Link\nName=x\n"), None);
    assert_eq!(
        parse_link("[Desktop Entry]\nType=Link\nName=x\nURL=file:///bin/sh\n"),
        None
    );
}

#[test]
fn parses_application_and_strips_codes() {
    let e = parse_desktop(FIREFOX, "firefox.desktop").unwrap();
    assert_eq!(e.name, "Firefox");
    assert_eq!(launch_argv(&e, "foot"), ["firefox"]);
    assert!(!e.terminal);
    assert_eq!(
        strip_field_codes("app %F --flag %c \"%k\" %%"),
        "app --flag \"\" %"
    );
    assert_eq!(
        strip_field_codes("app   \"two  spaces\"  x"),
        "app \"two  spaces\" x",
        "whitespace inside quotes is the argument's"
    );
}

#[test]
fn exec_argv_follows_the_spec_not_the_shell() {
    assert_eq!(exec_argv("app -a %f --b"), vec!["app", "-a", "--b"]);
    assert_eq!(
        exec_argv("\"/opt/my app/bin\" \"a \\\"quoted\\\" $HOME\" `x`;y 100%%"),
        vec!["/opt/my app/bin", "a \"quoted\" $HOME", "`x`;y", "100%"]
    );
    assert_eq!(exec_argv("  spaced   out  "), vec!["spaced", "out"]);
    assert_eq!(
        exec_argv("\"\""),
        vec![""],
        "an empty quoted argument stays"
    );
    assert!(exec_argv("").is_empty());
}

#[test]
fn a_quoted_program_splits_off_its_arguments_and_round_trips_through_in_terminal() {
    assert_eq!(
        exec_program_and_rest("  \"/opt/My Term/term\"  --hold -o \"a b\" "),
        ("/opt/My Term/term".to_string(), "--hold -o \"a b\"")
    );
    assert_eq!(exec_program_and_rest("foot"), ("foot".to_string(), ""));
    assert_eq!(exec_program_and_rest(""), (String::new(), ""));
    for program in ["foot", "/home/u/My Apps/x.AppImage", "a\"b`c$d\\e%f", ""] {
        let terminal = format!("{} --hold", exec_quote(program));
        assert_eq!(exec_program_and_rest(&terminal).0, program, "{terminal}");
        assert_eq!(
            in_terminal(&terminal, vec!["top".into()]),
            [program, "--hold", "-e", "top"].map(OsString::from),
            "{terminal}"
        );
    }
    assert_eq!(exec_quote("foot"), "foot", "a plain program stays bare");
}

#[test]
fn string_values_decode_the_spec_escapes() {
    let kv = desktop_entry_kv("[Desktop Entry]\nName=a\\sb\\tc\\\\d\\;e\\\n");
    assert_eq!(
        kv["Name"], "a b\tc\\d\\;e\\",
        "a pair it does not know stays"
    );
    for v in ["plain", "back\\slash", "line\nbreak", "tab\tcr\r", "\\s"] {
        let text = format!("[Desktop Entry]\nName={}\n", escape_value(v));
        assert_eq!(desktop_entry_kv(&text)["Name"], v, "{text}");
    }
}

#[test]
fn a_hostile_label_stays_on_its_line_and_a_hostile_command_is_refused() {
    let text = launcher_text(
        "Shortcut to a\nExec=evil",
        Some("i\rcon"),
        Launch::Exec("xdg-open x"),
    )
    .unwrap();
    assert_eq!(
        text,
        "[Desktop Entry]\nType=Application\nName=Shortcut to aExec=evil\n\
         Icon=icon\nExec=xdg-open x\n"
    );
    let path = Path::new("/home/u/a\nExec=evil");
    assert_eq!(
        launcher_text("x", None, Launch::Exec(&open_command_for(path).unwrap())),
        None,
        "a command with a line break has no value that keeps it"
    );
    assert_eq!(
        launcher_text("x", None, Launch::Action("show\ndesktop")).unwrap(),
        "[Desktop Entry]\nType=Application\nName=x\nX-Reprise-Action=showdesktop\n"
    );
}

#[test]
fn a_new_name_replaces_every_name_of_the_entry_and_nothing_else() {
    let text = "# keep\r\n[Desktop Entry]\r\nType=Application\r\nName[de]=Feuerfuchs\r\n\
                Name = Firefox\r\nGenericName=Browser\r\nName=Second\r\nExec=firefox\r\n\
                [Desktop Action new]\r\nName=New Window\r\nExec=firefox -n\r\n";
    assert_eq!(
        with_name(text, "Web\nExec=evil \\ s"),
        "# keep\r\n[Desktop Entry]\r\nType=Application\r\n\
         Name=WebExec=evil \\\\ s\r\nGenericName=Browser\r\nExec=firefox\r\n\
         [Desktop Action new]\r\nName=New Window\r\nExec=firefox -n\r\n"
    );
    assert_eq!(
        desktop_entry_kv(&with_name(FIREFOX, "a\\b"))["Name"],
        "a\\b",
        "the value reads back as typed"
    );
}

#[test]
fn a_written_launcher_is_executable_and_trusted_and_a_plain_file_is_not() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("app.desktop");
    write_launcher(&path, FIREFOX).unwrap();
    assert!(is_trusted(&path));
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "executable");
    let plain = t.path().join("plain.desktop");
    std::fs::write(&plain, FIREFOX).unwrap();
    assert!(!is_trusted(&plain));
    assert!(!is_trusted(&t.path().join("missing.desktop")));
}

#[test]
fn renaming_a_launcher_rewrites_it_in_place_and_keeps_its_mode_and_trust() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("firefox.desktop");
    std::fs::write(&path, FIREFOX).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
    set_trusted(&path).unwrap();
    set_launcher_name(&path, "Web").unwrap();
    assert!(is_trusted(&path), "the rewrite keeps the trust marker");
    let names: Vec<_> = std::fs::read_dir(t.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["firefox.desktop"], "no temp file is left behind");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        FIREFOX.replace("Name=Firefox", "Name=Web")
    );
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o7777, 0o750);
    assert_eq!(
        set_launcher_name(&path, "\n").unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput,
        "a name of control characters only is refused"
    );
}

#[test]
fn a_shortcut_opens_the_path_it_was_made_for() {
    assert_eq!(
        launcher_text(
            "Shortcut to notes.txt",
            Some("text-plain"),
            Launch::Exec(&open_command_for(Path::new("/home/u/notes.txt")).unwrap())
        )
        .unwrap(),
        "[Desktop Entry]\nType=Application\nName=Shortcut to notes.txt\n\
         Icon=text-plain\nExec=xdg-open /home/u/notes.txt\n"
    );
    for p in [
        "/home/u/my  notes.txt",
        "/home/u/it's",
        "/home/u/$HOME `id` \"q\"",
        "/home/u/back\\slash\\\"",
        "/home/u/50% off %f",
        "/home/u/x[1]*?.txt",
        "/home/u/a;b&c|d>e",
    ] {
        let cmd = open_command_for(Path::new(p)).unwrap();
        let text = launcher_text("s", None, Launch::Exec(&cmd)).unwrap();
        let entry = parse_desktop(&text, "s.desktop").unwrap();
        assert_eq!(launch_argv(&entry, "foot"), ["xdg-open", p], "{text}");
    }
}

#[test]
fn a_shortcut_to_a_name_that_is_not_utf8_is_refused() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/home/u/a\xffb.txt"));
    assert_eq!(open_command_for(path), None);
}

#[test]
fn open_argv_for_keeps_a_name_that_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/nonexistent/a\xffb.txt"));
    assert_eq!(
        open_argv_for(path),
        [OsString::from("xdg-open"), path.as_os_str().to_owned()]
    );
    let dir = std::env::temp_dir();
    assert_eq!(
        open_argv_for(&dir)[0],
        "reprise-explorer",
        "a folder opens in Explorer"
    );
}

#[test]
fn excludes_hidden_nodisplay_wrong_type_and_showin() {
    assert!(parse_desktop("[Desktop Entry]\nType=Link\nName=x\nExec=x\n", "a").is_none());
    let quiet = "[Desktop Entry]\nType=Application\nName=x\nExec=x\nNoDisplay=true\n";
    assert!(parse_desktop(quiet, "a").is_none());
    assert!(
        !parse_desktop_any(quiet, "a").unwrap().shown,
        "the index keeps it, unshown"
    );
    assert!(
        parse_desktop("[Desktop Entry]\nType=Application\nName=x\nExec=x\n", "a")
            .unwrap()
            .shown
    );
    assert!(
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=x\nExec=x\nHidden=true\n",
            "a"
        )
        .is_none()
    );
    assert!(
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=x\nExec=x\nOnlyShowIn=GNOME;\n",
            "a"
        )
        .is_none()
    );
    assert!(
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=x\nExec=x\nOnlyShowIn=reprise;GNOME;\n",
            "a"
        )
        .is_some()
    );
    assert!(
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=x\nExec=x\nNotShowIn=reprise;\n",
            "a"
        )
        .is_none()
    );
    assert!(parse_desktop("[Desktop Entry]\nType=Application\nName=x\n", "a").is_none());
    assert!(parse_desktop("[Other]\nType=Application\nName=x\nExec=x\n", "a").is_none());
}

#[test]
fn default_terminal_is_reprise() {
    let argv = in_terminal(DEFAULT_TERMINAL, vec!["top".into()]);
    assert_eq!(argv, ["reprise-terminal", "-e", "top"]);
}

#[test]
fn terminal_entries_wrap_in_terminal() {
    let e = parse_desktop(
        "[Desktop Entry]\nType=Application\nName=htop\nExec=htop\nTerminal=true\n",
        "htop.desktop",
    )
    .unwrap();
    assert!(e.terminal);
    assert_eq!(launch_argv(&e, "alacritty"), ["alacritty", "-e", "htop"]);
}

#[test]
fn launch_argv_passes_shell_syntax_through_as_plain_arguments() {
    let e = parse_desktop(
        "[Desktop Entry]\nType=Application\nName=x\n\
         Exec=prog $HOME `id`; *.txt \"a b\" %U\n",
        "x.desktop",
    )
    .unwrap();
    assert_eq!(
        launch_argv(&e, "foot"),
        ["prog", "$HOME", "`id`;", "*.txt", "a b"]
    );
}

#[test]
fn open_argv_fills_the_file_slot_with_the_path_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let entry = |exec: &str, terminal: bool| {
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=x\nExec={exec}\nTerminal={terminal}\n"
        );
        parse_desktop(&text, "x.desktop").unwrap()
    };
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/home/u/it's a\xff.txt"));
    let open = |exec, terminal, term| -> Vec<Vec<u8>> {
        open_argv(&entry(exec, terminal), term, path)
            .iter()
            .map(|a| a.as_bytes().to_vec())
            .collect()
    };
    let p = path.as_os_str().as_bytes().to_vec();
    let b = |s: &str| s.as_bytes().to_vec();
    assert_eq!(
        open("ed --new %f --x", false, ""),
        [b("ed"), b("--new"), p.clone(), b("--x")]
    );
    assert_eq!(open("ed %F", false, ""), [b("ed"), p.clone()]);
    assert_eq!(
        open("web %u", false, ""),
        [b("web"), b("file:///home/u/it%27s%20a%FF.txt")]
    );
    assert_eq!(
        open("ed %f %f", false, ""),
        [b("ed"), p.clone()],
        "one file code"
    );
    assert_eq!(
        open("\"/opt/my ed\" -i %i", false, ""),
        [b("/opt/my ed"), b("-i"), p.clone()]
    );
    assert_eq!(
        open("vim %F", true, "foot --title \"a b\""),
        [
            b("foot"),
            b("--title"),
            b("a b"),
            b("-e"),
            b("vim"),
            p.clone()
        ]
    );
}

#[test]
fn parses_icon_and_startup_wm_class() {
    let e = parse_desktop(FIREFOX, "firefox.desktop").unwrap();
    assert_eq!(e.icon.as_deref(), Some("firefox"));
    assert_eq!(e.startup_wm_class, None);
    let e = parse_desktop(
        "[Desktop Entry]\nType=Application\nName=Code\nExec=code\nIcon=/opt/code/code.png\nStartupWMClass=Code\n",
        "code.desktop",
    )
    .unwrap();
    assert_eq!(e.icon.as_deref(), Some("/opt/code/code.png"));
    assert_eq!(e.startup_wm_class.as_deref(), Some("Code"));
}

#[test]
fn icon_for_app_id_matches_id_then_wm_class() {
    let entries = vec![
        parse_desktop(FIREFOX, "org.mozilla.firefox.desktop").unwrap(),
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Code\nExec=code\nIcon=vscode\nStartupWMClass=Code\n",
            "code-oss.desktop",
        )
        .unwrap(),
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=NoIcon\nExec=x\nStartupWMClass=x\n",
            "x.desktop",
        )
        .unwrap(),
    ];
    assert_eq!(
        icon_for_app_id(&entries, "org.mozilla.firefox"),
        Some("firefox")
    );
    assert_eq!(
        icon_for_app_id(&entries, "ORG.MOZILLA.FIREFOX"),
        Some("firefox")
    );
    assert_eq!(icon_for_app_id(&entries, "Code"), Some("vscode"));
    assert_eq!(icon_for_app_id(&entries, "code"), Some("vscode"));
    assert_eq!(icon_for_app_id(&entries, "x"), None);
    assert_eq!(icon_for_app_id(&entries, "nothing"), None);
    assert_eq!(icon_for_app_id(&entries, ""), None);
}

#[test]
fn icon_for_app_id_prefers_the_id_match_over_a_class_match() {
    let entries = vec![
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Wrapper\nExec=w\nIcon=wrapper\nStartupWMClass=code\n",
            "wrapper.desktop",
        )
        .unwrap(),
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Code\nExec=code\nIcon=vscode\n",
            "code.desktop",
        )
        .unwrap(),
    ];
    assert_eq!(icon_for_app_id(&entries, "code"), Some("vscode"));
}

#[test]
fn scan_orders_and_overrides() {
    let base = std::env::temp_dir().join(format!("reprise-apps-{}", std::process::id()));
    let user = base.join("user");
    let sys = base.join("sys");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(&sys).unwrap();
    std::fs::write(
        sys.join("b.desktop"),
        "[Desktop Entry]\nType=Application\nName=beta\nExec=b\n",
    )
    .unwrap();
    std::fs::write(
        sys.join("a.desktop"),
        "[Desktop Entry]\nType=Application\nName=Alpha\nExec=a-sys\n",
    )
    .unwrap();
    std::fs::write(
        user.join("a.desktop"),
        "[Desktop Entry]\nType=Application\nName=alpha\nExec=a-user\n",
    )
    .unwrap();
    std::fs::write(
        sys.join("c.desktop"),
        "[Desktop Entry]\nType=Application\nName=gamma\nExec=c-sys\n",
    )
    .unwrap();
    std::fs::write(
        user.join("c.desktop"),
        "[Desktop Entry]\nType=Application\nName=gamma\nExec=c-user\nHidden=true\n",
    )
    .unwrap();
    std::fs::write(sys.join("notes.txt"), "ignored").unwrap();
    let list = scan(&[user.clone(), sys.clone()]);
    assert_eq!(
        list.iter()
            .map(|e| e.exec_line.as_str())
            .collect::<Vec<_>>(),
        vec!["a-user", "b"]
    );
    assert_eq!(list[0].name, "alpha");
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn group_files_apps_by_first_matching_category() {
    let app = |name: &str, cats: &[&str]| AppEntry {
        name: name.into(),
        exec_line: name.to_lowercase(),
        terminal: false,
        id: format!("{name}.desktop"),
        icon: None,
        startup_wm_class: None,
        categories: cats.iter().map(|c| c.to_string()).collect(),
        mime_types: Vec::new(),
        shown: true,
    };
    let apps = [
        app("Zed", &["Development", "Utility"]),
        app("Firefox", &["Network", "WebBrowser"]),
        app("Calc", &["Utility"]),
        app("Mystery", &[]),
        app("Chess", &["Game", "BoardGame"]),
        app("Alacritty", &["System", "TerminalEmulator"]),
    ];
    let groups = group(&apps);
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Accessories",
            "Games",
            "Internet",
            "Programming",
            "System Tools",
            "Other"
        ]
    );
    assert_eq!(groups[3].apps, [0], "Zed: the first category wins");
    assert_eq!(groups[5].apps, [3], "Mystery");
    assert!(group(&[]).is_empty());
}

#[test]
fn mime_db_answers_per_table_in_order() {
    let db = MimeDb::parse([
        "50:text/x-mine:*.txt\n",
        "50:text/plain:*.txt\n50:text/x-c:*.c:cs\n",
    ]);
    assert_eq!(db.mime_of_name("A.TXT").as_deref(), Some("text/x-mine"));
    assert_eq!(db.mime_of_name("main.c").as_deref(), Some("text/x-c"));
    assert_eq!(db.mime_of_name("MAIN.C"), None);
    assert!(MimeDb::empty().is_empty());
    assert_eq!(MimeDb::empty().mime_of_name("a.txt"), None);
}

#[test]
fn mime_description_is_memoised() {
    let db = MimeDb::empty();
    let first = db.description("application/x-reprise-no-such-type");
    assert_eq!(first, None);
    assert!(
        db.descriptions
            .lock()
            .unwrap()
            .contains_key("application/x-reprise-no-such-type")
    );
}

#[test]
fn mime_globs_pick_the_heaviest_longest_match() {
    let globs = "# comment\n50:text/plain:*.txt\n50:text/x-readme:readme\n\
                 80:text/html:*.htm\n50:text/html:*.html\n\
                 55:application/x-perl:*.pl\n50:text/x-c:*.c:cs\n\
                 50:text/x-makefile:makefile\n60:application/x-tar:*.tar.gz\n\
                 50:application/gzip:*.gz\n";
    assert_eq!(
        mime_from_globs(globs, "Notes.TXT").as_deref(),
        Some("text/plain")
    );
    assert_eq!(
        mime_from_globs(globs, "index.html").as_deref(),
        Some("text/html")
    );
    assert_eq!(
        mime_from_globs(globs, "README").as_deref(),
        Some("text/x-readme")
    );
    assert_eq!(
        mime_from_globs(globs, "a.tar.gz").as_deref(),
        Some("application/x-tar")
    );
    assert_eq!(
        mime_from_globs(globs, "main.c").as_deref(),
        Some("text/x-c")
    );
    assert_eq!(mime_from_globs(globs, "MAIN.C"), None);
    assert_eq!(mime_from_globs(globs, "photo.jpg"), None);
}

#[test]
fn mime_globs_match_dotless_suffixes_and_break_ties_by_line_order() {
    let globs = "50:application/x-trash:*~\n50:text/x-suffix:*a.b\n\
                 50:text/x-literal:a.b\n50:text/x-upper:*.C:cs\n";
    assert_eq!(
        mime_from_globs(globs, "notes.txt~").as_deref(),
        Some("application/x-trash")
    );
    assert_eq!(
        mime_from_globs(globs, "a.b").as_deref(),
        Some("text/x-suffix"),
        "equal weight and length: the earlier line"
    );
    assert_eq!(
        mime_from_globs(globs, "Main.C").as_deref(),
        Some("text/x-upper")
    );
}

#[test]
fn mime_globs_fold_mixed_case_patterns_and_prefer_longer_dotted_suffixes() {
    let user = "50:text/x-user:*.Notes\n";
    let system = "50:text/x-notes:*.notes\n50:application/x-compressed-tar:*.tar.GZ\n\
                  50:application/gzip:*.gz\n50:text/x-cmake:CMakeLists.txt\n\
                  50:text/plain:*.txt\n50:text/x-greek:*.ΣΤ\n60:text/x-upper:*.TXT:cs\n";
    let db = MimeDb::parse([user, system]);
    let cases = [
        ("today.notes", Some("text/x-user")),
        ("TODAY.NOTES", Some("text/x-user")),
        ("backup.TAR.gz", Some("application/x-compressed-tar")),
        ("backup.tar.gz.part", None),
        ("only.gz", Some("application/gzip")),
        ("CMAKELISTS.txt", Some("text/x-cmake")),
        ("a.TXT", Some("text/x-upper")),
        ("a.Txt", Some("text/plain")),
        ("βιβλίο.στ", Some("text/x-greek")),
        ("tar.gz", Some("application/gzip")),
    ];
    for (name, mime) in cases {
        assert_eq!(db.mime_of_name(name).as_deref(), mime, "{name}");
    }
    let ext = db.extensions();
    assert_eq!(
        ext.get("application/x-compressed-tar").map(String::as_str),
        Some(".tar.gz")
    );
    assert_eq!(ext.get("text/x-user").map(String::as_str), Some(".notes"));
}

#[test]
fn a_types_extension_is_its_heaviest_suffix() {
    let user = "50:text/x-c:*.cc\n";
    let system = "50:text/x-c:*.c\n60:text/plain:*.txt\n50:text/plain:*.asc\n\
                  50:image/jpeg:*.jpg\n50:image/jpeg:*.jpeg\n50:text/x-makefile:makefile\n";
    let ext = MimeDb::parse([user, system]).extensions();
    assert_eq!(ext.get("text/plain").map(String::as_str), Some(".txt"));
    assert_eq!(ext.get("image/jpeg").map(String::as_str), Some(".jpg"));
    assert_eq!(ext.get("text/x-c").map(String::as_str), Some(".cc"));
    assert_eq!(
        ext.get("text/x-makefile"),
        None,
        "a literal name is no extension"
    );
}

#[test]
fn mime_globs_parse_again_only_when_a_file_changes() {
    let dir = std::env::temp_dir().join(format!("reprisefs-globs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("globs2");
    let mut globs = MimeGlobs::at(vec![dir.join("missing"), file.clone()]);
    let mime = |g: &mut MimeGlobs| g.current().mime_of_name("a.txt");
    assert_eq!(mime(&mut globs), None, "no file yet: no type");

    std::fs::write(&file, "50:text/plain:*.txt\n").unwrap();
    assert_eq!(mime(&mut globs).as_deref(), Some("text/plain"));

    let stamp = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::fs::write(&file, "50:text/x-other:*.txt\n").unwrap();
    let f = std::fs::File::options().write(true).open(&file).unwrap();
    f.set_modified(stamp).unwrap();
    assert_eq!(mime(&mut globs).as_deref(), Some("text/plain"));

    f.set_modified(stamp + std::time::Duration::from_secs(1))
        .unwrap();
    assert_eq!(mime(&mut globs).as_deref(), Some("text/x-other"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_with_lists_the_handlers_by_name_and_text_editors_for_text() {
    let app = |name: &str, mimes: &[&str]| AppEntry {
        name: name.into(),
        exec_line: name.to_lowercase(),
        terminal: name == "Vim",
        id: format!("{name}.desktop"),
        icon: None,
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: mimes.iter().map(|m| m.to_string()).collect(),
        shown: true,
    };
    let apps = vec![
        app("Vim", &["text/plain", "text/x-c"]),
        app("Firefox", &["text/html", "image/png"]),
        app("Eye of GNOME", &["image/png"]),
        app("Files", &["inode/directory"]),
    ];
    assert_eq!(open_with(&apps, "image/png"), vec![2, 1]);
    assert_eq!(open_with(&apps, "text/html"), vec![1, 0]);
    assert_eq!(open_with(&apps, "text/plain"), vec![0]);
    assert_eq!(open_with(&apps, "inode/directory"), vec![3]);
    assert!(open_with(&apps, "audio/ogg").is_empty());
    assert_eq!(
        open_argv(&apps[0], "foot", Path::new("/home/u/my notes.txt")),
        ["foot", "-e", "vim", "/home/u/my notes.txt"]
    );
    assert!(
        parse_desktop(
            "[Desktop Entry]\nType=Application\nName=X\nExec=x %F\nMimeType=text/plain;;image/png;\n",
            "x.desktop"
        )
        .unwrap()
        .mime_types
            == ["text/plain", "image/png"]
    );
}

#[test]
fn a_browsed_program_gets_an_unshown_entry_that_opens_its_file() {
    let dir = std::env::temp_dir().join(format!("reprisefs-userapp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let prog = Path::new("/opt/My Tools/view$1");
    let e = program_entry_in(&dir, prog, Some("text/x-python")).unwrap();
    assert_eq!(e.id, "reprise-userapp-view_1.desktop");
    assert_eq!(e.name, "view$1");
    assert!(!e.shown, "kept out of the menus");
    assert_eq!(e.icon.as_deref(), Some("application-x-executable"));
    assert_eq!(e.mime_types, ["text/x-python"]);
    assert_eq!(
        open_argv(&e, "foot", Path::new("/home/u/a b.py")),
        ["/opt/My Tools/view$1", "/home/u/a b.py"]
    );
    let text = std::fs::read_to_string(dir.join(&e.id)).unwrap();
    assert_eq!(
        parse_desktop_any(&text, &e.id),
        Some(e.clone()),
        "the file says the same"
    );
    let again = program_entry_in(&dir, prog, Some("text/plain")).unwrap();
    assert_eq!(again.id, e.id, "the same program reuses its entry");
    assert_eq!(
        again.mime_types,
        ["text/x-python", "text/plain"],
        "and adds the new type"
    );
    let text = std::fs::read_to_string(dir.join(&e.id)).unwrap();
    assert_eq!(parse_desktop_any(&text, &e.id), Some(again.clone()));
    assert_eq!(
        program_entry_in(&dir, prog, Some("text/plain")).unwrap(),
        again,
        "a type it has is not added twice"
    );
    let other = program_entry_in(&dir, Path::new("/usr/bin/view$1"), None).unwrap();
    assert_eq!(
        other.id, "reprise-userapp-view_1-2.desktop",
        "another program with the name"
    );
    assert!(other.mime_types.is_empty());
    assert_eq!(
        program_entry_in(&dir, Path::new("rel/view"), None)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unshown_entry_is_left_out_of_the_lists_unless_it_is_the_default() {
    let mut quiet = parse_desktop_any(
        "[Desktop Entry]\nType=Application\nName=Quiet\nExec=q %f\nNoDisplay=true\nMimeType=text/plain;\nCategories=Utility;\n",
        "quiet.desktop",
    )
    .unwrap();
    let loud = AppEntry {
        name: "Loud".into(),
        id: "loud.desktop".into(),
        shown: true,
        ..quiet.clone()
    };
    let apps = vec![loud, quiet.clone()];
    assert_eq!(open_with(&apps, "text/plain"), [0]);
    assert_eq!(
        chooser_groups(&apps, Some("text/plain"), None),
        (vec![0], vec![])
    );
    assert_eq!(chooser_groups(&apps, None, None), (vec![], vec![0]));
    assert_eq!(
        chooser_groups(&apps, Some("text/plain"), Some(1)),
        (vec![0, 1], vec![]),
        "the default is listed however it is marked"
    );
    assert_eq!(
        group(&apps),
        [Group {
            name: "Accessories".into(),
            apps: vec![0]
        }]
    );
    quiet.shown = true;
    assert_eq!(open_with(&[quiet], "text/plain"), [0]);
}

#[test]
fn open_with_lists_a_browsed_program_for_its_types_only() {
    let dir = tempfile::tempdir().unwrap();
    let browsed = program_entry_in(dir.path(), Path::new("/opt/view"), Some("image/png")).unwrap();
    let quiet = parse_desktop_any(
        "[Desktop Entry]\nType=Application\nName=Quiet\nExec=q %f\nNoDisplay=true\nMimeType=image/png;\n",
        "quiet.desktop",
    )
    .unwrap();
    let shown = AppEntry {
        name: "Paint".into(),
        id: "paint.desktop".into(),
        shown: true,
        ..quiet.clone()
    };
    assert!(browsed.is_browsed() && !quiet.is_browsed());
    let apps = vec![quiet, shown, browsed];
    assert_eq!(open_with(&apps, "image/png"), [1, 2]);
    assert!(open_with(&apps, "image/jpeg").is_empty());
    assert_eq!(
        chooser_groups(&apps, Some("image/png"), None),
        (vec![1, 2], vec![]),
        "the chooser recommends it for its type"
    );
    assert_eq!(
        chooser_groups(&apps, Some("image/jpeg"), None),
        (vec![], vec![1]),
        "and leaves it out for any other"
    );
}

#[test]
fn chooser_groups_put_the_type_s_programs_and_default_first_and_the_rest_by_name() {
    let e = |name: &str, id: &str, mimes: &[&str]| AppEntry {
        name: name.into(),
        exec_line: String::new(),
        terminal: false,
        id: id.into(),
        icon: None,
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: mimes.iter().map(|m| m.to_string()).collect(),
        shown: true,
    };
    let apps = vec![
        e("Zed", "zed.desktop", &[]),
        e("Notepad", "notepad.desktop", &["text/plain"]),
        e("ark", "ark.desktop", &[]),
    ];
    assert_eq!(
        chooser_groups(&apps, Some("text/plain"), None),
        (vec![1], vec![2, 0])
    );
    assert_eq!(chooser_groups(&apps, None, None), (vec![], vec![2, 1, 0]));
    assert_eq!(
        chooser_groups(&apps, Some("text/plain"), Some(0)),
        (vec![1, 0], vec![2]),
        "the default is recommended even when it does not declare the type"
    );
    assert_eq!(
        chooser_groups(&apps, Some("text/plain"), Some(1)),
        (vec![1], vec![2, 0])
    );
}
