//! Whether this user's graphical session is on its seat: logind's `User.Display` session and its
//! `Active`, followed through `PropertiesChanged`.

use crate::lock::display_session;
use futures_util::{Stream, StreamExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

const LOGIN1: &str = "org.freedesktop.login1";
const DBUS: &str = "org.freedesktop.DBus";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// How long [`away`] waits for logind's first answer; until then the session counts as away.
const FIRST_READ: Duration = Duration::from_secs(2);
/// How long a seat follower that lost logind waits before it starts again.
const RETRY: Duration = Duration::from_secs(2);

/// True while this user's graphical session is not the active one on its seat, or while that
/// cannot be read. The first read is waited for here, so the client whose hand-over started this
/// server sees it.
pub fn away() -> impl Fn() -> bool + Send + Sync + 'static {
    on_thread(|away, read| {
        retrying(away, read, RETRY, |away, read| {
            zbus::block_on(follow(away, read))
        });
    })
}

/// Runs `follow` again `retry` after each end, counting the session as away until it reads.
fn retrying(
    away: &AtomicBool,
    read: &Sender<()>,
    retry: Duration,
    mut follow: impl FnMut(&AtomicBool, &Sender<()>) -> zbus::Result<()>,
) {
    let mut last = String::new();
    loop {
        let why = match follow(away, read) {
            Ok(()) => "logind's signals ended".to_string(),
            Err(e) => e.to_string(),
        };
        if why != last {
            log::warn!(
                "cannot tell whether the session is on its seat: {why}; trying again every {} s",
                retry.as_secs()
            );
        }
        last = why;
        set(away, true);
        let _ = read.send(());
        std::thread::sleep(retry);
    }
}

/// Runs `follow` on a thread of its own and waits up to [`FIRST_READ`] for its first read.
fn on_thread(
    follow: impl FnOnce(&AtomicBool, &Sender<()>) + Send + 'static,
) -> impl Fn() -> bool + Send + Sync + 'static {
    let away = Arc::new(AtomicBool::new(true));
    let (read, first) = std::sync::mpsc::channel();
    let flag = away.clone();
    let spawned = std::thread::Builder::new()
        .name("reprise-rdp-seat".into())
        .spawn(move || follow(&flag, &read));
    match spawned {
        Ok(_) => {
            let _ = first.recv_timeout(FIRST_READ);
        }
        Err(e) => log::warn!("no seat thread: {e}"),
    }
    move || away.load(Ordering::Relaxed)
}

async fn follow(away: &AtomicBool, read: &Sender<()>) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    let session = display_session(&conn).await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(LOGIN1)
        .path(session.as_str())
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .build()?;
    let owner_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DBUS)
        .path("/org/freedesktop/DBus")
        .interface(DBUS)
        .member("NameOwnerChanged")
        .add_arg(LOGIN1)
        .build()?;
    // Subscribed here so a failed AddMatch ends the follow. A signal only prompts a read from
    // logind, so its sender needs no check.
    let changes = zbus::MessageStream::for_match_rule(rule, &conn, None).await?;
    let owners = zbus::MessageStream::for_match_rule(owner_rule, &conn, None).await?;
    let (conn, session) = (&conn, &session);
    let reads = futures_util::stream::once(std::future::ready(()))
        .chain(futures_util::stream::select(
            changes.map(drop),
            owners.map(drop),
        ))
        .then(move |()| active(conn, session));
    fold(away, reads, read).await;
    Ok(())
}

/// Sets `away` from each read of the session's `Active`, a failed read counting as away, and
/// tells `read` after each.
async fn fold(away: &AtomicBool, reads: impl Stream<Item = zbus::Result<bool>>, read: &Sender<()>) {
    let mut reads = std::pin::pin!(reads);
    while let Some(active) = reads.next().await {
        match active {
            Ok(active) => set(away, !active),
            Err(e) => {
                log::warn!("cannot tell whether the session is on its seat: {e}");
                set(away, true);
            }
        }
        let _ = read.send(());
    }
}

fn set(flag: &AtomicBool, away: bool) {
    if flag.swap(away, Ordering::Relaxed) != away {
        match away {
            false => log::info!("the session is on its seat"),
            true => log::info!("the session is away from its seat; clients get lock mode"),
        }
    }
}

async fn active(conn: &Connection, session: &OwnedObjectPath) -> zbus::Result<bool> {
    let active: OwnedVariant = conn
        .call_method(
            Some(LOGIN1),
            session,
            Some(PROPERTIES),
            "Get",
            &("org.freedesktop.login1.Session", "Active"),
        )
        .await?
        .body()
        .deserialize()?;
    bool::try_from(active)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed() -> zbus::Result<bool> {
        Err(zbus::Error::Failure("logind went away".into()))
    }

    /// `away` after folding `reads` into a session first read as on its seat.
    fn folded(reads: Vec<zbus::Result<bool>>) -> bool {
        let away = AtomicBool::new(false);
        let (read, _first) = std::sync::mpsc::channel();
        zbus::block_on(fold(&away, futures_util::stream::iter(reads), &read));
        away.load(Ordering::Relaxed)
    }

    #[test]
    fn a_failed_seat_read_keeps_following() {
        assert!(
            folded(vec![Ok(true), failed(), Ok(false)]),
            "on seat, then away"
        );
        assert!(
            !folded(vec![Ok(false), failed(), Ok(true)]),
            "away, then on seat"
        );
    }

    #[test]
    fn a_failed_seat_read_counts_as_away() {
        assert!(folded(vec![Ok(true), failed()]));
    }

    #[test]
    fn a_failed_or_missing_first_read_counts_as_away() {
        let failed_first = on_thread(|away, read| {
            let reads = futures_util::stream::iter([failed()]);
            zbus::block_on(fold(away, reads, read));
        });
        assert!(failed_first(), "a failed first read");
        let never_read = on_thread(|_, _| {});
        assert!(never_read(), "no read at all");
    }

    #[test]
    fn a_seat_follower_that_ends_starts_again() {
        let mut runs = 0;
        let seat = on_thread(move |away, read| {
            retrying(away, read, Duration::ZERO, |away, read| {
                runs += 1;
                if runs == 1 {
                    return Err(zbus::Error::Failure("no system bus".into()));
                }
                zbus::block_on(fold(away, futures_util::stream::iter([Ok(true)]), read));
                loop {
                    std::thread::park();
                }
            })
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while seat() {
            assert!(
                std::time::Instant::now() < deadline,
                "the second run reads the session as on its seat"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
