use super::*;

fn unit(name: &str, load: &str, active: &str, sub: &str) -> Loaded {
    Loaded {
        name: name.into(),
        description: format!("{name} described"),
        load: load.into(),
        active: active.into(),
        sub: sub.into(),
    }
}

fn file(path: &str, state: &str) -> UnitFile {
    UnitFile {
        path: path.into(),
        state: state.into(),
    }
}

#[test]
fn a_status_reads_systemd_s_active_and_sub_states() {
    assert_eq!(Status::of("active", "running"), Status::Running);
    assert_eq!(Status::of("reloading", "reload"), Status::Running);
    assert_eq!(Status::of("active", "exited"), Status::Finished);
    assert_eq!(Status::of("activating", "start-pre"), Status::Starting);
    assert_eq!(Status::of("deactivating", "stop-sigterm"), Status::Stopping);
    assert_eq!(Status::of("failed", "failed"), Status::Failed);
    assert_eq!(Status::of("inactive", "dead"), Status::Stopped);
}

#[test]
fn startup_reads_the_unit_file_state() {
    assert_eq!(Startup::of("enabled"), Startup::Automatic);
    assert_eq!(Startup::of("disabled"), Startup::Manual);
    assert_eq!(Startup::of("masked"), Startup::Disabled);
    assert_eq!(Startup::of("masked-runtime"), Startup::Disabled);
    for on_demand in [
        "static",
        "indirect",
        "generated",
        "transient",
        "enabled-runtime",
        "",
    ] {
        assert_eq!(Startup::of(on_demand), Startup::OnDemand, "{on_demand:?}");
    }
}

#[test]
fn rows_join_the_loaded_services_and_the_files_not_loaded() {
    let loaded = [
        unit("wireplumber.service", "loaded", "active", "running"),
        unit("pipewire.service", "loaded", "active", "running"),
        unit("gone.service", "not-found", "inactive", "dead"),
        unit("getty@tty1.service", "loaded", "active", "running"),
    ];
    let files = [
        file("/usr/lib/systemd/user/pipewire.service", "enabled"),
        file("/usr/lib/systemd/user/wireplumber.service", "disabled"),
        file("/home/u/.config/systemd/user/sync.service", "disabled"),
        file("/usr/lib/systemd/user/getty@.service", "static"),
        file("/usr/lib/systemd/user/pipewire-alias.service", "alias"),
        file("/etc/systemd/user/hidden.service", "masked"),
    ];
    let mut asked = Vec::new();
    let got = rows(Manager::User, &loaded, &files, |p| {
        asked.push(p.to_string());
        "Sync files".into()
    });
    let row = |name: &str, status, startup, description: &str| Service {
        name: name.into(),
        description: description.into(),
        manager: Manager::User,
        status,
        startup,
    };
    assert_eq!(
        got,
        [
            row(
                "getty@tty1.service",
                Status::Running,
                Startup::OnDemand,
                "getty@tty1.service described"
            ),
            row(
                "hidden.service",
                Status::Stopped,
                Startup::Disabled,
                "Sync files"
            ),
            row(
                "pipewire.service",
                Status::Running,
                Startup::Automatic,
                "pipewire.service described"
            ),
            row(
                "sync.service",
                Status::Stopped,
                Startup::Manual,
                "Sync files"
            ),
            row(
                "wireplumber.service",
                Status::Running,
                Startup::Manual,
                "wireplumber.service described"
            ),
        ],
        "a missing unit, a template and an alias are left out"
    );
    assert_eq!(
        asked,
        [
            "/home/u/.config/systemd/user/sync.service",
            "/etc/systemd/user/hidden.service"
        ],
        "only a file not loaded is read for its description"
    );
}

#[test]
fn a_description_is_the_last_one_the_unit_section_sets() {
    let text = "# Description=commented\n[Unit]\nDescription = First\nDescription=Sync files\n\
                After=network.target\n[Service]\nDescription=not the unit's\n";
    assert_eq!(description(text).as_deref(), Some("Sync files"));
    assert_eq!(description("[Service]\nDescription=x\n"), None);
    assert_eq!(description("[Unit]\nDescription=\n"), None);
    assert_eq!(description("[Unit]\nDescriptionX=y\n"), None);
}

#[test]
fn a_file_that_is_no_plain_file_describes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.service");
    std::fs::write(&path, "[Unit]\nDescription=A\n").unwrap();
    assert_eq!(describe_file(path.to_str().unwrap()), "A");
    assert_eq!(describe_file("/dev/null"), "");
    assert_eq!(describe_file(dir.path().to_str().unwrap()), "");
    assert_eq!(describe_file("/nonexistent/a.service"), "");
    let fifo = dir.path().join("fifo.service");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: mkfifo reads a NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert_eq!(
        describe_file(fifo.to_str().unwrap()),
        "",
        "a FIFO with no writer does not hold the reader"
    );
}

fn sh(script: &str) -> Command {
    let mut c = Command::new("sh");
    c.args(["-c", script]);
    c
}

#[test]
fn a_command_s_output_is_capped_and_a_slow_one_killed() {
    let second = Duration::from_secs(10);
    assert_eq!(capped(sh("printf one"), 1024, second), Ok(b"one".to_vec()));
    assert_eq!(
        capped(sh("seq 1 100000"), 20, second),
        Ok(b"99998\n99999\n100000\n".to_vec()),
        "past the cap the newest lines stay, from a line's start"
    );
    assert_eq!(
        capped(sh("echo bad >&2; exit 3"), 1024, second),
        Err(LogError::Exited(3, "bad".into()))
    );
    assert_eq!(
        capped(sh("kill -9 $$"), 1024, second),
        Err(LogError::Killed(String::new()))
    );
    assert_eq!(
        capped(sh("sleep 30"), 1024, Duration::from_millis(100)),
        Err(LogError::TookTooLong)
    );
    assert_eq!(
        capped(Command::new("/nonexistent/journalctl"), 1024, second),
        Err(LogError::NotStarted)
    );
}

#[test]
fn the_jobs_a_service_takes_follow_its_state() {
    let s = |manager, status, startup| Service {
        name: "a.service".into(),
        description: String::new(),
        manager,
        status,
        startup,
    };
    let takes = |s: &Service| {
        [
            Verb::Start,
            Verb::Stop,
            Verb::Restart,
            Verb::Enable,
            Verb::Disable,
        ]
        .map(|v| s.takes(v))
    };
    use Manager::*;
    assert_eq!(
        takes(&s(User, Status::Running, Startup::Automatic)),
        [false, true, true, false, true]
    );
    assert_eq!(
        takes(&s(User, Status::Stopped, Startup::Manual)),
        [true, false, false, true, false]
    );
    assert_eq!(
        takes(&s(User, Status::Failed, Startup::OnDemand)),
        [true, false, false, false, false],
        "a file with no install section is not enabled"
    );
    assert_eq!(
        takes(&s(User, Status::Stopped, Startup::Disabled)),
        [false; 5],
        "a masked service does not start"
    );
    assert_eq!(
        takes(&s(System, Status::Stopped, Startup::Manual)),
        [true, false, false, false, false],
        "a system unit file is not enabled here"
    );
    assert_eq!(
        takes(&s(System, Status::Starting, Startup::Automatic)),
        [false, true, false, false, false]
    );
}

#[test]
fn the_log_is_asked_for_the_one_unit_of_its_manager() {
    assert_eq!(
        journal_args(Manager::User, "pipewire.service"),
        [
            "--user-unit=pipewire.service",
            "--lines=200",
            "--no-pager",
            "--no-hostname",
            "--quiet"
        ]
    );
    assert_eq!(
        journal_args(Manager::System, "sshd.service")[0],
        "--unit=sshd.service"
    );
    assert_eq!(
        journal(Manager::User, "*.service"),
        Err(LogError::NotAService),
        "a pattern is never passed"
    );
}
