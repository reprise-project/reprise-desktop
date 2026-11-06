//! Locking the session once the last logged-in client has left, so the computer is not left open.

use crate::listen::{End, Leave};
use repriseconf::remote::RemoteDesktop;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use zbus::Connection;
use zbus::{OwnedObjectPath, OwnedVariant};

const LOGIN1: &str = "org.freedesktop.login1";

/// How long the portal session may take to close before the lock goes ahead anyway.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// How long logind may take to lock.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// The user's display session, through logind: `GetUser(uid)`, its `Display`, `Session.Lock`;
/// no `Lock` once `given_up` is set, as the display session may be a newer one by then.
pub async fn lock_session(conn: &Connection, given_up: &AtomicBool) -> zbus::Result<()> {
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
    if given_up.load(Ordering::SeqCst) {
        return Err(zbus::Error::Failure("given up on".into()));
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
    let locked = on_thread(LOCK_WAIT, |given_up| {
        zbus::block_on(async {
            let conn = Connection::system().await?;
            lock_session(&conn, given_up).await
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

/// `$XDG_RUNTIME_DIR/reprise/rdp/logged-in`: there from a client being shown the session until
/// it locked after, so a server stopped or killed in between locks on its way out or next start.
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

/// `job` on a thread of its own, given up on after `wait`; the flag it is passed is set then.
async fn on_thread(
    wait: Duration,
    job: impl FnOnce(&AtomicBool) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let given_up = Arc::new(AtomicBool::new(false));
    let flag = given_up.clone();
    std::thread::Builder::new()
        .name("reprise-rdp-lock".into())
        .spawn(move || {
            let _ = tx.send(job(&flag));
        })
        .map_err(|e| e.to_string())?;
    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(done)) => done,
        Ok(Err(_)) => Err("the lock thread ended".into()),
        Err(_) => {
            given_up.store(true, Ordering::SeqCst);
            Err(format!("no answer in {} s", wait.as_secs()))
        }
    }
}

/// Locks once the last client shown the session ([`Leave`]) has left and the portal sessions are
/// closed; docs/security.md has the rules (`lock_on_leave`, `lock_grace_s`, the marker).
pub struct LockOnLeave<F> {
    marker: Option<PathBuf>,
    present: Cell<usize>,
    /// Wakes a last leaver's close wait: a login there means no lock.
    arrival: Notify,
    /// Counts the last leaves; a grace that ends after a later one leaves the lock to it.
    last_leaves: Cell<u64>,
    config: watch::Receiver<RemoteDesktop>,
    lock: F,
}

impl LockOnLeave<()> {
    pub fn new(
        marker: Option<PathBuf>,
        config: watch::Receiver<RemoteDesktop>,
    ) -> LockOnLeave<impl AsyncFn() -> bool> {
        LockOnLeave {
            config,
            ..LockOnLeave::with_lock(marker, lock)
        }
    }
}

impl<F: AsyncFn() -> bool> LockOnLeave<F> {
    fn with_lock(marker: Option<PathBuf>, lock: F) -> Self {
        Self {
            marker,
            present: Cell::new(0),
            arrival: Notify::new(),
            last_leaves: Cell::new(0),
            config: watch::channel(RemoteDesktop::default()).1,
            lock,
        }
    }
}

impl<F: AsyncFn() -> bool> Leave for LockOnLeave<F> {
    fn arrived(&self) {
        self.present.set(self.present.get() + 1);
        self.arrival.notify_waiters();
        if self.config.borrow().lock_on_leave
            && let Some(m) = &self.marker
        {
            mark(m);
        }
    }

    async fn left(&self, why: End) {
        let present = self.present.get().checked_sub(1).unwrap_or_else(|| {
            log::error!("a client left with none logged in; locking");
            0
        });
        self.present.set(present);
        if present > 0 {
            return;
        }
        let turn = self.last_leaves.get() + 1;
        self.last_leaves.set(turn);
        if why == End::LockMode {
            log::info!("the last client left lock mode; no lock");
            if let Some(m) = &self.marker {
                let _ = fs::remove_file(m);
            }
            return;
        }
        let RemoteDesktop {
            lock_on_leave,
            lock_grace_s,
            ..
        } = *self.config.borrow();
        if lock_on_leave && why == End::ScreenGone && lock_grace_s > 0 {
            log::info!(
                "the last client's screen went; locking in {lock_grace_s} s unless a client is shown the session"
            );
            tokio::time::sleep(Duration::from_secs(lock_grace_s.into())).await;
            if self.last_leaves.get() != turn {
                return;
            }
            if self.present.get() > 0 {
                log::info!("a client was shown the session in the grace; no lock");
                return;
            }
        }
        if !self.config.borrow().lock_on_leave {
            log::info!("the last client left; lock_on_leave is off");
            if let Some(m) = &self.marker {
                let _ = fs::remove_file(m);
            }
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

    /// Locks if a client is still logged in, while its connection is open.
    async fn stopping(&self) {
        lock_if_marked(self.marker.as_deref(), (self.lock)()).await;
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
    use crate::listen::Ending;
    use crate::portal::Screen;
    use futures_util::FutureExt;
    use std::cell::RefCell;
    use std::rc::Rc;
    use tokio::task::LocalSet;

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

    #[tokio::test]
    async fn a_stop_with_a_client_logged_in_locks_and_clears_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(Some(marker.clone()), counted(&locks));
        leave.stopping().await;
        assert_eq!(locks.get(), 0, "no client logged in, no lock");
        leave.arrived();
        leave.stopping().await;
        assert_eq!(locks.get(), 1);
        assert!(!marker.exists());
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
        leave.left(End::Left).await;
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
        leave.left(End::Left).await;
        leave.left(End::Left).await;
        assert_eq!(locks.get(), 1, "the last leaver locks once");
        assert!(!marker.exists(), "the lock clears the marker");
    }

    #[tokio::test]
    async fn with_lock_on_leave_off_the_last_leaver_does_not_lock() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let (config, rx) = watch::channel(RemoteDesktop::default());
        let leave = LockOnLeave {
            config: rx,
            ..LockOnLeave::with_lock(Some(marker.clone()), counted(&locks))
        };
        leave.arrived();
        config.send_modify(|c| c.lock_on_leave = false);
        leave.left(End::Left).await;
        assert_eq!(locks.get(), 0, "no lock with lock_on_leave off");
        assert!(!marker.exists(), "the leave clears the login's marker");
        leave.arrived();
        assert!(
            !marker.exists(),
            "a login with lock_on_leave off leaves no marker"
        );
        config.send_modify(|c| c.lock_on_leave = true);
        leave.left(End::Left).await;
        assert_eq!(locks.get(), 1, "the next leave reads lock_on_leave again");
    }

    #[tokio::test]
    async fn a_leave_without_an_arrival_still_locks() {
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(None, counted(&locks));
        leave.left(End::Left).await;
        assert_eq!(locks.get(), 1, "an unpaired leave locks");
        leave.arrived();
        leave.left(End::Left).await;
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
        let mut left = std::pin::pin!(leave.left(End::Left));
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
        let mut left = std::pin::pin!(leave.left(End::Left));
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

    /// `leave.left(why)` on a task of its own, in the LocalSet the test runs in.
    fn leaving<F: AsyncFn() -> bool + 'static>(
        leave: &Rc<LockOnLeave<F>>,
        why: End,
    ) -> tokio::task::JoinHandle<()> {
        let leave = leave.clone();
        tokio::task::spawn_local(async move { leave.left(why).await })
    }

    /// A [`LockOnLeave`] with a counted lock and `[remote_desktop]` from `config`.
    fn configured(
        marker: &Path,
        locks: &Rc<Cell<usize>>,
        config: watch::Receiver<RemoteDesktop>,
    ) -> Rc<LockOnLeave<impl AsyncFn() -> bool + use<>>> {
        Rc::new(LockOnLeave {
            config,
            ..LockOnLeave::with_lock(Some(marker.to_path_buf()), counted(locks))
        })
    }

    /// The end of a connection in a run that `locks` the console or not, granted screens cast
    /// from a virtual output or a monitor, as `virtual_outputs` says, that went as `gone` says.
    fn granted(locks: bool, virtual_outputs: &[bool], gone: bool) -> End {
        let ending = Ending::default();
        let screens: Vec<Screen> = (virtual_outputs.iter())
            .map(|&virtual_output| Screen {
                node: 0,
                position: (0, 0),
                size: (1920, 1080),
                mapping_id: None,
                virtual_output,
            })
            .collect();
        ending.granted(locks, &screens);
        if gone {
            ending.screen_gone();
        }
        ending.end()
    }

    #[tokio::test]
    async fn a_lock_mode_run_never_locks_on_leave() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(Some(marker.clone()), counted(&locks));
        for gone in [false, true] {
            leave.arrived();
            assert!(marker.exists());
            leave.left(granted(true, &[true, true], gone)).await;
            assert_eq!(locks.get(), 0, "no lock, the screen gone: {gone}");
            assert!(!marker.exists(), "the marker is cleared");
        }
    }

    #[tokio::test]
    async fn a_run_that_fell_back_to_screens_locks_on_leave() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(Some(marker.clone()), counted(&locks));
        leave.arrived();
        let why = granted(true, &[true, false], false);
        assert_eq!(why, End::Left, "a monitor among the screens is show mode");
        leave.left(why).await;
        assert_eq!(locks.get(), 1);
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn a_show_mode_run_granted_only_virtual_screens_still_locks_on_leave() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(Some(marker.clone()), counted(&locks));
        leave.arrived();
        let why = granted(false, &[true], false);
        assert_eq!(why, End::Left, "show mode on the standby screen");
        leave.left(why).await;
        assert_eq!(locks.get(), 1);
        assert!(!marker.exists());
    }

    #[tokio::test(start_paused = true)]
    async fn a_last_leave_because_the_screen_went_locks_after_the_grace() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let leave = LockOnLeave::with_lock(Some(marker.clone()), counted(&locks));
        leave.arrived();
        let mut left = std::pin::pin!(leave.left(End::ScreenGone));
        assert!(
            tokio::time::timeout(Duration::from_millis(4900), &mut left)
                .await
                .is_err(),
            "the lock waits the 5 s grace"
        );
        assert_eq!(locks.get(), 0, "no lock in the grace");
        assert!(marker.exists(), "the marker stays through the grace");
        tokio::time::timeout(Duration::from_secs(60), left)
            .await
            .expect("the grace ends");
        assert_eq!(
            locks.get(),
            1,
            "no client was shown the session in the grace"
        );
        assert!(!marker.exists(), "the lock clears the marker");
    }

    #[tokio::test(start_paused = true)]
    async fn a_client_shown_the_session_in_the_grace_keeps_it_unlocked() {
        LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let marker = dir.path().join("logged-in");
                let locks = Rc::new(Cell::new(0));
                let leave = configured(&marker, &locks, watch::channel(RemoteDesktop::default()).1);
                leave.arrived();
                let gone = leaving(&leave, End::ScreenGone);
                tokio::time::sleep(Duration::from_secs(2)).await;
                leave.arrived();
                tokio::time::timeout(Duration::from_secs(60), gone)
                    .await
                    .expect("the grace ends")
                    .unwrap();
                assert_eq!(locks.get(), 0, "the client reconnected in the grace");
                assert!(marker.exists(), "the reconnected client keeps the marker");
                leave.left(End::Left).await;
                assert_eq!(locks.get(), 1, "its own leave locks");
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_leave_that_is_not_the_screen_going_locks_without_the_grace() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let (config, rx) = watch::channel(RemoteDesktop::default());
        config.send_modify(|c| c.lock_grace_s = 60);
        let leave = LockOnLeave {
            config: rx,
            ..LockOnLeave::with_lock(Some(marker.clone()), counted(&locks))
        };
        leave.arrived();
        tokio::time::timeout(Duration::from_secs(10), leave.left(End::Left))
            .await
            .expect("a client that closed its connection locks without the 60 s grace");
        assert_eq!(locks.get(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_grace_of_0_locks_when_the_screen_goes() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let (config, rx) = watch::channel(RemoteDesktop::default());
        config.send_modify(|c| c.lock_grace_s = 0);
        let leave = LockOnLeave {
            config: rx,
            ..LockOnLeave::with_lock(Some(marker.clone()), counted(&locks))
        };
        leave.arrived();
        tokio::time::timeout(Duration::from_secs(10), leave.left(End::ScreenGone))
            .await
            .expect("no grace to wait");
        assert_eq!(locks.get(), 1);
        assert!(!marker.exists());
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_screen_going_in_the_grace_starts_a_grace_of_its_own() {
        LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let marker = dir.path().join("logged-in");
                let locks = Rc::new(Cell::new(0));
                let leave = configured(&marker, &locks, watch::channel(RemoteDesktop::default()).1);
                leave.arrived();
                let first = leaving(&leave, End::ScreenGone);
                tokio::time::sleep(Duration::from_secs(3)).await;
                leave.arrived();
                let second = leaving(&leave, End::ScreenGone);
                tokio::time::sleep(Duration::from_millis(2500)).await;
                assert!(first.is_finished(), "the first grace ran out at 5 s");
                assert_eq!(locks.get(), 0, "the second leave decides the lock");
                tokio::time::sleep(Duration::from_secs(2)).await;
                assert_eq!(locks.get(), 0, "the second grace runs to 8 s");
                assert!(marker.exists());
                second.await.unwrap();
                assert_eq!(locks.get(), 1, "the second grace locks once it runs out");
                assert!(!marker.exists());
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_client_that_leaves_in_the_grace_locks_once() {
        LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let marker = dir.path().join("logged-in");
                let locks = Rc::new(Cell::new(0));
                let leave = configured(&marker, &locks, watch::channel(RemoteDesktop::default()).1);
                leave.arrived();
                let gone = leaving(&leave, End::ScreenGone);
                tokio::time::sleep(Duration::from_secs(1)).await;
                leave.arrived();
                leave.left(End::Left).await;
                assert_eq!(locks.get(), 1, "the other client's leave locks at once");
                tokio::time::sleep(Duration::from_secs(20)).await;
                assert!(gone.is_finished());
                assert_eq!(
                    locks.get(),
                    1,
                    "the grace leaves the lock to the later leave"
                );
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_stop_in_the_grace_locks_at_once() {
        LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let marker = dir.path().join("logged-in");
                let locks = Rc::new(Cell::new(0));
                let leave = configured(&marker, &locks, watch::channel(RemoteDesktop::default()).1);
                leave.arrived();
                let _gone = leaving(&leave, End::ScreenGone);
                tokio::time::sleep(Duration::from_secs(1)).await;
                let started = tokio::time::Instant::now();
                leave.stopping().await;
                assert_eq!(locks.get(), 1, "the stop locks");
                assert_eq!(
                    started.elapsed(),
                    Duration::ZERO,
                    "without waiting the grace out"
                );
                assert!(!marker.exists());
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn with_lock_on_leave_off_the_screen_going_neither_waits_nor_locks() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("logged-in");
        let locks = Rc::new(Cell::new(0));
        let (config, rx) = watch::channel(RemoteDesktop::default());
        config.send_modify(|c| c.lock_on_leave = false);
        let leave = LockOnLeave {
            config: rx,
            ..LockOnLeave::with_lock(Some(marker.clone()), counted(&locks))
        };
        leave.arrived();
        let started = tokio::time::Instant::now();
        leave.left(End::ScreenGone).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert_eq!(locks.get(), 0);
        assert!(!marker.exists());
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_logind_is_given_up_after_5_s() {
        let (_hold, hung) = std::sync::mpsc::channel::<()>();
        let started = tokio::time::Instant::now();
        let lock = on_thread(LOCK_WAIT, move |_| {
            let _ = hung.recv();
            Ok(())
        });
        let locked = tokio::time::timeout(Duration::from_secs(60), lock)
            .await
            .expect("the lock is given up on");
        assert_eq!(locked, Err("no answer in 5 s".into()));
        assert_eq!(started.elapsed(), LOCK_WAIT);
    }

    #[tokio::test(start_paused = true)]
    async fn a_lock_that_answers_after_being_given_up_on_sees_the_flag() {
        let (late, answer) = std::sync::mpsc::channel::<()>();
        let (saw, seen) = std::sync::mpsc::channel();
        let lock = on_thread(LOCK_WAIT, move |given_up| {
            let _ = answer.recv();
            let _ = saw.send(given_up.load(Ordering::SeqCst));
            Ok(())
        });
        assert_eq!(lock.await, Err("no answer in 5 s".into()));
        late.send(()).unwrap();
        assert_eq!(seen.recv(), Ok(true), "the job is told it was given up on");
    }

    #[tokio::test]
    async fn a_lock_that_answers_in_time_is_not_given_up_on() {
        let (saw, seen) = std::sync::mpsc::channel();
        let lock = on_thread(LOCK_WAIT, move |given_up| {
            let _ = saw.send(given_up.load(Ordering::SeqCst));
            Ok(())
        });
        assert_eq!(lock.await, Ok(()));
        assert_eq!(seen.recv(), Ok(false));
    }
}
