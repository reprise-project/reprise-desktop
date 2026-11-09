//! reprise-rdp: the session's Remote Desktop server, one per user session. `--hand-over` serves
//! the connections reprise-login hands over on the socket systemd passes.

#[cfg(test)]
reprisebus::test_session!();

mod auth;
mod client;
mod clipboard;
mod console;
mod control;
mod desklock;
mod display;
mod encode;
mod geometry;
mod gfx;
mod handover;
mod input;
mod intake;
mod keymap;
mod layout;
mod listen;
mod lock;
mod portal;
mod seat;
mod sink;
mod sound;
mod stream;
mod viewers;
mod wayland;
mod wlclip;

use encode::choice;
use listen::{End, Leave};
use reprisebus::journal;
use repriseconf::ConfigWatch;
use repriseconf::remote::{EncoderChoice, RemoteDesktop};
use reprisetls::{cert, tcp};
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, mpsc, oneshot, watch};

#[cfg(test)]
use display::Grey;

/// The logger, with `default_filter` unless RUST_LOG is set. ironrdp and sspi stay at info even
/// under RUST_LOG=debug: ironrdp's debug records carry the mstshash cookie, a typed user name.
fn logger(default_filter: &str, prefixed: bool) -> journal::Builder {
    let mut logger = journal::builder(default_filter, prefixed);
    for target in ["ironrdp", "sspi"] {
        logger.filter_module(target, log::LevelFilter::Info);
    }
    logger
}

fn main() -> ExitCode {
    logger("info,rustls=error", journal::on_stream()).init();
    ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
    let Some(user) = std::env::var("USER").ok().filter(|u| !u.is_empty()) else {
        log::error!("USER is not set");
        return ExitCode::FAILURE;
    };
    let hand_over = match std::env::args().nth(1).as_deref() {
        Some("--hand-over") => match handover::activated() {
            Ok(listener) => Some(listener),
            Err(e) => {
                log::error!("{e}");
                return ExitCode::FAILURE;
            }
        },
        _ => None,
    };
    if hand_over.is_none() && reprisebus::login::manager_runs() {
        log::info!("reprise-login takes this computer's remote connections; this server stays off");
        return ExitCode::SUCCESS;
    }
    let cfg = repriseconf::remote::remote_desktop();
    let dir = cert_dir();
    let identity = match hand_over {
        Some(_) => None,
        None => match cert::load_or_create(&dir) {
            Ok(identity) => {
                log::info!("certificate fingerprint {}", identity.fingerprint);
                Some(identity)
            }
            Err(e) => {
                log::error!("{}: no certificate: {e}", dir.display());
                return ExitCode::FAILURE;
            }
        },
    };
    let seat_away = seat::away();
    sink::recover();
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("no runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (status, status_rx) = watch::channel(control::Status {
        listening: match hand_over {
            Some(_) => "login manager".into(),
            None => String::new(),
        },
        encoder: choice::PROBING_LABEL.into(),
        encoders: choice::names(&[]),
        fingerprint: (identity.as_ref()).map_or_else(String::new, |i| i.fingerprint.clone()),
        ..control::Status::default()
    });
    let (disconnect, disconnect_rx) = mpsc::unbounded_channel();
    let (regenerate, regenerate_rx) = mpsc::unbounded_channel();
    // The portal's lock mode reads the ConsoleLayout of the name's owner, and refuses a hand-over
    // server that does not own it.
    let server = match hand_over {
        Some(_) => control::Server::HandOver,
        None => control::Server::Own(regenerate),
    };
    let claim = control::spawn(status_rx, disconnect, server);
    let claimed = (claim.recv()).unwrap_or_else(|_| Err(zbus::Error::Failure("no thread".into())));
    if let Err(e) = claimed {
        match &hand_over {
            Some(listener) => {
                log::error!(
                    "hand-overs need {} on the session bus ({e}); refusing them until the next \
                     hand-over starts the server again",
                    control::NAME
                );
                handover::refuse_waiting(listener);
            }
            None => log::error!(
                "{} is not this server's on the session bus ({e}); Settings would show its \
                 holder's fingerprint and clients, so this server stays off",
                control::NAME
            ),
        }
        return ExitCode::from(NAME_HELD);
    }
    let identity = identity.map(watch::channel);
    let (config, config_rx) = watch::channel(cfg);
    let (display, probed) = match hand_over {
        Some(_) => {
            let (connected, display) = oneshot::channel();
            (Some(display), spawn_probe(Some(connected)))
        }
        None => (None, spawn_probe(None)),
    };
    let marker = lock::marker_path();
    let local = tokio::task::LocalSet::new();
    run_to_end(&rt, local, async {
        // Up and taking hand-overs during the lock wait: reprise-login's login waits on the
        // intake's first report and on its Took. Serving starts after the lock's first try.
        let mut intake = match hand_over.map(tokio::net::UnixListener::from_std) {
            Some(Ok(listener)) => Some(handover::HandOverIntake::new(
                listener,
                ROOT,
                status.subscribe(),
            )),
            Some(Err(e)) => {
                log::error!("hand-over socket: {e}");
                return ExitCode::FAILURE;
            }
            None => None,
        };
        let lock = lock::LockOnLeave::new(marker, config_rx.clone());
        let lock_wait = lock.lock_if_marked(true);
        match &mut intake {
            Some(intake) => intake.take_during(lock_wait).await,
            None => lock_wait.await,
        }
        let _watch = follow_config(config, status.clone(), probed.clone());
        tokio::task::spawn_local(show_encoders(
            probed.clone(),
            config_rx.clone(),
            status.clone(),
        ));
        let identity_rx = identity.map(|(identity, identity_rx)| {
            tokio::task::spawn_local(regenerate_on_request(
                regenerate_rx,
                dir,
                identity,
                status.clone(),
            ));
            identity_rx
        });
        let last_told = Arc::new(Mutex::new(display::UNSHARED));
        let links = display::Links {
            config: config_rx.clone(),
            status: status.clone(),
            lives: Arc::default(),
            portal_wait: match intake {
                Some(_) => wayland::SESSION_READY_WAIT,
                None => Duration::ZERO,
            },
        };
        let console = console::Console::new(
            status.clone(),
            config_rx.clone(),
            repriseconf::locker_set,
            seat_away,
        );
        let tcp_status = status.clone();
        let listed = status.subscribe();
        let session_user = user.clone();
        let idle = Rc::new(Notify::new());
        let owed = lock.owed();
        let viewers = Rc::<RefCell<viewers::Viewers>>::default();
        let leave = Leaving {
            inner: lock,
            status: status.subscribe(),
            idle: idle.clone(),
        };
        let server = listen::Server {
            identity: identity_rx,
            user,
            status,
            config: config_rx.clone(),
            disconnect: disconnect_rx,
            leave,
            console: console.clone(),
            viewers: viewers.clone(),
        };
        let sync_owner = input::SyncOwner::default();
        let desktop_lock = desklock::follow();
        let handlers = |shown: listen::Shown, ending: listen::Ending| {
            let on_shown = shown.subscribe();
            let cut = ending.clipboard();
            let (input, feed) = input::Input::new(sync_owner.clone());
            let h264 = probed
                .borrow()
                .as_ref()
                .is_none_or(|p| !p.working.is_empty());
            let gfx = h264.then(gfx::Gfx::default);
            let factory = gfx.as_ref().map(gfx::Gfx::factory);
            let layout = client::ClientLayout::default();
            let connection = display::Connection {
                shown,
                ending,
                client: layout.clone(),
                console: console.clone(),
            };
            let display = display::Display::new(
                last_told.clone(),
                feed,
                probed.clone(),
                gfx,
                links.clone(),
                connection,
            );
            let clipboard: Box<dyn ironrdp_server::CliprdrServerFactory> =
                Box::new(clipboard::Factory::new(on_shown, desktop_lock.clone(), cut));
            let sound: Box<dyn ironrdp_server::SoundServerFactory> =
                Box::new(sound::Factory::new(config_rx.clone()));
            (
                display,
                input,
                factory,
                Some(clipboard),
                Some(sound),
                layout,
            )
        };
        match intake {
            Some(intake) => {
                let logins = auth::HandedOverLogins::new(session_user);
                let stop = async {
                    tokio::select! {
                        () = terminated() => {}
                        () = display_gone(display) => {}
                        () = settled(|| idle.notified(), &owed, &viewers) => {
                            log::info!("no client is connected; stopping until the next hand-over");
                        }
                        () = settled(|| unused(listed.clone()), &owed, &viewers) => {
                            log::info!(
                                "no client connected in {} s; stopping until the next hand-over",
                                QUIET.as_secs()
                            );
                        }
                    }
                };
                listen::serve(server, intake, logins, handlers, stop).await;
            }
            None => {
                let intake = intake::TcpIntake::new(tcp::ALL, config_rx.clone(), tcp_status);
                let logins = auth::PasswordLogins::new(session_user);
                listen::serve(server, intake, logins, handlers, terminated()).await;
            }
        }
        ExitCode::SUCCESS
    })
}

/// Runs `serve` on `local` to its exit code. The connections its stop dropped on the runtime give
/// the shared sink's default back on threads of their own; the exit waits for that.
fn run_to_end(
    rt: &tokio::runtime::Runtime,
    local: tokio::task::LocalSet,
    serve: impl Future<Output = ExitCode>,
) -> ExitCode {
    let code = local.block_on(rt, serve);
    drop(local);
    sink::settle();
    code
}

/// The exit when another process holds the bus name: the units' `RestartPreventExitStatus=` keeps
/// systemd from restarting the server into the start limit, which would fail the hand-over socket.
const NAME_HELD: u8 = 75;

/// The only peer the hand-over socket takes: reprise-login runs as root.
const ROOT: u32 = 0;

/// The session's [`listen::Leave`]: [`lock::LockOnLeave`], then a notice on `idle` once a
/// client has left and none is listed. Hand-over mode stops on it; the socket starts it again.
struct Leaving<L> {
    inner: L,
    status: watch::Receiver<control::Status>,
    idle: Rc<Notify>,
}

impl<L: Leave> Leave for Leaving<L> {
    fn arrived(&self) {
        self.inner.arrived();
    }

    async fn left(&self, why: End) {
        self.inner.left(why).await;
        if self.status.borrow().clients.is_empty() {
            self.idle.notify_one();
        }
    }

    fn stopping(&self) -> impl Future<Output = ()> {
        self.inner.stopping()
    }

    fn console_unlocked(&self) -> impl Future<Output = bool> {
        self.inner.console_unlocked()
    }
}

/// `quiet`, then once no relock is owed, if no connection holds a place, one still logging in
/// included; else `quiet` again. An idle stop so never drops a lock logind refused, nor a login
/// handed over meanwhile.
async fn settled<Q: Future<Output = ()>>(
    quiet: impl Fn() -> Q,
    owed: &lock::Owed,
    viewers: &RefCell<viewers::Viewers>,
) {
    loop {
        quiet().await;
        owed.settled().await;
        if viewers.borrow().is_empty() {
            return;
        }
    }
}

/// How long a hand-over server waits with no client listed before it stops.
const QUIET: Duration = Duration::from_secs(60);

/// Ends once no client has been listed for [`QUIET`].
async fn unused(mut status: watch::Receiver<control::Status>) {
    loop {
        if status.wait_for(|s| s.clients.is_empty()).await.is_err() {
            return std::future::pending().await;
        }
        let listed = status.wait_for(|s| !s.clients.is_empty());
        if tokio::time::timeout(QUIET, listed).await.is_err() {
            return;
        }
    }
}

/// Ends once the session's Wayland display closes, or once it is not up in time.
async fn display_gone(
    display: Option<oneshot::Receiver<Result<wayland_client::Connection, String>>>,
) {
    let Some(display) = display else {
        return std::future::pending().await;
    };
    match display.await {
        Ok(Ok(conn)) => {
            wayland::closed(conn).await;
            log::info!("the session's display closed; stopping");
        }
        Ok(Err(e)) => log::error!("{e}; stopping"),
        Err(_) => log::error!("the probe thread ended without the display; stopping"),
    }
}

async fn terminated() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            term.recv().await;
        }
        Err(e) => {
            log::warn!("SIGTERM is not handled: {e}");
            std::future::pending().await
        }
    }
}

/// Keeps `config` at the file's `[remote_desktop]` while the watch lives; every write is passed on,
/// so an unbound port is retried on the next. A new encoder choice shows in `status` at once.
fn follow_config(
    config: watch::Sender<RemoteDesktop>,
    status: watch::Sender<control::Status>,
    probed: display::Probed,
) -> Option<ConfigWatch> {
    let (wake, mut woken) = mpsc::unbounded_channel();
    let Some(watch) = repriseconf::watch(move || {
        let _ = wake.send(());
    }) else {
        log::warn!("live config changes are off");
        return None;
    };
    tokio::task::spawn_local(async move {
        while woken.recv().await.is_some() {
            let new = repriseconf::remote::reread_remote_desktop(*config.borrow());
            let old = config.send_replace(new);
            if (old.quality, old.frame_rate) != (new.quality, new.frame_rate) {
                log::info!(
                    "quality {:?} (QP {}), up to {} frames a second",
                    new.quality,
                    new.quality.qp(),
                    new.frame_rate
                );
            }
            if old.encoder != new.encoder
                && let Some(picture) = probed.borrow().clone()
            {
                let label = choice::configured(new.encoder, &picture.backends());
                log::info!("encoder {}: {label}", new.encoder.name());
                status.send_modify(|s| s.encoder = label);
            }
        }
    });
    Some(watch)
}

/// Answers the bus's RegenerateCertificate: new connections get the new certificate.
async fn regenerate_on_request(
    mut requests: mpsc::UnboundedReceiver<control::Regenerate>,
    dir: PathBuf,
    identity: watch::Sender<cert::Identity>,
    status: watch::Sender<control::Status>,
) {
    while let Some(reply) = requests.recv().await {
        let made = match cert::regenerate(&dir) {
            Ok(new) => {
                log::info!("new certificate fingerprint {}", new.fingerprint);
                status.send_modify(|s| s.fingerprint = new.fingerprint.clone());
                identity.send_replace(new);
                Ok(())
            }
            Err(e) => {
                log::error!("{}: no new certificate: {e}", dir.display());
                Err(format!("no new certificate: {e}"))
            }
        };
        let _ = reply.send(made);
    }
}

/// Probes the H.264 backends on a thread of their own, so the bus and the listener start at
/// once; the returned watch holds the find once it is in. Given `display`, the thread first
/// connects to the session's display, sends it there and probes only once it is up.
fn spawn_probe(
    display: Option<oneshot::Sender<Result<wayland_client::Connection, String>>>,
) -> display::Probed {
    let (tx, rx) = watch::channel(None);
    let spawned = std::thread::Builder::new()
        .name("reprise-rdp-probe".into())
        .spawn({
            let tx = tx.clone();
            move || {
                if let Some(display) = display {
                    let connected = wayland::connect_when_ready();
                    let up = connected.is_ok();
                    let _ = display.send(connected);
                    if !up {
                        tx.send_replace(Some(Arc::default()));
                        return;
                    }
                }
                tx.send_replace(Some(Arc::new(picture())));
            }
        });
    if let Err(e) = spawned {
        log::error!("no probe thread: {e}");
        tx.send_replace(Some(Arc::default()));
    }
    rx
}

/// The H.264 backends that work and the stream modifiers each maps.
fn picture() -> display::Picture {
    let (device, working) = encode::probe::probe();
    for (backend, modifiers) in &working {
        log::info!(
            "encoder {} works on {}, modifiers {modifiers:#x?}",
            backend.label(),
            device.display(),
        );
    }
    if working.is_empty() {
        log::info!("encoder: {}", choice::BITMAP_LABEL);
    }
    display::Picture { device, working }
}

/// Once the probe is in: `Encoders` lists what works and `Encoder` names what the config picks,
/// with one line when the chosen encoder is not among them.
async fn show_encoders(
    mut probed: display::Probed,
    config: watch::Receiver<RemoteDesktop>,
    status: watch::Sender<control::Status>,
) {
    let picture = display::probed(&mut probed).await;
    let backends = picture.backends();
    let cfg = *config.borrow();
    let label = choice::configured(cfg.encoder, &backends);
    let chosen = cfg.encoder;
    if !matches!(chosen, EncoderChoice::Auto | EncoderChoice::None)
        && !backends.iter().any(|b| b.choice() == chosen)
    {
        log::warn!(
            "encoder {} is unavailable here; using {label}",
            chosen.name()
        );
    }
    status.send_modify(|s| {
        s.encoder = label;
        s.encoders = choice::names(&backends);
    });
}

/// `$XDG_DATA_HOME/reprise/rdp`.
fn cert_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_default()
        .join("reprise/rdp")
}

#[cfg(test)]
mod tests {
    use log::Log as _;

    #[tokio::test(start_paused = true)]
    async fn a_hand_over_server_shown_no_client_stops_after_the_quiet_wait() {
        use std::time::Duration;
        let (status, status_rx) = tokio::sync::watch::channel(super::control::Status::default());
        let second = Duration::from_secs(1);
        let never_shown = tokio::task::spawn(super::unused(status_rx.clone()));
        (tokio::time::timeout(super::QUIET + second, never_shown).await)
            .expect("a server never shown a client stops")
            .unwrap();
        let mut unused = tokio::task::spawn(super::unused(status_rx));
        tokio::time::sleep(super::QUIET - second).await;
        assert!(!unused.is_finished(), "a server just started waits");
        status.send_modify(|s| {
            s.clients.push(super::control::Client {
                address: "192.0.2.1:50000".into(),
                user: "alice".into(),
                since: 0,
            })
        });
        tokio::time::sleep(super::QUIET + second).await;
        assert!(!unused.is_finished(), "a listed client keeps it serving");
        status.send_modify(|s| s.clients.clear());
        tokio::time::sleep(super::QUIET - second).await;
        assert!(
            !unused.is_finished(),
            "the wait starts again once the list empties"
        );
        (tokio::time::timeout(2 * second, &mut unused).await)
            .expect("no client listed for the quiet wait stops it")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_stop_waits_out_an_owed_lock_and_a_connection_meanwhile() {
        let viewers = std::rc::Rc::<std::cell::RefCell<super::viewers::Viewers>>::default();
        let owed = super::lock::Owed::default();
        let idle = std::rc::Rc::new(tokio::sync::Notify::new());
        let relock = owed.start();
        let local = tokio::task::LocalSet::new();
        let stop = local.spawn_local({
            let (idle, owed, viewers) = (idle.clone(), owed.clone(), viewers.clone());
            async move { super::settled(|| idle.notified(), &owed, &viewers).await }
        });
        local
            .run_until(async {
                idle.notify_one();
                tokio::task::yield_now().await;
                assert!(!stop.is_finished(), "a relock is owed");
                let peer = "192.0.2.1:50000".parse().unwrap();
                let (logging_in, _kicked) = viewers.borrow_mut().join(peer, 4).unwrap();
                drop(relock);
                tokio::task::yield_now().await;
                assert!(
                    !stop.is_finished(),
                    "a connection still logging in arrived meanwhile"
                );
                viewers.borrow_mut().leave(logging_in);
                idle.notify_one();
                stop.await.unwrap();
            })
            .await;
    }

    #[test]
    fn neither_unit_restarts_a_server_whose_name_is_held() {
        for unit in [
            "app-org.reprise.RemoteDesktop.service",
            "reprise-rdp-handover.service",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../data/systemd")
                .join(unit);
            let text = std::fs::read_to_string(path).unwrap();
            let prevented: Vec<u8> = (text.lines())
                .filter_map(|l| l.trim().strip_prefix("RestartPreventExitStatus="))
                .flat_map(str::split_whitespace)
                .filter_map(|code| code.parse().ok())
                .collect();
            assert!(
                prevented.contains(&super::NAME_HELD),
                "{unit}: {prevented:?}"
            );
        }
    }

    #[test]
    fn ironrdp_and_sspi_debug_records_stay_out_of_the_log() {
        let logger = super::logger("debug", false).build();
        let enabled = |target, level| {
            logger.enabled(&log::Metadata::builder().target(target).level(level).build())
        };
        assert!(!enabled("ironrdp_acceptor::connection", log::Level::Debug));
        assert!(!enabled("sspi::ntlm", log::Level::Debug));
        assert!(enabled("ironrdp_server::server", log::Level::Info));
    }
}
