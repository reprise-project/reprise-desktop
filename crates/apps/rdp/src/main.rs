//! reprise-rdp: the session's Remote Desktop server, one per user session. `--hand-over` serves
//! the connections reprise-login hands over on the socket systemd passes.

#[cfg(test)]
reprisebus::test_session!();

mod auth;
mod client;
mod clipboard;
mod console;
mod control;
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
use repriseconf::ConfigWatch;
use repriseconf::remote::{EncoderChoice, RemoteDesktop};
use reprisetls::{cert, tcp};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, mpsc, oneshot, watch};

#[cfg(test)]
use display::Grey;

fn main() -> ExitCode {
    reprisebus::journal::init_logger("info,rustls=error");
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
    if hand_over.is_none() && login_manager_runs() {
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
    // Under reprise-login the machine's settings run remote logins, so no bus name is claimed.
    if hand_over.is_none() {
        control::spawn(status_rx, disconnect, regenerate);
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
    let code = local.block_on(&rt, async {
        lock::lock_if_marked(marker.as_deref(), lock::lock()).await;
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
            portal_wait: match hand_over {
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
        let viewers = status.subscribe();
        let session_user = user.clone();
        let idle = Rc::new(Notify::new());
        let leave = Leaving {
            inner: lock::LockOnLeave::new(marker, config_rx.clone()),
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
        };
        let sync_owner = input::SyncOwner::default();
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
                Box::new(clipboard::Factory::new(on_shown, cut));
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
        match hand_over {
            Some(listener) => {
                let listener = match tokio::net::UnixListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(e) => {
                        log::error!("hand-over socket: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                let intake = handover::HandOverIntake::new(listener, ROOT, viewers);
                let logins = auth::HandedOverLogins::new(session_user);
                let stop = async {
                    tokio::select! {
                        () = terminated() => {}
                        () = display_gone(display) => {}
                        () = idle.notified() => {
                            log::info!("no client is connected; stopping until the next hand-over");
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
    });
    // Dropping the connections gives the shared sink's default back before the process ends.
    drop(local);
    code
}

/// The only peer the hand-over socket takes: reprise-login runs as root.
const ROOT: u32 = 0;

/// How long the system bus may take to answer whether reprise-login runs.
const BUS_ANSWER: Duration = Duration::from_secs(5);

/// Whether reprise-login runs, asked once on the system bus.
fn login_manager_runs() -> bool {
    zbus::block_on(async {
        let system = zbus::connection::Builder::system()
            .method_timeout(BUS_ANSWER)
            .build();
        match system.await {
            Ok(system) => reprisebus::login::manager_active(&system).await,
            Err(e) => {
                log::debug!("no system bus: {e}");
                false
            }
        }
    })
}

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
                    let connected = wayland::connect();
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
