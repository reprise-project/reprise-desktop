//! PackageKit's offline updates: whether an update is prepared and triggered, triggering it for
//! the next boot with what the computer does afterwards, and cancelling the trigger. systemd
//! installs a triggered update at boot (`/system-update`); PackageKit asks polkit
//! ([`TRIGGER_ACTION`]) before it triggers or cancels.

use crate::{Props, prop};
use std::time::Duration;

use zbus::Connection;
use zbus::proxy::{CacheProperties, MethodFlags, Proxy};

pub const NAME: &str = "org.freedesktop.PackageKit";
pub const PATH: &str = "/org/freedesktop/PackageKit";
pub const OFFLINE: &str = "org.freedesktop.PackageKit.Offline";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The polkit action PackageKit checks before it triggers or cancels.
pub const TRIGGER_ACTION: &str = "org.freedesktop.packagekit.trigger-offline-update";

/// What the computer does once the update is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    PowerOff,
    Reboot,
}

impl After {
    /// `Trigger`'s action argument.
    pub fn action(self) -> &'static str {
        match self {
            After::PowerOff => "power-off",
            After::Reboot => "reboot",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub prepared: bool,
    pub triggered: bool,
}

impl Status {
    fn from_props(props: &Props) -> Status {
        Status {
            prepared: prop(props, "UpdatePrepared").unwrap_or(false),
            triggered: prop(props, "UpdateTriggered").unwrap_or(false),
        }
    }
}

/// The offline update's state, None when PackageKit is absent or does not answer. Reading starts
/// PackageKit when it is not running.
pub async fn status(conn: &Connection) -> Option<Status> {
    let props = async {
        let proxy: Proxy<'_> = zbus::proxy::Builder::new(conn)
            .destination(NAME)
            .path(PATH)
            .interface(PROPERTIES)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        proxy.call::<_, _, Props>("GetAll", &(OFFLINE,)).await
    };
    match props.await {
        Ok(p) => Some(Status::from_props(&p)),
        Err(e) => {
            log::info!("offline update: PackageKit unavailable ({e})");
            None
        }
    }
}

async fn offline(conn: &Connection) -> zbus::Result<Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(NAME)
        .path(PATH)
        .interface(OFFLINE)
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

/// Install the prepared update at the next boot, then do `after`. polkit may ask for a password
/// through the session's agent when `interactive`.
pub async fn trigger(conn: &Connection, after: After, interactive: bool) -> zbus::Result<()> {
    offline(conn)
        .await?
        .call_with_flags::<_, _, ()>("Trigger", flags(interactive), &(after.action(),))
        .await
        .map(|_| ())
}

/// Take back a trigger, leaving the update prepared.
pub async fn cancel(conn: &Connection, interactive: bool) -> zbus::Result<()> {
    offline(conn)
        .await?
        .call_with_flags::<_, _, ()>("Cancel", flags(interactive), &())
        .await
        .map(|_| ())
}

fn flags(interactive: bool) -> MethodFlags {
    match interactive {
        true => MethodFlags::ALLOW_INTERACTIVE_AUTH,
        false => MethodFlags::empty(),
    }
}

/// How PackageKit's engine errors reach the bus: unmapped GLib errors, by code.
const ENGINE_ERROR: &str = "org.gtk.GDBus.UnmappedGError.Quark._pk_2dengine_2derror_2dquark.Code";

/// Whether an error named `name` is PackageKit or the bus saying the caller may not: polkit
/// refused, or its prompt was dismissed. The engine's codes 0 and 2 are Denied and
/// RefusedByPolicy.
pub fn refused(name: &str) -> bool {
    name == "org.freedesktop.DBus.Error.AccessDenied"
        || name
            .strip_prefix(ENGINE_ERROR)
            .is_some_and(|code| code == "0" || code == "2")
}

/// How long a Trigger or Cancel may take. PackageKit waits on its password prompt for as long as
/// the prompt stays up, so a call that runs out withdraws it.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// [`trigger`] on a system bus connection of its own, waiting [`CALL_TIMEOUT`] at most. The error
/// is what to tell the user; None when the prompt was dismissed.
pub fn trigger_now(after: After, interactive: bool) -> Result<(), Option<String>> {
    let call = async { trigger(&timed().await?, after, interactive).await };
    let answer = zbus::block_on(within(CALL_TIMEOUT, call));
    settled(answer, interactive, |s| s.triggered).map_err(|e| {
        log::warn!("offline update: Trigger({}): {e}", after.action());
        to_tell(&e)
    })
}

/// [`cancel`], as [`trigger_now`] calls.
pub fn cancel_now(interactive: bool) -> Result<(), Option<String>> {
    let call = async { cancel(&timed().await?, interactive).await };
    let answer = zbus::block_on(within(CALL_TIMEOUT, call));
    settled(answer, interactive, |s| !s.triggered).map_err(|e| {
        log::warn!("offline update: Cancel: {e}");
        to_tell(&e)
    })
}

/// A Trigger or Cancel's `answer`. One that timed out withdraws its prompt, and stands as done
/// when PackageKit's state then shows it `done`: a password typed just before went through.
fn settled(
    answer: zbus::Result<()>,
    interactive: bool,
    done: impl FnOnce(Status) -> bool,
) -> zbus::Result<()> {
    let Err(e) = answer else {
        return Ok(());
    };
    crate::polkit::withdraw_timed_out(&e, interactive);
    if interactive && crate::error::timed_out(&e) && read().is_some_and(done) {
        log::info!("offline update: done as its prompt was withdrawn: {e}");
        return Ok(());
    }
    Err(e)
}

/// `call`, or a timeout error once `limit` passes: zbus applies a connection's method timeout to
/// plain calls only, not to `call_with_flags`.
async fn within<T>(
    limit: Duration,
    call: impl Future<Output = zbus::Result<T>>,
) -> zbus::Result<T> {
    let late = async {
        async_io::Timer::after(limit).await;
        let e = std::io::Error::from(std::io::ErrorKind::TimedOut);
        Err(zbus::Error::InputOutput(std::sync::Arc::new(e)))
    };
    futures_lite::future::or(call, late).await
}

async fn timed() -> zbus::Result<Connection> {
    zbus::connection::Builder::system()
        .method_timeout(CALL_TIMEOUT)
        .build()
        .await
}

fn to_tell(e: &zbus::Error) -> Option<String> {
    use crate::polkit::{NOT_ALLOWED, Refusal, TIMED_OUT, dismissed, refusal};
    match refusal(e, refused, dismissed) {
        Some(Refusal::Dismissed) => None,
        Some(Refusal::Denied) => Some(NOT_ALLOWED.to_string()),
        Some(Refusal::TimedOut) => Some(TIMED_OUT.to_string()),
        None => Some(crate::error::message(e).map_or_else(|| e.to_string(), String::from)),
    }
}

/// [`status`] on [`crate::system`]'s connection, blocking.
pub fn read() -> Option<Status> {
    match crate::system() {
        Ok(conn) => crate::wait(status(&conn)),
        Err(e) => {
            log::info!("offline update: no system bus ({e})");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{bus, bus_with_no_services, connect, serve};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Fake {
        prepared: bool,
        action: Arc<Mutex<Option<String>>>,
        refuse: bool,
    }

    #[zbus::interface(name = "org.freedesktop.PackageKit.Offline")]
    impl Fake {
        fn trigger(&self, action: &str) -> zbus::fdo::Result<()> {
            if self.refuse {
                return Err(zbus::fdo::Error::AccessDenied("no".into()));
            }
            *self.action.lock().unwrap() = Some(action.to_string());
            Ok(())
        }
        fn cancel(&self) {
            *self.action.lock().unwrap() = None;
        }
        #[zbus(property)]
        fn update_prepared(&self) -> bool {
            self.prepared
        }
        #[zbus(property)]
        fn update_triggered(&self) -> bool {
            self.action.lock().unwrap().is_some()
        }
        #[zbus(property)]
        fn trigger_action(&self) -> String {
            self.action.lock().unwrap().clone().unwrap_or_default()
        }
    }

    #[test]
    fn a_prepared_update_is_triggered_for_power_off_and_cancelled() {
        let Some(bus) = bus() else {
            return;
        };
        zbus::block_on(async move {
            let action = Arc::new(Mutex::new(None));
            let pk = connect(&bus.1).await;
            let fake = Fake {
                prepared: true,
                action: action.clone(),
                ..Fake::default()
            };
            serve(&pk, PATH, fake).await;
            pk.request_name(NAME).await.unwrap();

            let conn = connect(&bus.1).await;
            let ready = Status {
                prepared: true,
                triggered: false,
            };
            assert_eq!(status(&conn).await, Some(ready));
            trigger(&conn, After::PowerOff, false).await.unwrap();
            assert_eq!(action.lock().unwrap().as_deref(), Some("power-off"));
            assert!(status(&conn).await.unwrap().triggered);
            cancel(&conn, false).await.unwrap();
            assert_eq!(status(&conn).await, Some(ready));
            trigger(&conn, After::Reboot, false).await.unwrap();
            assert_eq!(action.lock().unwrap().as_deref(), Some("reboot"));
        });
    }

    #[test]
    fn a_refused_trigger_is_told_apart() {
        let Some(bus) = bus() else {
            return;
        };
        zbus::block_on(async move {
            let pk = connect(&bus.1).await;
            let fake = Fake {
                refuse: true,
                ..Fake::default()
            };
            serve(&pk, PATH, fake).await;
            pk.request_name(NAME).await.unwrap();
            let conn = connect(&bus.1).await;
            let e = trigger(&conn, After::Reboot, false).await.unwrap_err();
            assert!(crate::error::name(&e).is_some_and(|n| refused(&n)));
        });
    }

    #[test]
    fn a_call_past_its_limit_fails_as_timed_out() {
        let call = futures_lite::future::pending::<zbus::Result<()>>();
        let e = zbus::block_on(within(Duration::from_millis(20), call)).unwrap_err();
        assert!(crate::error::timed_out(&e), "{e}");
        assert_eq!(
            zbus::block_on(within(Duration::from_secs(60), async { Ok(7) })).unwrap(),
            7
        );
    }

    #[test]
    fn no_packagekit_means_nothing_known() {
        let Some(bus) = bus_with_no_services() else {
            return;
        };
        let conn = zbus::block_on(connect(&bus.1));
        assert_eq!(zbus::block_on(status(&conn)), None);
    }

    #[test]
    fn packagekits_refusals_are_named_as_such() {
        let engine = |code: u32| format!("{ENGINE_ERROR}{code}");
        assert!(refused("org.freedesktop.DBus.Error.AccessDenied"));
        assert!(refused(&engine(0)), "failed to obtain auth");
        assert!(refused(&engine(2)));
        assert!(!refused(&engine(1)), "invalid state");
        assert!(!refused(&engine(10)));
        assert!(!refused("org.freedesktop.DBus.Error.ServiceUnknown"));
    }
}
