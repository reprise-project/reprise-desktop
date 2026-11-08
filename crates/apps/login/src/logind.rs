//! logind and localed on the system bus: the proxies, and the reads and calls the daemon makes.

use std::cell::RefCell;
use std::future::Future;
use std::os::fd::OwnedFd;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use futures_util::StreamExt;
use repriselogin::greeter::{Keyboard, PowerAction};
use zbus::OwnedObjectPath;
use zbus::proxy::CacheProperties;
use zbus::{Connection, names::InterfaceName};

use crate::daemon::{FrontUnit, Inhibit, LogindSession, Unread};

const LOGIN1: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";
const SEAT0: &str = "/org/freedesktop/login1/seat/seat0";
/// A session logind keeps after its leader is gone, until its last process exits.
const CLOSING: &str = "closing";
/// logind's error for a session id it does not know.
const NO_SUCH_SESSION: &str = "org.freedesktop.login1.NoSuchSession";
/// How long a method call on the system bus waits for its reply.
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);
/// The front's unit, which the daemon starts and stops by `[remote_desktop] enabled`.
pub const FRONT_UNIT: &str = "reprise-login-rdp.service";
const INHIBIT_WHO: &str = "Reprise login manager";

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    /// Each session as (id, uid, user name, seat, path).
    fn list_sessions(&self) -> zbus::Result<Vec<(String, u32, String, String, OwnedObjectPath)>>;
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
    fn get_session(&self, id: &str) -> zbus::Result<OwnedObjectPath>;
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn terminate_session(&self, id: &str) -> zbus::Result<()>;
    fn kill_session(&self, id: &str, who: &str, signal: i32) -> zbus::Result<()>;
    fn unlock_session(&self, id: &str) -> zbus::Result<()>;
    fn lock_session(&self, id: &str) -> zbus::Result<()>;
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<zbus::OwnedFd>;
    #[zbus(signal)]
    fn session_removed(&self, id: String, path: OwnedObjectPath) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Seat",
    default_service = "org.freedesktop.login1"
)]
pub trait Seat {
    fn switch_to(&self, vtnr: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn active_session(&self) -> zbus::Result<(String, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
pub trait Session {
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn user(&self) -> zbus::Result<(u32, OwnedObjectPath)>;
    #[zbus(property, name = "VTNr")]
    fn vtnr(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn seat(&self) -> zbus::Result<(String, OwnedObjectPath)>;
    #[zbus(property)]
    fn class(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn service(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn leader(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn scope(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait Systemd {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn restart_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.locale1",
    default_service = "org.freedesktop.locale1",
    default_path = "/org/freedesktop/locale1"
)]
pub trait Locale {
    #[zbus(property, name = "X11Layout")]
    fn x11_layout(&self) -> zbus::Result<String>;
    #[zbus(property, name = "X11Variant")]
    fn x11_variant(&self) -> zbus::Result<String>;
    #[zbus(property, name = "X11Options")]
    fn x11_options(&self) -> zbus::Result<String>;
}

/// The console keyboard as localed names it; an empty or missing layout is `us`.
pub fn keyboard_from(
    layout: Option<String>,
    variant: Option<String>,
    options: Option<String>,
) -> Keyboard {
    Keyboard {
        layout: layout
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "us".into()),
        variant: variant.unwrap_or_default(),
        options: options.unwrap_or_default(),
    }
}

/// Whether `e` says the session is gone: logind's `NoSuchSession`, or `UnknownObject` from a
/// session object that went away. A greeter of the last daemon is often gone this way.
pub fn session_gone(e: &zbus::Error) -> bool {
    match e {
        zbus::Error::FDO(e) => matches!(**e, zbus::fdo::Error::UnknownObject(_)),
        zbus::Error::MethodError(name, ..) => name.as_str() == NO_SUCH_SESSION,
        _ => false,
    }
}

/// logind's sessions as one `ListSessions` lists them.
#[derive(Debug, Default)]
pub struct Listing {
    /// Those whose properties could be read, closing ones included.
    pub found: Vec<LogindSession>,
    /// Those whose properties could not be read.
    pub unread: Vec<Unread>,
}

/// Whether a session is gone or closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Yes,
    No,
    /// logind could not be read.
    Unknown,
}

/// What a read of whether a session is closing says of its end.
fn ended_by(closing: zbus::Result<bool>) -> Ended {
    match closing {
        Ok(true) => Ended::Yes,
        Ok(false) => Ended::No,
        Err(e) if session_gone(&e) => Ended::Yes,
        Err(_) => Ended::Unknown,
    }
}

/// The daemon's system bus connection with logind's manager on it.
#[derive(Clone)]
pub struct Logind {
    conn: Connection,
    manager: ManagerProxy<'static>,
}

impl Logind {
    pub async fn connect() -> zbus::Result<Logind> {
        let conn = zbus::connection::Builder::system()
            .method_timeout(METHOD_TIMEOUT)
            .build()
            .await?;
        let manager = ManagerProxy::builder(&conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        Ok(Logind { conn, manager })
    }

    pub fn manager(&self) -> &ManagerProxy<'static> {
        &self.manager
    }

    async fn session(&self, path: OwnedObjectPath) -> zbus::Result<SessionProxy<'static>> {
        SessionProxy::builder(&self.conn)
            .path(path)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// logind's sessions whose properties can be read, closing ones included.
    pub async fn listed(&self) -> zbus::Result<Vec<LogindSession>> {
        Ok(self.listing().await?.found)
    }

    pub async fn listing(&self) -> zbus::Result<Listing> {
        let mut listing = Listing::default();
        for (id, uid, user, seat, path) in self.manager.list_sessions().await? {
            match self.describe(path).await {
                Ok((vt, class, service, closing)) => listing.found.push(LogindSession {
                    id,
                    user,
                    uid,
                    vt,
                    seat,
                    class,
                    service,
                    closing,
                }),
                Err(e) if session_gone(&e) => log::debug!("logind session {id}: {e}"),
                Err(e) => {
                    log::warn!("logind session {id}: {e}");
                    listing.unread.push(Unread {
                        id,
                        uid,
                        user,
                        seat,
                    });
                }
            }
        }
        Ok(listing)
    }

    /// Describes session `u` again.
    pub async fn read(&self, u: &Unread) -> zbus::Result<LogindSession> {
        let path = self.manager.get_session(&u.id).await?;
        let (vt, class, service, closing) = self.describe(path).await?;
        Ok(LogindSession {
            id: u.id.clone(),
            user: u.user.clone(),
            uid: u.uid,
            vt,
            seat: u.seat.clone(),
            class,
            service,
            closing,
        })
    }

    /// The VT, class and service of the session at `path`, and whether it is closing.
    async fn describe(&self, path: OwnedObjectPath) -> zbus::Result<(u16, String, String, bool)> {
        let s = self.session(path).await?;
        let closing = s.state().await? == CLOSING;
        let vt = u16::try_from(s.vtnr().await?).unwrap_or(0);
        Ok((vt, s.class().await?, s.service().await?, closing))
    }

    /// Stops session `id`'s scope, which `TerminateSession` leaves running once the session
    /// is closing.
    pub async fn stop_session(&self, id: &str) -> zbus::Result<()> {
        let path = self.manager.get_session(id).await?;
        let scope = self.session(path).await?.scope().await?;
        self.systemd().await?.stop_unit(&scope, "replace").await?;
        Ok(())
    }

    async fn systemd(&self) -> zbus::Result<SystemdProxy<'static>> {
        SystemdProxy::builder(&self.conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// Starts, restarts or stops [`FRONT_UNIT`].
    pub async fn front_unit(&self, unit: FrontUnit) -> zbus::Result<()> {
        let systemd = self.systemd().await?;
        match unit {
            FrontUnit::Start => systemd.start_unit(FRONT_UNIT, "replace").await?,
            FrontUnit::Restart => systemd.restart_unit(FRONT_UNIT, "replace").await?,
            FrontUnit::Stop => systemd.stop_unit(FRONT_UNIT, "replace").await?,
        };
        Ok(())
    }

    /// A block inhibitor of `what`, held until the fd is dropped.
    pub async fn inhibit(&self, what: &str, why: &str) -> zbus::Result<OwnedFd> {
        let fd = self
            .manager
            .inhibit(what, INHIBIT_WHO, why, "block")
            .await?;
        Ok(cloexec(fd.into())?)
    }

    /// The pid of session `id`'s leader.
    pub async fn leader(&self, id: &str) -> Option<u32> {
        let path = self.manager.get_session(id).await.ok()?;
        self.session(path).await.ok()?.leader().await.ok()
    }

    /// The open logind session of process `pid`.
    pub async fn session_of(&self, pid: u32) -> Option<String> {
        let path = self.manager.get_session_by_pid(pid).await.ok()?;
        let s = self.session(path).await.ok()?;
        match s.state().await.ok()?.as_str() {
            CLOSING => None,
            _ => s.id().await.ok(),
        }
    }

    /// Whether session `id` is gone or closing; `Unknown` when logind fails to say.
    pub async fn ended(&self, id: &str) -> Ended {
        let state = match self.manager.get_session(id).await {
            Ok(path) => match self.session(path).await {
                Ok(s) => s.state().await,
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        if let Err(e) = &state {
            log::debug!("logind session {id}: {e}");
        }
        ended_by(state.map(|state| state == CLOSING))
    }

    async fn seat0(&self) -> zbus::Result<SeatProxy<'static>> {
        SeatProxy::builder(&self.conn)
            .path(SEAT0)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// seat0's active session, `None` when it has none.
    pub async fn seat_active(&self) -> Option<String> {
        let (id, _) = self.seat0().await.ok()?.active_session().await.ok()?;
        (!id.is_empty()).then_some(id)
    }

    pub async fn switch_to(&self, vt: u16) -> zbus::Result<()> {
        self.seat0().await?.switch_to(vt.into()).await
    }

    pub async fn power(&self, action: PowerAction) -> zbus::Result<()> {
        match action {
            PowerAction::PowerOff => self.manager.power_off(false).await,
            PowerAction::Reboot => self.manager.reboot(false).await,
        }
    }

    /// `UnlockSession(id)`, then true once the session's `LockedHint` is false, or false
    /// when it is still set after `wait`.
    pub async fn unlock(&self, id: &str, wait: Duration) -> zbus::Result<bool> {
        let path = self.manager.get_session(id).await?;
        let props = zbus::fdo::PropertiesProxy::builder(&self.conn)
            .destination(LOGIN1)
            .path(path.clone())
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        // Subscribed before the call, so a change in between is seen.
        let mut changes = props.receive_properties_changed().await?;
        self.manager.unlock_session(id).await?;
        let session = self.session(path).await?;
        if !session.locked_hint().await? {
            return Ok(true);
        }
        let iface = InterfaceName::from_static_str_unchecked(SESSION);
        let dropped = async {
            while let Some(change) = changes.next().await {
                let Ok(args) = change.args() else { continue };
                if *args.interface_name() != iface {
                    continue;
                }
                let value = match args.changed_properties().get("LockedHint") {
                    Some(v) => bool::try_from(v).ok(),
                    None if args.invalidated_properties().contains(&"LockedHint") => {
                        session.locked_hint().await.ok()
                    }
                    None => continue,
                };
                if value == Some(false) {
                    return true;
                }
            }
            false
        };
        Ok(tokio::time::timeout(wait, dropped).await.unwrap_or(false))
    }

    /// The console keyboard from localed, `us` when it names none.
    pub async fn keyboard(&self) -> Keyboard {
        let locale = LocaleProxy::builder(&self.conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await;
        let Ok(locale) = locale else {
            return keyboard_from(None, None, None);
        };
        keyboard_from(
            locale.x11_layout().await.ok(),
            locale.x11_variant().await.ok(),
            locale.x11_options().await.ok(),
        )
    }
}

/// `fd` with `FD_CLOEXEC` set. zbus receives fds without it, and a worker spawned while one
/// is open would keep it, and with it the inhibitor.
fn cloexec(fd: OwnedFd) -> std::io::Result<OwnedFd> {
    use std::os::fd::AsRawFd;
    // SAFETY: F_SETFD on an fd this function owns has no other preconditions.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

/// logind's `what` and `why` for an inhibitor; `None` for none.
fn inhibit_args(inhibit: Inhibit) -> Option<(&'static str, &'static str)> {
    match inhibit {
        Inhibit::Nothing => None,
        Inhibit::Idle => Some(("idle", "a session runs in the background")),
        Inhibit::SleepAndIdle => Some(("sleep:idle", "a remote client is connected")),
    }
}

/// Takes an inhibitor of `what` for `why`; production passes [`Logind::inhibit`].
pub type TakeInhibitor =
    Rc<dyn Fn(&'static str, &'static str) -> Pin<Box<dyn Future<Output = zbus::Result<OwnedFd>>>>>;

/// The inhibitor the daemon holds. A new one is taken before the old one is dropped, and
/// one that arrives after a later change is dropped at once.
pub struct Inhibitor {
    take: TakeInhibitor,
    /// Each change's number, and the fd held.
    held: Rc<RefCell<(u64, Option<OwnedFd>)>>,
}

impl Inhibitor {
    pub fn new(take: TakeInhibitor) -> Inhibitor {
        Inhibitor {
            take,
            held: Rc::default(),
        }
    }

    /// Holds `want` from now on, in a task of the current `LocalSet`.
    pub fn set(&self, want: Inhibit) {
        let mut held = self.held.borrow_mut();
        held.0 += 1;
        let change = held.0;
        let Some((what, why)) = inhibit_args(want) else {
            held.1 = None;
            return;
        };
        drop(held);
        let (take, held) = (self.take.clone(), self.held.clone());
        tokio::task::spawn_local(async move {
            match take(what, why).await {
                Ok(fd) => {
                    let mut held = held.borrow_mut();
                    if held.0 == change {
                        held.1 = Some(fd);
                    }
                }
                Err(e) if held.borrow().1.is_some() => {
                    log::warn!("logind's {what} inhibitor: {e}; the previous one stays held");
                }
                Err(e) => log::warn!("logind's {what} inhibitor: {e}"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_reply(name: &str) -> zbus::Error {
        let call = zbus::Message::method_call("/org/freedesktop/login1", "TerminateSession")
            .build(&())
            .unwrap();
        let reply = zbus::Message::error(&call.header(), name)
            .build(&("gone",))
            .unwrap();
        reply.into()
    }

    #[test]
    fn a_session_that_went_away_is_told_from_other_failures() {
        assert!(session_gone(&error_reply(NO_SUCH_SESSION)));
        assert!(session_gone(&error_reply(
            "org.freedesktop.DBus.Error.UnknownObject"
        )));
        assert!(!session_gone(&error_reply(
            "org.freedesktop.DBus.Error.AccessDenied"
        )));
        assert!(!session_gone(&error_reply(
            "org.freedesktop.login1.NoSuchUser"
        )));
        assert!(!session_gone(&zbus::Error::InvalidReply));
    }

    #[test]
    fn a_session_is_ended_only_when_gone_or_closing() {
        assert_eq!(ended_by(Ok(true)), Ended::Yes);
        assert_eq!(ended_by(Ok(false)), Ended::No);
        assert_eq!(ended_by(Err(error_reply(NO_SUCH_SESSION))), Ended::Yes);
        assert_eq!(
            ended_by(Err(error_reply("org.freedesktop.DBus.Error.UnknownObject"))),
            Ended::Yes
        );
        assert_eq!(
            ended_by(Err(error_reply("org.freedesktop.DBus.Error.NoReply"))),
            Ended::Unknown
        );
        assert_eq!(
            ended_by(Err(error_reply("org.freedesktop.DBus.Error.AccessDenied"))),
            Ended::Unknown
        );
        assert_eq!(ended_by(Err(zbus::Error::InvalidReply)), Ended::Unknown);
    }

    #[test]
    fn keyboard_falls_back_to_us() {
        let us = Keyboard {
            layout: "us".into(),
            ..Keyboard::default()
        };
        assert_eq!(keyboard_from(None, None, None), us);
        assert_eq!(keyboard_from(Some(String::new()), None, None), us);
        assert_eq!(
            keyboard_from(
                Some("de,us".into()),
                Some("nodeadkeys,".into()),
                Some("grp:alt_shift_toggle".into())
            ),
            Keyboard {
                layout: "de,us".into(),
                variant: "nodeadkeys,".into(),
                options: "grp:alt_shift_toggle".into(),
            }
        );
    }

    #[test]
    fn a_received_fd_is_closed_on_exec() {
        use std::os::fd::{AsRawFd, FromRawFd};
        let mut ends = [0; 2];
        // SAFETY: `ends` has room for the two fds pipe writes.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        // SAFETY: pipe just returned both fds, and nothing else owns them.
        let (r, _w) = unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
        let flags = |fd: &OwnedFd| {
            // SAFETY: F_GETFD on an open fd has no other preconditions.
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) }
        };
        assert_eq!(flags(&r) & libc::FD_CLOEXEC, 0, "a bare pipe is inherited");
        let r = cloexec(r).unwrap();
        assert_eq!(flags(&r) & libc::FD_CLOEXEC, libc::FD_CLOEXEC);
    }

    /// Lets every ready task of the `LocalSet` run.
    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    fn open(w: &mut std::io::PipeWriter) -> bool {
        use std::io::Write;
        w.write(b"x").is_ok()
    }

    #[tokio::test]
    async fn an_inhibitor_is_held_until_the_next_change_and_a_late_one_is_dropped() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let taken: Rc<RefCell<Vec<(&str, std::io::PipeWriter)>>> = Rc::default();
                let slow = Rc::new(tokio::sync::Notify::new());
                let hold = Rc::new(std::cell::Cell::new(false));
                let take: TakeInhibitor = {
                    let (taken, slow, hold) = (taken.clone(), slow.clone(), hold.clone());
                    Rc::new(move |what, _why| {
                        let (r, w) = std::io::pipe().unwrap();
                        taken.borrow_mut().push((what, w));
                        let (slow, hold) = (slow.clone(), hold.get());
                        Box::pin(async move {
                            if hold {
                                slow.notified().await;
                            }
                            Ok(r.into())
                        })
                    })
                };
                let inhibitor = Inhibitor::new(take);
                inhibitor.set(Inhibit::Idle);
                settle().await;
                inhibitor.set(Inhibit::SleepAndIdle);
                settle().await;
                {
                    let mut t = taken.borrow_mut();
                    assert_eq!((t[0].0, t[1].0), ("idle", "sleep:idle"));
                    assert!(!open(&mut t[0].1), "the idle one is dropped once replaced");
                    assert!(open(&mut t[1].1), "the sleep:idle one is held");
                }
                inhibitor.set(Inhibit::Nothing);
                assert!(
                    !open(&mut taken.borrow_mut()[1].1),
                    "Nothing drops it at once"
                );

                hold.set(true);
                inhibitor.set(Inhibit::Idle);
                settle().await;
                inhibitor.set(Inhibit::Nothing);
                slow.notify_one();
                settle().await;
                assert!(
                    !open(&mut taken.borrow_mut()[2].1),
                    "an inhibitor arriving after a later change is dropped"
                );
            })
            .await;
    }
}
