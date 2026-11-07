//! `ACCOUNT_SOCKET`: any local user's requests about their own Remote Desktop password, one
//! per connection, answered for the peer's uid and then closed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::os::unix::net::UnixListener as StdListener;
use std::rc::Rc;
use std::time::Duration;

use repriselogin::account::{AccountRefusal, AccountReply, AccountRequest};
use repriselogin::frame;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

use crate::daemon::Input;
use crate::ipc::{self, Checks, Frames, InFlight};
use crate::users;

/// How long a connection has to send its request, and NSS to name its peer.
pub const WAIT: Duration = Duration::from_secs(5);

/// The connections waiting for the daemon's answer, by link.
#[derive(Clone, Default)]
pub struct Replies(Rc<RefCell<HashMap<u64, oneshot::Sender<AccountReply>>>>);

impl Replies {
    /// Answers `link`; a link that is gone drops the reply.
    pub fn send(&self, link: u64, reply: AccountReply) {
        if let Some(tx) = self.0.borrow_mut().remove(&link) {
            let _ = tx.send(reply);
        }
    }
}

/// Accepts on `listener` in a task of the current `LocalSet`. Each request goes to `input` as
/// `Input::Account`; the reply comes back through the returned [`Replies`].
pub fn serve(listener: StdListener, input: mpsc::Sender<Input>) -> io::Result<Replies> {
    let listener = UnixListener::from_std(listener)?;
    let replies = Replies::default();
    tokio::task::spawn_local(accept(listener, input, replies.clone()));
    Ok(replies)
}

async fn accept(listener: UnixListener, input: mpsc::Sender<Input>, replies: Replies) {
    let checks = Rc::new(RefCell::new(Checks::default()));
    let mut links = 0;
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                log::warn!("account socket: accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(uid) = stream.peer_cred().map(|c| c.uid()) else {
            continue;
        };
        // Held until the reply is written, so it also caps a uid's requests in the daemon.
        let Some(in_flight) = InFlight::begin(&checks, uid) else {
            log::debug!("account socket: uid {uid} has too many connections open");
            continue;
        };
        links += 1;
        let (link, input, replies) = (links, input.clone(), replies.clone());
        tokio::task::spawn_local(async move {
            let _in_flight = in_flight;
            if let Err(why) = answer(stream, link, uid, &input, &replies).await {
                log::info!("account socket: uid {uid}: {why}");
            }
        });
    }
}

/// Reads the link's one request, hands it to the daemon and writes its reply.
async fn answer(
    mut stream: UnixStream,
    link: u64,
    uid: u32,
    input: &mpsc::Sender<Input>,
    replies: &Replies,
) -> Result<(), String> {
    let read = tokio::time::timeout(WAIT, Frames::new(&mut stream).next()).await;
    let body = read
        .map_err(|_| "no request in time".to_string())?
        .map_err(|e| ipc::error_kind(&e))?;
    let request: AccountRequest = frame::decode(&body).map_err(|e| ipc::error_kind(&e))?;
    let reply = match name_of(uid).await {
        Some(user) => {
            let (tx, rx) = oneshot::channel();
            replies.0.borrow_mut().insert(link, tx);
            let account = Input::Account {
                link,
                uid,
                user,
                request,
            };
            if input.send(account).await.is_err() {
                replies.0.borrow_mut().remove(&link);
                return Ok(());
            }
            rx.await
                .map_err(|_| "the daemon dropped the request".to_string())?
        }
        None => AccountReply::Refused {
            reason: AccountRefusal::Failed,
        },
    };
    let frame = frame::encode(&reply).map_err(|e| ipc::error_kind(&e))?;
    (stream.write_all(&frame).await).map_err(|e| format!("writing the reply: {}", e.kind()))
}

/// `uid`'s account name through NSS, off the loop and within `WAIT`.
async fn name_of(uid: u32) -> Option<String> {
    let found = tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || users::by_uid(uid)),
    );
    let why = match found.await {
        Ok(Ok(Ok(Some(account)))) => return Some(account.name),
        Ok(Ok(Ok(None))) => "no account".to_string(),
        Ok(Ok(Err(e))) => e.to_string(),
        Ok(Err(e)) => e.to_string(),
        Err(_) => format!("no answer within {WAIT:?}"),
    };
    log::warn!("account socket: looking up uid {uid}: {why}");
    None
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;

    use super::*;

    const TEST_WAIT: Duration = Duration::from_secs(5);

    fn own_uid() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    async fn reply_of(stream: &mut UnixStream) -> AccountReply {
        let mut frames = Frames::new(stream);
        let body = tokio::time::timeout(TEST_WAIT, frames.next())
            .await
            .expect("a reply in time")
            .expect("a reply frame");
        frame::decode(&body).unwrap()
    }

    /// Whether the peer closes `stream` within `TEST_WAIT`.
    async fn closed(stream: &mut UnixStream) -> bool {
        let mut buf = [0u8; 64];
        matches!(
            tokio::time::timeout(TEST_WAIT, stream.read(&mut buf)).await,
            Ok(Ok(0) | Err(_))
        )
    }

    #[tokio::test]
    async fn a_request_is_read_with_its_peers_uid_and_the_link_closes_after_the_reply() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("account.sock");
                let (tx, mut rx) = mpsc::channel(ipc::QUEUE);
                let replies = serve(ipc::bind_open(&path).unwrap(), tx).unwrap();
                let mut stream = UnixStream::connect(&path).await.unwrap();
                let request = AccountRequest::ClearRemotePassword {
                    login_password: zeroize::Zeroizing::new("hunter2".into()),
                };
                stream
                    .write_all(&frame::encode(&request).unwrap())
                    .await
                    .unwrap();
                let got = tokio::time::timeout(TEST_WAIT, rx.recv())
                    .await
                    .expect("an input in time")
                    .expect("the channel is open");
                let me = users::by_uid(own_uid()).unwrap().unwrap().name;
                let Input::Account {
                    link,
                    uid,
                    user,
                    request: got,
                } = got
                else {
                    panic!("an account request, got {got:?}");
                };
                assert_eq!((uid, user, got), (own_uid(), me, request));
                replies.send(link, AccountReply::Done);
                assert_eq!(reply_of(&mut stream).await, AccountReply::Done);
                assert!(closed(&mut stream).await, "the link closes after its reply");
            })
            .await;
    }

    #[tokio::test]
    async fn a_uids_fifth_open_connection_is_closed_unread() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("account.sock");
                let (tx, mut rx) = mpsc::channel(ipc::QUEUE);
                let _replies = serve(ipc::bind_open(&path).unwrap(), tx).unwrap();
                let mut open = Vec::new();
                for _ in 0..4 {
                    open.push(UnixStream::connect(&path).await.unwrap());
                }
                tokio::task::yield_now().await;
                let mut fifth = UnixStream::connect(&path).await.unwrap();
                assert!(closed(&mut fifth).await, "the fifth connection is closed");
                let mut first = open.remove(0);
                first
                    .write_all(&frame::encode(&AccountRequest::Status).unwrap())
                    .await
                    .unwrap();
                let got = tokio::time::timeout(TEST_WAIT, rx.recv()).await;
                assert!(
                    matches!(got, Ok(Some(Input::Account { .. }))),
                    "the first four are read: {got:?}"
                );
                let mode = std::fs::metadata(&path).unwrap();
                assert_eq!(
                    std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o777,
                    0o666
                );
            })
            .await;
    }
}
