//! Where connections come from: [`TcpIntake`] listens on the configured port, follows it and
//! reports where it listens.

use crate::control::Status;
use repriseconf::remote::RemoteDesktop;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

const KEEPALIVE_IDLE: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const KEEPALIVE_PROBES: u32 = 3;
/// Keepalive probes wait for unacknowledged data; this ends a link that dies mid-send.
const USER_TIMEOUT: Duration = Duration::from_secs(30);

/// All addresses: IPv4 included through the IPv6 socket, IPv4 alone where IPv6 is off.
pub const ALL: IpAddr = IpAddr::V6(Ipv6Addr::UNSPECIFIED);

/// What an [`Intake`] yields.
pub enum Arrival {
    /// A new connection and its peer, with an IPv4-mapped address given as IPv4.
    Connection {
        stream: TcpStream,
        peer: SocketAddr,
        admission: Admission,
    },
    /// The listening address changed; every connection is ended.
    Moved,
}

/// What a connection must do to be let in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Log in with the session user's password.
    Password,
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
                        if let Err(e) = tune(&stream) {
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
    let bound = match bind(SocketAddr::new(ip, port)) {
        Err(_) if ip == ALL => bind(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port)),
        bound => bound,
    };
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

/// A listener on `addr`; an IPv6 one takes IPv4 too, whatever `net.ipv6.bindv6only` says.
fn bind(addr: SocketAddr) -> io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    TcpListener::from_std(socket.into())
}

/// Keepalive and a user timeout end a dead link in about 30 s; no Nagle delay for small updates.
fn tune(stream: &TcpStream) -> io::Result<()> {
    let socket = socket2::SockRef::from(stream);
    socket.set_tcp_keepalive(
        &socket2::TcpKeepalive::new()
            .with_time(KEEPALIVE_IDLE)
            .with_interval(KEEPALIVE_INTERVAL)
            .with_retries(KEEPALIVE_PROBES),
    )?;
    socket.set_tcp_user_timeout(Some(USER_TIMEOUT))?;
    socket.set_tcp_nodelay(true)
}

async fn accept(listener: Option<&TcpListener>) -> io::Result<(TcpStream, SocketAddr)> {
    match listener {
        Some(l) => l.accept().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_accepted_link_gives_up_after_30_s_unacknowledged_and_sends_at_once() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        tune(&stream).unwrap();
        let socket = socket2::SockRef::from(&stream);
        assert_eq!(socket.tcp_user_timeout().unwrap(), Some(USER_TIMEOUT));
        assert!(socket.tcp_nodelay().unwrap());
        assert!(socket.keepalive().unwrap());
        assert_eq!(socket.tcp_keepalive_time().unwrap(), KEEPALIVE_IDLE);
    }

    #[tokio::test]
    async fn listening_on_all_addresses_takes_ipv4() {
        let listener = bind(SocketAddr::new(ALL, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        if addr.is_ipv6() {
            let only_v6 = socket2::SockRef::from(&listener).only_v6().unwrap();
            assert!(!only_v6, "IPV6_V6ONLY is off");
        }
        let v4 = SocketAddr::from(([127, 0, 0, 1], addr.port()));
        let (client, accepted) = tokio::join!(TcpStream::connect(v4), listener.accept());
        client.expect("an IPv4 client connects");
        let (_, peer) = accepted.unwrap();
        assert_eq!(peer.ip().to_canonical(), v4.ip());
    }
}
