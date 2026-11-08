//! reprise-login-rdp: the Remote Desktop front of reprise-login.

mod accept;
mod handoff;
mod limits;
mod link;
mod listen;
mod negotiate;
mod refusals;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use reprisebus::journal;
use repriselogin::front::{FRONT_SOCKET, FrontEvent, FrontRequest, Secret};
use repriselogin::machine::{MACHINE_CONFIG, MachineConfig};
use reprisetls::{cert, tcp};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use accept::{Ctx, Pending, Refused};
use limits::{Limits, MAX_PREAUTH, PREAUTH_PER_IP, Permit, VERDICT_WAIT};
use refusals::REPORT_EVERY;

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
    if !machine_config().remote.enabled {
        log::info!("remote logins are off in {MACHINE_CONFIG}; the front stays off");
        return ExitCode::SUCCESS;
    }
    let listener = match listen::passed() {
        Ok(listener) => listener,
        Err(e) => {
            log::error!("{e}");
            return ExitCode::FAILURE;
        }
    };
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
    if let Err(e) = cert::replace(
        &fingerprint,
        format!("{}\n", identity.fingerprint).as_bytes(),
        0o644,
    ) {
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
        host: cert::hostname(),
        nla_throttle: RefCell::default(),
        refusals: RefCell::default(),
    });
    tokio::task::LocalSet::new().block_on(&rt, serve(listener, ctx))
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

/// The first directory systemd names in `var`.
fn unit_dir(var: &str) -> Option<PathBuf> {
    let value = std::env::var_os(var)?;
    let first = value.to_str()?.split(':').next()?;
    (!first.is_empty()).then(|| PathBuf::from(first))
}

/// A random start for the conn ids, so a result reprise-login sent for an earlier front's conn
/// never names one of this front's. `RandomState`'s keys come from the OS.
fn first_conn() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(std::process::id())
}

async fn serve(listener: std::net::TcpListener, ctx: Rc<Ctx>) -> ExitCode {
    let listener = match TcpListener::from_std(listener) {
        Ok(listener) => listener,
        Err(e) => {
            log::error!("the passed socket: {e}");
            return ExitCode::FAILURE;
        }
    };
    match listener.local_addr() {
        Ok(addr) => log::info!("listening on port {}", addr.port()),
        Err(e) => log::warn!("the passed socket: {e}"),
    }
    let conns = Conns::default();
    let limits = Rc::new(RefCell::new(Limits::new(MAX_PREAUTH, PREAUTH_PER_IP)));
    let front = PathBuf::from(FRONT_SOCKET);
    tokio::task::spawn_local(follow_link(ctx.clone(), conns.clone(), front));
    tokio::task::spawn_local(report_refusals(ctx.clone()));
    let mut next_conn = first_conn();
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
        if ctx.link.borrow().is_none() {
            refused(
                &ctx,
                peer,
                Some("no reprise-login"),
                "reprise-login is not reachable",
            );
            continue;
        }
        let permit = match limits.borrow_mut().admit(peer.ip(), accepted) {
            Ok(permit) => permit,
            Err(refusal) => {
                refused(&ctx, peer, Some(refusal.kind()), refusal);
                continue;
            }
        };
        if let Err(e) = tcp::tune(&tcp) {
            log::debug!("{peer}: {e}");
        }
        next_conn = next_conn.wrapping_add(1);
        let conn = next_conn;
        let (tell, heard) = mpsc::unbounded_channel();
        let task = tokio::task::spawn_local(connection(
            Arrival {
                tcp,
                peer,
                conn,
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
        mut heard,
        _permit,
    } = arrival;
    let _forget = Forget(conns.clone(), conn);
    let pending = match accept::to_client_info(tcp, conn, &ctx, &mut heard).await {
        Ok(pending) => pending,
        Err(Refused::Denied { backoff_s }) => {
            log::info!("{peer}: the logon was refused");
            back_off(&limits, peer, backoff_s);
            return;
        }
        Err(refusal) => {
            refused(&ctx, peer, refusal.counted_as(), refusal);
            return;
        }
    };
    if !pending.nla {
        let secret = Secret::Password(pending.password.clone());
        let user = pending.user.clone();
        if let Err(e) = accept::authenticate(&ctx, conn, peer, user, secret) {
            log::warn!("{peer}: reprise-login: {e}");
            return;
        }
    }
    // Under NLA the verdict can come before Client Info; it waits in `heard`.
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
        Some(FrontEvent::Accepted { .. }) => hand_off(&ctx, peer, pending),
        Some(_) | None => {}
    }
}

/// Hands the logged-on connection to reprise-login and logs it once written; when that fails it
/// hears the connection is gone, and the connection is closed.
fn hand_off(ctx: &Ctx, peer: SocketAddr, pending: Pending) {
    let link = ctx.link.borrow();
    let Some(link) = &*link else {
        return;
    };
    let conn = pending.conn;
    let sent = match handoff::prepare(pending) {
        Ok((tcp, handover)) => handoff::send(link, tcp, handover).map_err(|e| e.to_string()),
        Err(e) => {
            let _ = link.send(&FrontRequest::Gone { conn }, None);
            Err(e.to_string())
        }
    };
    match sent {
        Ok(written) => {
            tokio::spawn(async move {
                match written.await {
                    Ok(()) => log::info!("{peer}: logged on"),
                    Err(_) => log::warn!(
                        "{peer}: logged on, but the hand-over failed: the link to reprise-login was lost"
                    ),
                }
            });
        }
        Err(e) => log::warn!("{peer}: logged on, but the hand-over failed: {e}"),
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

/// Logs a refusal before the logon in full, or counts it as `kind` for the next report.
/// Returns whether it was logged in full.
fn refused(
    ctx: &Ctx,
    peer: SocketAddr,
    kind: Option<&'static str>,
    why: impl fmt::Display,
) -> bool {
    let in_full = kind.is_none_or(|kind| ctx.refusals.borrow_mut().log_in_full(kind));
    if in_full {
        log::info!("{peer}: refused: {why}");
    }
    in_full
}

async fn report_refusals(ctx: Rc<Ctx>) {
    let mut every = tokio::time::interval(REPORT_EVERY);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        let report = ctx.refusals.borrow_mut().report();
        if let Some(line) = report {
            log::info!("{line}");
        }
    }
}

/// Keeps the link to reprise-login up: the latest Remote Desktop users, each verdict to its
/// connection, and every connection still logging in closed when the link is lost.
async fn follow_link(ctx: Rc<Ctx>, conns: Conns, path: PathBuf) {
    let mut told = false;
    loop {
        match link::connect_to(&path).await {
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
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    #[test]
    fn the_fingerprint_is_readable_by_all_under_a_private_umask() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fingerprint");
        // SAFETY: umask only swaps the process's file mode mask.
        let old = unsafe { libc::umask(0o077) };
        let written = cert::replace(&path, b"AB:CD\n", 0o644);
        // SAFETY: as above.
        unsafe { libc::umask(old) };
        written.unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "AB:CD\n");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "{mode:o}");
    }

    #[test]
    fn the_fingerprint_replaces_symlinks_without_following_them() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "kept").unwrap();
        let path = dir.path().join("fingerprint");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        std::os::unix::fs::symlink(&target, path.with_extension("tmp")).unwrap();
        cert::replace(&path, b"AB:CD\n", 0o644).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "kept");
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_file());
        assert_eq!(fs::read_to_string(&path).unwrap(), "AB:CD\n");
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_refused_as_a_second_one_is_tried_again() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("front.sock");
                let listener = UnixListener::bind(&path).unwrap();
                let ctx = Rc::new(accept::tests::ctx());
                tokio::task::spawn_local(follow_link(ctx.clone(), Conns::default(), path));
                let (refused, _) = listener.accept().await.unwrap();
                drop(refused);
                let retried = tokio::time::timeout(RECONNECT * 10, listener.accept());
                let (daemon, _) = retried.await.expect("the front connects again").unwrap();
                let mut reader = FdFrameReader::new();
                assert_eq!(request(&daemon, &mut reader).await, FrontRequest::Hello);
                assert!(ctx.link.borrow().is_some(), "the front holds the new link");
            })
            .await;
    }

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
                let permit = limits
                    .borrow_mut()
                    .admit(peer.ip(), Instant::now())
                    .unwrap();
                let conns = Conns::default();
                let (tell, heard) = mpsc::unbounded_channel();
                let arrival = Arrival {
                    tcp,
                    peer,
                    conn: 5,
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
                let start = Instant::now();
                // The handshake runs on the real clock; the wait is stepped through paused.
                tokio::time::pause();
                let slack = Duration::from_millis(100);
                let other: std::net::IpAddr = "192.0.2.9".parse().unwrap();
                tokio::time::sleep_until(start + VERDICT_WAIT - slack).await;
                assert_eq!(
                    limits.borrow_mut().admit(other, Instant::now()).err(),
                    Some(Refusal::Full),
                    "the slot is held while the verdict is due"
                );

                let gone = tokio::time::timeout(2 * slack, request(&daemon, &mut reader))
                    .await
                    .expect("Gone is sent once the verdict wait ends");
                assert_eq!(gone, FrontRequest::Gone { conn: 5 });
                let waited = start.elapsed();
                assert!(
                    waited.abs_diff(VERDICT_WAIT) <= slack,
                    "Gone comes VERDICT_WAIT after Authenticate, not {waited:?}"
                );
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
