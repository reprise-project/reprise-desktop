//! reprise-login-rdp: the Remote Desktop front of reprise-login.

mod accept;
mod limits;
mod link;
mod negotiate;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use reprisebus::journal;
use repriselogin::front::{FrontEvent, FrontRequest, Secret};
use repriselogin::machine::{MACHINE_CONFIG, MachineConfig};
use reprisetls::{cert, tcp};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use accept::{Ctx, Refused};
use limits::{Limits, MAX_PREAUTH, PREAUTH_PER_IP, Permit, VERDICT_WAIT};

const RECONNECT: Duration = Duration::from_secs(1);
/// A failing accept (out of fds) is retried after this, not at once.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// A connection between its accept and its verdict.
struct Conn {
    abort: AbortHandle,
    /// reprise-login's events for the connection.
    tell: mpsc::UnboundedSender<FrontEvent>,
}

type Conns = Rc<RefCell<HashMap<u64, Conn>>>;

/// Kept at warn even under RUST_LOG=debug: their debug records carry the NTLM messages, which
/// an attacker can crack offline.
const CREDSSP_LOGS: [&str; 2] = ["ironrdp", "sspi"];

fn main() -> ExitCode {
    let mut logger = journal::builder(journal::PROGRAM_FILTER, journal::on_stream());
    for target in CREDSSP_LOGS {
        logger.filter_module(target, log::LevelFilter::Warn);
    }
    logger.init();
    let port = machine_config().remote.port;
    let (Some(state), Some(runtime)) = (unit_dir("STATE_DIRECTORY"), unit_dir("RUNTIME_DIRECTORY"))
    else {
        log::error!("STATE_DIRECTORY and RUNTIME_DIRECTORY are set by reprise-login-rdp.service");
        return ExitCode::FAILURE;
    };
    let identity = match cert::load_or_create(&state) {
        Ok(identity) => identity,
        Err(e) => {
            log::error!("{}: no certificate: {e}", state.display());
            return ExitCode::FAILURE;
        }
    };
    log::info!("certificate fingerprint {}", identity.fingerprint);
    let fingerprint = runtime.join("fingerprint");
    if let Err(e) = write_public(&fingerprint, &format!("{}\n", identity.fingerprint)) {
        log::warn!("{}: {e}", fingerprint.display());
    }
    // The unit allows 64 tasks; the front runs on one thread.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(2)
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("no runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let ctx = Rc::new(Ctx {
        identity,
        nla_users: Rc::default(),
        link: RefCell::new(None),
        host: hostname(),
    });
    tokio::task::LocalSet::new().block_on(&rt, serve(port, ctx))
}

fn machine_config() -> MachineConfig {
    let text = match fs::read_to_string(MACHINE_CONFIG) {
        Ok(text) => text,
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log::warn!("{MACHINE_CONFIG}: {e}");
            }
            String::new()
        }
    };
    let (config, warnings) = MachineConfig::parse(&text);
    for w in warnings {
        log::warn!("{MACHINE_CONFIG}: {w}");
    }
    config
}

fn hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_owned())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

/// The first directory systemd names in `var`.
fn unit_dir(var: &str) -> Option<PathBuf> {
    let value = std::env::var_os(var)?;
    let first = value.to_str()?.split(':').next()?;
    (!first.is_empty()).then(|| PathBuf::from(first))
}

/// Replaces `path` with `text`, readable by all whatever the umask.
fn write_public(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)?;
    f.set_permissions(fs::Permissions::from_mode(0o644))?;
    f.write_all(text.as_bytes())?;
    fs::rename(&tmp, path)
}

async fn serve(port: u16, ctx: Rc<Ctx>) -> ExitCode {
    let addr = SocketAddr::new(tcp::ALL, port);
    let listener = match tcp::bind(addr) {
        Ok(listener) => listener,
        Err(e) => {
            log::error!("port {port}: {e}");
            return ExitCode::FAILURE;
        }
    };
    log::info!("listening on port {port}");
    let conns = Conns::default();
    let limits = Rc::new(RefCell::new(Limits::new(MAX_PREAUTH, PREAUTH_PER_IP)));
    tokio::task::spawn_local(follow_link(ctx.clone(), conns.clone()));
    let mut next_conn = 0;
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                log::warn!("accept: {e}");
                tokio::time::sleep(ACCEPT_RETRY).await;
                continue;
            }
        };
        let accepted = Instant::now();
        if let Err(e) = tcp::tune(&tcp) {
            log::debug!("{peer}: {e}");
        }
        if ctx.link.borrow().is_none() {
            log::info!("{peer}: refused: reprise-login is not reachable");
            continue;
        }
        let permit = match limits.borrow_mut().admit(peer.ip(), accepted) {
            Ok(permit) => permit,
            Err(refusal) => {
                log::info!("{peer}: refused: {refusal}");
                continue;
            }
        };
        next_conn += 1;
        let conn = next_conn;
        let (tell, heard) = mpsc::unbounded_channel();
        let task = tokio::task::spawn_local(connection(
            Arrival {
                tcp,
                peer,
                conn,
                accepted,
                heard,
                _permit: permit,
            },
            ctx.clone(),
            conns.clone(),
            limits.clone(),
        ));
        let abort = task.abort_handle();
        conns.borrow_mut().insert(conn, Conn { abort, tell });
    }
}

struct Arrival {
    tcp: TcpStream,
    peer: SocketAddr,
    conn: u64,
    accepted: Instant,
    heard: mpsc::UnboundedReceiver<FrontEvent>,
    /// Held until the connection ends.
    _permit: Permit,
}

/// Forgets its connection when the task ends or is aborted.
struct Forget(Conns, u64);

impl Drop for Forget {
    fn drop(&mut self) {
        if let Ok(mut conns) = self.0.try_borrow_mut() {
            conns.remove(&self.1);
        }
    }
}

async fn connection(arrival: Arrival, ctx: Rc<Ctx>, conns: Conns, limits: Rc<RefCell<Limits>>) {
    let Arrival {
        tcp,
        peer,
        conn,
        accepted,
        mut heard,
        _permit,
    } = arrival;
    let _forget = Forget(conns.clone(), conn);
    let pending = match accept::to_client_info(tcp, conn, accepted, &ctx, &mut heard).await {
        Ok(pending) => pending,
        Err(Refused::Denied { backoff_s }) => {
            log::info!("{peer}: the logon was refused");
            back_off(&limits, peer, backoff_s);
            return;
        }
        Err(refused) => {
            log::info!("{peer}: refused: {refused}");
            return;
        }
    };
    if !pending.nla {
        let secret = Secret::Password(pending.password.clone());
        let user = pending.user.clone();
        if let Err(e) = accept::authenticate(&ctx, conn, peer, accepted, user, secret) {
            log::warn!("{peer}: reprise-login: {e}");
            return;
        }
    }
    let Ok(verdict) = tokio::time::timeout(VERDICT_WAIT, verdict(&mut heard)).await else {
        log::warn!("{peer}: reprise-login gave no verdict");
        if let Some(link) = &*ctx.link.borrow() {
            let _ = link.send(&FrontRequest::Gone { conn }, None);
        }
        return;
    };
    match verdict {
        Some(FrontEvent::Denied { backoff_s, .. }) => {
            log::info!("{peer}: the logon was refused");
            back_off(&limits, peer, backoff_s);
            accept::deny(pending).await;
        }
        Some(FrontEvent::Accepted { .. }) => log::info!("{peer}: logged on"),
        Some(_) | None => {}
    }
}

/// The next `Accepted` or `Denied`; `Verified` only says the password was right.
async fn verdict(heard: &mut mpsc::UnboundedReceiver<FrontEvent>) -> Option<FrontEvent> {
    loop {
        match heard.recv().await? {
            FrontEvent::Verified { .. } => {}
            event => return Some(event),
        }
    }
}

fn back_off(limits: &RefCell<Limits>, peer: SocketAddr, backoff_s: u32) {
    let for_ = Duration::from_secs(backoff_s.into());
    limits
        .borrow_mut()
        .back_off(peer.ip(), for_, Instant::now());
}

/// Keeps the link to reprise-login up: the latest Remote Desktop users, each verdict to its
/// connection, and every connection still logging in closed when the link is lost.
async fn follow_link(ctx: Rc<Ctx>, conns: Conns) {
    let mut told = false;
    loop {
        match link::connect().await {
            Ok((sender, mut events)) => {
                log::info!("connected to reprise-login");
                told = false;
                *ctx.link.borrow_mut() = Some(sender);
                while let Some(event) = events.next().await {
                    if let FrontEvent::Passwords { users } = event {
                        *ctx.nla_users.borrow_mut() = users;
                    } else if let Some(conn) = accept::event_conn(&event)
                        && let Some(c) = conns.borrow().get(&conn)
                    {
                        let _ = c.tell.send(event);
                    }
                }
                log::warn!("lost reprise-login; closing every connection still logging in");
                *ctx.link.borrow_mut() = None;
                ctx.nla_users.borrow_mut().clear();
                let aborts: Vec<AbortHandle> =
                    conns.borrow_mut().drain().map(|(_, c)| c.abort).collect();
                for abort in aborts {
                    abort.abort();
                }
            }
            Err(e) if !told => {
                log::warn!(
                    "reprise-login: {e}; retrying every {} s",
                    RECONNECT.as_secs()
                );
                told = true;
            }
            Err(_) => {}
        }
        tokio::time::sleep(RECONNECT).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accept::tests::request;
    use limits::Refusal;
    use repriselogin::fdpass::FdFrameReader;
    use tokio::net::{TcpListener, UnixListener};

    #[tokio::test]
    async fn a_logon_with_no_verdict_is_gone_after_the_verdict_wait() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("front.sock");
                let listener = UnixListener::bind(&path).unwrap();
                let ctx = Rc::new(accept::tests::ctx());
                let (sender, _events) = link::connect_to(&path).await.unwrap();
                *ctx.link.borrow_mut() = Some(sender);
                let (daemon, _) = listener.accept().await.unwrap();
                let mut reader = FdFrameReader::new();
                assert_eq!(request(&daemon, &mut reader).await, FrontRequest::Hello);

                let rdp = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let client = tokio::spawn(accept::tests::client(
                    rdp.local_addr().unwrap(),
                    "anna".into(),
                    "secret".into(),
                ));
                let (tcp, peer) = rdp.accept().await.unwrap();
                let limits = Rc::new(RefCell::new(Limits::new(1, 1)));
                let start = Instant::now();
                let permit = limits.borrow_mut().admit(peer.ip(), start).unwrap();
                let conns = Conns::default();
                let (tell, heard) = mpsc::unbounded_channel();
                let arrival = Arrival {
                    tcp,
                    peer,
                    conn: 5,
                    accepted: start,
                    heard,
                    _permit: permit,
                };
                let task = tokio::task::spawn_local(connection(
                    arrival,
                    ctx.clone(),
                    conns.clone(),
                    limits.clone(),
                ));
                conns.borrow_mut().insert(
                    5,
                    Conn {
                        abort: task.abort_handle(),
                        tell,
                    },
                );
                let authenticate = request(&daemon, &mut reader).await;
                assert!(
                    matches!(authenticate, FrontRequest::Authenticate { conn: 5, .. }),
                    "{authenticate:?}"
                );
                // The handshake runs on the real clock; the wait is stepped through paused.
                tokio::time::pause();
                let other: std::net::IpAddr = "192.0.2.9".parse().unwrap();
                tokio::time::sleep_until(start + VERDICT_WAIT - Duration::from_millis(1)).await;
                assert_eq!(
                    limits.borrow_mut().admit(other, Instant::now()).err(),
                    Some(Refusal::Full),
                    "the slot is held while the verdict is due"
                );

                let gone =
                    tokio::time::timeout(Duration::from_secs(1), request(&daemon, &mut reader))
                        .await
                        .expect("Gone is sent once the verdict wait ends");
                assert_eq!(gone, FrontRequest::Gone { conn: 5 });
                assert!(start.elapsed() >= VERDICT_WAIT, "{:?}", start.elapsed());
                limits
                    .borrow_mut()
                    .admit(other, Instant::now())
                    .expect("the slot is free once the logon is given up");
                assert!(conns.borrow().is_empty(), "the connection is forgotten");
                client.abort();
            })
            .await;
    }
}
