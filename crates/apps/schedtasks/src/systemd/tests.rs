use super::*;
use reprisebus::testbus;
use zbus::fdo;

const S: u64 = 1_000_000;
const HOME: &str = "/home/u/.config/systemd/user";

/// What one fake unit reports.
#[derive(Clone, Default)]
struct Spec {
    load: &'static str,
    active: &'static str,
    file: &'static str,
    description: &'static str,
    /// A timer's unit; None for a service.
    runs: Option<&'static str>,
    calendar: Vec<(String, String, u64)>,
    monotonic: Vec<(String, u64, u64)>,
    next_realtime: u64,
    next_monotonic: u64,
    last: u64,
    exec: Vec<Exec>,
    result: &'static str,
    code: i32,
    status: i32,
}

fn object(name: &str) -> String {
    let escaped: String = name
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => (b as char).to_string(),
            b => format!("_{b:02x}"),
        })
        .collect();
    format!("{UNITS}/{escaped}")
}

/// The calls the manager took, as `Method arg`.
type Calls = Arc<Mutex<Vec<String>>>;

struct Manager {
    units: Vec<(&'static str, Spec)>,
    /// Unit files with no loaded unit behind them.
    files: Vec<&'static str>,
    calls: Calls,
}

impl Manager {
    fn note(&self, call: String) {
        lock(&self.calls).push(call);
    }
}

fn path(name: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(object(name)).unwrap()
}

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl Manager {
    fn list_units_by_patterns(&self, _states: Vec<String>, patterns: Vec<String>) -> Vec<Listed> {
        assert_eq!(patterns, ["*.timer"]);
        let job = OwnedObjectPath::try_from("/").unwrap();
        (self.units.iter())
            .filter(|(n, s)| n.ends_with(".timer") && !s.load.is_empty())
            .map(|(n, s)| {
                let (n, load, active) = (n.to_string(), s.load.into(), s.active.into());
                (
                    n.clone(),
                    n.clone(),
                    load,
                    active,
                    "".into(),
                    "".into(),
                    path(&n),
                    0,
                    "".into(),
                    job.clone(),
                )
            })
            .collect()
    }

    fn list_unit_files_by_patterns(
        &self,
        _states: Vec<String>,
        _patterns: Vec<String>,
    ) -> Vec<(String, String)> {
        (self.files.iter())
            .map(|f| (format!("{HOME}/{f}"), "disabled".to_string()))
            .collect()
    }

    fn load_unit(&self, name: String) -> fdo::Result<OwnedObjectPath> {
        Ok(path(&name))
    }

    fn start_unit(&self, name: String, mode: String) -> fdo::Result<OwnedObjectPath> {
        if name == "refused.service" {
            return Err(fdo::Error::AccessDenied(
                "Interactive authentication required.".into(),
            ));
        }
        self.note(format!("StartUnit {name} {mode}"));
        Ok(path("job"))
    }

    fn stop_unit(&self, name: String, mode: String) -> OwnedObjectPath {
        self.note(format!("StopUnit {name} {mode}"));
        path("job")
    }

    fn enable_unit_files(&self, files: Vec<String>, runtime: bool, force: bool) -> bool {
        self.note(format!(
            "EnableUnitFiles {} {runtime} {force}",
            files.join(",")
        ));
        true
    }

    fn disable_unit_files(
        &self,
        files: Vec<String>,
        runtime: bool,
    ) -> Vec<(String, String, String)> {
        self.note(format!("DisableUnitFiles {} {runtime}", files.join(",")));
        Vec::new()
    }

    fn reload(&self) {
        self.note("Reload".into());
    }
}

struct Unit(Spec);

#[zbus::interface(name = "org.freedesktop.systemd1.Unit")]
impl Unit {
    #[zbus(property)]
    fn load_state(&self) -> String {
        self.0.load.into()
    }
    #[zbus(property)]
    fn active_state(&self) -> String {
        self.0.active.into()
    }
    #[zbus(property)]
    fn unit_file_state(&self) -> String {
        self.0.file.into()
    }
    #[zbus(property)]
    fn description(&self) -> String {
        self.0.description.into()
    }
    #[zbus(property)]
    fn fragment_path(&self) -> String {
        format!("{HOME}/{}", self.0.description)
    }
}

struct Timer(Spec);

#[zbus::interface(name = "org.freedesktop.systemd1.Timer")]
impl Timer {
    #[zbus(property)]
    fn unit(&self) -> String {
        self.0.runs.unwrap_or_default().into()
    }
    #[zbus(property)]
    fn timers_calendar(&self) -> Vec<(String, String, u64)> {
        self.0.calendar.clone()
    }
    #[zbus(property)]
    fn timers_monotonic(&self) -> Vec<(String, u64, u64)> {
        self.0.monotonic.clone()
    }
    #[zbus(property, name = "NextElapseUSecRealtime")]
    fn next_realtime(&self) -> u64 {
        self.0.next_realtime
    }
    #[zbus(property, name = "NextElapseUSecMonotonic")]
    fn next_monotonic(&self) -> u64 {
        self.0.next_monotonic
    }
    #[zbus(property, name = "LastTriggerUSec")]
    fn last(&self) -> u64 {
        self.0.last
    }
}

struct Service(Spec);

#[zbus::interface(name = "org.freedesktop.systemd1.Service")]
impl Service {
    #[zbus(property)]
    fn exec_start(&self) -> Vec<Exec> {
        self.0.exec.clone()
    }
    #[zbus(property)]
    fn result(&self) -> String {
        self.0.result.into()
    }
    #[zbus(property)]
    fn exec_main_code(&self) -> i32 {
        self.0.code
    }
    #[zbus(property)]
    fn exec_main_status(&self) -> i32 {
        self.0.status
    }
}

fn exec(argv: &[&str]) -> Exec {
    let argv: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    (argv[0].clone(), argv, false, 0, 0, 0, 0, 0, 0, 0)
}

fn units() -> Vec<(&'static str, Spec)> {
    let loaded = Spec {
        load: "loaded",
        ..Spec::default()
    };
    vec![
        (
            "backup.timer",
            Spec {
                active: "active",
                file: "enabled",
                description: "backup.timer",
                runs: Some("backup.service"),
                calendar: vec![("OnCalendar".into(), "*-*-* 03:00:00".into(), 0)],
                next_realtime: 2_000_000 * S,
                last: 1_900_000 * S,
                ..loaded.clone()
            },
        ),
        (
            "backup.service",
            Spec {
                active: "inactive",
                description: "Copy the photos",
                exec: vec![exec(&["/usr/bin/rsync", "-a", "/home/u/My Photos", "/mnt"])],
                result: "exit-code",
                code: libc::CLD_EXITED,
                status: 23,
                ..loaded.clone()
            },
        ),
        (
            "gone.timer",
            Spec {
                load: "not-found",
                active: "inactive",
                ..Spec::default()
            },
        ),
        (
            "off.timer",
            Spec {
                active: "inactive",
                file: "disabled",
                description: "off.timer",
                runs: Some("off.service"),
                monotonic: vec![("OnBootUSec".into(), 900 * S, 0)],
                next_realtime: NEVER,
                next_monotonic: NEVER,
                ..loaded.clone()
            },
        ),
        (
            "off.service",
            Spec {
                load: "not-found",
                active: "inactive",
                description: "off.service",
                ..Spec::default()
            },
        ),
    ]
}

/// A bus with the fake manager on it, a connection to it and the manager's call log.
async fn manager(bus: &testbus::Bus) -> (Connection, Connection, Calls) {
    let fake = testbus::connect(&bus.1).await;
    let calls = Calls::default();
    let units = units();
    for (name, spec) in &units {
        let at = object(name);
        testbus::serve(&fake, at.as_str(), Unit(spec.clone())).await;
        match spec.runs {
            Some(_) => testbus::serve(&fake, at.as_str(), Timer(spec.clone())).await,
            None if spec.load == "loaded" => {
                testbus::serve(&fake, at.as_str(), Service(spec.clone())).await
            }
            None => {}
        }
    }
    let m = Manager {
        units,
        files: vec!["off.timer", "backup.timer", "nightly@.timer"],
        calls: Arc::clone(&calls),
    };
    testbus::serve(&fake, PATH, m).await;
    fake.request_name(DEST).await.unwrap();
    (fake, testbus::connect(&bus.1).await, calls)
}

#[test]
fn the_timers_read_from_the_manager_loaded_and_unit_files_alike() {
    let Some(bus) = testbus::bus_with_no_services() else {
        return;
    };
    zbus::block_on(async move {
        let (_fake, conn, _) = manager(&bus).await;
        let now = (1_950_000 * S, 100 * S);
        let (tasks, paths) = read(&conn, now).await.unwrap();
        let names: Vec<&str> = tasks.iter().map(|t| t.timer.as_str()).collect();
        assert_eq!(
            names,
            ["backup.timer", "off.timer"],
            "a missing timer and a template are left out"
        );
        let backup = &tasks[0];
        assert_eq!(backup.name(), "Copy the photos");
        assert_eq!(backup.service, "backup.service");
        assert_eq!(backup.timer_state, "active");
        assert_eq!(backup.file_state, "enabled");
        assert_eq!(backup.service_state, "inactive");
        assert_eq!(
            backup.calendar,
            [("OnCalendar".to_string(), "*-*-* 03:00:00".to_string())]
        );
        assert_eq!(backup.next, Some(2_000_000 * S));
        assert_eq!(backup.last, Some(1_900_000 * S));
        assert_eq!(
            backup.command,
            "/usr/bin/rsync -a \"/home/u/My Photos\" /mnt"
        );
        assert_eq!(backup.result_text(), "Exited with status 23");
        assert_eq!(backup.file, format!("{HOME}/backup.timer"));
        let off = &tasks[1];
        assert_eq!(off.name(), "off");
        assert_eq!(off.next, None);
        assert_eq!(off.last, None);
        assert_eq!(off.service_state, "", "its unit has no file");
        assert!(!off.can_run());
        assert_eq!(off.monotonic, [("OnBootUSec".to_string(), 900 * S)]);
        assert!(paths.contains(&object("backup.timer")));
        assert!(paths.contains(&object("backup.service")));
        assert!(!paths.contains(&object("gone.timer")));
    });
}

#[test]
fn run_now_starts_the_timers_unit() {
    let Some(bus) = testbus::bus_with_no_services() else {
        return;
    };
    zbus::block_on(async move {
        let (_fake, conn, calls) = manager(&bus).await;
        act(&conn, &Cmd::Run("backup.service".into()))
            .await
            .unwrap();
        assert_eq!(*lock(&calls), ["StartUnit backup.service replace"]);
        let refused = act(&conn, &Cmd::Run("refused.service".into())).await;
        assert_eq!(
            refused.as_ref().map_err(told),
            Err("Interactive authentication required.".to_string())
        );
    });
}

fn turn(timer: &str, file_state: &str, on: bool) -> Cmd {
    Cmd::Turn {
        timer: timer.into(),
        file_state: file_state.into(),
        on,
    }
}

#[test]
fn enable_and_disable_change_the_unit_file_and_start_or_stop_the_timer() {
    let Some(bus) = testbus::bus_with_no_services() else {
        return;
    };
    zbus::block_on(async move {
        let (_fake, conn, calls) = manager(&bus).await;
        act(&conn, &turn("off.timer", "disabled", true))
            .await
            .unwrap();
        assert_eq!(
            std::mem::take(&mut *lock(&calls)),
            [
                "EnableUnitFiles off.timer false false",
                "Reload",
                "StartUnit off.timer replace"
            ]
        );
        act(&conn, &turn("backup.timer", "enabled", false))
            .await
            .unwrap();
        assert_eq!(
            std::mem::take(&mut *lock(&calls)),
            [
                "DisableUnitFiles backup.timer false",
                "Reload",
                "StopUnit backup.timer replace"
            ]
        );
        act(&conn, &turn("run.timer", "enabled-runtime", false))
            .await
            .unwrap();
        assert_eq!(
            std::mem::take(&mut *lock(&calls))[0],
            "DisableUnitFiles run.timer true"
        );
    });
}

#[test]
fn a_static_or_transient_timer_is_only_started_or_stopped() {
    let Some(bus) = testbus::bus_with_no_services() else {
        return;
    };
    zbus::block_on(async move {
        let (_fake, conn, calls) = manager(&bus).await;
        act(&conn, &turn("tmp.timer", "static", true))
            .await
            .unwrap();
        act(&conn, &turn("run-u5.timer", "transient", false))
            .await
            .unwrap();
        assert_eq!(
            *lock(&calls),
            [
                "StartUnit tmp.timer replace",
                "StopUnit run-u5.timer replace"
            ]
        );
    });
}

fn signal(
    path: &str,
    iface: &str,
    member: &str,
    body: &(impl serde::Serialize + DynamicType),
) -> Message {
    Message::signal(path, iface, member).build(body).unwrap()
}

#[test]
fn a_read_follows_timers_shown_units_unit_files_and_finished_reloads() {
    let shown = HashSet::from([object("backup.service")]);
    let changed = |path: &str, iface: &str| {
        let body = (iface, Props::new(), Vec::<String>::new());
        relevant(
            &signal(path, PROPERTIES, "PropertiesChanged", &body),
            &shown,
        )
    };
    assert!(changed(&object("any.timer"), TIMER));
    assert!(changed(&object("backup.service"), UNIT));
    assert!(
        !changed(&object("other.service"), UNIT),
        "a unit no timer shown runs"
    );
    assert!(relevant(
        &signal(PATH, MANAGER, "UnitFilesChanged", &()),
        &shown
    ));
    assert!(relevant(
        &signal(PATH, MANAGER, "Reloading", &false),
        &shown
    ));
    assert!(
        !relevant(&signal(PATH, MANAGER, "Reloading", &true), &shown),
        "a reload starting"
    );
}
