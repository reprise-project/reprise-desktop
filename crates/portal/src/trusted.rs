//! RemoteDesktop sessions skip the dialog when the request comes from Reprise's installed remote
//! desktop server: its enabled user unit (enabling it is the consent) or reprise-login's hand-over.

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub const APP_ID: &str = "org.reprise.RemoteDesktop";
pub const UNIT: &str = "app-org.reprise.RemoteDesktop.service";
/// The server reprise-login's hand-overs start; the frontend gives its requests no app id.
pub const HANDOVER_UNIT: &str = "reprise-rdp-handover.service";

/// What the user manager says of a server's unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitFacts {
    pub file_state: String,
    pub active_state: String,
    /// 0 without a main process.
    pub main_pid: u32,
    /// The main process's executable; None without one.
    pub exe: Option<PathBuf>,
}

/// Where root installs the server; " (deleted)" follows once an upgrade replaced the running file.
const INSTALLED: [&str; 2] = ["/usr/bin/reprise-rdp", "/usr/local/bin/reprise-rdp"];

/// Whether RemoteDesktop.Start skips the dialog: `app_id` is [`APP_ID`] and `caller` (the request's
/// bus peer) is the main process of `unit`, enabled and active with the installed server.
pub fn trusted(app_id: &str, caller: Option<u32>, unit: Option<&UnitFacts>) -> bool {
    app_id == APP_ID && unit.is_some_and(|u| u.file_state == "enabled" && serves(caller, u))
}

/// Which server a RemoteDesktop.Start that skips the dialog comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// The user's own enabled server.
    Own,
    /// The server reprise-login's hand-overs start.
    HandOver,
}

/// Whether RemoteDesktop.Start from `caller` skips the dialog: [`trusted`] for the per-user
/// server `own`, or `caller` is the main process of the running hand-over server `handover`.
pub fn start_trusted(
    app_id: &str,
    caller: Option<u32>,
    own: Option<&UnitFacts>,
    handover: Option<&UnitFacts>,
) -> Option<Trust> {
    if trusted(app_id, caller, own) {
        return Some(Trust::Own);
    }
    handover
        .is_some_and(|u| serves(caller, u))
        .then_some(Trust::HandOver)
}

/// Whether `caller` is the main process of `unit`, active and running an installed server.
fn serves(caller: Option<u32>, unit: &UnitFacts) -> bool {
    if unit.main_pid == 0 || caller != Some(unit.main_pid) {
        return false;
    }
    let installed = unit.exe.as_deref().is_some_and(|exe| {
        let exe = exe.as_os_str().as_bytes();
        let exe = exe.strip_suffix(b" (deleted)").unwrap_or(exe);
        INSTALLED.iter().any(|p| exe == p.as_bytes())
    });
    unit.active_state == "active" && installed
}

/// The unique bus name that made the session at `session_handle`, named by the frontend as
/// `/org/freedesktop/portal/desktop/session/SENDER/TOKEN`: SENDER without `:`, every `.` as `_`.
pub fn session_sender(session_handle: &str) -> Option<String> {
    let rest = session_handle.strip_prefix("/org/freedesktop/portal/desktop/session/")?;
    let (sender, token) = rest.split_once('/')?;
    if sender.is_empty() || token.is_empty() {
        return None;
    }
    Some(format!(":{}", sender.replace('_', ".")))
}

/// The pid behind the connection that made the session at `session_handle`; None when it has
/// gone or the bus does not answer.
pub async fn session_caller(conn: &zbus::Connection, session_handle: &str) -> Option<u32> {
    pid_of(conn, &session_sender(session_handle)?).await
}

/// The pid behind the connection that owns `name`; None when it has gone or the bus does not
/// answer.
pub async fn pid_of(conn: &zbus::Connection, name: &str) -> Option<u32> {
    let pid = conn
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetConnectionUnixProcessID",
            &(name,),
        )
        .await
        .and_then(|reply| reply.body().deserialize::<u32>());
    match pid {
        Ok(pid) => Some(pid),
        Err(e) => {
            log::info!("{name}: no pid: {e}");
            None
        }
    }
}

/// `unit` as the user manager on `conn` sees it; None when it is not loaded or the manager
/// does not answer.
pub async fn unit_facts(conn: &zbus::Connection, unit: &str) -> Option<UnitFacts> {
    match read_unit(conn, unit).await {
        Ok(facts) => Some(facts),
        Err(e) => {
            log::info!("{unit}: {e}");
            None
        }
    }
}

const DEST: &str = "org.freedesktop.systemd1";

async fn read_unit(conn: &zbus::Connection, unit: &str) -> zbus::Result<UnitFacts> {
    let path: zbus::OwnedObjectPath = conn
        .call_method(
            Some(DEST),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "GetUnit",
            &(unit,),
        )
        .await?
        .body()
        .deserialize()?;
    let unit = proxy(conn, &path, "org.freedesktop.systemd1.Unit").await?;
    let service = proxy(conn, &path, "org.freedesktop.systemd1.Service").await?;
    let pid: u32 = service.get_property("MainPID").await?;
    Ok(UnitFacts {
        file_state: unit.get_property("UnitFileState").await?,
        active_state: unit.get_property("ActiveState").await?,
        main_pid: pid,
        exe: (pid != 0)
            .then(|| std::fs::read_link(format!("/proc/{pid}/exe")).ok())
            .flatten(),
    })
}

async fn proxy<'a>(
    conn: &zbus::Connection,
    path: &'a zbus::OwnedObjectPath,
    interface: &'a str,
) -> zbus::Result<zbus::Proxy<'a>> {
    zbus::proxy::Builder::new(conn)
        .destination(DEST)
        .path(path)
        .interface(interface)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: u32 = 4242;

    fn running(exe: &str) -> UnitFacts {
        UnitFacts {
            file_state: "enabled".into(),
            active_state: "active".into(),
            main_pid: SERVER,
            exe: Some(exe.into()),
        }
    }

    fn trusted(app_id: &str, unit: Option<&UnitFacts>) -> bool {
        super::trusted(app_id, Some(SERVER), unit)
    }

    #[test]
    fn a_request_from_another_process_is_not_trusted() {
        let unit = running("/usr/bin/reprise-rdp");
        assert!(super::trusted(APP_ID, Some(SERVER), Some(&unit)));
        for caller in [Some(SERVER + 1), Some(0), None] {
            assert!(
                !super::trusted(APP_ID, caller, Some(&unit)),
                "a sandbox's bus proxy or any other process: {caller:?}"
            );
        }
        let no_main = UnitFacts {
            main_pid: 0,
            ..running("/usr/bin/reprise-rdp")
        };
        assert!(!super::trusted(APP_ID, Some(0), Some(&no_main)));
    }

    #[test]
    fn the_sender_is_read_from_the_session_handle() {
        let s = |h| session_sender(h);
        assert_eq!(
            s("/org/freedesktop/portal/desktop/session/1_42/reprise_rdp1"),
            Some(":1.42".into())
        );
        assert_eq!(
            s("/org/freedesktop/portal/desktop/session/1_42/token/u7"),
            Some(":1.42".into()),
            "a token taken twice gets a suffix"
        );
        for bad in [
            "/org/freedesktop/portal/desktop/request/1_42/token",
            "/org/freedesktop/portal/desktop/session/1_42",
            "/org/freedesktop/portal/desktop/session//token",
            "/org/example/session/1_42/token",
        ] {
            assert_eq!(s(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_enabled_running_server_is_trusted() {
        assert!(trusted(APP_ID, Some(&running("/usr/bin/reprise-rdp"))));
        assert!(trusted(
            APP_ID,
            Some(&running("/usr/local/bin/reprise-rdp"))
        ));
        assert!(
            trusted(APP_ID, Some(&running("/usr/bin/reprise-rdp (deleted)"))),
            "an upgrade replaced the running file"
        );
    }

    #[test]
    fn a_disabled_unit_is_not_trusted() {
        for state in ["disabled", "masked", "static", "enabled-runtime"] {
            let unit = UnitFacts {
                file_state: state.into(),
                ..running("/usr/bin/reprise-rdp")
            };
            assert!(!trusted(APP_ID, Some(&unit)), "{state}");
        }
    }

    #[test]
    fn a_stopped_unit_is_not_trusted() {
        for state in ["inactive", "activating", "deactivating", "failed"] {
            let unit = UnitFacts {
                active_state: state.into(),
                ..running("/usr/bin/reprise-rdp")
            };
            assert!(!trusted(APP_ID, Some(&unit)), "{state}");
        }
        let no_process = UnitFacts {
            exe: None,
            ..running("/usr/bin/reprise-rdp")
        };
        assert!(!trusted(APP_ID, Some(&no_process)));
    }

    #[test]
    fn another_binary_in_the_unit_is_not_trusted() {
        for exe in [
            "/home/x/reprise-rdp",
            "/usr/bin/reprise-rdp-old",
            "/usr/bin/sh",
            "/usr/bin/../../home/x/reprise-rdp",
        ] {
            assert!(!trusted(APP_ID, Some(&running(exe))), "{exe}");
        }
    }

    #[test]
    fn another_app_id_is_not_trusted() {
        let unit = running("/usr/bin/reprise-rdp");
        for app in ["org.example.Remote", "", "org.reprise.RemoteDesktop.Other"] {
            assert!(!trusted(app, Some(&unit)), "{app:?}");
        }
    }

    #[test]
    fn no_unit_is_not_trusted() {
        assert!(!trusted(APP_ID, None));
    }

    fn handover(exe: &str) -> UnitFacts {
        UnitFacts {
            file_state: "static".into(),
            ..running(exe)
        }
    }

    #[test]
    fn the_hand_over_server_is_trusted_without_an_app_id() {
        let unit = handover("/usr/bin/reprise-rdp");
        let hand_over = Some(Trust::HandOver);
        assert_eq!(
            start_trusted("", Some(SERVER), None, Some(&unit)),
            hand_over
        );
        assert_eq!(start_trusted("", Some(SERVER + 1), None, Some(&unit)), None);
        assert_eq!(start_trusted("", None, None, Some(&unit)), None);
        for state in ["inactive", "failed"] {
            let stopped = UnitFacts {
                active_state: state.into(),
                ..handover("/usr/bin/reprise-rdp")
            };
            assert_eq!(
                start_trusted("", Some(SERVER), None, Some(&stopped)),
                None,
                "{state}"
            );
        }
        let other = handover("/home/x/reprise-rdp");
        assert_eq!(start_trusted("", Some(SERVER), None, Some(&other)), None);
        assert_eq!(start_trusted("", Some(SERVER), None, None), None);
    }

    #[test]
    fn the_per_user_server_still_needs_its_app_id_beside_the_hand_over_unit() {
        let own = running("/usr/bin/reprise-rdp");
        assert_eq!(
            start_trusted(APP_ID, Some(SERVER), Some(&own), None),
            Some(Trust::Own)
        );
        assert_eq!(start_trusted("", Some(SERVER), Some(&own), None), None);
    }
}
