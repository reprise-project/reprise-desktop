//! A tokio stream over a socket whose TLS records the kernel encrypts and decrypts.

use std::io;
use std::net::SocketAddr;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use repriselogin::handover::TlsState;
use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf};
use tokio::net::TcpStream;

use super::{Alert, Direction, RecordAction, alert, key_update, next_keys, record_action};

const ALERT: u8 = 21;
const HANDSHAKE: u8 = 22;
const DATA: u8 = 23;
const CLOSE_NOTIFY: [u8; 2] = [1, 0];
const KEY_UPDATE: [u8; 5] = [24, 0, 0, 1, 0];
/// Writes between looks at the send sequence. Each write is at most a socket buffer, a few
/// hundred records, so the records sent between looks stay far inside `REKEY_MARGIN`.
const CHECK_EVERY: u32 = 1024;
/// The least room a receive gets, so a record longer than a key update shows as longer.
const MIN_RECV: usize = KEY_UPDATE.len() + 1;

/// Reads give the peer's data, end at its close_notify and fail on any other alert; a TLS 1.3
/// key update re-keys them. Writes send a key update before the record limit on TLS 1.3 and
/// close on TLS 1.2. Shutting down sends a close_notify.
pub struct KtlsStream {
    stream: TcpStream,
    tls: TlsState,
    eof: bool,
    close_notify_sent: bool,
    /// The peer asked for a key update, or the send sequence neared the limit: the next write
    /// sends one first.
    update_tx: bool,
    writes: u32,
    /// Data received for a read with less room than `MIN_RECV`, which later reads give first.
    stash: Vec<u8>,
}

impl KtlsStream {
    /// `stream` already carries `tls` in the kernel ([`super::install`]).
    pub fn new(stream: TcpStream, tls: TlsState) -> KtlsStream {
        KtlsStream {
            stream,
            tls,
            eof: false,
            close_notify_sent: false,
            update_tx: false,
            writes: 0,
            stash: Vec::new(),
        }
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    /// Sends `body` as one whole record of `record_type`.
    fn poll_send(
        &mut self,
        cx: &mut Context<'_>,
        record_type: u8,
        body: &[u8],
    ) -> Poll<io::Result<()>> {
        let fd = self.stream.as_raw_fd();
        loop {
            ready!(self.stream.poll_write_ready(cx))?;
            match self
                .stream
                .try_io(Interest::WRITABLE, || send_record(fd, record_type, body))
            {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Ok(n) if n == body.len() => return Poll::Ready(Ok(())),
                Ok(n) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        format!("a TLS record of type {record_type} cut to {n} bytes"),
                    )));
                }
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
    }

    /// Sends a key update under the old keys, then sends under the next ones.
    fn poll_update_tx(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_send(cx, HANDSHAKE, &KEY_UPDATE))?;
        self.update_tx = false;
        let keys = next_keys(self.tls.cipher, &self.tls.tx)?;
        let (version, cipher) = (self.tls.version, self.tls.cipher);
        super::set_keys(self.stream.as_fd(), Direction::Tx, version, cipher, &keys)?;
        self.tls.tx = keys;
        Poll::Ready(Ok(()))
    }

    /// Takes a key update the peer sent: the next receive keys, and its request for ours.
    fn update_rx(&mut self, record: &[u8]) -> io::Result<()> {
        let requested = key_update(self.tls.version, record)?;
        let keys = next_keys(self.tls.cipher, &self.tls.rx)?;
        let (version, cipher) = (self.tls.version, self.tls.cipher);
        super::set_keys(self.stream.as_fd(), Direction::Rx, version, cipher, &keys)?;
        self.tls.rx = keys;
        self.update_tx |= requested;
        Ok(())
    }
}

impl AsyncRead for KtlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.stash.is_empty() {
            let n = this.stash.len().min(buf.remaining());
            buf.put_slice(&this.stash[..n]);
            this.stash.drain(..n);
            return Poll::Ready(Ok(()));
        }
        if this.eof || buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let fd = this.stream.as_raw_fd();
        loop {
            ready!(this.stream.poll_read_ready(cx))?;
            let mut small = [0; MIN_RECV];
            let direct = buf.remaining() >= MIN_RECV;
            let dst = if direct {
                buf.initialize_unfilled()
            } else {
                &mut small[..]
            };
            let (n, record_type) = match this
                .stream
                .try_io(Interest::READABLE, || recv_record(fd, dst))
            {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                received => received?,
            };
            let got = if direct {
                &buf.initialize_unfilled()[..n]
            } else {
                &small[..n]
            };
            return Poll::Ready(match record_type {
                DATA if direct => {
                    buf.advance(n);
                    Ok(())
                }
                DATA => {
                    let fits = n.min(buf.remaining());
                    buf.put_slice(&small[..fits]);
                    this.stash.extend_from_slice(&small[fits..n]);
                    Ok(())
                }
                HANDSHAKE => {
                    this.update_rx(got)?;
                    continue;
                }
                ALERT => match alert(got) {
                    Alert::CloseNotify => {
                        this.eof = true;
                        Ok(())
                    }
                    Alert::Other { level, description } => Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        format!("TLS alert, level {level}, description {description}"),
                    )),
                },
                other => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("TLS record of type {other}"),
                )),
            });
        }
    }
}

impl AsyncWrite for KtlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.close_notify_sent {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "a write after close_notify",
            )));
        }
        if this.writes >= CHECK_EVERY {
            let seq = super::sent_seq(this.stream.as_fd(), this.tls.cipher)?;
            match record_action(this.tls.version, seq, this.tls.record_limit) {
                RecordAction::Continue => {}
                RecordAction::KeyUpdate => this.update_tx = true,
                RecordAction::Close => {
                    ready!(this.poll_send(cx, ALERT, &CLOSE_NOTIFY))?;
                    this.close_notify_sent = true;
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "the TLS record limit was reached",
                    )));
                }
            }
            this.writes = 0;
        }
        if this.update_tx {
            ready!(this.poll_update_tx(cx))?;
        }
        let n = ready!(Pin::new(&mut this.stream).poll_write(cx, buf))?;
        this.writes += 1;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.close_notify_sent {
            ready!(this.poll_send(cx, ALERT, &CLOSE_NOTIFY))?;
            this.close_notify_sent = true;
        }
        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}

/// Room for one control message carrying a one-byte record type, aligned for `cmsghdr`.
type Control = [u64; 4];

/// One `recvmsg`: data, or one whole non-data record, and its TLS content type.
fn recv_record(fd: RawFd, buf: &mut [u8]) -> io::Result<(usize, u8)> {
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let mut control: Control = [0; 4];
    // SAFETY: an all-zero `msghdr` is a valid empty one.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = size_of::<Control>() as _;
    // SAFETY: `msg` points at `iov` (over `buf`) and `control`, which outlive the call.
    let n = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_DONTWAIT) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut record_type = DATA;
    // SAFETY: the walk follows the control messages `recvmsg` wrote inside `control`.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_TLS && (*c).cmsg_type == libc::TLS_GET_RECORD_TYPE {
                record_type = *libc::CMSG_DATA(c);
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    Ok((n as usize, record_type))
}

/// Sends `data` as one record of TLS content type `record_type`.
fn send_record(fd: RawFd, record_type: u8, data: &[u8]) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: data.as_ptr().cast_mut().cast(),
        iov_len: data.len(),
    };
    let mut control: Control = [0; 4];
    // SAFETY: an all-zero `msghdr` is a valid empty one.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    // SAFETY: CMSG_SPACE only computes a length.
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(1) } as _;
    // SAFETY: `control` holds CMSG_SPACE(1) bytes, so the first header and its one data byte
    // lie inside it.
    unsafe {
        let c = libc::CMSG_FIRSTHDR(&msg);
        (*c).cmsg_level = libc::SOL_TLS;
        (*c).cmsg_type = libc::TLS_SET_RECORD_TYPE;
        (*c).cmsg_len = libc::CMSG_LEN(1) as _;
        *libc::CMSG_DATA(c) = record_type;
    }
    // SAFETY: `msg` points at `iov` (over `data`) and `control`, which outlive the call; the
    // kernel only reads `data`.
    let n = unsafe { libc::sendmsg(fd, &msg, libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriselogin::handover::{Cipher, DirectionKeys, TlsVersion};
    use tokio::io::AsyncWriteExt;
    use zeroize::Zeroizing;

    #[tokio::test]
    async fn a_write_after_close_notify_is_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let keys = || DirectionKeys {
            seq: 0,
            key: Zeroizing::new(vec![0; 16]),
            iv: Zeroizing::new(vec![0; 12]),
            secret: None,
        };
        let tls = TlsState {
            version: TlsVersion::Tls13,
            cipher: Cipher::Aes128Gcm,
            tx: keys(),
            rx: keys(),
            record_limit: 1 << 24,
        };
        let mut stream = KtlsStream::new(tcp, tls);
        stream.close_notify_sent = true;
        let err = stream.write(b"late").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
