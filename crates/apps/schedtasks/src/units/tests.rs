use super::*;

fn task() -> NewTask {
    NewTask {
        name: "Back up photos".into(),
        program: "/usr/bin/rsync".into(),
        args: vec!["-a".into(), "/home/u/My Photos".into(), "/mnt".into()],
        schedule: Schedule::Daily { at: (9, 5) },
    }
}

/// `dir` as the own folder, with no other folder searched.
fn own(dir: &Path) -> Folders {
    Folders {
        own: dir.into(),
        search: vec![],
    }
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_slug_keeps_lower_case_letters_and_digits() {
    assert_eq!(slug("Back up photos"), "back-up-photos");
    assert_eq!(slug("  ../../etc/passwd "), "etc-passwd");
    assert_eq!(slug("Nightly 2.0!"), "nightly-2-0");
    assert_eq!(slug("Fotók mentése"), "fot-k-ment-se");
    assert_eq!(slug("日本"), "task", "nothing ASCII left");
    assert_eq!(slug(""), "task");
    let long = slug(&"a b ".repeat(40));
    assert_eq!(long.len(), 39, "40 bytes with its trailing - cut");
    assert!(!long.ends_with('-'));
}

#[test]
fn only_a_stem_new_task_makes_is_ours() {
    for stem in ["reprise-task-backup", "reprise-task-a-2", "reprise-task-0"] {
        assert!(is_our_stem(stem), "{stem}");
    }
    for stem in [
        "backup",
        "reprise-task-",
        "reprise-task--x",
        "reprise-task-A",
        "reprise-task-a.b",
        "reprise-task-a/b",
        "reprise-task-../x",
        "reprise-task-a b",
        "reprise-taskx",
    ] {
        assert!(!is_our_stem(stem), "{stem}");
    }
}

#[test]
fn arguments_split_as_a_shell_splits_them() {
    assert_eq!(split_args(""), Ok(vec![]));
    assert_eq!(split_args("  -a   -b "), Ok(args(&["-a", "-b"])));
    assert_eq!(
        split_args(r#"-a "/home/u/My Photos" '$HOME' x\ y"#),
        Ok(args(&["-a", "/home/u/My Photos", "$HOME", "x y"]))
    );
    assert_eq!(
        split_args(r#""say \"hi\"" "a\\b" "c\d""#),
        Ok(args(&[r#"say "hi""#, r"a\b", r"c\d"]))
    );
    assert_eq!(split_args(r#""" ''"#), Ok(args(&["", ""])), "empty words");
    assert_eq!(split_args(r#"a"b"'c'"#), Ok(args(&["abc"])));
    assert_eq!(split_args("\"open"), Err(Problem::Quote));
    assert_eq!(split_args("'open"), Err(Problem::Quote));
    assert_eq!(
        split_args(r"a\"),
        Err(Problem::Quote),
        "a lone \\ at the end"
    );
    assert_eq!(
        split_args("Rapport\u{a0}final\tx\ny"),
        Ok(args(&["Rapport\u{a0}final", "x", "y"])),
        "a no-break space stays in its word"
    );
}

#[test]
fn every_exec_start_word_is_quoted_and_escaped() {
    let exec = |p: &str, a: &[&str]| exec_start(Path::new(p), &args(a));
    assert_eq!(
        exec("/opt/My App/run", &["a b", "-x"]),
        Ok(r#":"/opt/My App/run" "a b" "-x""#.into())
    );
    assert_eq!(
        exec(
            "/bin/echo",
            &[r#"say "hi""#, r"a\b", "%h", "$HOME", "${X}", ";", ""]
        ),
        Ok(r#":"/bin/echo" "say \"hi\"" "a\\b" "%%h" "$HOME" "${X}" ";" """#.into()),
        "quotes, backslashes, specifiers, variables and a lone ; stay literal"
    );
    assert_eq!(
        exec("/home/u/bin/cost$calc", &[]),
        Ok(r#":"/home/u/bin/cost$calc""#.into())
    );
    assert_eq!(exec("echo", &[]), Err(Problem::Program), "not absolute");
    assert_eq!(exec("-/bin/echo", &[]), Err(Problem::Program), "a prefix");
    for odd in [
        "/usr//bin/echo",
        "/opt/./tool",
        "/usr/bin/../bin/echo",
        "/bin/echo/",
    ] {
        assert_eq!(exec(odd, &[]), Err(Problem::Program), "{odd}");
    }
    for odd in [
        r"/opt/a\b",
        r#"/opt/"x""#,
        "/home/u/Bob's backup.sh",
        "/opt/*",
        "/opt/what?",
        "/home/u/sync [nas].sh",
    ] {
        assert_eq!(exec(odd, &[]), Err(Problem::Special), "{odd}");
    }
    assert_eq!(
        exec("/bin/echo", &["it's", "*", "[x]"]),
        Ok(r#":"/bin/echo" "it's" "*" "[x]""#.into()),
        "an argument may hold them"
    );
    assert_eq!(exec("/bin/echo", &["a\nb"]), Err(Problem::Control));
    assert_eq!(exec("/bin/echo", &["a\tb"]), Err(Problem::Control));
}

#[test]
fn each_schedule_maps_to_one_timer_setting() {
    let at = (9, 5);
    assert_eq!(
        Schedule::Daily { at }.setting(),
        "OnCalendar=*-*-* 09:05:00"
    );
    assert_eq!(
        Schedule::Weekly { day: 4, at }.setting(),
        "OnCalendar=Fri *-*-* 09:05:00"
    );
    assert_eq!(
        Schedule::Monthly {
            day: 3,
            at: (23, 0)
        }
        .setting(),
        "OnCalendar=*-*-03 23:00:00"
    );
    assert_eq!(
        Schedule::Once {
            date: (2027, 1, 31),
            at: (0, 30)
        }
        .setting(),
        "OnCalendar=2027-01-31 00:30:00"
    );
    assert_eq!(Schedule::AtLogOn.setting(), "OnStartupSec=30s");
}

#[test]
fn the_files_carry_the_marker_name_command_and_schedule() {
    let (service, timer) = texts(&task()).unwrap();
    assert_eq!(
        service,
        format!(
            "{MARKER}\n[Unit]\nDescription=Back up photos\n\n[Service]\nType=oneshot\n\
             ExecStart=:\"/usr/bin/rsync\" \"-a\" \"/home/u/My Photos\" \"/mnt\"\n"
        )
    );
    assert_eq!(
        timer,
        format!(
            "{MARKER}\n[Unit]\nDescription=Back up photos\n\n[Timer]\n\
             OnCalendar=*-*-* 09:05:00\n\n[Install]\nWantedBy=timers.target\n"
        )
    );
    let odd = NewTask {
        name: " 100% done \\".into(),
        ..task()
    };
    assert!(
        texts(&odd)
            .unwrap()
            .0
            .contains("\nDescription=100%% done \n")
    );
    let broken = NewTask {
        name: "a\nExecStart=/bin/rm".into(),
        ..task()
    };
    assert_eq!(texts(&broken), Err(Problem::Control));
}

/// Every name in `dir`.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

#[test]
fn write_makes_the_pair_under_a_free_name_and_leaves_nothing_else() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("systemd/user");
    let timer = write(&own(&dir), &task(), &[]).unwrap();
    assert_eq!(timer, "reprise-task-back-up-photos.timer");
    let (service, timer_text) = texts(&task()).unwrap();
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap();
    assert_eq!(read("reprise-task-back-up-photos.service"), service);
    assert_eq!(read("reprise-task-back-up-photos.timer"), timer_text);
    let taken = vec!["reprise-task-back-up-photos-2.timer".to_string()];
    assert_eq!(
        write(&own(&dir), &task(), &taken).unwrap(),
        "reprise-task-back-up-photos-3.timer",
        "a file in the folder and a timer the manager knows both take a name"
    );
    assert_eq!(
        names(&dir),
        [
            "reprise-task-back-up-photos-3.service",
            "reprise-task-back-up-photos-3.timer",
            "reprise-task-back-up-photos.service",
            "reprise-task-back-up-photos.timer",
        ],
        "no temp file stays"
    );
}

#[test]
fn a_drop_in_folder_in_any_searched_folder_takes_its_units_name() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("user");
    let control = tmp.path().join("user.control");
    let runtime = tmp.path().join("runtime");
    for d in [&dir, &control, &runtime] {
        std::fs::create_dir(d).unwrap();
    }
    std::fs::create_dir(dir.join("reprise-task-x.service.d")).unwrap();
    std::fs::create_dir(control.join("reprise-task-x-2.service.d")).unwrap();
    std::fs::create_dir(runtime.join("reprise-task-x-3.timer.d")).unwrap();
    let folders = Folders {
        own: dir.clone(),
        search: vec![control, dir, runtime],
    };
    assert_eq!(free_stem(&folders, "x", &[]), "reprise-task-x-4");
}

#[test]
fn the_folders_are_the_absolute_ones_of_the_search_path() {
    let search = args(&[
        "/home/u/.config/systemd/user.control",
        "relative",
        "/home/u/.config/systemd/user",
    ]);
    let folders = Folders::of(&search).unwrap();
    assert_eq!(folders.own, PathBuf::from("/home/u/.config/systemd/user"));
    assert_eq!(
        folders.search,
        [
            PathBuf::from("/home/u/.config/systemd/user.control"),
            PathBuf::from("/home/u/.config/systemd/user"),
        ]
    );
    assert_eq!(Folders::of(&args(&["/etc/systemd/user"])), None);
}

#[test]
fn a_dangling_enable_link_takes_its_timers_name() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let wants = dir.join("timers.target.wants");
    std::fs::create_dir(&wants).unwrap();
    let link = wants.join("reprise-task-x.timer");
    std::os::unix::fs::symlink(dir.join("reprise-task-x.timer"), link).unwrap();
    assert_eq!(free_stem(&own(dir), "x", &[]), "reprise-task-x-2");
}

#[test]
fn remove_takes_the_pair_and_its_enable_link_and_skips_what_is_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let timer = write(&own(dir), &task(), &[]).unwrap();
    let wants = dir.join("timers.target.wants");
    std::fs::create_dir(&wants).unwrap();
    std::os::unix::fs::symlink(dir.join(&timer), wants.join(&timer)).unwrap();
    remove(dir, &timer).unwrap();
    assert_eq!(names(dir), ["timers.target.wants"]);
    assert!(names(&wants).is_empty());
    remove(dir, &timer).unwrap();
    assert!(
        remove(dir, "backup.timer").is_err(),
        "a name New Task never makes"
    );
}

#[test]
fn the_folder_is_the_config_one_in_the_managers_search_path() {
    let search = |v: &[&str]| config_dir(&args(v));
    let user = [
        "/home/u/.config/systemd/user.control",
        "/run/user/1000/systemd/user.control",
        "/run/user/1000/systemd/transient",
        "/run/user/1000/systemd/generator.early",
        "/home/u/.config/systemd/user",
        "/etc/xdg/systemd/user",
        "/etc/systemd/user",
        "/run/user/1000/systemd/user",
        "/run/systemd/user",
        "/usr/lib/systemd/user",
    ];
    assert_eq!(search(&user), Some("/home/u/.config/systemd/user".into()));
    assert_eq!(search(&user[1..]), None, "the runtime pair alone");
    assert_eq!(
        search(&["/etc/systemd/user", "/usr/lib/systemd/user"]),
        None
    );
}

#[test]
fn a_new_file_never_replaces_one_already_there() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("reprise-task-x.timer");
    std::fs::write(&path, "theirs").unwrap();
    let e = put_new(&path, "ours").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
    assert_eq!(names(tmp.path()), ["reprise-task-x.timer"]);
}

#[test]
fn only_a_marked_pair_loaded_from_the_folder_is_ours() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let timer = write(&own(dir), &task(), &[]).unwrap();
    let fragment = dir.join(&timer).to_string_lossy().into_owned();
    assert!(ours(dir, &timer, &fragment), "a pair New Task wrote");
    assert!(
        !ours(dir, &timer, "/etc/systemd/user/x.timer"),
        "the manager loaded another file"
    );
    std::fs::write(dir.join("backup.timer"), format!("{MARKER}\n")).unwrap();
    std::fs::write(dir.join("backup.service"), format!("{MARKER}\n")).unwrap();
    let theirs = dir.join("backup.timer").to_string_lossy().into_owned();
    assert!(
        !ours(dir, "backup.timer", &theirs),
        "a name New Task never makes"
    );
    let service = dir.join("reprise-task-back-up-photos.service");
    std::fs::write(&service, "[Service]\nExecStart=/bin/true\n").unwrap();
    assert!(!ours(dir, &timer, &fragment), "an unmarked service");
    std::fs::remove_file(&service).unwrap();
    assert!(
        ours(dir, &timer, &fragment),
        "a Delete that removed only the service is done again"
    );
    std::os::unix::fs::symlink(dir.join(&timer), &service).unwrap();
    assert!(!ours(dir, &timer, &fragment), "a link");
}

#[test]
fn a_program_is_an_executable_path_or_a_name_on_path() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let tool = bin.join("tool");
    std::fs::write(&tool, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let plain = bin.join("notes.txt");
    std::fs::write(&plain, "").unwrap();
    let link = bin.join("tool-link");
    std::os::unix::fs::symlink(&tool, &link).unwrap();
    let path = std::env::join_paths(["relative", bin.to_str().unwrap()]).unwrap();
    let find = |t: &str| find_program(t, Some(&path));
    assert_eq!(find("tool"), Some(tool.clone()));
    assert_eq!(find(tool.to_str().unwrap()), Some(tool.clone()));
    assert_eq!(find(link.to_str().unwrap()), Some(link), "the link kept");
    assert_eq!(find("notes.txt"), None, "not executable");
    assert_eq!(find("missing"), None);
    assert_eq!(find("bin/tool"), None, "a relative path");
    let odd = |p: &str| find(&p.replace("BIN", bin.to_str().unwrap()));
    assert_eq!(odd("BIN//tool"), Some(tool.clone()), "// as one /");
    assert_eq!(odd("BIN/./tool"), Some(tool.clone()), "a . dropped");
    assert_eq!(odd("BIN/../bin/tool"), None, "a .. refused");
    assert_eq!(find_program("tool", None), None);
    assert_ne!(
        find_program("~//bin/sh", None),
        Some("/bin/sh".into()),
        "~// stays under home"
    );
}

#[test]
fn a_path_entry_with_dot_dot_is_passed_over_for_the_next() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let tool = bin.join("tool");
    std::fs::write(&tool, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let up = bin.join("../bin");
    let path = std::env::join_paths([&up, &bin]).unwrap();
    assert_eq!(find_program("tool", Some(&path)), Some(tool));
}
