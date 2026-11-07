//! Whether this user's graphical session is on its seat: logind's `User.Display` session and its
//! `Active`, followed through `PropertiesChanged`.

use crate::lock::display_session;
use futures_util::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

const LOGIN1: &str = "org.freedesktop.login1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// How long [`away`] waits for logind's first answer; until then the session counts as on its seat.
const FIRST_READ: Duration = Duration::from_secs(2);

/// True while this user's graphical session is not the active one on its seat. The first read
/// is waited for here, so the client whose hand-over started this server sees it.
pub fn away() -> impl Fn() -> bool + Send + Sync + 'static {
    let away = Arc::new(AtomicBool::new(false));
    let (read, first) = std::sync::mpsc::channel();
    let flag = away.clone();
    let spawned = std::thread::Builder::new()
        .name("reprise-rdp-seat".into())
        .spawn(move || {
            if let Err(e) = zbus::block_on(follow(&flag, read)) {
                log::warn!("cannot tell whether the session is on its seat: {e}");
            }
        });
    match spawned {
        Ok(_) => {
            let _ = first.recv_timeout(FIRST_READ);
        }
        Err(e) => log::warn!("no seat thread: {e}"),
    }
    move || away.load(Ordering::Relaxed)
}

async fn follow(away: &AtomicBool, read: std::sync::mpsc::Sender<()>) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    let session = display_session(&conn).await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(LOGIN1)
        .path(session.as_str())
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .build()?;
    let mut changes = reprisebus::signals(&conn, LOGIN1, vec![rule.into()]).await;
    set(away, active(&conn, &session).await?);
    let _ = read.send(());
    while changes.next().await.is_some() {
        set(away, active(&conn, &session).await?);
    }
    Ok(())
}

fn set(away: &AtomicBool, active: bool) {
    if away.swap(!active, Ordering::Relaxed) != !active {
        match active {
            true => log::info!("the session is on its seat"),
            false => log::info!("the session is away from its seat; clients get lock mode"),
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
