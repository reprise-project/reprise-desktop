//! The computer turning off or restarting, whoever asked: the server holds a logind shutdown
//! delay lock, and at `PrepareForShutdown(true)` hands it to the accept loop with
//! [`Request::ShuttingDown`], so every client hears the session logged off before systemd stops
//! the server. A shutdown called off takes the lock again.

use crate::control::Request;
use futures_util::StreamExt;
use reprisebus::Signal;
use std::os::fd::OwnedFd;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use zbus::{Connection, OwnedFd as ZOwnedFd};

const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
/// How long a follower that lost logind waits before it starts again.
const RETRY: Duration = Duration::from_secs(2);

/// Follow logind's shutdowns on a thread of its own for the process's life.
pub fn follow(requests: UnboundedSender<Request>) {
    let spawned = std::thread::Builder::new()
        .name("reprise-rdp-shutdown".into())
        .spawn(move || {
            loop {
                let why = match zbus::block_on(follow_on_system(&requests)) {
                    Ok(()) => return,
                    Err(e) => e.to_string(),
                };
                log::warn!(
                    "a shutdown may drop the clients unwarned: {why}; trying again in {} s",
                    RETRY.as_secs()
                );
                std::thread::sleep(RETRY);
            }
        });
    if let Err(e) = spawned {
        log::warn!("no shutdown thread: {e}");
    }
}

async fn follow_on_system(requests: &UnboundedSender<Request>) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    watch(&conn, requests).await
}

/// Hold the delay lock on `conn` and pass each shutdown on to `requests`; Ok once the accept
/// loop is gone, an error once logind's signals end.
async fn watch(conn: &Connection, requests: &UnboundedSender<Request>) -> zbus::Result<()> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(LOGIN1)
        .path(LOGIN1_PATH)
        .interface(MANAGER)
        .member("PrepareForShutdown")
        .build()?;
    // Only logind's own broadcasts count, so a spoofed signal cannot end the clients.
    let mut signals = reprisebus::signals(conn, LOGIN1, vec![rule.into()]).await;
    let mut delay = take_delay(conn).await;
    while let Some(signal) = signals.next().await {
        let starting = match signal {
            Signal::OwnerChanged => {
                delay = take_delay(conn).await;
                continue;
            }
            Signal::Message(m) => m.body().deserialize::<bool>(),
        };
        match starting {
            Ok(true) => {
                log::info!("the computer is turning off; ending every client");
                if requests.send(Request::ShuttingDown(delay.take())).is_err() {
                    return Ok(());
                }
            }
            Ok(false) => delay = take_delay(conn).await,
            Err(e) => log::warn!("PrepareForShutdown: {e}"),
        }
    }
    Err(zbus::Error::Failure("logind's signals ended".into()))
}

/// A shutdown delay lock, or None when logind refuses one.
async fn take_delay(conn: &Connection) -> Option<OwnedFd> {
    let what = (
        "shutdown",
        "Reprise Remote Desktop",
        "Telling remote clients the computer is turning off",
        "delay",
    );
    let reply = conn
        .call_method(Some(LOGIN1), LOGIN1_PATH, Some(MANAGER), "Inhibit", &what)
        .await;
    match reply.and_then(|m| m.body().deserialize::<ZOwnedFd>()) {
        Ok(fd) => Some(fd.into()),
        Err(e) => {
            log::warn!("no shutdown delay lock ({e}); a shutdown may drop the clients unwarned");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisebus::testbus::bus;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;
    use zbus::connection::Builder;
    use zbus::object_server::SignalEmitter;

    /// logind's Manager as far as the delay lock goes: each Inhibit hands out one end of a new
    /// socket pair, keeps the other and says so on `taken`.
    struct Logind {
        kept: Arc<Mutex<Vec<UnixStream>>>,
        taken: mpsc::UnboundedSender<()>,
    }

    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Logind {
        fn inhibit(&self, what: &str, _who: &str, _why: &str, mode: &str) -> ZOwnedFd {
            assert_eq!((what, mode), ("shutdown", "delay"));
            let (given, kept) = UnixStream::pair().unwrap();
            self.kept.lock().unwrap().push(kept);
            let _ = self.taken.send(());
            ZOwnedFd::from(OwnedFd::from(given))
        }

        #[zbus(signal)]
        async fn prepare_for_shutdown(e: &SignalEmitter<'_>, start: bool) -> zbus::Result<()>;
    }

    #[test]
    fn a_shutdown_hands_the_delay_lock_to_the_accept_loop_and_one_called_off_takes_another() {
        let Some(bus) = bus() else {
            return;
        };
        let kept = Arc::new(Mutex::new(Vec::new()));
        let (taken, mut takes) = mpsc::unbounded_channel();
        let logind = Logind {
            kept: kept.clone(),
            taken,
        };
        zbus::block_on(async {
            let server = Builder::address(bus.1.as_str())
                .name(LOGIN1)
                .serve_at(LOGIN1_PATH, logind)
                .build()
                .await
                .unwrap();
            let conn = reprisebus::testbus::connect(bus.1.as_str()).await;
            let (requests, mut got) = mpsc::unbounded_channel();
            let emit = async |start| {
                let iface = (server.object_server())
                    .interface::<_, Logind>(LOGIN1_PATH)
                    .await
                    .unwrap();
                Logind::prepare_for_shutdown(iface.signal_emitter(), start)
                    .await
                    .unwrap();
            };
            let driving = async {
                takes.recv().await.unwrap();
                emit(true).await;
                let Some(Request::ShuttingDown(Some(delay))) = got.recv().await else {
                    panic!("the shutdown reaches the accept loop with the lock");
                };
                drop(delay);
                let mut first = kept.lock().unwrap()[0].try_clone().unwrap();
                assert_eq!(
                    first.read(&mut [0u8; 1]).unwrap(),
                    0,
                    "the lock goes once the loop lets it go"
                );
                emit(false).await;
                takes.recv().await.unwrap();
                drop(got);
                emit(true).await;
            };
            let (watched, ()) = futures_util::future::join(watch(&conn, &requests), driving).await;
            assert!(
                watched.is_ok(),
                "the follow ends once the accept loop is gone"
            );
        });
    }
}
