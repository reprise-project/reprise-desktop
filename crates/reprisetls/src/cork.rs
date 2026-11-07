//! A reader that never crosses a TLS record boundary.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const HEADER: usize = 5;
/// The longest TLS 1.2 ciphertext, which also bounds TLS 1.3's.
const MAX_RECORD: usize = 16384 + 2048;

/// Reads stop at the end of each TLS record, so rustls never holds ciphertext of a record it
/// has not processed when its keys move into the kernel.
pub struct Cork<S> {
    inner: S,
    header: [u8; HEADER],
    header_len: usize,
    body_left: usize,
    /// The length of an over-long record, which every later read reports again.
    too_long: Option<usize>,
}

impl<S> Cork<S> {
    pub fn new(inner: S) -> Cork<S> {
        Cork {
            inner,
            header: [0; HEADER],
            header_len: 0,
            body_left: 0,
            too_long: None,
        }
    }

    /// Nothing of a partial record has been read.
    pub fn at_boundary(&self) -> bool {
        self.header_len == 0 && self.body_left == 0
    }

    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Cork<S> {
    /// Reads at most the rest of the current record's header or body.
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(len) = this.too_long {
            return Poll::Ready(Err(too_long(len)));
        }
        let in_header = this.body_left == 0;
        let want = if in_header {
            HEADER - this.header_len
        } else {
            this.body_left
        };
        let dst = buf.initialize_unfilled_to(want.min(buf.remaining()));
        let mut part = ReadBuf::new(dst);
        ready!(Pin::new(&mut this.inner).poll_read(cx, &mut part))?;
        let got = part.filled().len();
        if in_header {
            let end = this.header_len + got;
            this.header[this.header_len..end].copy_from_slice(part.filled());
            this.header_len = end;
            if end == HEADER {
                let len = usize::from(u16::from_be_bytes([this.header[3], this.header[4]]));
                this.header_len = 0;
                if len > MAX_RECORD {
                    this.too_long = Some(len);
                    return Poll::Ready(Err(too_long(len)));
                }
                this.body_left = len;
            }
        } else {
            this.body_left -= got;
        }
        buf.advance(got);
        Poll::Ready(Ok(()))
    }
}

fn too_long(len: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("a TLS record of {len} bytes, over the {MAX_RECORD} allowed"),
    )
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Cork<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// Hands out one byte per read and, like a socket with nothing to read, stays pending once
    /// it runs out.
    struct Trickle<'a>(&'a [u8]);

    impl AsyncRead for Trickle<'_> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let Some((&b, rest)) = self.0.split_first() else {
                return Poll::Pending;
            };
            if buf.remaining() > 0 {
                buf.put_slice(&[b]);
                self.0 = rest;
            }
            Poll::Ready(Ok(()))
        }
    }

    fn poll_once<S: AsyncRead + Unpin>(cork: &mut Cork<S>) -> Poll<io::Result<usize>> {
        let mut buf = [0; 64];
        let mut buf = ReadBuf::new(&mut buf);
        let cx = &mut Context::from_waker(std::task::Waker::noop());
        Pin::new(cork)
            .poll_read(cx, &mut buf)
            .map_ok(|()| buf.filled().len())
    }

    fn record(len: u16) -> Vec<u8> {
        let mut r = vec![23, 3, 3];
        r.extend(len.to_be_bytes());
        r.extend(std::iter::repeat_n(len as u8, len.into()));
        r
    }

    #[tokio::test]
    async fn cork_never_reads_past_a_record_boundary() {
        let lens = [10, 3, 20];
        let sent: Vec<u8> = lens.iter().flat_map(|&len| record(len)).collect();
        let mut cork = Cork::new(&sent[..]);
        let mut buf = [0; 4096];
        let mut read = Vec::new();
        assert!(cork.at_boundary(), "at a boundary before the first read");
        for len in lens {
            let end = 5 + usize::from(len);
            let mut got = 0;
            while got < end {
                let n = cork.read(&mut buf).await.unwrap();
                assert!(n > 0, "a record of {end} bytes ended after {got}");
                got += n;
                assert!(got <= end, "{got} bytes read from a record of {end}");
                assert_eq!(cork.at_boundary(), got == end, "after {got} of {end} bytes");
                read.extend_from_slice(&buf[..n]);
            }
        }
        assert_eq!(cork.read(&mut buf).await.unwrap(), 0);
        assert!(cork.at_boundary(), "at a boundary at the end");
        assert_eq!(read, sent);
    }

    #[tokio::test]
    async fn cork_rejects_a_record_longer_than_tls_allows() {
        let mut sent = vec![23, 3, 3];
        sent.extend(18433u16.to_be_bytes());
        sent.resize(5 + 18433, 0);
        let mut cork = Cork::new(&sent[..]);
        let err = cork.read(&mut [0; 4096]).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn cork_keeps_failing_after_an_over_long_record() {
        let mut cork = Cork::new(Trickle(&[23, 3, 3, 0x48, 0x01]));
        for _ in 0..4 {
            assert!(matches!(poll_once(&mut cork), Poll::Ready(Ok(1))));
        }
        for read in ["the header's last byte", "the next read"] {
            match poll_once(&mut cork) {
                Poll::Ready(Err(e)) => assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{read}"),
                other => panic!("{read}: {other:?}, not InvalidData"),
            }
        }
    }

    #[tokio::test]
    async fn cork_takes_a_record_of_the_longest_length_tls_allows() {
        let mut sent = vec![23, 3, 3];
        sent.extend(18432u16.to_be_bytes());
        sent.resize(5 + 18432, 0);
        let mut cork = Cork::new(&sent[..]);
        let mut read = Vec::new();
        cork.read_to_end(&mut read).await.unwrap();
        assert_eq!(read.len(), sent.len());
        assert!(cork.at_boundary());
    }

    #[tokio::test]
    async fn cork_joins_a_header_split_across_reads() {
        let sent: Vec<u8> = [4, 2].iter().flat_map(|&len| record(len)).collect();
        let mut cork = Cork::new(Trickle(&sent));
        let mut buf = [0; 4096];
        let mut read = Vec::new();
        for len in [4, 2] {
            for got in 1..=5 + len {
                let n = cork.read(&mut buf).await.unwrap();
                assert_eq!(n, 1);
                read.push(buf[0]);
                assert_eq!(cork.at_boundary(), got == 5 + len, "after {got} bytes");
            }
        }
        assert_eq!(read, sent);
    }
}
