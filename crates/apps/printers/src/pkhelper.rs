//! cups-pk-helper, a system service that makes the admin requests as root once polkit says yes.
//! cups.rs tries the socket's peer credentials first and comes here only when cupsd says no.

use reprisebus::mutex::lock;
use std::collections::BTreeMap;
use std::sync::Mutex;

use reprisebus::polkit::Refusal;

use crate::cups::{Error, Result};

const DEST: &str = "org.opensuse.CupsPkHelper.Mechanism";
const PATH: &str = "/";
const IFACE: &str = "org.opensuse.CupsPkHelper.Mechanism";
/// What the service answers when polkit said no: the password prompt was
/// dismissed, or the password was wrong.
const NOT_PRIVILEGED: &str = "org.opensuse.CupsPkHelper.Mechanism.NotPrivileged";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    PrinterAdd {
        name: String,
        uri: String,
        ppd: String,
        info: String,
        location: String,
    },
    PrinterAddWithPpdFile {
        name: String,
        uri: String,
        file: String,
        info: String,
        location: String,
    },
    PrinterSetDevice(String, String),
    PrinterSetDefault(String),
    PrinterSetEnabled(String, bool),
    PrinterSetAcceptJobs(String, bool),
    PrinterRename(String, String),
    PrinterDelete(String),
    PrinterSetInfo(String, String),
    PrinterSetLocation(String, String),
    PrinterSetShared(String, bool),
    /// Sets the option's Default line in the queue's PPD.
    PrinterAddOption(String, String, Vec<String>),
    PrinterAddOptionDefault(String, String, Vec<String>),
    JobCancelPurge(i32, bool),
    JobRestart(i32),
    JobSetHoldUntil(i32, String),
}

impl Call {
    pub fn method(&self) -> &'static str {
        match self {
            Call::PrinterAdd { .. } => "PrinterAdd",
            Call::PrinterAddWithPpdFile { .. } => "PrinterAddWithPpdFile",
            Call::PrinterSetDevice(..) => "PrinterSetDevice",
            Call::PrinterSetDefault(_) => "PrinterSetDefault",
            Call::PrinterSetEnabled(..) => "PrinterSetEnabled",
            Call::PrinterSetAcceptJobs(..) => "PrinterSetAcceptJobs",
            Call::PrinterRename(..) => "PrinterRename",
            Call::PrinterDelete(_) => "PrinterDelete",
            Call::PrinterSetInfo(..) => "PrinterSetInfo",
            Call::PrinterSetLocation(..) => "PrinterSetLocation",
            Call::PrinterSetShared(..) => "PrinterSetShared",
            Call::PrinterAddOption(..) => "PrinterAddOption",
            Call::PrinterAddOptionDefault(..) => "PrinterAddOptionDefault",
            Call::JobCancelPurge(..) => "JobCancelPurge",
            Call::JobRestart(_) => "JobRestart",
            Call::JobSetHoldUntil(..) => "JobSetHoldUntil",
        }
    }
}

pub trait Helper: Send + Sync + std::fmt::Debug {
    fn installed(&self) -> bool;
    fn call(&self, call: &Call) -> Result<()>;
    fn devices(&self, timeout: i32, exclude: &[&str]) -> Result<BTreeMap<String, String>>;
}

pub fn run(h: &dyn Helper, calls: &[Call]) -> Result<()> {
    calls.iter().try_for_each(|c| h.call(c))
}

fn outcome(method: &str, error: &str) -> Result<()> {
    if error.is_empty() {
        return Ok(());
    }
    log::warn!("{method}: {error}");
    // A refusal that sets no status of its own leaves the last request's.
    if error == "successful-ok" {
        return Err(Error::Refused(
            "cups-pk-helper could not make the change.".into(),
        ));
    }
    Err(Error::Refused(crate::cups::sentence(error)))
}

fn bus_error(method: &str, e: zbus::Error) -> Error {
    describe(method, e, reprisebus::polkit::dismissed)
}

fn describe(method: &str, e: zbus::Error, dismissed: impl FnOnce() -> bool) -> Error {
    match reprisebus::polkit::refusal(&e, |n| n == NOT_PRIVILEGED, dismissed) {
        Some(Refusal::Dismissed) => return Error::Dismissed,
        Some(Refusal::Denied) => return Error::NotAuthorized,
        Some(Refusal::TimedOut) | None => {}
    }
    log::warn!("{method}: {e}");
    Error::Refused(format!("cups-pk-helper could not make the change ({e})."))
}

#[derive(Debug, Default)]
pub struct Bus {
    conn: Mutex<Option<zbus::blocking::Connection>>,
}

impl Bus {
    fn conn(&self) -> Result<zbus::blocking::Connection> {
        let mut c = lock(&self.conn);
        if let Some(c) = c.as_ref() {
            return Ok(c.clone());
        }
        let conn = zbus::blocking::Connection::system()
            .map_err(|e| Error::Refused(format!("The system bus is not there ({e}).")))?;
        *c = Some(conn.clone());
        Ok(conn)
    }

    fn proxy(&self) -> Result<zbus::blocking::Proxy<'static>> {
        let conn = self.conn()?;
        zbus::blocking::proxy::Builder::new(&conn)
            .destination(DEST)
            .and_then(|b| b.path(PATH))
            .and_then(|b| b.interface(IFACE))
            .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
            .and_then(|b| b.build())
            .map_err(|e| bus_error("proxy", e))
    }

    fn send<B>(&self, method: &str, body: &B) -> Result<()>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let proxy = self.proxy()?;
        let call = || {
            proxy
                .call_with_flags(
                    method,
                    zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
                    body,
                )
                .map(Option::unwrap_or_default)
        };
        send_with(method, call, || self.running())
    }

    fn running(&self) -> bool {
        let Ok(conn) = self.conn() else {
            return false;
        };
        let Ok(name) = zbus::names::BusName::try_from(DEST) else {
            return false;
        };
        zbus::blocking::fdo::DBusProxy::new(&conn)
            .is_ok_and(|d| d.name_has_owner(name).unwrap_or(false))
    }
}

/// `call` once more when the service quit under it. cups-pk-helper exits 30 s after
/// its last request, counted through a password prompt, so a call queued behind
/// one can reach it as it goes: the bus answers NoReply, or the instance, its
/// object already gone, UnknownMethod. The service is bus-activated, so a send
/// after the old instance left starts a new one.
fn send_with(
    method: &str,
    mut call: impl FnMut() -> zbus::Result<String>,
    running: impl Fn() -> bool,
) -> Result<()> {
    let error = match call() {
        Err(e) if quit_under(&e, running) => {
            log::info!("{method}: cups-pk-helper quit under the call ({e}), sending it again");
            call()
        }
        r => r,
    }
    .map_err(|e| bus_error(method, e))?;
    outcome(method, &error)
}

/// How long an instance that dropped its object may take to leave the bus.
const LEAVING: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether `e` came from an instance that was exiting, and it has left. NoReply
/// from a running service is a prompt left open past the call's timeout.
fn quit_under(e: &zbus::Error, running: impl Fn() -> bool) -> bool {
    let zbus::Error::MethodError(name, _, _) = e else {
        return false;
    };
    match name.as_str() {
        "org.freedesktop.DBus.Error.NoReply" => !running(),
        "org.freedesktop.DBus.Error.UnknownMethod" => {
            let until = std::time::Instant::now() + LEAVING;
            while running() {
                if std::time::Instant::now() >= until {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            true
        }
        _ => false,
    }
}

impl Helper for Bus {
    fn installed(&self) -> bool {
        let Ok(conn) = self.conn() else {
            return false;
        };
        let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else {
            return false;
        };
        let has = |names: Vec<zbus::names::OwnedBusName>| names.iter().any(|n| n.as_str() == DEST);
        dbus.list_activatable_names().is_ok_and(has) || dbus.list_names().is_ok_and(has)
    }

    fn call(&self, call: &Call) -> Result<()> {
        let m = call.method();
        match call {
            Call::PrinterAdd {
                name,
                uri,
                ppd: file,
                info,
                location,
            }
            | Call::PrinterAddWithPpdFile {
                name,
                uri,
                file,
                info,
                location,
            } => self.send(m, &(name, uri, file, info, location)),
            Call::PrinterSetDefault(n) | Call::PrinterDelete(n) => self.send(m, &(n,)),
            Call::PrinterSetDevice(n, s)
            | Call::PrinterRename(n, s)
            | Call::PrinterSetInfo(n, s)
            | Call::PrinterSetLocation(n, s) => self.send(m, &(n, s)),
            Call::PrinterSetEnabled(n, b) | Call::PrinterSetShared(n, b) => self.send(m, &(n, b)),
            Call::PrinterSetAcceptJobs(n, b) => self.send(m, &(n, b, "")),
            Call::PrinterAddOption(n, o, v) | Call::PrinterAddOptionDefault(n, o, v) => {
                self.send(m, &(n, o, v))
            }
            Call::JobCancelPurge(id, purge) => self.send(m, &(id, purge)),
            Call::JobRestart(id) => self.send(m, &(id,)),
            Call::JobSetHoldUntil(id, when) => self.send(m, &(id, when)),
        }
    }

    fn devices(&self, timeout: i32, exclude: &[&str]) -> Result<BTreeMap<String, String>> {
        let none: &[&str] = &[];
        let (error, found): (String, BTreeMap<String, String>) = self
            .proxy()?
            .call_with_flags(
                "DevicesGet",
                zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
                &(timeout, 0i32, none, exclude),
            )
            .map_err(|e| bus_error("DevicesGet", e))?
            .unwrap_or_default();
        outcome("DevicesGet", &error).map(|()| found)
    }
}

/// DevicesGet's dictionary as one map per device, in the service's
/// order: "device-uri:3" is the fourth device's address.
pub fn devices_from(found: &BTreeMap<String, String>) -> Vec<BTreeMap<String, String>> {
    let mut by: BTreeMap<usize, BTreeMap<String, String>> = BTreeMap::new();
    for (k, v) in found {
        if let Some((attr, i)) = k.rsplit_once(':')
            && let Ok(i) = i.parse::<usize>()
        {
            by.entry(i).or_default().insert(attr.to_string(), v.clone());
        }
    }
    by.into_values().collect()
}

#[cfg(test)]
pub mod fake {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Fake {
        pub installed: bool,
        pub answer: Option<Error>,
        pub calls: Mutex<Vec<Call>>,
        pub found: BTreeMap<String, String>,
        /// How many DevicesGet calls, each a password prompt, were made.
        pub scans: Mutex<usize>,
    }

    impl Fake {
        pub fn installed() -> Fake {
            Fake {
                installed: true,
                ..Fake::default()
            }
        }

        pub fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Helper for Fake {
        fn installed(&self) -> bool {
            self.installed
        }

        fn call(&self, call: &Call) -> Result<()> {
            self.calls.lock().unwrap().push(call.clone());
            self.answer.clone().map_or(Ok(()), Err)
        }

        fn devices(&self, _: i32, _: &[&str]) -> Result<BTreeMap<String, String>> {
            *self.scans.lock().unwrap() += 1;
            Ok(self.found.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_argument_is_the_outcome() {
        assert_eq!(outcome("PrinterDelete", ""), Ok(()));
        assert_eq!(
            outcome("PrinterDelete", "printer not found"),
            Err(Error::Refused("Printer not found.".into()))
        );
        assert_eq!(
            outcome("PrinterRename", "successful-ok"),
            Err(Error::Refused(
                "cups-pk-helper could not make the change.".into()
            ))
        );
    }

    #[test]
    fn not_privileged_is_a_dismissal_or_a_refusal() {
        let msg = zbus::Message::method_call("/", "PrinterDelete")
            .and_then(|b| b.build(&()))
            .expect("a method call");
        let error = |name: &'static str| {
            zbus::Error::MethodError(
                zbus::names::OwnedErrorName::try_from(name).expect("an error name"),
                Some("Not authorized".into()),
                msg.clone(),
            )
        };
        let not_privileged = || error(NOT_PRIVILEGED);
        assert_eq!(
            describe("PrinterDelete", not_privileged(), || true),
            Error::Dismissed
        );
        assert_eq!(
            describe("PrinterDelete", not_privileged(), || false),
            Error::NotAuthorized
        );
        let other = describe(
            "PrinterDelete",
            error("org.opensuse.CupsPkHelper.Mechanism.CupsError"),
            || panic!("only NotPrivileged asks whether the prompt was dismissed"),
        );
        assert!(matches!(other, Error::Refused(_)));
    }

    fn bus_failure(name: &'static str, text: &str) -> zbus::Error {
        let msg = zbus::Message::method_call("/", "PrinterSetEnabled")
            .and_then(|b| b.build(&()))
            .expect("a method call");
        zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from(name).expect("an error name"),
            Some(text.into()),
            msg,
        )
    }

    fn no_reply() -> zbus::Error {
        bus_failure(
            "org.freedesktop.DBus.Error.NoReply",
            "Remote peer disconnected",
        )
    }

    #[test]
    fn a_call_an_exiting_instance_lost_is_sent_after_it_leaves() {
        let mut sent = 0;
        let call = || {
            sent += 1;
            if sent == 1 {
                Err(bus_failure(
                    "org.freedesktop.DBus.Error.UnknownMethod",
                    "Object does not exist at path \u{201c}/\u{201d}",
                ))
            } else {
                Ok(String::new())
            }
        };
        let asked = std::cell::Cell::new(0);
        let running = || {
            asked.set(asked.get() + 1);
            asked.get() < 3
        };
        assert_eq!(send_with("PrinterSetEnabled", call, running), Ok(()));
        assert_eq!(sent, 2, "sent again once the old instance left");
        assert_eq!(asked.get(), 3, "waited while it was still on the bus");
    }

    #[test]
    fn a_call_the_service_quit_under_is_sent_again() {
        let mut sent = 0;
        let call = || {
            sent += 1;
            if sent == 1 {
                Err(no_reply())
            } else {
                Ok(String::new())
            }
        };
        assert_eq!(send_with("PrinterSetEnabled", call, || false), Ok(()));
        assert_eq!(sent, 2, "the second send reaches the started service");
    }

    #[test]
    fn no_reply_from_a_running_service_is_not_sent_again() {
        let mut sent = 0;
        let call = || {
            sent += 1;
            Err(no_reply())
        };
        let r = send_with("PrinterSetEnabled", call, || true);
        assert!(matches!(r, Err(Error::Refused(_))), "{r:?}");
        assert_eq!(sent, 1, "a prompt left open too long is not asked again");
    }

    #[test]
    fn a_failed_call_stops_the_rest() {
        let f = fake::Fake {
            answer: Some(Error::NotAuthorized),
            ..fake::Fake::installed()
        };
        let r = run(
            &f,
            &[
                Call::PrinterDelete("A".into()),
                Call::PrinterDelete("B".into()),
            ],
        );
        assert_eq!(r, Err(Error::NotAuthorized));
        assert_eq!(f.calls(), [Call::PrinterDelete("A".into())]);
    }

    #[test]
    fn found_devices_group_by_their_index() {
        let found: BTreeMap<String, String> = [
            ("device-uri:0", "usb://HP/LaserJet"),
            ("device-class:0", "direct"),
            ("device-uri:10", "socket://10.0.0.9"),
            ("device-uri:2", "ipp://10.0.0.5/ipp/print"),
            ("device-info:2", "Lab"),
            ("junk", "x"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let d = devices_from(&found);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0]["device-uri"], "usb://HP/LaserJet");
        assert_eq!(d[0]["device-class"], "direct");
        assert_eq!(d[1]["device-info"], "Lab");
        assert_eq!(d[2]["device-uri"], "socket://10.0.0.9");
    }
}
