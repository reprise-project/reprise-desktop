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
    assert_eq!(Schedule::Hourly.setting(), "OnCalendar=*-*-* *:00:00");
    assert_eq!(
        Schedule::Minutes { every: 15 }.setting(),
        "OnCalendar=*-*-* *:00/15:00"
    );
    assert_eq!(Schedule::AtLogOn.setting(), "OnStartupSec=30s");
    assert_eq!(Schedule::AtStartup.setting(), "OnStartupSec=30s");
}

const NOTIFIER: &str = "/usr/bin/reprise-schedtasks";

#[test]
fn an_at_startup_task_is_skipped_by_its_stamp_and_stamped_before_each_run() {
    let t = NewTask {
        schedule: Schedule::AtStartup,
        ..task()
    };
    let (service, timer) = texts(&t, Some(Path::new(NOTIFIER))).unwrap();
    assert_eq!(
        service,
        format!(
            "{MARKER}\n[Unit]\nDescription=Back up photos\n\
             ConditionPathExists=!%S/reprise/schedtasks/%N.%b\n\n\
             [Service]\nType=oneshot\n\
             ExecStartPre=:\"{NOTIFIER}\" \"--once-per-boot\" \"%S/reprise/schedtasks\" \"%N\" \"%b\"\n\
             ExecStart=:\"/usr/bin/rsync\" \"-a\" \"/home/u/My Photos\" \"/mnt\"\n\
             ExecStopPost=-:\"{NOTIFIER}\" \"--failed\" \"Back up photos\"\n"
        )
    );
    assert!(timer.contains("\n[Timer]\nOnStartupSec=30s\n"), "{timer}");
}

#[test]
fn an_at_startup_task_needs_a_helper_it_can_run() {
    let t = NewTask {
        schedule: Schedule::AtStartup,
        ..task()
    };
    for helper in [None, Some(Path::new("/opt/x[1]/reprise-schedtasks"))] {
        assert_eq!(texts(&t, helper), Err(Problem::Helper), "{helper:?}");
        assert_eq!(check(&t, helper), Err(Problem::Helper), "{helper:?}");
    }
    assert_eq!(check(&t, Some(Path::new(NOTIFIER))), Ok(()));
    assert_eq!(check(&task(), None), Ok(()), "other schedules need none");
}

#[test]
fn an_awkward_name_never_reaches_the_stamp_lines_and_reads_back() {
    for name in [
        "Back up \"my\" files",
        "100% done",
        "$HOME and ${x}",
        "it's %N %b",
    ] {
        let t = NewTask {
            name: name.into(),
            schedule: Schedule::AtStartup,
            ..task()
        };
        let (service, timer) = texts(&t, Some(Path::new(NOTIFIER))).unwrap();
        for key in ["ConditionPathExists=", "ExecStartPre="] {
            let line = service.lines().find(|l| l.starts_with(key)).unwrap();
            assert!(
                line.ends_with("%N.%b") || line.ends_with("\"%N\" \"%b\""),
                "{line}"
            );
            assert!(
                !line.contains("Back") && !line.contains("HOME"),
                "{name}: {line}"
            );
        }
        assert_eq!(parse(&service, &timer).as_ref(), Some(&t), "{name}");
    }
}

#[test]
fn a_log_on_pair_with_a_stamp_step_reads_as_at_startup_and_one_without_as_at_log_on() {
    let notifier = Some(Path::new(NOTIFIER));
    let logon = NewTask {
        schedule: Schedule::AtLogOn,
        ..task()
    };
    let (service, timer) = texts(&logon, notifier).unwrap();
    assert_eq!(parse(&service, &timer), Some(logon.clone()));
    let startup = NewTask {
        schedule: Schedule::AtStartup,
        ..logon
    };
    let (service, timer) = texts(&startup, notifier).unwrap();
    assert_eq!(parse(&service, &timer), Some(startup));
    for changed in [
        service.replace("\"%N\"", "\"reprise-task-x\""),
        service.replace(
            &format!("ExecStartPre=:\"{NOTIFIER}"),
            "ExecStartPre=:\"/bin/true",
        ),
        service.replace("--once-per-boot", "--failed"),
        service.replace("ConditionPathExists=!", "ConditionPathExists="),
    ] {
        assert_eq!(parse(&changed, &timer), None, "{changed}");
    }
}

const BOOT: &str = "20956c8391bb4f9da73ecc05a087448b";
const EARLIER: &str = "9841c25773014c34b3a2e3d1cf4cbbdd";

#[test]
fn a_run_stamps_its_boot_and_drops_its_other_stamps_only() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state dir/reprise/schedtasks");
    let stem = "reprise-task-x";
    std::fs::create_dir_all(&dir).unwrap();
    for name in [
        format!("{stem}.{EARLIER}"),
        format!("{stem}-2.{EARLIER}"),
        format!("{stem}.notes"),
    ] {
        std::fs::write(dir.join(name), "").unwrap();
    }
    stamp_run(&dir, stem, BOOT).unwrap();
    assert_eq!(
        names(&dir),
        [
            format!("{stem}-2.{EARLIER}"),
            format!("{stem}.{BOOT}"),
            format!("{stem}.notes"),
        ],
        "this boot's stamp; another task's and a file of another form stay"
    );
    stamp_run(&dir, stem, BOOT).unwrap();
    forget_runs(&dir, stem, None).unwrap();
    assert_eq!(
        names(&dir),
        [format!("{stem}-2.{EARLIER}"), format!("{stem}.notes")]
    );
    forget_runs(&tmp.path().join("gone"), stem, None).unwrap();
}

#[test]
fn a_stamp_run_refuses_arguments_its_unit_never_passes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    for (dir, stem, boot) in [
        (Path::new("relative"), "reprise-task-x", BOOT),
        (dir, "../reprise-task-x", BOOT),
        (dir, "backup", BOOT),
        (
            dir,
            "reprise-task-x",
            "9841c257-7301-4c34-b3a2-e3d1cf4cbbdd",
        ),
        (dir, "reprise-task-x", "%b"),
    ] {
        assert!(stamp_run(dir, stem, boot).is_err(), "{dir:?} {stem} {boot}");
    }
    assert!(names(dir).is_empty());
}

#[test]
fn the_stamp_folder_comes_from_the_path_the_manager_resolved() {
    let stem = "reprise-task-x";
    let resolved = format!("/home/u/.state %/reprise/schedtasks/{stem}.{BOOT}");
    assert_eq!(
        stamps_dir(&resolved, stem),
        Some(Path::new("/home/u/.state %/reprise/schedtasks"))
    );
    for other in [
        format!("relative/{stem}.{BOOT}"),
        format!("/s/{stem}-2.{BOOT}"),
        format!("/s/{stem}.boot"),
        format!("/s/{stem}"),
    ] {
        assert_eq!(stamps_dir(&other, stem), None, "{other}");
    }
    assert_eq!(stamps_dir(&format!("/s/backup.{BOOT}"), "backup"), None);
}

#[test]
fn the_files_carry_the_marker_name_command_and_schedule() {
    let (service, timer) = texts(&task(), None).unwrap();
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
    let (told, _) = texts(&task(), Some(Path::new("/usr/bin/reprise-schedtasks"))).unwrap();
    assert_eq!(
        told,
        service.clone()
            + "ExecStopPost=-:\"/usr/bin/reprise-schedtasks\" \"--failed\" \"Back up photos\"\n"
    );
    let odd = NewTask {
        name: " 100% done \\".into(),
        ..task()
    };
    assert!(
        texts(&odd, None)
            .unwrap()
            .0
            .contains("\nDescription=100%% done\n")
    );
    let broken = NewTask {
        name: "a\nExecStart=/bin/rm".into(),
        ..task()
    };
    assert_eq!(texts(&broken, None), Err(Problem::Control));
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
    let timer = write(&own(&dir), &task(), &[], None).unwrap();
    assert_eq!(timer, "reprise-task-back-up-photos.timer");
    let (service, timer_text) = texts(&task(), None).unwrap();
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap();
    assert_eq!(read("reprise-task-back-up-photos.service"), service);
    assert_eq!(read("reprise-task-back-up-photos.timer"), timer_text);
    let taken = vec!["reprise-task-back-up-photos-2.timer".to_string()];
    assert_eq!(
        write(&own(&dir), &task(), &taken, None).unwrap(),
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
fn a_unit_file_in_any_searched_folder_takes_its_name() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("user");
    let control = tmp.path().join("user.control");
    let system = tmp.path().join("etc");
    for d in [&dir, &control, &system] {
        std::fs::create_dir(d).unwrap();
    }
    std::fs::write(control.join("reprise-task-x.service"), "").unwrap();
    std::fs::write(system.join("reprise-task-x-2.timer"), "").unwrap();
    let folders = Folders {
        own: dir.clone(),
        search: vec![control, dir, system],
    };
    assert_eq!(free_stem(&folders, "x", &[]), "reprise-task-x-3");
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
    let timer = write(&own(dir), &task(), &[], None).unwrap();
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
    let timer = write(&own(dir), &task(), &[], None).unwrap();
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

#[test]
fn a_pair_new_task_wrote_reads_back_as_its_task() {
    let schedules = [
        Schedule::Minutes { every: 5 },
        Schedule::Minutes { every: 30 },
        Schedule::Hourly,
        Schedule::Daily { at: (0, 0) },
        Schedule::Daily { at: (23, 59) },
        Schedule::Weekly { day: 6, at: (9, 5) },
        Schedule::Monthly {
            day: 31,
            at: (12, 0),
        },
        Schedule::Once {
            date: (2027, 1, 2),
            at: (8, 30),
        },
        Schedule::AtLogOn,
    ];
    for schedule in schedules {
        let t = NewTask {
            name: "100% \"done\"".into(),
            args: args(&["-c", "echo \"%h\" \\ 'x'", ""]),
            schedule,
            ..task()
        };
        for notifier in [None, Some(Path::new("/usr/bin/reprise-schedtasks"))] {
            let (service, timer) = texts(&t, notifier).unwrap();
            assert_eq!(parse(&service, &timer).as_ref(), Some(&t), "{schedule:?}");
        }
    }
}

#[test]
fn a_pair_changed_by_hand_does_not_read_as_a_task() {
    let (service, timer) = texts(&task(), None).unwrap();
    for (service, timer) in [
        (
            service.replace("Type=oneshot", "Type=simple"),
            timer.clone(),
        ),
        (
            service.clone() + "ExecStart=:\"/bin/true\"\n",
            timer.clone(),
        ),
        (service.replace("ExecStart=:", "ExecStart="), timer.clone()),
        (service.clone(), timer.replace("09:05:00", "09:05:30")),
        (service.clone(), timer.replace("*-*-* 09", "*-*-1 09")),
        (
            service.clone(),
            timer.replace("OnCalendar=*-*-* 09:05:00", "OnCalendar=*-*-* *:00/7:00"),
        ),
        (
            service.clone(),
            timer.replace("OnCalendar=*-*-* 09:05:00", "OnStartupSec=1min"),
        ),
    ] {
        assert_eq!(parse(&service, &timer), None, "{service}{timer}");
    }
}

#[test]
fn the_arguments_field_reads_back_the_words_it_shows() {
    let words = args(&[
        "-a",
        "My Photos",
        "",
        "it's",
        "say \"hi\"",
        "back\\slash",
        "%h",
    ]);
    let shown = join_args(&words);
    assert_eq!(
        shown,
        r#"-a 'My Photos' '' "it's" 'say "hi"' 'back\slash' %h"#
    );
    assert_eq!(split_args(&shown), Ok(words));
    let both = args(&["it's \"x\" \\"]);
    assert_eq!(split_args(&join_args(&both)), Ok(both));
}

#[test]
fn rewrite_replaces_the_pair_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let timer = write(&own(tmp.path()), &task(), &[], None).unwrap();
    let changed = NewTask {
        name: "Back up all".into(),
        schedule: Schedule::Hourly,
        ..task()
    };
    rewrite(
        tmp.path(),
        &timer,
        &changed,
        Some(Path::new("/usr/bin/reprise-schedtasks")),
    )
    .unwrap();
    assert_eq!(read(tmp.path(), &timer), Some(changed));
    assert_eq!(
        names(tmp.path()),
        [
            "reprise-task-back-up-photos.service",
            "reprise-task-back-up-photos.timer"
        ],
        "the unit names stay, and no temporary file is left"
    );
    assert!(rewrite(tmp.path(), "backup.timer", &task(), None).is_err());
}

#[test]
fn a_name_ending_in_a_backslash_is_kept_without_it_and_reads_back() {
    assert_eq!(kept_name(" a \\ \\ "), "a");
    let t = NewTask {
        name: "Back up \\".into(),
        ..task()
    };
    let notifier = Some(Path::new("/usr/bin/reprise-schedtasks"));
    let (service, timer) = texts(&t, notifier).unwrap();
    assert!(service.contains("\nDescription=Back up\n"), "{service}");
    assert!(service.contains(" \"--failed\" \"Back up\"\n"), "{service}");
    let read = parse(&service, &timer).unwrap();
    assert_eq!(read.name, "Back up");
    assert_eq!(texts(&read, notifier).unwrap(), (service, timer));
}

#[test]
fn a_timer_that_cannot_be_rewritten_puts_the_service_back() {
    let tmp = tempfile::tempdir().unwrap();
    let timer = write(&own(tmp.path()), &task(), &[], None).unwrap();
    let service = tmp.path().join(timer.replace(".timer", ".service"));
    let before = std::fs::read_to_string(&service).unwrap();
    std::fs::remove_file(tmp.path().join(&timer)).unwrap();
    std::fs::create_dir(tmp.path().join(&timer)).unwrap();
    let changed = NewTask {
        name: "Back up all".into(),
        ..task()
    };
    assert!(rewrite(tmp.path(), &timer, &changed, None).is_err());
    assert_eq!(std::fs::read_to_string(&service).unwrap(), before);
}
