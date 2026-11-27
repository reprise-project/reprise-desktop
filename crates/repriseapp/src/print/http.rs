//! The HTTP under IPP: one request per connection to cupsd, over its local
//! socket when present (cupsd takes the peer's uid as credentials), else localhost:631.

use std::io::{Read, Write};
use std::net::ToSocketAddrs;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Server {
    Socket(PathBuf),
    Tcp(String),
}

const SOCKET: &str = "/run/cups/cups.sock";

impl Server {
    /// `CUPS_SERVER` (socket path or `host[:port]`), else cupsd's socket, else localhost.
    pub fn find() -> Server {
        Server::from_env(std::env::var("CUPS_SERVER").ok().as_deref())
    }

    fn from_env(var: Option<&str>) -> Server {
        match var.map(str::trim).filter(|s| !s.is_empty()) {
            Some(p) if p.starts_with('/') => Server::Socket(p.into()),
            Some(host) if host.contains(':') => Server::Tcp(host.into()),
            Some(host) => Server::Tcp(format!("{host}:631")),
            None if std::path::Path::new(SOCKET).exists() => Server::Socket(SOCKET.into()),
            None => Server::Tcp("localhost:631".into()),
        }
    }
}

/// What went wrong; the strings are the system's own words, for the log.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Unreachable(String),
    /// 401 or 403.
    Denied,
    /// The connection broke or timed out mid-request.
    Io(String),
    /// Any other status but 2xx.
    Status(u16),
    /// A reply that is not HTTP, or longer than [`MAX_REPLY`].
    BadReply(&'static str),
}

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

/// How long a read or write may stall: printers-app requests wait for cupsd's device
/// discovery, which takes its own `timeout` first, and a print job's upload for a busy cupsd.
pub const TIMEOUT: Duration = Duration::from_secs(60);
/// How long a host named in `CUPS_SERVER` may take to accept the connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest reply read; a PPD list is a few MB.
pub const MAX_REPLY: usize = 64 << 20;

fn connect(server: &Server, timeout: Duration) -> Result<Box<dyn Stream>, Error> {
    let unreachable = |e: std::io::Error| Error::Unreachable(e.to_string());
    Ok(match server {
        Server::Socket(p) => {
            let s = UnixStream::connect(p).map_err(unreachable)?;
            s.set_read_timeout(Some(timeout)).ok();
            s.set_write_timeout(Some(timeout)).ok();
            Box::new(s)
        }
        Server::Tcp(addr) => {
            let mut last = None;
            let mut stream = None;
            for a in addr.to_socket_addrs().map_err(unreachable)? {
                match std::net::TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(e) => last = Some(e),
                }
            }
            let s = stream.ok_or_else(|| {
                unreachable(last.unwrap_or_else(|| std::io::ErrorKind::NotFound.into()))
            })?;
            s.set_read_timeout(Some(timeout)).ok();
            s.set_write_timeout(Some(timeout)).ok();
            Box::new(s)
        }
    })
}

/// The request head. On the local socket every request carries `PeerCred`, which
/// cupsd checks against the peer uid and then treats as authenticated (admin operations need it).
fn head(method: &str, resource: &str, len: Option<usize>, server: &Server, user: &str) -> String {
    let mut h = format!("{method} {resource} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    if let Some(len) = len {
        h.push_str(&format!(
            "Content-Type: application/ipp\r\nContent-Length: {len}\r\n"
        ));
    }
    if matches!(server, Server::Socket(_)) && !user.is_empty() {
        h.push_str(&format!("Authorization: PeerCred {user}\r\n"));
    }
    h.push_str("\r\n");
    h
}

pub fn post(server: &Server, resource: &str, body: &[u8], user: &str) -> Result<Reply, Error> {
    post_parts(server, resource, &[body], user, TIMEOUT)
}

/// A POST whose body is `parts` one after the other, each written as it is, with `timeout` for
/// every read and write.
pub fn post_parts(
    server: &Server,
    resource: &str,
    parts: &[&[u8]],
    user: &str,
    timeout: Duration,
) -> Result<Reply, Error> {
    let mut s = connect(server, timeout)?;
    let len = parts.iter().map(|p| p.len()).sum();
    s.write_all(head("POST", resource, Some(len), server, user).as_bytes())
        .map_err(io)?;
    for p in parts {
        s.write_all(p).map_err(io)?;
    }
    s.flush().map_err(io)?;
    read_reply(&mut s)
}

fn io(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

pub fn get(server: &Server, resource: &str, user: &str) -> Result<Reply, Error> {
    let mut s = connect(server, TIMEOUT)?;
    s.write_all(head("GET", resource, None, server, user).as_bytes())
        .map_err(io)?;
    read_reply(&mut s)
}

/// Read one response. cupsd can leave the connection open after an error reply
/// despite `Connection: close`, so the body's length (or last chunk) says when it's complete.
fn read_reply(s: &mut dyn Stream) -> Result<Reply, Error> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    let mut progress = Progress::default();
    while !progress.whole(&raw) {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) if raw.len() + n > MAX_REPLY => return Err(Error::BadReply("too long")),
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io(e)),
        }
    }
    let reply = parse_reply(&raw)?;
    match reply.status {
        200..=299 => Ok(reply),
        401 | 403 => Err(Error::Denied),
        code => Err(Error::Status(code)),
    }
}

/// Split a whole response into status and body, undoing chunked coding; with neither length nor chunking the body runs to the end.
fn parse_reply(raw: &[u8]) -> Result<Reply, Error> {
    let bad = Error::BadReply;
    let Head {
        end,
        status,
        chunked,
        length,
    } = reply_head(raw).ok_or_else(|| bad("no header end"))?;
    let status = status.ok_or_else(|| bad("status line"))?;
    let rest = &raw[end..];
    let body = if chunked {
        dechunk(rest).ok_or_else(|| bad("chunks"))?
    } else if let Some(n) = length {
        rest.get(..n).ok_or_else(|| bad("short body"))?.to_vec()
    } else {
        rest.to_vec()
    };
    Ok(Reply { status, body })
}

struct Head {
    end: usize,
    status: Option<u16>,
    chunked: bool,
    length: Option<usize>,
}

fn reply_head(raw: &[u8]) -> Option<Head> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let text = String::from_utf8_lossy(&raw[..end]);
    let mut lines = text.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok());
    let mut chunked = false;
    let mut length = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let v = v.trim();
        if k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked") {
            chunked = true;
        } else if k.eq_ignore_ascii_case("content-length") {
            length = v.parse::<usize>().ok();
        }
    }
    Some(Head {
        end: end + 4,
        status,
        chunked,
        length,
    })
}

/// How far a reply being read has got, so each read scans only the bytes the last one did not.
#[derive(Default)]
struct Progress {
    head: Option<Head>,
    /// Bytes searched for the head's end so far.
    scanned: usize,
    /// Where the next chunk's size line starts.
    chunk: usize,
}

impl Progress {
    /// Whether `raw`, the reply read so far (only ever grown), is whole: the head, and a body of
    /// the stated length or up to the last chunk (else to stream end). A bad chunk size ends it
    /// too, for `parse_reply` to refuse.
    fn whole(&mut self, raw: &[u8]) -> bool {
        if self.head.is_none() {
            let from = self.scanned.saturating_sub(3);
            self.scanned = raw.len();
            if !raw[from..].windows(4).any(|w| w == b"\r\n\r\n") {
                return false;
            }
            self.head = reply_head(raw);
            self.chunk = self.head.as_ref().map_or(0, |h| h.end);
        }
        match &self.head {
            Some(h) if h.chunked => self.last_chunk_in(raw),
            Some(h) => h.length.is_some_and(|n| raw.len() >= h.end + n),
            None => false,
        }
    }

    fn last_chunk_in(&mut self, raw: &[u8]) -> bool {
        loop {
            let b = &raw[self.chunk..];
            let Some(eol) = b.windows(2).position(|w| w == b"\r\n") else {
                return false;
            };
            let Some(size) = chunk_size(&b[..eol]) else {
                return true;
            };
            if size == 0 {
                return true;
            }
            // A size past the end of memory is a reply `parse_reply` refuses.
            let Some(next) = size.checked_add(eol + 4) else {
                return true;
            };
            if b.len() < next {
                return false;
            }
            self.chunk += next;
        }
    }
}

/// A chunk's size line, its extensions dropped.
fn chunk_size(line: &[u8]) -> Option<usize> {
    let s = std::str::from_utf8(line).ok()?;
    usize::from_str_radix(s.split(';').next()?.trim(), 16).ok()
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let size = chunk_size(&b[..eol])?;
        b = &b[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size.checked_add(2)?..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_comes_from_cups_server_first() {
        assert_eq!(
            Server::from_env(Some("/tmp/c.sock")),
            Server::Socket("/tmp/c.sock".into())
        );
        assert_eq!(
            Server::from_env(Some("print.lan")),
            Server::Tcp("print.lan:631".into())
        );
        assert_eq!(
            Server::from_env(Some("print.lan:8631")),
            Server::Tcp("print.lan:8631".into())
        );
    }

    #[test]
    fn peer_credentials_go_only_over_the_socket() {
        let sock = Server::Socket("/s".into());
        let h = head("POST", "/admin/", Some(3), &sock, "ann");
        assert!(h.starts_with("POST /admin/ HTTP/1.1\r\n"));
        assert!(h.contains("Content-Length: 3\r\n"));
        assert!(h.contains("Authorization: PeerCred ann\r\n"));
        assert!(h.ends_with("\r\n\r\n"));
        let tcp = Server::Tcp("localhost:631".into());
        assert!(!head("POST", "/", Some(3), &tcp, "ann").contains("Authorization"));
    }

    fn whole(raw: &[u8]) -> bool {
        Progress::default().whole(raw)
    }

    /// Hands out its reply `step` bytes per read.
    struct Trickle {
        reply: Vec<u8>,
        at: usize,
        step: usize,
    }

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.step.min(buf.len()).min(self.reply.len() - self.at);
            buf[..n].copy_from_slice(&self.reply[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    impl Write for Trickle {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_reply_read_a_byte_at_a_time_is_the_reply_read_whole() {
        let body: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let mut chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for part in body.chunks(7000) {
            chunked.extend_from_slice(format!("{:x};ext=1\r\n", part.len()).as_bytes());
            chunked.extend_from_slice(part);
            chunked.extend_from_slice(b"\r\n");
        }
        chunked.extend_from_slice(b"0\r\n\r\n");
        let mut sized =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        sized.extend_from_slice(&body);
        for reply in [chunked, sized] {
            for step in [1, reply.len()] {
                let mut s = Trickle {
                    reply: reply.clone(),
                    at: 0,
                    step,
                };
                let got = read_reply(&mut s).ok().map(|r| r.body);
                assert_eq!(got.as_deref(), Some(body.as_slice()), "{step} bytes a read");
            }
        }
    }

    #[test]
    fn a_reply_is_whole_by_its_length_or_last_chunk() {
        assert!(!whole(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 3\r\n"));
        assert!(!whole(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 3\r\n\r\nab"
        ));
        assert!(whole(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 3\r\n\r\nabc"
        ));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n";
        assert!(!whole(chunked));
        assert!(whole(&[&chunked[..], b"0\r\n\r\n"].concat()));
        assert!(
            !whole(b"HTTP/1.0 200 OK\r\n\r\nno length"),
            "runs to the end"
        );
    }

    /// cupsd answers a refused job operation and keeps the socket open; the reply is read by length.
    #[test]
    fn an_error_reply_on_an_open_socket_reads_at_once() {
        let (mut ours, mut cupsd) = UnixStream::pair().unwrap();
        cupsd
            .write_all(
                b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 2\r\n\r\nno",
            )
            .unwrap();
        ours.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(read_reply(&mut ours).err(), Some(Error::Denied));
        drop(cupsd);
    }

    #[test]
    fn bodies_by_length_by_chunks_and_to_the_end() {
        let r = parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcdef").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"abc"[..]));
        let r = parse_reply(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(r.body, b"abcde");
        let r = parse_reply(b"HTTP/1.0 401 Unauthorized\r\n\r\nno").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (401, &b"no"[..]));
        assert!(parse_reply(b"HTTP/1.1 200 OK\r\n").is_err());
        assert!(
            parse_reply(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\nab").is_err()
        );
    }

    #[test]
    fn a_chunk_size_past_memory_ends_the_read_and_is_refused() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffec\r\nab";
        assert!(whole(raw), "the read stops instead of spinning");
        assert!(parse_reply(raw).is_err());
    }
}
