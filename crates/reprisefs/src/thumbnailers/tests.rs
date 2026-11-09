use super::*;

fn entry(exec: &str, mimes: &str) -> String {
    format!("[Thumbnailer Entry]\nTryExec=/bin/sh\nExec={exec}\nMimeType={mimes}\n")
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&vec![0x80; (w * h * 4) as usize])
        .unwrap();
    wr.finish().unwrap();
    out
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn runner(dir: &Path, deadline: Duration) -> Runner {
    Runner {
        slot_dir: dir.to_path_buf(),
        scratch: dir.to_path_buf(),
        deadline,
        jail: Jail::Bare,
    }
}

#[test]
fn exec_splits_without_a_shell_and_substitutes_the_field_codes() {
    let t = Thumbnailer::parse(
        &entry(
            r#"thumb -s %s --uri=%u "%i" %o 100%% %f "a \"b\" $HOME;`x`""#,
            "application/pdf;",
        ),
        "t",
    )
    .unwrap();
    let argv = t.argv(
        Path::new("/home/u/My Docs/a#1.pdf"),
        Path::new("/run/out.png"),
        256,
    );
    assert_eq!(
        argv,
        [
            "thumb",
            "-s",
            "256",
            "--uri=file:///home/u/My%20Docs/a%231.pdf",
            "/home/u/My Docs/a#1.pdf",
            "/run/out.png",
            "100%",
            r#"a "b" $HOME;`x`"#,
        ]
        .map(OsString::from)
    );
}

#[test]
fn an_entry_needs_exec_and_a_mime_type() {
    assert!(Thumbnailer::parse("[Thumbnailer Entry]\nMimeType=a/b;\n", "t").is_none());
    assert!(Thumbnailer::parse("[Thumbnailer Entry]\nExec=x %i\n", "t").is_none());
    assert!(Thumbnailer::parse("[Desktop Entry]\nExec=x\nMimeType=a/b;\n", "t").is_none());
    let t = Thumbnailer::parse(&entry("x %i %o", "Video/MP4;video/*;"), "t").unwrap();
    assert_eq!(t.mimes, ["video/mp4", "video/*"]);
}

#[test]
fn an_exact_type_beats_a_wildcard_and_an_earlier_folder_wins() {
    let home = tempfile::tempdir().unwrap();
    let sys = tempfile::tempdir().unwrap();
    let w = |dir: &Path, name: &str, text: String| std::fs::write(dir.join(name), text).unwrap();
    w(
        sys.path(),
        "evince.thumbnailer",
        entry("evince %i %o", "application/pdf;"),
    );
    w(
        sys.path(),
        "anyvideo.thumbnailer",
        entry("vid %i %o", "video/*;"),
    );
    w(
        sys.path(),
        "mp4.thumbnailer",
        entry("mp4 %i %o", "video/mp4;"),
    );
    w(
        sys.path(),
        "fonts.thumbnailer",
        entry("sysfont %i %o", "font/ttf;"),
    );
    w(
        home.path(),
        "fonts.thumbnailer",
        entry("myfont %i %o", "font/otf;"),
    );
    w(
        home.path(),
        "b.thumbnailer",
        entry("mine %i %o", "application/pdf;"),
    );
    w(home.path(), "gone.thumbnailer", {
        "[Thumbnailer Entry]\nTryExec=/nonexistent/x\nExec=x %i\nMimeType=image/x-xcf;\n".into()
    });
    w(home.path(), "notes.txt", entry("no %i", "text/plain;"));
    let all = Thumbnailers::load(&[home.path().into(), sys.path().into()]);
    let program = |mime: &str| {
        all.for_mime(mime)
            .map(|t| crate::apps::exec_argv(&t.exec)[0].clone())
    };
    assert_eq!(program("application/pdf").as_deref(), Some("mine"));
    assert_eq!(program("video/mp4").as_deref(), Some("mp4"));
    assert_eq!(program("VIDEO/WEBM").as_deref(), Some("vid"));
    assert_eq!(program("font/otf").as_deref(), Some("myfont"));
    assert_eq!(
        program("font/ttf"),
        None,
        "the user's fonts.thumbnailer hides it"
    );
    assert_eq!(program("image/x-xcf"), None, "its TryExec is not installed");
    assert_eq!(program("text/plain"), None);
}

#[test]
fn a_thumbnailer_runs_at_idle_priority_and_its_png_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("fixture.png");
    std::fs::write(&fixture, png(40, 30)).unwrap();
    let nice = dir.path().join("nice");
    let fake = script(
        dir.path(),
        "fake.sh",
        &format!(
            "cut -d' ' -f19 /proc/$$/stat > '{}'\n[ \"$3\" = 128 ] && cp '{}' \"$2\"",
            nice.display(),
            fixture.display()
        ),
    );
    let t =
        Thumbnailer::parse(&entry(&format!("{} %i %o %s", fake.display()), "a/b"), "f").unwrap();
    let input = dir.path().join("doc.pdf");
    std::fs::write(&input, b"%PDF").unwrap();
    let r = runner(dir.path(), Duration::from_secs(5));
    assert_eq!(r.run(&t, &input, 128), Ok(png(40, 30)));
    assert_eq!(std::fs::read_to_string(&nice).unwrap().trim(), "19");
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("reprise-thumbnailer-") && !n.ends_with(".lock"))
        .collect();
    assert!(left.is_empty(), "the scratch folder goes: {left:?}");
}

#[test]
fn no_png_a_bad_png_or_a_failing_exit_is_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("doc.pdf");
    std::fs::write(&input, b"%PDF").unwrap();
    let r = runner(dir.path(), Duration::from_secs(5));
    let huge = dir.path().join("huge.png");
    std::fs::write(&huge, png(MAX_SIDE + 1, 1)).unwrap();
    let good = dir.path().join("good.png");
    std::fs::write(&good, png(2, 2)).unwrap();
    for (name, body) in [
        ("none.sh", "exit 0".to_string()),
        ("text.sh", "echo hello > \"$2\"".into()),
        ("huge.sh", format!("cp '{}' \"$2\"", huge.display())),
        ("exit.sh", format!("cp '{}' \"$2\"; exit 3", good.display())),
    ] {
        let fake = script(dir.path(), name, &body);
        let t =
            Thumbnailer::parse(&entry(&format!("{} %i %o", fake.display()), "a/b"), name).unwrap();
        assert!(
            matches!(r.run(&t, &input, 128), Err(RunError::Failed(_))),
            "{name}"
        );
    }
}

#[test]
fn past_the_deadline_the_whole_process_group_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let fake = script(
        dir.path(),
        "hang.sh",
        &format!("sleep 60 &\necho $! > '{}'\nwait", pid_file.display()),
    );
    let t =
        Thumbnailer::parse(&entry(&format!("{} %i %o", fake.display()), "a/b"), "hang").unwrap();
    let r = runner(dir.path(), Duration::from_millis(300));
    let asked = Instant::now();
    assert!(matches!(
        r.run(&t, Path::new("/home/u/a.mp4"), 128),
        Err(RunError::Failed(_))
    ));
    assert!(
        asked.elapsed() < Duration::from_secs(3),
        "{:?}",
        asked.elapsed()
    );
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    let gone = loop {
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok();
        // Killed and waiting to be reaped counts as gone.
        if state.is_none_or(|s| s.rsplit(") ").next().is_some_and(|r| r.starts_with('Z'))) {
            break true;
        }
        if Instant::now() > end {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        gone,
        "the backgrounded sleep {pid} is killed with its group"
    );
}

#[test]
fn with_every_slot_taken_nothing_runs() {
    use std::os::fd::AsRawFd;
    let dir = tempfile::tempdir().unwrap();
    let ran = dir.path().join("ran");
    let fake = script(dir.path(), "f.sh", &format!("touch '{}'", ran.display()));
    let t = Thumbnailer::parse(&entry(&format!("{} %i %o", fake.display()), "a/b"), "f").unwrap();
    let held: Vec<std::fs::File> = (0..SLOTS)
        .map(|n| {
            let f = std::fs::File::create(dir.path().join(format!("reprise-thumbnailer-{n}.lock")))
                .unwrap();
            // SAFETY: `f` is open for the call.
            assert_eq!(unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) }, 0);
            f
        })
        .collect();
    let r = runner(dir.path(), Duration::from_millis(200));
    assert_eq!(
        r.run(&t, Path::new("/home/u/a.pdf"), 128),
        Err(RunError::Busy)
    );
    assert!(!ran.exists());
    drop(held);
    assert!(matches!(
        r.run(&t, Path::new("/home/u/a.pdf"), 128),
        Err(RunError::Failed(_))
    ));
    assert!(ran.exists(), "it runs once a slot frees");
}

/// A runner in `dir` confined as thumbnailers run in a session; None where bubblewrap or a
/// systemd user manager cannot run.
fn jailed(dir: &Path, deadline: Duration) -> Option<Runner> {
    let jail = Jail::find(std::env::var_os("PATH").as_deref())?;
    let Jail::Bwrap { bwrap, systemd_run } = &jail else {
        return None;
    };
    let runs = |argv: &[&std::ffi::OsStr]| {
        Command::new(argv[0])
            .args(&argv[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let bw = [
        bwrap.as_os_str(),
        "--ro-bind".as_ref(),
        "/".as_ref(),
        "/".as_ref(),
    ];
    let scope = [
        systemd_run.as_os_str(),
        "--user".as_ref(),
        "--scope".as_ref(),
    ];
    let ok = runs(
        &[
            &bw[..],
            &["--unshare-all".as_ref(), "/usr/bin/true".as_ref()],
        ]
        .concat(),
    ) && runs(&[&scope[..], &["--quiet".as_ref(), "/usr/bin/true".as_ref()]].concat());
    ok.then(|| Runner {
        jail,
        ..runner(dir, deadline)
    })
}

#[test]
fn without_bubblewrap_there_is_no_jail() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), "systemd-run", "true");
    assert_eq!(Jail::find(Some(dir.path().as_os_str())), None);
    script(dir.path(), "bwrap", "true");
    assert!(Jail::find(Some(dir.path().as_os_str())).is_some());
}

#[test]
fn a_jailed_thumbnailer_sees_only_its_input_and_output_and_no_network() {
    let dir = tempfile::tempdir().unwrap();
    let Some(r) = jailed(dir.path(), Duration::from_secs(10)) else {
        eprintln!("SKIPPED: bubblewrap or a systemd user manager cannot run here");
        return;
    };
    let host_net = std::fs::read_link("/proc/self/ns/net").unwrap();
    let secret = dir.path().join("secret");
    std::fs::write(&secret, "x").unwrap();
    let escaped = dir.path().join("escaped");
    let input = dir.path().join("doc.pdf");
    std::fs::write(&input, png(4, 4)).unwrap();
    let fake = script(
        dir.path(),
        "jail.sh",
        &format!(
            "touch '{escaped}' 2>/dev/null\n\
             [ -e '{secret}' ] && exit 10\n\
             [ \"$(readlink /proc/self/ns/net)\" = '{net}' ] && exit 11\n\
             [ \"$$\" -lt 10 ] || exit 12\n\
             cp \"$1\" \"$2\"",
            escaped = escaped.display(),
            secret = secret.display(),
            net = host_net.display(),
        ),
    );
    let t = Thumbnailer::parse(&entry(&format!("{} %i %o", fake.display()), "a/b"), "j").unwrap();
    assert_eq!(r.run(&t, &input, 128), Ok(png(4, 4)));
    assert!(!escaped.exists(), "nothing outside the output is written");
}
