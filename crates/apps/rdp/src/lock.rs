//! Locking the session once the last logged-in client has left, so the computer is not left open.

use crate::listen::Leave;
use std::cell::Cell;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::Notify;
use zbus::Connection;
use zbus::{OwnedObjectPath, OwnedVariant};

const LOGIN1: &str = "org.freedesktop.login1";

/// How long the portal session may take to close before the lock goes ahead anyway.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// How long logind may take to lock.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// The user's display session, through logind: `GetUser(uid)`, its `Display`, `Session.Lock`.
pub async fn lock_session(conn: &Connection) -> zbus::Result<()> {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let user: OwnedObjectPath = conn
        .call_method(
            Some(LOGIN1),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "GetUser",
            &(uid,),
        )
        .await?
        .body()
        .deserialize()?;
    let display: OwnedVariant = conn
        .call_method(
            Some(LOGIN1),
            &user,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.User", "Display"),
        )
        .await?
        .body()
        .deserialize()?;
    let (id, session) = <(String, OwnedObjectPath)>::try_from(display)?;
    if id.is_empty() {
        return Err(zbus::Error::Failure(
            "the user has no display session".into(),
        ));
    }
    conn.call_method(
        Some(LOGIN1),
        &session,
        Some("org.freedesktop.login1.Session"),
        "Lock",
        &(),
    )
    .await?;
    Ok(())
}

/// [`lock_session`] on the system bus, from a thread of its own; whether it locked.
pub async fn lock() -> bool {
    let locked = on_thread(LOCK_WAIT, || {
        zbus::block_on(async {
            let conn = Connection::system().await?;
            lock_session(&conn).await
        })
        .map_err(|e| e.to_string())
    });
    match locked.await {
        Ok(()) => {
            log::info!("locked the session");
            true
        }
        Err(e) => {
            log::error!("cannot lock the session: {e}");
            false
        }
    }
}

/// `$XDG_RUNTIME_DIR/reprise/rdp/logged-in`: there from a login until the session locked after it,
/// so a server stopped or killed in between locks on its way out or on its next start.
pub fn marker_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .map(|d| d.join("reprise/rdp/logged-in"))
}

fn mark(marker: &Path) {
    let written = marker
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(marker, b""));
    if let Err(e) = written {
        log::warn!("{}: {e}", marker.display());
    }
}

/// Locks through `lock` when `marker` is there and removes it once locked.
pub async fn lock_if_marked(marker: Option<&Path>, lock: impl Future<Output = bool>) {
    let Some(marker) = marker.filter(|m| m.exists()) else {
        return;
    };
    log::info!("{}: no lock followed a login; locking", marker.display());
    if lock.await {
        let _ = fs::remove_file(marker);
    }
}

/// Serves until `stop`, then locks if a client is still logged in. Awaited inside the LocalSet
/// that runs the connections, so the lock comes before they are dropped.
pub async fn until_stopped(
    serving: impl Future<Output = Infallible>,
    stop: impl Future<Output = ()>,
    marker: Option<&Path>,
    lock: impl Future<Output = bool>,
) {
    tokio::select! {
        never = serving => match never {},
        () = stop => log::info!("stopping"),
    }
    lock_if_marked(marker, lock).await;
}

/// `job` on a thread of its own, given up on after `wait`.
async fn on_thread(
    wait: Duration,
    job: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("reprise-rdp-lock".into())
        .spawn(move || {
            let _ = tx.send(job());
        })
        .map_err(|e| e.to_string())?;
    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(done)) => done,
        Ok(Err(_)) => Err("the lock thread ended".into()),
        Err(_) => Err(format!("no answer in {} s", wait.as_secs())),
    }
}

/// Locks once the last logged-in client has left and the portal sessions are closed, which is
/// after its held input was released. Its marker is kept from the first login until then.
pub struct LockOnLeave<F> {
    marker: Option<PathBuf>,
    present: Cell<usize>,
    /// Wakes a last leaver's close wait: a login there means no lock.
    arrival: Notify,
    lock: F,
}

impl LockOnLeave<()> {
    pub fn new(marker: Option<PathBuf>) -> LockOnLeave<impl AsyncFn() -> bool> {
        LockOnLeave::with_lock(marker, lock)
    }
}

impl<F: AsyncFn() -> bool> LockOnLeave<F> {
    fn with_lock(marker: Option<PathBuf>, lock: F) -> Self {
        Self {
            marker,
            present: Cell::new(0),
            arrival: Notify::new(),
            lock,
        }
    }
}

impl<F: AsyncFn() -> bool> Leave for LockOnLeave<F> {
    fn arrived(&self) {
        self.present.set(self.present.get() + 1);
        self.arrival.notify_waiters();
        if self.present.get() == 1
            && let Some(m) = &self.marker
        {
            mark(m);
        }
    }

    async fn left(&self) {
        let present = self.present.get().checked_sub(1).unwrap_or_else(|| {
            log::error!("a client left with none logged in; locking");
            0
        });
        self.present.set(present);
        if present > 0 {
            return;
        }
        tokio::select! {
            biased;
            () = self.arrival.notified() => return,
            () = close_wait() => {}
        }
        // A client may log in while logind locks.
        if (self.lock)().await
            && self.present.get() == 0
            && let Some(m) = &self.marker
        {
            let _ = fs::remove_file(m);
        }
    }
}

/// `then`, once every portal session is closed or [`CLOSE_WAIT`] has passed.
#[cfg(test)]
pub async fn after_close(then: impl Future<Output = ()>) {
    close_wait().await;
    then.await;
}

/// Until every portal session is closed or [`CLOSE_WAIT`] has passed.
async fn close_wait() {
    if tokio::time::timeout(CLOSE_WAIT, crate::portal::closed())
        .await
        .is_err()
    {
        log::warn!(
            "the screen share did not close in {} s; locking anyway",
            CLOSE_WAIT.as_secs()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[tokio::test]
    async fn a_start_after_a_login_left_unlocked_locks_and_clears_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("reprise/rdp/logged-in");
        let locks = Rc::new(RefCell::new(0));
        let lock = || {
            let locks = locks.clone();
            async move {
                *locks.borrow_mut() += 1;
                true
            }
        };
        lock_if_marked(Some(&marker), lock()).await;
        assert_eq!(*locks.borrow(), 0, "no marker, no lock");

        LockOnLeave::with_lock(Some(marker.clone()), async || true).arrived();
        assert!(marker.exists(), "a login leaves the marker");
        lock_if_marked(Some(&marker), lock()).await;
        assert_eq!(*locks.borrow(), 1, "the marker locks");
        assert!(!marker.exists(), "the lock clears the marker");

        mark(&marker);
        lock_if_marked(Some(&marker), async { false }).await;
        assert!(
            marker.exists(),
            "a failed lock keeps the marker for next time"
        );
        lock_if_marked(None, lock()).await;
        assert_eq!(*locks.borrow(), 1, "no runtime dir, no lock");
    }

    /// Records its name as it is dropped.
    struct Open(Rc<RefCell<Vec<&'static str>>>);

    impl Drop for Open {
        fn drop(&mut self) {
            self.0.borrow_mut().push("dropped");
        }
    }

    #[test]
    fn a_stop_with_a_client_logged_in_locks_before_the_connection_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        mark(&marker);
        let steps = Rc::new(RefCell::new(Vec::new()));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            let connection = Open(steps.clone());
            tokio::task::spawn_local(async move {
                let _connection = connection;
                std::future::pending::<()>().await
            });
            tokio::task::yield_now().await;
            let lock = async {
                steps.borrow_mut().push("locked");
                true
            };
            until_stopped(std::future::pending(), async {}, Some(&marker), lock).await;
        });
        drop(local);
        assert_eq!(*steps.borrow(), ["locked", "dropped"]);
        assert!(!marker.exists());
    }

    #[test]
    fn a_stop_with_no_client_logged_in_does_not_lock() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let locked = Rc::new(RefCell::new(false));
        rt.block_on(until_stopped(
            std::future::pending(),
            async {},
            Some(&marker),
            async {
                *locked.borrow_mut() = true;
                true
            },
        ));
        assert!(!*locked.borrow());
    }

    /// A lock that counts its calls and succeeds.
    fn counted(locks: &Rc<Cell<usize>>) -> impl AsyncFn() -> bool + use<> {
        let locks = locks.clone();
        async move || {
            locks.set(locks.get() + 1);
            true
        }
    }

    #[tokio::test]
    async fn the_first_leaver_of_two_does_not_lock() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(Some(marker.clone()), counted(&locks));
        leave.arrived();
        leave.arrived();
        leave.left().await;
        assert_eq!(locks.get(), 0, "one viewer is still logged in");
        assert!(
            marker.exists(),
            "the marker stays while a viewer is logged in"
        );
    }

    #[tokio::test]
    async fn the_last_leaver_locks_and_clears_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(Some(marker.clone()), counted(&locks));
        leave.arrived();
        leave.arrived();
        leave.left().await;
        leave.left().await;
        assert_eq!(locks.get(), 1, "the last leaver locks once");
        assert!(!marker.exists(), "the lock clears the marker");
    }

    #[tokio::test]
    async fn a_leave_without_an_arrival_still_locks() {
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(None, counted(&locks));
        leave.left().await;
        assert_eq!(locks.get(), 1, "an unpaired leave locks");
        leave.arrived();
        leave.left().await;
        assert_eq!(locks.get(), 2, "the next viewer's leave still locks");
    }

    #[tokio::test]
    async fn a_login_during_the_lock_call_keeps_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let started = Rc::new(tokio::sync::Notify::new());
        let gate = Rc::new(tokio::sync::Notify::new());
        let lock = {
            let (started, gate) = (started.clone(), gate.clone());
            async move || {
                started.notify_one();
                gate.notified().await;
                true
            }
        };
        let leave = LockOnLeave::with_lock(Some(marker.clone()), lock);
        leave.arrived();
        let mut left = std::pin::pin!(leave.left());
        tokio::select! {
            biased;
            () = &mut left => panic!("the lock call is held"),
            () = started.notified() => {}
        }
        leave.arrived();
        gate.notify_one();
        tokio::time::timeout(Duration::from_secs(10), left)
            .await
            .expect("the leave ends once the lock call returns");
        assert!(
            marker.exists(),
            "the viewer logged in during the lock keeps the marker"
        );
    }

    #[tokio::test]
    async fn a_login_during_the_last_leavers_close_wait_skips_the_lock() {
        crate::listen::tests::capture_log();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(Some(marker.clone()), counted(&locks));
        let held = crate::portal::tests::grant();
        leave.arrived();
        let mut left = std::pin::pin!(leave.left());
        assert!(
            (&mut left).now_or_never().is_none(),
            "the leave waits for the portal session to close"
        );
        leave.arrived();
        let ended = (&mut left).now_or_never().is_some();
        drop(held);
        assert!(
            ended,
            "the login ends the wait with the portal session open"
        );
        assert_eq!(locks.get(), 0, "a viewer logged in during the wait");
        assert!(marker.exists(), "the new viewer keeps the marker");
        let timed_out = "the screen share did not close in 5 s; locking anyway";
        assert!(
            !crate::listen::tests::logged(timed_out),
            "no timeout is logged"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_logind_is_given_up_after_5_s() {
        let (_hold, hung) = std::sync::mpsc::channel::<()>();
        let started = tokio::time::Instant::now();
        let lock = on_thread(LOCK_WAIT, move || {
            let _ = hung.recv();
            Ok(())
        });
        let locked = tokio::time::timeout(Duration::from_secs(60), lock)
            .await
            .expect("the lock is given up on");
        assert_eq!(locked, Err("no answer in 5 s".into()));
        assert_eq!(started.elapsed(), LOCK_WAIT);
    }
}
