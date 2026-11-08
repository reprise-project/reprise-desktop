//! The listening socket and the options of an accepted link.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

const KEEPALIVE_IDLE: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const KEEPALIVE_PROBES: u32 = 3;
/// Keepalive probes wait for unacknowledged data; this ends a link that dies mid-send.
const USER_TIMEOUT: Duration = Duration::from_secs(30);

/// All addresses: IPv4 included through the IPv6 socket; [`bind`] takes IPv4 alone where
/// IPv6 is off.
pub const ALL: IpAddr = IpAddr::V6(Ipv6Addr::UNSPECIFIED);

/// A listener on `addr`; an IPv6 one takes IPv4 too, whatever `net.ipv6.bindv6only` says.
/// [`ALL`] on a host without IPv6 listens on IPv4 alone. A port another socket holds is an
/// error, so a user holding `[::]` IPv6-only never gets this server's IPv6 clients.
pub fn bind(addr: SocketAddr) -> io::Result<TcpListener> {
    bind_with(addr, bind_one)
}

fn bind_with(
    addr: SocketAddr,
    one: impl Fn(SocketAddr) -> io::Result<TcpListener>,
) -> io::Result<TcpListener> {
    match one(addr) {
        Err(e) if addr.ip() == ALL && no_ipv6(&e) => {
            one(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), addr.port()))
        }
        bound => bound,
    }
}

fn no_ipv6(e: &io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::EAFNOSUPPORT | libc::EADDRNOTAVAIL | libc::EPROTONOSUPPORT)
    )
}

fn bind_one(addr: SocketAddr) -> io::Result<TcpListener> {
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
pub fn tune(stream: &TcpStream) -> io::Result<()> {
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
    async fn all_addresses_fall_back_to_ipv4_where_ipv6_cannot_bind() {
        let tried = std::cell::RefCell::new(Vec::new());
        let no_ipv6 = |addr: SocketAddr| {
            tried.borrow_mut().push(addr);
            match addr.is_ipv6() {
                true => Err(io::Error::from_raw_os_error(libc::EAFNOSUPPORT)),
                false => bind_one(addr),
            }
        };
        let listener = bind_with(SocketAddr::new(ALL, 0), no_ipv6).unwrap();
        let addr = listener.local_addr().unwrap();
        assert_eq!(addr.ip(), IpAddr::from([0, 0, 0, 0]));
        assert_eq!(
            *tried.borrow(),
            [SocketAddr::new(ALL, 0), SocketAddr::from(([0, 0, 0, 0], 0))]
        );

        let one = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 0));
        tried.borrow_mut().clear();
        assert!(bind_with(one, no_ipv6).is_err());
        assert_eq!(*tried.borrow(), [one], "only all addresses fall back");
    }

    #[tokio::test]
    async fn all_addresses_held_by_another_socket_is_an_error_and_not_ipv4_alone() {
        let tried = std::cell::RefCell::new(Vec::new());
        let held = |addr: SocketAddr| {
            tried.borrow_mut().push(addr);
            match addr.is_ipv6() {
                true => Err(io::Error::from_raw_os_error(libc::EADDRINUSE)),
                false => bind_one(addr),
            }
        };
        let e = bind_with(SocketAddr::new(ALL, 0), held).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(*tried.borrow(), [SocketAddr::new(ALL, 0)]);
    }

    #[tokio::test]
    async fn a_port_held_ipv6_only_is_refused_to_all_addresses() {
        use socket2::{Domain, Protocol, Socket, Type};
        let squatter = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP)).unwrap();
        squatter.set_only_v6(true).unwrap();
        squatter.set_reuse_address(true).unwrap();
        let any = SocketAddr::new(ALL, 0);
        if squatter.bind(&any.into()).is_err() {
            return;
        }
        squatter.listen(1).unwrap();
        let port = squatter.local_addr().unwrap().as_socket().unwrap().port();
        let e = bind(SocketAddr::new(ALL, port)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
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
