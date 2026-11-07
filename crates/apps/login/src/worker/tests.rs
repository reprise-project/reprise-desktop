use super::*;
use repriselogin::frame::FrameError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn anna() -> PasswdEntry {
    PasswdEntry {
        name: "anna".into(),
        uid: 1000,
        gid: 1000,
        gecos: "Anna Berg".into(),
        home: PathBuf::from("/home/anna"),
        shell: "/bin/bash".into(),
    }
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn modes_name_their_pam_service_and_argument() {
    let services: Vec<_> = MODES.iter().map(|m| m.service()).collect();
    assert_eq!(
        services,
        [
            "reprise-login",
            "reprise-login",
            "reprise-login-greeter",
            "reprise-login-autologin",
            "reprise-login-remote",
            "reprise-login-remote-nla",
            "reprise-login-check"
        ]
    );
    let args: Vec<_> = MODES.iter().map(|m| m.as_arg()).collect();
    assert_eq!(
        args[4..],
        ["remote", "remote-nla", "check"],
        "the remote and check modes' arguments"
    );
    for m in MODES {
        assert_eq!(Mode::parse(m.as_arg()), Some(m));
    }
    assert_eq!(Mode::parse("root"), None);
}

#[test]
fn a_greeter_worker_runs_only_for_the_greeter_account() {
    assert!(Mode::Greeter.runs_for(GREETER_USER, None));
    assert!(!Mode::Greeter.runs_for("anna", None));
    assert!(!Mode::Greeter.runs_for("root", Some("root")));
    for m in [
        Mode::Login,
        Mode::Reauth,
        Mode::Remote,
        Mode::RemoteNla,
        Mode::Check,
    ] {
        assert!(m.runs_for("anna", None));
    }
}

/// The index of the first `auth` line of `stack` running `module`.
fn auth_line(stack: &str, module: &str) -> Option<usize> {
    (stack.lines()).position(|l| l.starts_with("auth") && l.contains(module))
}

#[test]
fn the_passwordless_stacks_check_the_account_before_pam_permit() {
    let greeter = include_str!("../../../../../data/pam.d/reprise-login-greeter");
    let guard = auth_line(greeter, "pam_succeed_if.so user = reprise-greeter");
    assert!(guard.is_some(), "{greeter}");
    assert!(guard < auth_line(greeter, "pam_permit.so"));
    let autologin = include_str!("../../../../../data/pam.d/reprise-login-autologin");
    let guard = auth_line(autologin, "pam_succeed_if.so uid >= 1000");
    assert!(guard.is_some(), "{autologin}");
    assert!(guard < auth_line(autologin, "pam_permit.so"));
}

#[test]
fn the_login_stack_refuses_root_before_any_password_is_checked() {
    let login = include_str!("../../../../../data/pam.d/reprise-login");
    let delay = auth_line(login, "pam_faildelay.so delay=2000000");
    let guard = auth_line(login, "pam_succeed_if.so quiet_success uid ne 0");
    let include = auth_line(login, "system-login");
    assert!(delay.is_some() && delay < guard, "{login}");
    assert!(guard.is_some() && guard < include, "{login}");
    let guard = login.lines().nth(guard.unwrap()).unwrap();
    assert!(
        guard.contains("[success=ok user_unknown=ignore default=die]"),
        "an unknown user still reaches the password prompt: {guard}"
    );
}

#[test]
fn an_autologin_worker_runs_only_for_the_configured_user() {
    assert!(Mode::Autologin.runs_for("anna", Some("anna")));
    assert!(!Mode::Autologin.runs_for("root", Some("anna")));
    assert!(!Mode::Autologin.runs_for("anna", None));
}

#[test]
fn a_secret_prompt_round_trips_over_the_socket() {
    let (worker, mut daemon) = UnixStream::pair().unwrap();
    let daemon = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for answer in [Some("hunter2"), None] {
            seen.push(read_msg::<FromWorker>(&mut daemon).unwrap());
            let answer = answer.map(|a| Zeroizing::new(a.to_owned()));
            write_msg(&mut daemon, &ToWorker::Answer(answer)).unwrap();
        }
        seen
    });
    let mut conv = SocketConverse::stoppable(worker, &NEVER);
    let answer = conv.prompt(AuthKind::Secret, "Password:");
    assert_eq!(answer.as_deref().map(String::as_str), Some("hunter2"));
    assert_eq!(conv.prompt(AuthKind::Info, "Welcome"), None);
    assert!(!conv.ended().get());
    assert_eq!(
        daemon.join().unwrap(),
        [
            FromWorker::Prompt {
                kind: AuthKind::Secret,
                text: "Password:".into()
            },
            FromWorker::Prompt {
                kind: AuthKind::Info,
                text: "Welcome".into()
            },
        ]
    );
}

#[test]
fn cancel_while_prompting_ends_the_conversation() {
    let (worker, mut daemon) = UnixStream::pair().unwrap();
    worker
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut conv = SocketConverse::stoppable(worker, &NEVER);
    let ended = conv.ended();
    let daemon = std::thread::spawn(move || {
        let first = read_msg::<FromWorker>(&mut daemon).unwrap();
        write_msg(&mut daemon, &ToWorker::Cancel).unwrap();
        (first, read_msg::<FromWorker>(&mut daemon))
    });
    assert_eq!(conv.prompt(AuthKind::Secret, "Password:"), None);
    assert!(ended.get());
    assert_eq!(conv.prompt(AuthKind::Secret, "Password:"), None);
    drop(conv);
    let (first, after) = daemon.join().unwrap();
    assert!(matches!(first, FromWorker::Prompt { .. }), "{first:?}");
    assert_eq!(after.unwrap_err(), FrameError::Closed);
}

#[test]
fn a_closed_socket_ends_the_conversation() {
    let (worker, daemon) = UnixStream::pair().unwrap();
    drop(daemon);
    let mut conv = SocketConverse::stoppable(worker, &NEVER);
    assert_eq!(conv.prompt(AuthKind::Visible, "login:"), None);
    assert!(conv.ended().get());
}

#[test]
fn worker_messages_round_trip() {
    use repriselogin::frame::{decode, encode};
    let to = [
        ToWorker::Begin {
            user: "anna".into(),
            autologin_user: Some("anna".into()),
            greeter_socket: true,
            rhost: Some("192.0.2.1".into()),
        },
        ToWorker::Answer(Some(Zeroizing::new("pw".into()))),
        ToWorker::Answer(None),
        ToWorker::Start {
            vt: 7,
            background: true,
        },
        ToWorker::Cancel,
    ];
    for m in to {
        let frame = encode(&m).unwrap();
        assert_eq!(decode::<ToWorker>(&frame[4..]).unwrap(), m);
    }
    let from = [
        FromWorker::Prompt {
            kind: AuthKind::Visible,
            text: "login:".into(),
        },
        FromWorker::Authenticated {
            user: "anna".into(),
            uid: 1000,
        },
        FromWorker::Started {
            session_id: "3".into(),
            uid: 1000,
        },
        FromWorker::Failed {
            kind: ErrorKind::Auth,
            text: "Authentication failure".into(),
        },
    ];
    for m in from {
        let frame = encode(&m).unwrap();
        assert_eq!(decode::<FromWorker>(&frame[4..]).unwrap(), m);
    }
}

#[test]
fn an_account_refusal_carries_pams_text_and_a_failed_password_the_fixed_one() {
    let err = |code, text: &str| PamError {
        code,
        text: text.into(),
    };
    let expired = || Err(err(13, "User account has expired"));
    let (_, kind, text) = refusal(Ok(()), expired).expect("refused");
    assert_eq!(
        (kind, text.as_str()),
        (ErrorKind::Account, "User account has expired")
    );
    let (_, kind, text) =
        refusal(Err(err(7, "Authentication failure")), || unreachable!()).expect("refused");
    assert_eq!((kind, text.as_str()), (ErrorKind::Auth, AUTH_FAILED));
    assert!(refusal(Ok(()), || Ok(())).is_none());
}

#[test]
fn a_missing_account_is_named_by_pams_user_only() {
    let err = account_of(Some("anna".into()), |_| Ok(None)).unwrap_err();
    assert_eq!(err, "anna has no passwd entry");
    assert_eq!(
        account_of(None, |_| unreachable!()).unwrap_err(),
        "PAM names no user"
    );
}

#[test]
fn authenticated_names_the_account_pam_resolved() {
    let m = FromWorker::Authenticated {
        user: "anna".into(),
        uid: 1000,
    };
    let frame = repriselogin::frame::encode(&m).unwrap();
    assert_eq!(
        std::str::from_utf8(&frame[4..]).unwrap(),
        r#"{"type":"authenticated","user":"anna","uid":1000}"#
    );
}

#[test]
fn debug_of_an_answer_never_shows_it() {
    let m = ToWorker::Answer(Some(Zeroizing::new("hunter2".into())));
    assert_eq!(format!("{m:?}"), "Answer(<redacted>)");
    let begin = ToWorker::Begin {
        user: "hunter2".into(),
        autologin_user: None,
        greeter_socket: false,
        rhost: Some("192.0.2.1".into()),
    };
    assert_eq!(format!("{begin:?}"), "Begin { <redacted> }");
    assert_eq!(format!("{:?}", ToWorker::Answer(None)), "Answer(None)");
    assert_eq!(
        format!(
            "{:?}",
            ToWorker::Start {
                vt: 7,
                background: true
            }
        ),
        "Start { vt: 7, background: true }"
    );
}

#[test]
fn session_env_is_exactly_the_listed_variables() {
    let pam = pairs(&[
        ("XDG_SESSION_ID", "3"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
    ]);
    assert_eq!(
        session_env(&anna(), Mode::Login, pam),
        pairs(&[
            ("HOME", "/home/anna"),
            ("USER", "anna"),
            ("LOGNAME", "anna"),
            ("SHELL", "/bin/bash"),
            ("PATH", "/usr/local/sbin:/usr/local/bin:/usr/bin"),
            ("XDG_CURRENT_DESKTOP", "reprise"),
            ("DESKTOP_SESSION", "reprise"),
            ("REPRISE_LOGIN_SOCKET", "/run/reprise-login/session.sock"),
            ("XDG_SESSION_ID", "3"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ])
    );
}

#[test]
fn the_pam_environment_wins_a_clash() {
    let env = session_env(&anna(), Mode::Autologin, pairs(&[("PATH", "/opt/bin")]));
    let paths: Vec<_> = env.iter().filter(|(k, _)| k == "PATH").collect();
    assert_eq!(paths, [&("PATH".to_string(), "/opt/bin".to_string())]);
}

#[test]
fn the_greeter_gets_no_login_socket_variable() {
    let env = session_env(&anna(), Mode::Greeter, vec![]);
    assert!(
        env.iter().all(|(k, _)| k != "REPRISE_LOGIN_SOCKET"),
        "{env:?}"
    );
    assert_eq!(env.len(), 7);
}

#[test]
fn session_argv_runs_the_login_shell() {
    let argv = ["/bin/bash", "-l", "-c", "exec reprise"];
    assert_eq!(session_argv(&anna(), Mode::Login, false), argv);
    assert_eq!(session_argv(&anna(), Mode::Autologin, true), argv);
    let shell_less = PasswdEntry {
        shell: String::new(),
        ..anna()
    };
    assert_eq!(session_argv(&shell_less, Mode::Login, false)[0], "/bin/sh");
}

#[test]
fn the_greeter_argv_is_reprise_greeter_with_a_socket_only_when_asked() {
    assert_eq!(
        session_argv(&anna(), Mode::Greeter, false),
        ["reprise", "--greeter"]
    );
    assert_eq!(
        session_argv(&anna(), Mode::Greeter, true),
        ["reprise", "--greeter", "--greeter-socket"]
    );
}

#[test]
fn the_first_executable_on_the_path_wins() {
    let dir = tempfile::tempdir().unwrap();
    let (local, usr) = (dir.path().join("local"), dir.path().join("usr"));
    for (d, mode) in [(&local, 0o644), (&usr, 0o755)] {
        std::fs::create_dir(d).unwrap();
        std::fs::write(d.join("reprise"), "").unwrap();
        std::fs::set_permissions(d.join("reprise"), std::fs::Permissions::from_mode(mode)).unwrap();
    }
    let path = format!("{}:{}", local.display(), usr.display());
    assert_eq!(find_program("reprise", &path), Some(usr.join("reprise")));
    std::fs::set_permissions(
        local.join("reprise"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(find_program("reprise", &path), Some(local.join("reprise")));
    assert_eq!(find_program("nothing", &path), None);
    assert_eq!(
        find_program("/bin/bash", &path),
        Some(PathBuf::from("/bin/bash"))
    );
}

#[test]
fn locale_conf_gives_lang_and_lc_variables() {
    let conf = "# set by localectl\nLANG=de_DE.UTF-8\nLC_TIME=\"en_GB.UTF-8\"\nKEYMAP=de\n\n";
    assert_eq!(
        locale_env(conf),
        pairs(&[("LANG", "de_DE.UTF-8"), ("LC_TIME", "en_GB.UTF-8")])
    );
}

static NEVER: AtomicBool = AtomicBool::new(false);

#[test]
fn a_stop_while_prompting_ends_the_conversation() {
    static STOPPED: AtomicBool = AtomicBool::new(false);
    let (worker, mut daemon) = UnixStream::pair().unwrap();
    worker
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let mut conv = SocketConverse::stoppable(worker, &STOPPED);
    let daemon = std::thread::spawn(move || {
        let prompt = read_msg::<FromWorker>(&mut daemon).unwrap();
        STOPPED.store(true, Ordering::Relaxed);
        (prompt, daemon)
    });
    assert_eq!(conv.prompt(AuthKind::Secret, "Password:"), None);
    assert!(conv.ended().get());
    let (prompt, _daemon) = daemon.join().unwrap();
    assert!(matches!(prompt, FromWorker::Prompt { .. }), "{prompt:?}");
}

#[test]
fn sigterm_ends_a_blocked_socket_read() {
    use std::os::unix::thread::JoinHandleExt;
    install_stop_handler().unwrap();
    let (worker, daemon) = UnixStream::pair().unwrap();
    worker
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let reader = std::thread::spawn(move || read_until_stopped::<ToWorker>(&worker, &STOP));
    std::thread::sleep(Duration::from_millis(50));
    // SAFETY: the thread is alive until joined below; SIGTERM has the stop handler.
    unsafe { libc::pthread_kill(reader.as_pthread_t(), libc::SIGTERM) };
    let r = reader.join().unwrap();
    assert!(matches!(r, Err(FrameError::Io(_))), "{r:?}");
    assert!(STOP.load(Ordering::Relaxed));
    drop(daemon);
}

/// The worker's groups as `setgroups` leaves them; `pam_setcred` adds `added`.
#[derive(Default)]
struct FakeCreds {
    groups: Vec<libc::gid_t>,
    added: libc::gid_t,
    groups_fail: bool,
    deleted: bool,
}

impl Creds for FakeCreds {
    fn set_groups(&mut self, groups: &[libc::gid_t]) -> std::io::Result<()> {
        self.groups = groups.to_vec();
        Ok(())
    }

    fn establish_cred(&mut self) -> Result<(), PamError> {
        self.groups.push(self.added);
        Ok(())
    }

    fn groups(&self) -> std::io::Result<Vec<libc::gid_t>> {
        match self.groups_fail {
            true => Err(std::io::ErrorKind::OutOfMemory.into()),
            false => Ok(self.groups.clone()),
        }
    }

    fn delete_cred(&mut self) -> Result<(), PamError> {
        self.deleted = true;
        Ok(())
    }
}

#[test]
fn groups_added_by_setcred_reach_the_session() {
    let mut creds = FakeCreds {
        groups: vec![0],
        added: 990,
        ..FakeCreds::default()
    };
    assert_eq!(
        establish_creds(&mut creds, &[1000, 998]),
        Ok(vec![1000, 998, 990])
    );
    assert!(!creds.deleted);
}

#[test]
fn a_getgroups_failure_after_setcred_deletes_the_credentials() {
    let mut creds = FakeCreds {
        groups_fail: true,
        ..FakeCreds::default()
    };
    assert!(establish_creds(&mut creds, &[1000]).is_err());
    assert!(creds.deleted, "pam_setcred's credentials are deleted again");
}

#[test]
fn sighup_stops_the_worker_like_sigterm() {
    install_stop_handler().unwrap();
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: a query of the disposition into a zeroed local.
        let handler = unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            assert_eq!(libc::sigaction(sig, std::ptr::null(), &mut old), 0);
            old.sa_sigaction
        };
        assert_eq!(
            handler, on_stop as extern "C" fn(c_int) as libc::sighandler_t,
            "signal {sig}"
        );
    }
    assert!(STOP_SIGNALS.contains(&libc::SIGHUP));
}

#[test]
fn the_worker_socket_times_out_reads_and_writes() {
    let (worker, _daemon) = UnixStream::pair().unwrap();
    prepare_socket(&worker).unwrap();
    assert_eq!(worker.read_timeout().unwrap(), Some(STOP_POLL));
    assert_eq!(worker.write_timeout().unwrap(), Some(WRITE_WAIT));
}

#[test]
fn the_remote_stacks_refuse_root_and_skip_faillock() {
    let remote = include_str!("../../../../../data/pam.d/reprise-login-remote");
    let root = auth_line(remote, "pam_succeed_if.so quiet_success uid ne 0");
    let shells = auth_line(remote, "pam_shells.so");
    let unix = auth_line(remote, "pam_unix.so");
    assert!(root.is_some() && root < shells && shells < unix, "{remote}");
    let unix = remote.lines().nth(unix.unwrap()).unwrap();
    assert!(unix.ends_with("pam_unix.so nodelay"), "{unix}");
    let faillock = (remote.lines()).any(|l| !l.starts_with('#') && l.contains("pam_faillock"));
    assert!(!faillock, "{remote}");

    let nla = include_str!("../../../../../data/pam.d/reprise-login-remote-nla");
    let uid = auth_line(nla, "pam_succeed_if.so uid >= 1000");
    let shells = auth_line(nla, "pam_shells.so");
    assert!(uid.is_some() && uid < shells, "{nla}");
    assert!(shells < auth_line(nla, "pam_permit.so"), "{nla}");
}

#[test]
fn the_check_stack_refuses_root_before_its_password() {
    let check = include_str!("../../../../../data/pam.d/reprise-login-check");
    let stack: Vec<Vec<&str>> = (check.lines())
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split_whitespace().collect())
        .collect();
    let root: Vec<&str> = "auth [success=ok user_unknown=ignore default=die] pam_succeed_if.so \
                           quiet_success uid ne 0"
        .split_whitespace()
        .collect();
    assert_eq!(
        stack,
        [
            root,
            vec!["auth", "include", "system-auth"],
            vec!["account", "include", "system-auth"]
        ]
    );
}

type Calls = Rc<std::cell::RefCell<Vec<String>>>;

/// A PAM transaction that records its calls; `authenticate` asks `prompt` first if set.
struct FakePam {
    calls: Calls,
    conv: Box<dyn Converse>,
    prompt: Option<(AuthKind, &'static str)>,
    env: Vec<(String, String)>,
    groups: Vec<libc::gid_t>,
}

impl FakePam {
    fn call(&self, call: impl Into<String>) -> Result<(), PamError> {
        self.calls.borrow_mut().push(call.into());
        Ok(())
    }
}

impl Creds for FakePam {
    fn set_groups(&mut self, groups: &[libc::gid_t]) -> std::io::Result<()> {
        self.groups = groups.to_vec();
        Ok(())
    }

    fn establish_cred(&mut self) -> Result<(), PamError> {
        self.call("setcred")
    }

    fn groups(&self) -> std::io::Result<Vec<libc::gid_t>> {
        Ok(self.groups.clone())
    }

    fn delete_cred(&mut self) -> Result<(), PamError> {
        self.call("delete_cred")
    }
}

impl LoginPam for FakePam {
    fn set_item_rhost(&mut self, rhost: Option<&str>) -> Result<(), PamError> {
        self.call(format!("set_item_rhost({rhost:?})"))
    }

    fn authenticate(&mut self) -> Result<(), PamError> {
        self.call("authenticate")?;
        if let Some((kind, text)) = self.prompt
            && self.conv.prompt(kind, text).is_none()
        {
            return Err(PamError {
                code: crate::pam::ffi::PAM_CONV_ERR,
                text: "Conversation error".into(),
            });
        }
        Ok(())
    }

    fn account(&mut self) -> Result<(), PamError> {
        self.call("acct_mgmt")
    }

    fn user(&self) -> Option<String> {
        Some("anna".into())
    }

    fn set_tty(&mut self, tty: &str) -> Result<(), PamError> {
        self.env.push(("PAM_TTY".into(), tty.into()));
        Ok(())
    }

    fn putenv(&mut self, key: &str, value: &str) -> Result<(), PamError> {
        self.env.push((key.into(), value.into()));
        Ok(())
    }

    fn open_session(&mut self) -> Result<(), PamError> {
        self.call("open_session")
    }
}

#[derive(Default)]
struct FakeVts(Vec<String>);

impl Vts for FakeVts {
    type Held = ();

    fn hold(&mut self, vt: u16) -> std::io::Result<()> {
        self.0.push(format!("hold({vt})"));
        Ok(())
    }

    fn activate_and_wait(&mut self, vt: u16, _: &dyn Fn() -> bool) -> std::io::Result<()> {
        self.0.push(format!("activate_and_wait({vt})"));
        Ok(())
    }
}

/// What a `login` against the fakes did.
struct Ran {
    /// The exit status, or `None` when a session was opened.
    status: Option<i32>,
    pam: Vec<String>,
    vts: Vec<String>,
    env: Vec<(String, String)>,
    sent: Vec<FromWorker>,
}

/// Runs `login` in `mode` against the fakes, with `queued` waiting for the worker and the
/// daemon's side closed after them.
fn run_login(mode: Mode, queued: Vec<ToWorker>, prompt: Option<(AuthKind, &'static str)>) -> Ran {
    let (mut worker, mut daemon) = UnixStream::pair().unwrap();
    for m in &queued {
        write_msg(&mut daemon, m).unwrap();
    }
    daemon.shutdown(std::net::Shutdown::Write).unwrap();
    let calls = Calls::default();
    let start = |_: &str, _: &str, conv: Box<dyn Converse>| {
        Ok(FakePam {
            calls: calls.clone(),
            conv,
            prompt,
            env: Vec::new(),
            groups: Vec::new(),
        })
    };
    let mut vts = FakeVts::default();
    let lookup = |_: &str| Ok(Some(anna()));
    let (status, env) = match login(mode, &mut worker, &NEVER, start, lookup, &mut vts) {
        Login::Done(status) => (Some(status), Vec::new()),
        Login::Opened(opened) => (None, opened.pam.env),
    };
    drop(worker);
    let mut sent = Vec::new();
    while let Ok(m) = read_msg::<FromWorker>(&mut daemon) {
        sent.push(m);
    }
    let pam = calls.borrow().clone();
    Ran {
        status,
        pam,
        vts: vts.0,
        env,
        sent,
    }
}

fn begin(rhost: Option<&str>) -> ToWorker {
    ToWorker::Begin {
        user: "anna".into(),
        autologin_user: None,
        greeter_socket: false,
        rhost: rhost.map(str::to_owned),
    }
}

fn answer(pw: &str) -> ToWorker {
    ToWorker::Answer(Some(Zeroizing::new(pw.into())))
}

const PASSWORD: Option<(AuthKind, &str)> = Some((AuthKind::Secret, "Password:"));

#[test]
fn rhost_is_set_for_authentication_and_cleared_before_the_session_opens() {
    let start = ToWorker::Start {
        vt: 7,
        background: true,
    };
    let queued = vec![begin(Some("192.0.2.7")), answer("pw"), start];
    let ran = run_login(Mode::Remote, queued, PASSWORD);
    assert_eq!(ran.status, None, "the session opened: {:?}", ran.sent);
    assert_eq!(
        ran.pam,
        [
            r#"set_item_rhost(Some("192.0.2.7"))"#,
            "authenticate",
            "acct_mgmt",
            "set_item_rhost(None)",
            "setcred",
            "open_session"
        ]
    );
    assert_eq!(
        ran.sent,
        [
            FromWorker::Prompt {
                kind: AuthKind::Secret,
                text: "Password:".into()
            },
            FromWorker::Authenticated {
                user: "anna".into(),
                uid: 1000
            }
        ],
        "a remote login relays its prompt as a console one does"
    );
}

#[test]
fn a_background_start_never_activates_the_vt() {
    let start = |background| ToWorker::Start { vt: 7, background };
    let queued = vec![begin(Some("192.0.2.7")), answer("pw"), start(true)];
    let ran = run_login(Mode::Remote, queued, PASSWORD);
    assert_eq!(ran.status, None);
    assert_eq!(ran.vts, ["hold(7)"]);
    let console = run_login(
        Mode::Login,
        vec![begin(None), answer("pw"), start(false)],
        PASSWORD,
    );
    assert_eq!(console.vts, ["hold(7)", "activate_and_wait(7)"]);
    assert_eq!(
        ran.env, console.env,
        "a background session's tty and XDG variables"
    );
    for want in [
        ("PAM_TTY", "tty7"),
        ("XDG_VTNR", "7"),
        ("XDG_SESSION_CLASS", "user"),
        ("XDG_SESSION_TYPE", "wayland"),
    ] {
        assert!(
            ran.env.contains(&(want.0.into(), want.1.into())),
            "{want:?}"
        );
    }
}

#[test]
fn a_remote_nla_worker_relays_no_prompt() {
    let ran = run_login(Mode::RemoteNla, vec![begin(Some("192.0.2.7"))], PASSWORD);
    assert_eq!(ran.status, Some(1));
    assert_eq!(
        ran.sent,
        [FromWorker::Failed {
            kind: ErrorKind::Refused,
            text: REFUSED_PROMPT.into()
        }]
    );
    assert_eq!(
        ran.pam,
        [r#"set_item_rhost(Some("192.0.2.7"))"#, "authenticate"]
    );
}

#[test]
fn a_check_worker_ends_after_authenticated_and_opens_nothing() {
    let start = ToWorker::Start {
        vt: 7,
        background: false,
    };
    let ran = run_login(
        Mode::Check,
        vec![begin(None), answer("pw"), start],
        PASSWORD,
    );
    assert_eq!(ran.status, Some(0));
    assert_eq!(ran.pam, ["authenticate", "acct_mgmt"]);
    assert!(ran.vts.is_empty(), "{:?}", ran.vts);
    assert_eq!(
        ran.sent,
        [
            FromWorker::Prompt {
                kind: AuthKind::Secret,
                text: "Password:".into()
            },
            FromWorker::Authenticated {
                user: "anna".into(),
                uid: 1000
            }
        ]
    );
}
