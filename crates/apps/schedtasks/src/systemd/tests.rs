use super::*;
use crate::model::NEVER;
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
    /// A service's `ExecMainStartTimestamp` and a unit's `InactiveExitTimestamp`.
    main_start: u64,
    inactive_exit: u64,
    exec: Vec<Exec>,
    result: &'static str,
    code: i32,
    status: i32,
    /// The unit's `FragmentPath`; empty for `HOME/<description>`.
    fragment: String,
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

    #[zbus(property)]
    fn unit_path(&self) -> Vec<String> {
        vec![
            format!("{HOME}.control"),
            HOME.into(),
            "/etc/systemd/user".into(),
        ]
    }

    fn start_unit(&self, name: String, mode: String) -> fdo::Result<OwnedObjectPath> {
        if name == "refused.service" || name.contains("start-refused") {
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

    fn enable_unit_files(
        &self,
        files: Vec<String>,
        runtime: bool,
        force: bool,
    ) -> fdo::Result<bool> {
        if files.iter().any(|f| f.contains("enable-refused")) {
            return Err(fdo::Error::FileNotFound("Unit file does not exist.".into()));
        }
        self.note(format!(
            "EnableUnitFiles {} {runtime} {force}",
            files.join(",")
        ));
        Ok(true)
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

    fn reset_failed_unit(&self, name: String) {
        self.note(format!("ResetFailedUnit {name}"));
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
    fn inactive_exit_timestamp(&self) -> u64 {
        self.0.inactive_exit
    }
    #[zbus(property)]
    fn fragment_path(&self) -> String {
        match self.0.fragment.as_str() {
            "" => format!("{HOME}/{}", self.0.description),
            f => f.into(),
        }
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
    fn exec_main_start_timestamp(&self) -> u64 {
        self.0.main_start
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
                main_start: 1_940_000 * S,
                inactive_exit: 1_940_000 * S,
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
    manager_with(bus, Vec::new()).await
}

/// [`manager`] with `more` units loaded besides [`units`].
async fn manager_with(
    bus: &testbus::Bus,
    more: Vec<(&'static str, Spec)>,
) -> (Connection, Connection, Calls) {
    let fake = testbus::connect(&bus.1).await;
    let calls = Calls::default();
    let mut units = units();
    units.extend(more);
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
        assert_eq!(unit_dir(&conn).await.map(|f| f.own), Some(HOME.into()));
        let (tasks, paths) = read(&conn, now, None).await.unwrap();
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
        assert_eq!(
            backup.last,
            Some(1_940_000 * S),
            "Run Now after the timer last fired"
        );
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

/// `dir` as the own folder, with no other folder searched.
fn own(dir: &Path) -> units::Folders {
    units::Folders {
        own: dir.into(),
        search: vec![],
    }
}

fn new_task() -> NewTask {
    NewTask {
        name: "Back up".into(),
        program: "/bin/true".into(),
        args: Vec::new(),
        schedule: units::Schedule::Daily { at: (3, 0) },
    }
}

#[test]
fn new_task_writes_reloads_and_enables_its_timer() {
    let Some(bus) = testbus::bus_with_no_services() else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    zbus::block_on(async move {
        let (_fake, conn, calls) = manager(&bus).await;
        let taken = ["reprise-task-back-up.timer".to_string()];
        let timer = create(&conn, Some(&own(&dir)), &new_task(), &taken)
            .await
            .unwrap();
        assert_eq!(timer, "reprise-task-back-up-2.timer");
        assert!(dir.join("reprise-task-back-up-2.service").is_file());
        assert_eq!(
            *lock(&calls),
            [
                "EnableUnitFiles reprise-task-back-up-2.timer false false",
                "Reload",
                "StartUnit reprise-task-back-up-2.timer replace",
            ]
        );
        let none = create(&conn, None, &new_task(), &[]).await;
        assert_eq!(
            none.map_err(|e| told(&e)),
            Err("the user manager lists no folder for your own unit files".into())
        );
    });
}

/// Every name in `dir` and its `timers.target.wants`.
fn left(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = walk(dir)
        .chain(walk(&dir.join("timers.target.wants")))
        .collect();
    v.sort();
    v
}

fn walk(dir: &Path) -> impl Iterator<Item = String> {
    let read = std::fs::read_dir(dir).into_iter().flatten();
    read.map(|e| e.unwrap().file_name().into_string().unwrap())
}

#[test]
fn a_create_the_manager_refuses_after_writing_leaves_nothing() {
    let Some(bus) = testbus::bus_with_no_services() else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    zbus::block_on(async move {
        let (_fake, conn, calls) = manager(&bus).await;
        let task = |name: &str| NewTask {
            name: name.into(),
            ..new_task()
        };
        let refused = create(&conn, Some(&own(&dir)), &task("enable refused"), &[]).await;
        assert_eq!(
            refused.map_err(|e| told(&e)),
            Err("Unit file does not exist.".into())
        );
        assert_eq!(left(&dir), Vec::<String>::new());
        let stale = "reprise-task-start-refused.timer";
        let wants = dir.join("timers.target.wants");
        std::fs::create_dir(&wants).unwrap();
        std::os::unix::fs::symlink(dir.join(stale), wants.join(stale)).unwrap();
        let refused = create(&conn, Some(&own(&dir)), &task("start refused"), &[]).await;
        assert_eq!(
            refused.map_err(|e| told(&e)),
            Err("Interactive authentication required.".into())
        );
        assert_eq!(
            left(&dir),
            [stale, "timers.target.wants"],
            "the files are gone, and the link the Create did not make stays"
        );
        let timer = "reprise-task-start-refused-2.timer";
        assert_eq!(
            *lock(&calls),
            [
                "DisableUnitFiles reprise-task-enable-refused.timer false",
                "Reload",
                &format!("EnableUnitFiles {timer} false false"),
                "Reload",
                &format!("DisableUnitFiles {timer} false"),
                "Reload",
            ]
        );
    });
}

#[test]
fn delete_removes_only_a_pair_new_task_wrote() {
    let Some(bus) = testbus::bus_with_no_services() else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let folders = own(&dir);
    let timer = units::write(&folders, &new_task(), &[]).unwrap();
    assert_eq!(timer, "reprise-task-back-up.timer");
    let made = Spec {
        load: "loaded",
        runs: Some("other.service"),
        fragment: dir.join(&timer).to_string_lossy().into_owned(),
        ..Spec::default()
    };
    let theirs = dir.join("backup.timer");
    std::fs::write(&theirs, format!("{}\n", units::MARKER)).unwrap();
    std::fs::write(dir.join("backup.service"), format!("{}\n", units::MARKER)).unwrap();
    zbus::block_on(async move {
        let (_fake, conn, calls) =
            manager_with(&bus, vec![("reprise-task-back-up.timer", made)]).await;
        let refused = delete(&conn, Some(&dir), "backup.timer").await;
        assert_eq!(
            refused.map_err(|e| told(&e)),
            Err("Scheduled Tasks did not make this task".into())
        );
        assert!(
            theirs.exists() && lock(&calls).is_empty(),
            "nothing touched"
        );
        let service = timer.replace(".timer", ".service");
        delete(&conn, Some(&dir), &timer).await.unwrap();
        assert_eq!(
            *lock(&calls),
            [
                format!("DisableUnitFiles {timer} false"),
                format!("StopUnit {timer} replace"),
                format!("StopUnit {service} replace"),
                "Reload".to_string(),
                format!("ResetFailedUnit {service}"),
            ]
        );
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["backup.service", "backup.timer"],
            "the service named by the unit, not the timer's Unit="
        );
        units::write(&folders, &new_task(), &[]).unwrap();
        std::fs::remove_file(dir.join(&service)).unwrap();
        delete(&conn, Some(&dir), &timer).await.unwrap();
        assert!(
            !dir.join(&timer).exists(),
            "a half-done Delete is done again"
        );
    });
}

#[test]
fn edit_rewrites_only_a_pair_new_task_wrote_unchanged_since_and_reloads() {
    let Some(bus) = testbus::bus_with_no_services() else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let timer = units::write(&own(&dir), &new_task(), &[]).unwrap();
    let made = Spec {
        load: "loaded",
        fragment: dir.join(&timer).to_string_lossy().into_owned(),
        ..Spec::default()
    };
    let theirs = dir.join("backup.timer");
    std::fs::write(&theirs, format!("{}\n", units::MARKER)).unwrap();
    std::fs::write(dir.join("backup.service"), format!("{}\n", units::MARKER)).unwrap();
    let changed = NewTask {
        name: "Back up hourly".into(),
        schedule: units::Schedule::Hourly,
        ..new_task()
    };
    zbus::block_on(async move {
        let (_fake, conn, calls) =
            manager_with(&bus, vec![("reprise-task-back-up.timer", made)]).await;
        let was = new_task();
        let refused = edit(&conn, Some(&dir), "backup.timer", &was, &changed).await;
        assert_eq!(
            refused.map_err(|e| told(&e)),
            Err("Scheduled Tasks did not make this task".into())
        );
        assert_eq!(
            std::fs::read_to_string(&theirs).unwrap(),
            format!("{}\n", units::MARKER)
        );
        let path = dir.join(&timer).with_extension("service");
        let opened = std::fs::read_to_string(&path).unwrap();
        let by_hand = opened.clone() + "Environment=A=1\n";
        std::fs::write(&path, &by_hand).unwrap();
        let refused = edit(&conn, Some(&dir), &timer, &was, &changed).await;
        assert_eq!(
            refused.map_err(|e| told(&e)),
            Err("its files changed since Edit opened them".into())
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            by_hand,
            "a change made while the box was open stays"
        );
        std::fs::write(&path, &opened).unwrap();
        edit(&conn, Some(&dir), &timer, &was, &changed)
            .await
            .unwrap();
        assert_eq!(units::read(&dir, &timer), Some(changed));
        assert_eq!(*lock(&calls), ["Reload"]);
    });
}
