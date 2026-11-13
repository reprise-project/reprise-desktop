//! Files localsearch (the Tracker 3 file indexer) finds by their text and metadata, asked through
//! its SPARQL endpoint; the bus starts the indexer when it is installed and not yet running.

use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

use zbus::Connection;
use zbus::{Fd, Variant};

/// The indexer's bus name, owned by localsearch and by the tracker-miners releases before it.
pub const NAME: &str = "org.freedesktop.Tracker3.Miner.Files";
const PATH: &str = "/org/freedesktop/Tracker3/Endpoint";
const ENDPOINT: &str = "org.freedesktop.Tracker3.Endpoint";

/// Every indexed item whose text or metadata matches `~match`, best first: the URL of the file
/// that stores it, on a data source that is mounted now.
const QUERY: &str = "SELECT ?url WHERE { \
     ?ie fts:match ~match ; nie:isStoredAs ?file . \
     ?file nie:url ?url . \
     FILTER EXISTS { ?file nie:dataSource/tracker:available true } \
     FILTER (STRSTARTS(?url, \"file:///\")) \
     } ORDER BY DESC(fts:rank(?ie)) LIMIT ";

/// The full-text match for `query`: every word required, the last one as typed so far (a
/// prefix; the index takes a prefix on the last word only). Lower case, so a word never reads as
/// an operator. None when `query` holds no letter or digit.
pub fn fts_match(query: &str) -> Option<String> {
    let words: Vec<String> = (query.split(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    (!words.is_empty()).then(|| format!("{}*", words.join(" ")))
}

/// The `file://` URLs of at most `max` files whose text or metadata matches `query`
/// ([`fts_match`]), best first. Fails when the indexer is not installed, answers an error, or has
/// not sent every row by `timeout` after the call; blocks the calling thread meanwhile.
pub fn search(
    conn: &Connection,
    query: &str,
    max: usize,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let Some(words) = fts_match(query) else {
        return Ok(Vec::new());
    };
    let deadline = Instant::now() + timeout;
    let (read, write) = pipe().map_err(|e| e.to_string())?;
    let sparql = format!("{QUERY}{max}");
    let args = HashMap::from([("match", Variant::from(words.as_str()))]);
    crate::wait(call(conn, &sparql, Fd::from(write.as_fd()), &args)).map_err(|e| e.to_string())?;
    drop(write);
    let bytes = read_all(&read, deadline)?;
    let mut urls: Vec<String> = Vec::new();
    for row in rows(&bytes).ok_or("the indexer's answer did not parse")? {
        if let Some(url) = row.into_iter().next()
            && !urls.contains(&url)
        {
            urls.push(url);
        }
    }
    Ok(urls)
}

/// Whether the indexer runs now or the bus can start it: its package is installed.
pub async fn installed(conn: &Connection) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return false;
    };
    let name = zbus::names::BusName::try_from(NAME).expect("a valid bus name");
    if dbus.name_has_owner(name).await.unwrap_or(false) {
        return true;
    }
    (dbus.list_activatable_names().await)
        .is_ok_and(|names| names.iter().any(|n| n.as_str() == NAME))
}

async fn call(
    conn: &Connection,
    sparql: &str,
    fd: Fd<'_>,
    args: &HashMap<&str, Variant<'_>>,
) -> zbus::Result<()> {
    let body = (sparql, fd, args);
    conn.call_method(Some(NAME), PATH, Some(ENDPOINT), "Query", &body)
        .await?;
    Ok(())
}

fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: pipe2 succeeded, so both are open descriptors no one else owns.
    Ok(unsafe {
        use std::os::fd::FromRawFd;
        (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))
    })
}

/// Everything written to `fd` until the writer closes it; an error at `deadline`.
fn read_all(fd: &OwnedFd, deadline: Instant) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("the indexer did not finish its answer in time".into());
        }
        let mut pfd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        // SAFETY: `pfd` is one valid pollfd for the call's duration.
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ready < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.to_string());
        }
        if ready == 0 {
            continue;
        }
        // SAFETY: `buf` is writable for its whole length.
        let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        match n {
            0 => return Ok(out),
            n if n > 0 => out.extend_from_slice(&buf[..n as usize]),
            _ => {
                let e = std::io::Error::last_os_error();
                if e.kind() != std::io::ErrorKind::Interrupted {
                    return Err(e.to_string());
                }
            }
        }
        if out.len() > MAX_ANSWER {
            return Err("the indexer's answer is too long".into());
        }
    }
}

/// The most bytes an answer of a few dozen URLs takes, with room to spare.
const MAX_ANSWER: usize = 1 << 20;

/// The rows of a cursor as the endpoint writes them, each column's string. Per row, in host
/// byte order: the column count n, n value types, n end offsets (each string's closing NUL,
/// counted from the row's first string byte), then the n strings, each NUL-terminated with an
/// optional language tag after a NUL of its own. None when the bytes end mid-row.
fn rows(bytes: &[u8]) -> Option<Vec<Vec<String>>> {
    let int = |at: &mut usize| -> Option<usize> {
        let b = bytes.get(*at..*at + 4)?;
        *at += 4;
        usize::try_from(i32::from_ne_bytes(b.try_into().ok()?)).ok()
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let n = int(&mut at)?;
        if n.checked_mul(8)? > bytes.len() - at {
            return None;
        }
        at += 4 * n;
        let ends: Vec<usize> = (0..n).map(|_| int(&mut at)).collect::<Option<_>>()?;
        let len = ends.last().map_or(0, |e| e + 1);
        let data = bytes.get(at..at.checked_add(len)?)?;
        let mut start = 0;
        let mut row = Vec::with_capacity(n);
        for end in ends {
            let value = data.get(start..end)?.split(|&b| b == 0).next()?;
            row.push(String::from_utf8_lossy(value).into_owned());
            start = end + 1;
        }
        at += len;
        out.push(row);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    /// One cursor row as the endpoint writes it, each value with an optional language tag.
    fn row(values: &[(&str, Option<&str>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((values.len() as i32).to_ne_bytes());
        for _ in values {
            out.extend(2i32.to_ne_bytes());
        }
        let mut end = -1i32;
        let mut data = Vec::new();
        for (v, tag) in values {
            data.extend(v.as_bytes());
            end += v.len() as i32 + 1;
            if let Some(t) = tag {
                data.push(0);
                data.extend(t.as_bytes());
                end += t.len() as i32 + 1;
            }
            data.push(0);
            out.extend(end.to_ne_bytes());
        }
        out.extend(data);
        out
    }

    #[test]
    fn rows_read_back_every_column_and_drop_language_tags() {
        let mut bytes = row(&[("file:///home/u/a.txt", None), ("2.5", None)]);
        bytes.extend(row(&[("Résumé", Some("fr")), ("", None)]));
        assert_eq!(
            rows(&bytes),
            Some(vec![
                vec!["file:///home/u/a.txt".to_string(), "2.5".to_string()],
                vec!["Résumé".to_string(), String::new()],
            ])
        );
        assert_eq!(rows(&[]), Some(Vec::new()));
        assert_eq!(rows(&bytes[..bytes.len() - 1]), None, "cut mid-row");
        assert_eq!(
            rows(&i32::MAX.to_ne_bytes()),
            None,
            "a column count past the bytes"
        );
    }

    #[test]
    fn the_match_keeps_the_words_lower_case_with_the_last_a_prefix() {
        assert_eq!(fts_match("quarterly"), Some("quarterly*".into()));
        assert_eq!(
            fts_match("Tax  2025-report\" OR"),
            Some("tax 2025 report or*".into())
        );
        assert_eq!(fts_match("Ünïcode"), Some("ünïcode*".into()));
        assert_eq!(fts_match(" *\"()- "), None);
    }

    #[derive(Default)]
    struct Seen {
        queries: Vec<(String, HashMap<String, zbus::OwnedVariant>)>,
    }

    /// An endpoint that writes `rows` to the stream it is handed, or holds it open unanswered.
    struct Endpoint {
        seen: Arc<Mutex<Seen>>,
        rows: Vec<u8>,
        hang: bool,
    }

    #[zbus::interface(name = "org.freedesktop.Tracker3.Endpoint")]
    impl Endpoint {
        fn query(
            &self,
            query: String,
            stream: zbus::OwnedFd,
            args: HashMap<String, zbus::OwnedVariant>,
        ) -> Vec<String> {
            self.seen.lock().unwrap().queries.push((query, args));
            let fd: OwnedFd = stream.into();
            let rows = self.rows.clone();
            let hang = self.hang;
            std::thread::spawn(move || {
                if hang {
                    std::thread::sleep(Duration::from_secs(2));
                }
                let mut f = std::fs::File::from(fd);
                let _ = f.write_all(&rows);
            });
            vec!["url".into()]
        }
    }

    fn endpoint(rows: Vec<u8>, hang: bool) -> (Connection, Connection, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let ep = Endpoint {
            seen: seen.clone(),
            rows,
            hang,
        };
        let server = std::thread::spawn(move || {
            crate::connect(
                zbus::connection::Builder::unix_stream(a)
                    .server(zbus::Guid::generate())
                    .p2p()
                    .serve_at(PATH, ep),
            )
        });
        let client = crate::connect(zbus::connection::Builder::unix_stream(b).p2p()).unwrap();
        (server.join().unwrap().unwrap(), client, seen)
    }

    #[test]
    fn a_search_binds_the_words_and_returns_each_url_once_in_order() {
        let mut answer = row(&[("file:///home/u/b%20c.pdf", None)]);
        answer.extend(row(&[("file:///home/u/a.txt", None)]));
        answer.extend(row(&[("file:///home/u/b%20c.pdf", None)]));
        let (_server, client, seen) = endpoint(answer, false);
        let got = search(&client, "quarterly tax", 5, Duration::from_secs(5));
        assert_eq!(
            got,
            Ok(vec![
                "file:///home/u/b%20c.pdf".to_string(),
                "file:///home/u/a.txt".to_string()
            ])
        );
        let seen = seen.lock().unwrap();
        let (query, args) = &seen.queries[0];
        assert!(query.contains("fts:match ~match"), "{query}");
        assert!(query.ends_with("LIMIT 5"), "{query}");
        let words: &str = args["match"].downcast_ref().unwrap();
        assert_eq!(
            words, "quarterly tax*",
            "the text is bound, never spliced in"
        );
    }

    #[test]
    fn an_answer_not_finished_by_the_timeout_fails_without_waiting_for_it() {
        let (_server, client, _) = endpoint(row(&[("file:///a", None)]), true);
        let start = Instant::now();
        let got = search(&client, "late", 5, Duration::from_millis(300));
        assert!(got.is_err(), "{got:?}");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn text_with_no_word_asks_nothing() {
        let (_server, client, seen) = endpoint(Vec::new(), false);
        assert_eq!(
            search(&client, "--", 5, Duration::from_secs(1)),
            Ok(Vec::new())
        );
        assert!(seen.lock().unwrap().queries.is_empty());
    }
}
