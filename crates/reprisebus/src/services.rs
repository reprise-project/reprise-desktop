//! The services of systemd's user and system managers: the loaded units and the unit files, and
//! the jobs on one. A system service's job goes to the system manager, which asks polkit
//! (`org.freedesktop.systemd1.manage-units`) itself; a user service's goes to the user manager.
//! Only user unit files are enabled or disabled.

use zbus::proxy::MethodFlags;
use zbus::{Connection, OwnedObjectPath, Proxy};

use crate::polkit::Told;

const DEST: &str = "org.freedesktop.systemd1";
const PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
/// The polkit action the system manager checks a start, stop or restart against.
pub const MANAGE_UNITS: &str = "org.freedesktop.systemd1.manage-units";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Manager {
    User,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Start,
    Stop,
    Restart,
    /// Enable the user unit file, so the service starts at the next login.
    Enable,
    Disable,
}

/// A loaded service as `ListUnitsByPatterns` gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub name: String,
    pub description: String,
    /// `loaded`, `masked`, `not-found`, ...
    pub load: String,
    /// `active`, `inactive`, `failed`, `activating`, ...
    pub active: String,
    /// The type's own state: `running`, `exited`, `dead`, ...
    pub sub: String,
}

/// A service unit file: its path and its state (`enabled`, `disabled`, `static`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitFile {
    pub path: String,
    pub state: String,
}

/// Name, description, load, active and sub state, followed unit, object path, job id, job type
/// and job path.
type Listed = (
    String,
    String,
    String,
    String,
    String,
    String,
    OwnedObjectPath,
    u32,
    String,
    OwnedObjectPath,
);

/// Whether `name` is a service unit's name: systemd's unit name characters only, so it is no
/// option and no `journalctl` pattern, and the `.service` suffix.
pub fn is_service_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".service") else {
        return false;
    };
    !stem.is_empty()
        && name.len() <= 255
        && !name.starts_with('-')
        && (stem.bytes()).all(|b| b.is_ascii_alphanumeric() || b":-_.\\@".contains(&b))
}

/// The bus `m` serves on: the session bus for the user manager.
pub fn connection(m: Manager) -> zbus::Result<Connection> {
    match m {
        Manager::User => crate::session(),
        Manager::System => crate::system(),
    }
}

async fn call<B>(conn: &Connection, method: &str, body: &B) -> zbus::Result<zbus::Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(Some(DEST), PATH, Some(MANAGER), method, body)
        .await
}

/// The services the manager on `conn` has loaded.
pub async fn loaded(conn: &Connection) -> zbus::Result<Vec<Loaded>> {
    let (states, patterns): (&[&str], &[&str]) = (&[], &["*.service"]);
    let rows: Vec<Listed> = call(conn, "ListUnitsByPatterns", &(states, patterns))
        .await?
        .body()
        .deserialize()?;
    Ok(rows
        .into_iter()
        .map(|r| Loaded {
            name: r.0,
            description: r.1,
            load: r.2,
            active: r.3,
            sub: r.4,
        })
        .collect())
}

/// The service unit files the manager on `conn` finds.
pub async fn files(conn: &Connection) -> zbus::Result<Vec<UnitFile>> {
    let (states, patterns): (&[&str], &[&str]) = (&[], &["*.service"]);
    let rows: Vec<(String, String)> = call(conn, "ListUnitFilesByPatterns", &(states, patterns))
        .await?
        .body()
        .deserialize()?;
    Ok((rows.into_iter())
        .map(|(path, state)| UnitFile { path, state })
        .collect())
}

/// Whether polkit may let this user start, stop and restart system services, at once or after
/// a password. True when polkit does not answer, so the system manager itself says no.
pub fn may_manage_system() -> bool {
    match crate::system() {
        Ok(conn) => {
            crate::wait(crate::polkit::verdict(&conn, MANAGE_UNITS))
                != crate::polkit::Verdict::Refused
        }
        Err(_) => true,
    }
}

/// Why [`run`] did not run a job.
#[derive(Debug)]
pub enum Error {
    Bus(zbus::Error),
    /// The name is no service's; nothing was sent.
    NotAService,
    /// Enabling or disabling a system unit file; nothing was sent.
    SystemUnitFile,
    /// The user manager changed no link: the unit is enabled or disabled by a link outside the
    /// user's own folders (a package's `.wants` link, `systemctl --global`).
    Unchanged,
}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Error {
        Error::Bus(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Bus(e) => e.fmt(f),
            Error::NotAService => f.write_str("not a service name"),
            Error::SystemUnitFile => f.write_str("a system unit file is not enabled here"),
            Error::Unchanged => f.write_str("no unit file link changed"),
        }
    }
}

/// A unit file change `EnableUnitFiles` and `DisableUnitFiles` report: kind, link, target.
type Change = (String, String, String);

/// Runs `verb` on service `name` of `m` over `conn`. A system job lets polkit ask for a password
/// (up to [`crate::polkit::PROMPT_TIMEOUT`]); a system unit file is never enabled or disabled.
pub async fn run(conn: &Connection, m: Manager, name: &str, verb: Verb) -> Result<(), Error> {
    if !is_service_name(name) {
        return Err(Error::NotAService);
    }
    let job = match verb {
        Verb::Start => "StartUnit",
        Verb::Stop => "StopUnit",
        Verb::Restart => "RestartUnit",
        Verb::Enable | Verb::Disable if m == Manager::System => {
            return Err(Error::SystemUnitFile);
        }
        Verb::Enable | Verb::Disable => {
            let changes: Vec<Change> = if verb == Verb::Enable {
                let reply = call(conn, "EnableUnitFiles", &(&[name][..], false, false)).await?;
                reply.body().deserialize::<(bool, Vec<Change>)>()?.1
            } else {
                let reply = call(conn, "DisableUnitFiles", &(&[name][..], false)).await?;
                reply.body().deserialize()?
            };
            if changes.is_empty() {
                return Err(Error::Unchanged);
            }
            call(conn, "Reload", &()).await?;
            return Ok(());
        }
    };
    let body = (name, "replace");
    match m {
        Manager::User => Ok(call(conn, job, &body).await.map(drop)?),
        Manager::System => {
            let manager = Proxy::new(conn, DEST, PATH, MANAGER).await?;
            let call = manager.call_with_flags::<_, _, OwnedObjectPath>(
                job,
                MethodFlags::ALLOW_INTERACTIVE_AUTH,
                &body,
            );
            Ok(
                crate::polkit::asking(crate::polkit::PROMPT_TIMEOUT, true, call)
                    .await
                    .map(drop)?,
            )
        }
    }
}

/// How a job [`carry_out`] ran failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failed {
    /// The password prompt was dismissed: nothing to tell.
    Dismissed,
    /// systemd or polkit said no, in these words.
    Told(Told),
    /// [`Error::Unchanged`].
    Unchanged,
    /// [`Error::NotAService`] or [`Error::SystemUnitFile`]: nothing was sent.
    NotSent,
}

/// [`run`] on `m`'s bus, blocking.
pub fn carry_out(m: Manager, name: &str, verb: Verb) -> Result<(), Failed> {
    let ran = connection(m)
        .map_err(Error::Bus)
        .and_then(|conn| crate::wait(run(&conn, m, name, verb)));
    let e = match ran {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    log::warn!("{verb:?} {name} ({m:?}): {e}");
    let told = |e| match m {
        Manager::System => crate::error::to_tell(e, crate::polkit::dismissed),
        Manager::User => crate::error::to_tell(e, || false),
    };
    Err(match e {
        Error::Bus(e) => told(e).map_or(Failed::Dismissed, Failed::Told),
        Error::Unchanged => Failed::Unchanged,
        Error::NotAService | Error::SystemUnitFile => Failed::NotSent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::testbus::{bus, connect};

    #[test]
    fn a_service_name_is_systemd_s_characters_and_the_suffix() {
        for ok in [
            "pipewire.service",
            "getty@tty1.service",
            "dev-disk-by\\x2duuid.service",
            "a:b_c.d.service",
        ] {
            assert!(is_service_name(ok), "{ok} is a service name");
        }
        for bad in [
            "",
            ".service",
            "pipewire",
            "pipewire.socket",
            "-pipewire.service",
            "--user.service",
            "pipe wire.service",
            "pipe*.service",
            "pipe?.service",
            "a[b].service",
            "a/b.service",
            "é.service",
        ] {
            assert!(!is_service_name(bad), "{bad:?} is not a service name");
        }
        assert!(!is_service_name(&format!("{}.service", "a".repeat(250))));
    }

    type Calls = Arc<Mutex<Vec<String>>>;

    struct FakeSystemd {
        calls: Calls,
    }

    impl FakeSystemd {
        fn note(&self, hdr: &zbus::message::Header<'_>, what: String) {
            let interactive =
                (hdr.primary().flags()).contains(zbus::message::Flags::ALLOW_INTERACTIVE_AUTH);
            let mark = if interactive { " (may ask)" } else { "" };
            self.calls.lock().unwrap().push(format!("{what}{mark}"));
        }

        fn job(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1").unwrap()
        }
    }

    #[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
    impl FakeSystemd {
        fn list_units_by_patterns(
            &self,
            _states: Vec<String>,
            patterns: Vec<String>,
        ) -> Vec<Listed> {
            assert_eq!(patterns, ["*.service"]);
            let none = OwnedObjectPath::try_from("/").unwrap();
            vec![(
                "pipewire.service".into(),
                "PipeWire Multimedia Service".into(),
                "loaded".into(),
                "active".into(),
                "running".into(),
                String::new(),
                none.clone(),
                0,
                String::new(),
                none,
            )]
        }
        fn list_unit_files_by_patterns(
            &self,
            _states: Vec<String>,
            patterns: Vec<String>,
        ) -> Vec<(String, String)> {
            assert_eq!(patterns, ["*.service"]);
            vec![(
                "/usr/lib/systemd/user/pipewire.service".into(),
                "disabled".into(),
            )]
        }
        fn start_unit(
            &self,
            name: String,
            mode: String,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> OwnedObjectPath {
            self.note(&hdr, format!("StartUnit {name} {mode}"));
            self.job()
        }
        fn stop_unit(
            &self,
            name: String,
            mode: String,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> OwnedObjectPath {
            self.note(&hdr, format!("StopUnit {name} {mode}"));
            self.job()
        }
        fn restart_unit(
            &self,
            name: String,
            mode: String,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> OwnedObjectPath {
            self.note(&hdr, format!("RestartUnit {name} {mode}"));
            self.job()
        }
        fn enable_unit_files(
            &self,
            files: Vec<String>,
            runtime: bool,
            force: bool,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> (bool, Vec<(String, String, String)>) {
            self.note(&hdr, format!("EnableUnitFiles {files:?} {runtime} {force}"));
            (true, changes(&files, "symlink"))
        }
        fn disable_unit_files(
            &self,
            files: Vec<String>,
            runtime: bool,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> Vec<(String, String, String)> {
            self.note(&hdr, format!("DisableUnitFiles {files:?} {runtime}"));
            changes(&files, "unlink")
        }
        fn reload(&self, #[zbus(header)] hdr: zbus::message::Header<'_>) {
            self.note(&hdr, "Reload".into());
        }
    }

    /// The links the fake changes for `files`: none for `global.service`, which a link outside
    /// the user's folders enables.
    fn changes(files: &[String], kind: &str) -> Vec<Change> {
        (files.iter())
            .filter(|f| f.as_str() != "global.service")
            .map(|f| {
                (
                    kind.into(),
                    format!("/home/u/.config/systemd/user/{f}"),
                    String::new(),
                )
            })
            .collect()
    }

    /// A fake manager on a test bus, a connection to call it on, and the calls it took.
    fn with_systemd(test: impl AsyncFnOnce(&Connection, &Calls)) {
        let Some(bus) = bus() else {
            return;
        };
        zbus::block_on(async move {
            let calls = Calls::default();
            let systemd = connect(&bus.1).await;
            let fake = FakeSystemd {
                calls: calls.clone(),
            };
            crate::testbus::serve(&systemd, PATH, fake).await;
            systemd.request_name(DEST).await.unwrap();
            let conn = connect(&bus.1).await;
            test(&conn, &calls).await;
        });
    }

    #[test]
    fn the_services_and_their_files_are_read_from_the_manager() {
        with_systemd(async |conn, _| {
            assert_eq!(
                loaded(conn).await.unwrap(),
                [Loaded {
                    name: "pipewire.service".into(),
                    description: "PipeWire Multimedia Service".into(),
                    load: "loaded".into(),
                    active: "active".into(),
                    sub: "running".into(),
                }]
            );
            assert_eq!(
                files(conn).await.unwrap(),
                [UnitFile {
                    path: "/usr/lib/systemd/user/pipewire.service".into(),
                    state: "disabled".into(),
                }]
            );
        });
    }

    #[test]
    fn a_system_job_lets_polkit_ask_and_a_user_job_does_not() {
        with_systemd(async |conn, calls| {
            for verb in [Verb::Start, Verb::Stop, Verb::Restart] {
                run(conn, Manager::System, "sshd.service", verb)
                    .await
                    .unwrap();
            }
            run(conn, Manager::User, "pipewire.service", Verb::Restart)
                .await
                .unwrap();
            assert_eq!(
                calls.lock().unwrap()[..],
                [
                    "StartUnit sshd.service replace (may ask)",
                    "StopUnit sshd.service replace (may ask)",
                    "RestartUnit sshd.service replace (may ask)",
                    "RestartUnit pipewire.service replace",
                ]
            );
        });
    }

    #[test]
    fn only_a_user_unit_file_is_enabled_and_the_manager_reloads() {
        with_systemd(async |conn, calls| {
            run(conn, Manager::User, "syncthing.service", Verb::Enable)
                .await
                .unwrap();
            run(conn, Manager::User, "syncthing.service", Verb::Disable)
                .await
                .unwrap();
            for verb in [Verb::Enable, Verb::Disable] {
                assert!(
                    run(conn, Manager::System, "sshd.service", verb)
                        .await
                        .is_err(),
                    "{verb:?} on a system unit file is refused"
                );
            }
            assert_eq!(
                calls.lock().unwrap()[..],
                [
                    "EnableUnitFiles [\"syncthing.service\"] false false",
                    "Reload",
                    "DisableUnitFiles [\"syncthing.service\"] false",
                    "Reload",
                ]
            );
        });
    }

    #[test]
    fn a_link_the_user_manager_does_not_change_is_told_and_nothing_reloads() {
        with_systemd(async |conn, calls| {
            for verb in [Verb::Disable, Verb::Enable] {
                let e = run(conn, Manager::User, "global.service", verb).await;
                assert!(matches!(e, Err(Error::Unchanged)), "{verb:?}: {e:?}");
            }
            assert_eq!(
                calls.lock().unwrap()[..],
                [
                    "DisableUnitFiles [\"global.service\"] false",
                    "EnableUnitFiles [\"global.service\"] false false",
                ],
                "no Reload follows"
            );
        });
    }

    #[test]
    fn a_name_that_is_no_service_is_never_sent() {
        with_systemd(async |conn, calls| {
            for name in ["sshd.socket", "--now.service", "a b.service", "x*.service"] {
                for m in [Manager::User, Manager::System] {
                    assert!(run(conn, m, name, Verb::Start).await.is_err());
                }
            }
            assert!(calls.lock().unwrap().is_empty());
        });
    }
}
