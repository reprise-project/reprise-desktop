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

use repriselogin::front::{FrontEvent, FrontRequest, Secret};
use repriselogin::machine::{MACHINE_CONFIG, MachineConfig};
use reprisetls::{cert, tcp};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use accept::Ctx;
use limits::{Limits, MAX_PREAUTH, PREAUTH_PER_IP, Permit};

const RECONNECT: Duration = Duration::from_secs(1);
/// A failing accept (out of fds) is retried after this, not at once.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// A connection between its accept and its verdict.
struct Conn {
    abort: AbortHandle,
    verdict: Option<oneshot::Sender<FrontEvent>>,
}

type Conns = Rc<RefCell<HashMap<u64, Conn>>>;

fn main() -> ExitCode {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
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
    // The unit allows 64 tasks; the front runs on one thread and blocks on nothing.
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
        let task = tokio::task::spawn_local(connection(
            Arrival {
                tcp,
                peer,
                conn,
                accepted,
                _permit: permit,
            },
            ctx.clone(),
            conns.clone(),
            limits.clone(),
        ));
        let abort = task.abort_handle();
        conns.borrow_mut().insert(
            conn,
            Conn {
                abort,
                verdict: None,
            },
        );
    }
}

struct Arrival {
    tcp: TcpStream,
    peer: SocketAddr,
    conn: u64,
    accepted: Instant,
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
        _permit,
    } = arrival;
    let _forget = Forget(conns.clone(), conn);
    let pending = match accept::to_client_info(tcp, conn, &ctx).await {
        Ok(pending) => pending,
        Err(refused) => {
            log::info!("{peer}: refused: {refused}");
            return;
        }
    };
    let (tell, verdict) = oneshot::channel();
    if let Some(c) = conns.borrow_mut().get_mut(&conn) {
        c.verdict = Some(tell);
    }
    let request = FrontRequest::Authenticate {
        conn,
        peer,
        user: pending.user.clone(),
        secret: Secret::Password(pending.password.clone()),
        elapsed_ms: u32::try_from(accepted.elapsed().as_millis()).unwrap_or(u32::MAX),
    };
    let sent = match &*ctx.link.borrow() {
        Some(link) => link.send(&request, None),
        None => Err(io::ErrorKind::NotConnected.into()),
    };
    drop(request);
    if let Err(e) = sent {
        log::warn!("{peer}: reprise-login: {e}");
        return;
    }
    match verdict.await {
        Ok(FrontEvent::Denied { backoff_s, .. }) => {
            log::info!("{peer}: the logon was refused");
            limits.borrow_mut().back_off(
                peer.ip(),
                Duration::from_secs(backoff_s.into()),
                Instant::now(),
            );
            accept::deny(pending).await;
        }
        Ok(FrontEvent::Accepted { .. }) => log::info!("{peer}: logged on"),
        Ok(_) | Err(_) => {}
    }
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
                    } else if let Some(conn) = accept::verdict_conn(&event) {
                        let tell = conns
                            .borrow_mut()
                            .get_mut(&conn)
                            .and_then(|c| c.verdict.take());
                        if let Some(tell) = tell {
                            let _ = tell.send(event);
                        }
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
