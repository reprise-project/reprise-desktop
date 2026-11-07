//! Where connections come from: [`TcpIntake`] listens on the configured port, follows it and
//! reports where it listens; [`crate::handover::HandOverIntake`] takes them from reprise-login.

use crate::control::Status;
use repriseconf::remote::RemoteDesktop;
use repriselogin::handover::HandOver;
use reprisetls::tcp;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// What an [`Intake`] yields.
pub enum Arrival {
    /// A new connection and its peer, with an IPv4-mapped address given as IPv4.
    Connection {
        stream: TcpStream,
        peer: SocketAddr,
        admission: Admission,
    },
    /// A connection reprise-login authenticated, with the exchange it ran so far.
    HandedOver {
        tcp: std::net::TcpStream,
        handover: Box<HandOver>,
    },
    /// The listening address changed; every connection is ended.
    Moved,
}

/// What a connection must do to be let in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Log in with the session user's password.
    Password,
    /// Nothing more: reprise-login logged `user` in.
    HandedOver { user: String },
}

/// A source of connections.
pub trait Intake {
    fn next(&mut self) -> impl Future<Output = Arrival>;
}

/// Connections accepted on `ip` and the config's port; a change of the port rebinds.
pub struct TcpIntake {
    ip: IpAddr,
    port: u16,
    config: watch::Receiver<RemoteDesktop>,
    /// Its `listening` is kept here.
    status: watch::Sender<Status>,
    listener: Option<TcpListener>,
}

impl TcpIntake {
    /// Binds at once; a bind error is reported and retried on the next config write.
    pub fn new(
        ip: IpAddr,
        mut config: watch::Receiver<RemoteDesktop>,
        status: watch::Sender<Status>,
    ) -> TcpIntake {
        let port = config.borrow_and_update().port;
        let listener = listen_on(ip, port, &status);
        TcpIntake {
            ip,
            port,
            config,
            status,
            listener,
        }
    }
}

impl Intake for TcpIntake {
    async fn next(&mut self) -> Arrival {
        loop {
            tokio::select! {
                Ok(()) = self.config.changed() => {
                    let wanted = self.config.borrow_and_update().port;
                    if wanted == self.port && let Some(l) = &self.listener {
                        report(l, &self.status);
                        continue;
                    }
                    let Some(bound) = listen_on(self.ip, wanted, &self.status) else {
                        continue;
                    };
                    let moved = wanted != self.port;
                    self.port = wanted;
                    self.listener = Some(bound);
                    if moved {
                        return Arrival::Moved;
                    }
                }
                accepted = accept(self.listener.as_ref()) => match accepted {
                    Ok((stream, peer)) => {
                        let peer = SocketAddr::new(peer.ip().to_canonical(), peer.port());
                        if let Err(e) = tcp::tune(&stream) {
                            log::warn!("{peer}: TCP options: {e}");
                        }
                        let admission = Admission::Password;
                        return Arrival::Connection { stream, peer, admission };
                    }
                    Err(e) => {
                        log::warn!("accept: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
            }
        }
    }
}

/// Report where `listener` listens, as after a failed move it still does.
fn report(listener: &TcpListener, status: &watch::Sender<Status>) {
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    status.send_if_modified(|s| {
        let changed = s.listening != addr.to_string();
        s.listening = addr.to_string();
        changed
    });
}

/// Bind `ip`:`port` and report it, or the error, as `listening`.
fn listen_on(ip: IpAddr, port: u16, status: &watch::Sender<Status>) -> Option<TcpListener> {
    let bound = tcp::bind(SocketAddr::new(ip, port));
    let (listener, listening) = match bound.and_then(|l| Ok((l.local_addr()?, l))) {
        Ok((addr, l)) => {
            log::info!("listening on {addr}");
            (Some(l), addr.to_string())
        }
        Err(e) => {
            log::error!("cannot listen on port {port}: {e}");
            (None, format!("error: {e}"))
        }
    };
    status.send_modify(|s| s.listening = listening);
    listener
}

async fn accept(listener: Option<&TcpListener>) -> io::Result<(TcpStream, SocketAddr)> {
    match listener {
        Some(l) => l.accept().await,
        None => std::future::pending().await,
    }
}
