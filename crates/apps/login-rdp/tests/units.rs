use std::path::PathBuf;

fn data(path: &str) -> String {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../data")
        .join(path);
    std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()))
}

fn section<'a>(unit: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("[{name}]");
    let mut inside = false;
    let mut lines = Vec::new();
    for line in unit.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == header;
        } else if inside && !line.is_empty() && !line.starts_with('#') {
            lines.push(line);
        }
    }
    lines
}

fn assert_is(unit: &str, name: &str, wanted: &[&str]) {
    assert_eq!(
        section(unit, name),
        wanted,
        "[{name}] holds exactly these lines"
    );
}

fn assert_has(unit: &str, name: &str, wanted: &[&str]) {
    let lines = section(unit, name);
    for want in wanted {
        assert!(lines.contains(want), "[{name}] has `{want}`: {lines:?}");
    }
}

#[test]
fn the_front_unit_runs_unprivileged_and_sandboxed() {
    let unit = data("systemd/reprise-login-rdp.service");
    assert_is(
        &unit,
        "Unit",
        &[
            "Description=Reprise login manager: Remote Desktop logins",
            "PartOf=reprise-login.service",
            "Requires=reprise-login-rdp.socket",
            "After=reprise-login-rdp.socket reprise-login.service network.target",
        ],
    );
    assert_is(
        &unit,
        "Service",
        &[
            "ExecStartPre=-+/usr/bin/modprobe -q tls",
            "ExecStart=/usr/bin/reprise-login-rdp",
            "User=reprise-login-rdp",
            "Group=reprise-login-rdp",
            "StateDirectory=reprise-login-rdp",
            "StateDirectoryMode=0700",
            "RuntimeDirectory=reprise-login-rdp",
            "RuntimeDirectoryMode=0755",
            "Restart=on-failure",
            "UMask=0077",
            "NoNewPrivileges=yes",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "PrivateDevices=yes",
            "ProtectKernelTunables=yes",
            "ProtectKernelModules=yes",
            "ProtectKernelLogs=yes",
            "ProtectControlGroups=yes",
            "ProtectClock=yes",
            "ProtectHostname=yes",
            "ProtectProc=invisible",
            "ProcSubset=pid",
            "InaccessiblePaths=-/run/dbus",
            "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
            "RestrictNamespaces=yes",
            "RestrictRealtime=yes",
            "RestrictSUIDSGID=yes",
            "LockPersonality=yes",
            "MemoryDenyWriteExecute=yes",
            "SystemCallArchitectures=native",
            "SystemCallFilter=@system-service",
            "SystemCallFilter=~@privileged @resources",
            "CapabilityBoundingSet=",
            "MemoryMax=512M",
            "TasksMax=64",
        ],
    );
    assert!(
        !unit.lines().any(|l| l.trim() == "[Install]"),
        "reprise-login starts and stops the front, so it has no [Install]"
    );
}

#[test]
fn the_front_port_is_held_by_its_socket_from_boot() {
    let socket = data("systemd/reprise-login-rdp.socket");
    assert_is(
        &socket,
        "Socket",
        &[
            "ListenStream=3389",
            "BindIPv6Only=both",
            "Backlog=1024",
            "Accept=no",
        ],
    );
    assert_is(&socket, "Install", &["WantedBy=sockets.target"]);
    assert!(
        !section(&socket, "Unit")
            .iter()
            .any(|l| l.starts_with("PartOf=")),
        "the port stays held while reprise-login or the front restarts"
    );
}

#[test]
fn the_handover_socket_is_owner_only() {
    let socket = data("systemd/reprise-rdp-handover.socket");
    assert_has(
        &socket,
        "Socket",
        &[
            "ListenStream=%t/reprise-rdp/handover.sock",
            "SocketMode=0600",
            "DirectoryMode=0700",
        ],
    );
    assert_has(&socket, "Install", &["WantedBy=sockets.target"]);
}

#[test]
fn the_handover_service_starts_without_waiting_for_the_compositor() {
    let service = data("systemd/reprise-rdp-handover.service");
    assert_is(
        &service,
        "Service",
        &[
            "ExecStart=/usr/bin/reprise-rdp --hand-over",
            "Restart=on-failure",
            "RestartSec=2",
            "RestartSteps=5",
            "RestartMaxDelaySec=60",
            "RestartPreventExitStatus=75",
        ],
    );
    assert!(
        !service.contains("graphical-session.target"),
        "the hand-over starts as soon as the user manager is up: {service}"
    );
}

#[test]
fn the_handover_service_is_not_restarted_after_it_refuses_its_hand_overs() {
    let service = data("systemd/reprise-rdp-handover.service");
    assert_has(&service, "Service", &["RestartPreventExitStatus=75"]);
}

#[test]
fn the_handover_service_has_no_start_limit() {
    let service = data("systemd/reprise-rdp-handover.service");
    assert_is(
        &service,
        "Unit",
        &[
            "Description=Reprise Remote Desktop server for connections the login manager hands over",
            "StartLimitIntervalSec=0",
        ],
    );
}

#[test]
fn the_chvt_rule_refuses_only_inactive_subjects() {
    let rule = data("polkit-1/rules.d/50-reprise-login.rules");
    assert!(rule.contains(r#"action.id == "org.freedesktop.login1.chvt""#));
    assert!(rule.contains("!subject.active"));
    assert!(rule.contains("return polkit.Result.NO;"));
    assert_eq!(
        rule.matches("return ").count(),
        1,
        "an active subject falls through to the other rules: {rule}"
    );
}

#[test]
fn the_remote_group_and_front_user_are_declared() {
    let sysusers = data("sysusers.d/reprise-login.conf");
    let lines: Vec<&str> = sysusers.lines().map(str::trim).collect();
    for want in [
        r#"u reprise-login-rdp - "Reprise Remote Desktop logins" /var/lib/reprise-login-rdp /usr/bin/nologin"#,
        "g reprise-remote -",
    ] {
        assert!(lines.contains(&want), "sysusers has `{want}`: {lines:?}");
    }
}
