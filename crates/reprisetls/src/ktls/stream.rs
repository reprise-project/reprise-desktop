//! A tokio stream over a socket whose TLS records the kernel encrypts and decrypts.

use std::io;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use repriselogin::handover::TlsState;
use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf};
use tokio::net::TcpStream;

use super::{Alert, alert};

const ALERT: u8 = 21;
const HANDSHAKE: u8 = 22;
const DATA: u8 = 23;
const CLOSE_NOTIFY: [u8; 2] = [1, 0];

/// Reads give the peer's data, end at its close_notify and fail on any other alert;
/// shutting down sends a close_notify.
pub struct KtlsStream {
    stream: TcpStream,
    #[expect(dead_code, reason = "key updates and the record limit read it")]
    tls: TlsState,
    eof: bool,
    close_notify_sent: bool,
}

impl KtlsStream {
    /// `stream` already carries `tls` in the kernel ([`super::install`]).
    pub fn new(stream: TcpStream, tls: TlsState) -> KtlsStream {
        KtlsStream {
            stream,
            tls,
            eof: false,
            close_notify_sent: false,
        }
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }
}

impl AsyncRead for KtlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.eof || buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let fd = this.stream.as_raw_fd();
        loop {
            ready!(this.stream.poll_read_ready(cx))?;
            let dst = buf.initialize_unfilled();
            let (n, record_type) = match this
                .stream
                .try_io(Interest::READABLE, || recv_record(fd, dst))
            {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                received => received?,
            };
            return Poll::Ready(match record_type {
                DATA => {
                    buf.advance(n);
                    Ok(())
                }
                ALERT => match alert(&buf.initialize_unfilled()[..n]) {
                    Alert::CloseNotify => {
                        this.eof = true;
                        Ok(())
                    }
                    Alert::Other { level, description } => Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        format!("TLS alert, level {level}, description {description}"),
                    )),
                },
                HANDSHAKE => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TLS handshake message",
                )),
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
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let fd = this.stream.as_raw_fd();
        while !this.close_notify_sent {
            ready!(this.stream.poll_write_ready(cx))?;
            match this
                .stream
                .try_io(Interest::WRITABLE, || send_record(fd, ALERT, &CLOSE_NOTIFY))
            {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                sent => {
                    sent?;
                    this.close_notify_sent = true;
                }
            }
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
