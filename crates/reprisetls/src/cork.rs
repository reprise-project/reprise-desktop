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
}

impl<S> Cork<S> {
    pub fn new(inner: S) -> Cork<S> {
        Cork {
            inner,
            header: [0; HEADER],
            header_len: 0,
            body_left: 0,
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
                if len > MAX_RECORD {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("a TLS record of {len} bytes, over the {MAX_RECORD} allowed"),
                    )));
                }
                this.header_len = 0;
                this.body_left = len;
            }
        } else {
            this.body_left -= got;
        }
        buf.advance(got);
        Poll::Ready(Ok(()))
    }
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
}
