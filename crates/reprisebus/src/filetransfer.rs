//! Files handed between programs through the document portal's FileTransfer, which is how a
//! drag of files reaches a sandboxed (Flatpak) program.

use std::fs::{File, OpenOptions};
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use zbus::blocking::Connection;
use zbus::zvariant::{Fd, Value};

const NAME: &str = "org.freedesktop.portal.Documents";
const PATH: &str = "/org/freedesktop/portal/documents";
const INTERFACE: &str = "org.freedesktop.portal.FileTransfer";

/// Files per `AddFiles` call: a D-Bus message carries a bounded number of
/// fds. Each call's fds are opened just before it.
const FDS_PER_CALL: usize = 16;

/// Transfers one drag may start: a target reads the key each time the
/// drag enters it, and may read it again at the drop.
const MAX_PER_DRAG: usize = 16;

const MAX_SENDING: usize = 4;

const RETRY: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct FileTransfer {
    conn: Connection,
}

impl FileTransfer {
    pub fn on(conn: Connection) -> zbus::Result<FileTransfer> {
        let version: u32 = conn
            .call_method(
                Some(NAME),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(INTERFACE, "version"),
            )?
            .body()
            .deserialize::<zbus::zvariant::OwnedValue>()
            .and_then(|v| u32::try_from(v).map_err(Into::into))?;
        log::debug!("file transfer portal version {version}");
        Ok(FileTransfer { conn })
    }

    /// Register `paths` read-only in a new transfer that ends at its first
    /// retrieval, and return its key. A path that cannot be opened (gone,
    /// a dangling link) or is neither a file nor a folder (a pipe, a
    /// socket, a device) is left out; an error when none is left, or when
    /// the portal refuses one, which stops the transfer.
    pub fn send(&self, paths: &[PathBuf]) -> zbus::Result<String> {
        let mut fds = paths.iter().filter_map(|p| {
            open_path(p)
                .map_err(|e| log::info!("file transfer: leaving out {}: {e}", p.display()))
                .ok()
        });
        let mut chunk: Vec<OwnedFd> = fds.by_ref().take(FDS_PER_CALL).collect();
        if chunk.is_empty() {
            return Err(zbus::Error::Failure("no dragged file can be sent".into()));
        }
        let options = [
            ("writable", Value::from(false)),
            ("autostop", Value::from(true)),
        ];
        let key: String = self
            .call("StartTransfer", &(options_dict(&options),))?
            .body()
            .deserialize()?;
        while !chunk.is_empty() {
            let call: Vec<Fd> = chunk.iter().map(Fd::from).collect();
            let added = self.call("AddFiles", &(key.as_str(), call, options_dict(&[])));
            if let Err(e) = added {
                let _ = self.stop(&key);
                return Err(e);
            }
            chunk = fds.by_ref().take(FDS_PER_CALL).collect();
        }
        Ok(key)
    }

    pub fn retrieve(&self, key: &str) -> zbus::Result<Vec<PathBuf>> {
        let files: Vec<String> = self
            .call("RetrieveFiles", &(key, options_dict(&[])))?
            .body()
            .deserialize()?;
        Ok(files.into_iter().map(PathBuf::from).collect())
    }

    pub fn stop(&self, key: &str) -> zbus::Result<()> {
        self.call("StopTransfer", &(key,)).map(drop)
    }

    fn call<B>(&self, method: &str, body: &B) -> zbus::Result<zbus::message::Message>
    where
        B: zbus::export::serde::ser::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(NAME), PATH, Some(INTERFACE), method, body)
    }
}

enum Link {
    Down(Option<Instant>),
    Connecting,
    Up(FileTransfer),
}

static LINK: Mutex<Link> = Mutex::new(Link::Down(None));

fn link() -> MutexGuard<'static, Link> {
    LINK.lock().unwrap_or_else(|e| e.into_inner())
}

fn ready() -> Option<FileTransfer> {
    match &*link() {
        Link::Up(ft) => Some(ft.clone()),
        _ => None,
    }
}

/// Whether a connection may be tried now: never tried, or the last try
/// failed more than `RETRY` before `now`.
fn due(link: &Link, now: Instant) -> bool {
    match link {
        Link::Down(failed) => failed.is_none_or(|at| now.duration_since(at) >= RETRY),
        Link::Connecting | Link::Up(_) => false,
    }
}

pub fn shared() -> Option<FileTransfer> {
    {
        let mut l = link();
        if let Link::Up(ft) = &*l {
            return Some(ft.clone());
        }
        if !due(&l, Instant::now()) {
            return None;
        }
        *l = Link::Connecting;
    }
    let made = crate::session().and_then(FileTransfer::on);
    let mut l = link();
    match made {
        Ok(ft) => {
            *l = Link::Up(ft.clone());
            Some(ft)
        }
        Err(e) => {
            log::info!("file transfer portal: {e}; drags offer no portal key");
            *l = Link::Down(Some(Instant::now()));
            None
        }
    }
}

pub fn connect() {
    if due(&link(), Instant::now()) {
        spawn("file transfer connect", || {
            shared();
        });
    }
}

/// The transfers one drag's source started. A drag offers its files as a
/// key only when made with a connection ([`Transfers::offered`]). Dropping
/// it, when the drag ends, stops each transfer the target has not traded.
pub struct Transfers {
    ft: Option<FileTransfer>,
    sent: Arc<Mutex<Sent>>,
}

#[derive(Default)]
struct Sent {
    keys: Vec<String>,
    requests: usize,
    /// The drag ended: a key registered after this is stopped at once.
    closed: bool,
}

fn lock(sent: &Mutex<Sent>) -> MutexGuard<'_, Sent> {
    sent.lock().unwrap_or_else(|e| e.into_inner())
}

impl Transfers {
    pub fn for_drag() -> Transfers {
        connect();
        Transfers::on(ready())
    }

    pub fn on(ft: Option<FileTransfer>) -> Transfers {
        Transfers {
            ft,
            sent: Arc::default(),
        }
    }

    pub fn offered(&self) -> bool {
        self.ft.is_some()
    }

    /// Register `paths` in a new transfer on a thread and hand `write` the
    /// key's bytes, for the pipe of a target that asked for it.
    /// Each request is a transfer of its own: the portal ends one at its
    /// first retrieval. Past `MAX_PER_DRAG` requests, with `MAX_SENDING`
    /// threads busy, or on failure, `write` is dropped uncalled, which
    /// closes the target's pipe empty.
    pub fn send_key(&self, paths: Vec<PathBuf>, write: impl FnOnce(Vec<u8>) + Send + 'static) {
        let Some(ft) = self.ft.clone() else {
            return;
        };
        {
            let mut s = lock(&self.sent);
            if s.closed || s.requests >= MAX_PER_DRAG {
                log::debug!("file transfer: {MAX_PER_DRAG} keys sent for this drag already");
                return;
            }
            s.requests += 1;
        }
        let Some(slot) = Sending::enter() else {
            log::debug!("file transfer: {MAX_SENDING} registrations busy, no key");
            return;
        };
        let sent = self.sent.clone();
        spawn("file transfer send", move || {
            let _slot = slot;
            let key = match ft.send(&paths) {
                Ok(key) => key,
                Err(e) => {
                    log::warn!("file transfer: registering the dragged files: {e}");
                    return;
                }
            };
            let mut s = lock(&sent);
            if s.closed {
                drop(s);
                let _ = ft.stop(&key);
                return;
            }
            s.keys.push(key.clone());
            drop(s);
            write(encode_key(&key));
        });
    }
}

impl Drop for Transfers {
    fn drop(&mut self) {
        let keys = {
            let mut s = lock(&self.sent);
            s.closed = true;
            std::mem::take(&mut s.keys)
        };
        let Some(ft) = self.ft.take().filter(|_| !keys.is_empty()) else {
            return;
        };
        spawn("file transfer stop", move || {
            for key in keys {
                // A traded transfer ended at its retrieval already.
                if let Err(e) = ft.stop(&key) {
                    log::debug!("file transfer: stopping {key}: {e}");
                }
            }
        });
    }
}

struct Sending;

static SENDING: AtomicUsize = AtomicUsize::new(0);

impl Sending {
    fn enter() -> Option<Sending> {
        SENDING
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_SENDING).then_some(n + 1)
            })
            .ok()
            .map(|_| Sending)
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        SENDING.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Trade the key in a drop's `bytes` for its files on a thread, then call
/// `done` there with them; None when there is no key, no portal, or the
/// trade failed or brought no files, and the target reads the drag some
/// other way.
pub fn retrieve_key(bytes: &[u8], done: impl FnOnce(Option<Vec<PathBuf>>) + Send + 'static) {
    let Some(key) = decode_key(bytes) else {
        log::info!("file transfer: a drag sent no key");
        done(None);
        return;
    };
    spawn("file transfer retrieve", move || {
        let files = shared()
            .and_then(|ft| {
                ft.retrieve(&key)
                    .map_err(|e| log::warn!("file transfer: retrieving {key}: {e}"))
                    .ok()
            })
            .filter(|files| !files.is_empty());
        done(files);
    });
}

pub fn encode_key(key: &str) -> Vec<u8> {
    let mut bytes = key.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

pub fn decode_key(bytes: &[u8]) -> Option<String> {
    let key = std::str::from_utf8(bytes).ok()?;
    let key = key.trim_end_matches(|c: char| c == '\0' || c.is_whitespace());
    (!key.is_empty() && !key.contains('\0')).then(|| key.to_string())
}

fn spawn(name: &str, run: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(run) {
        log::warn!("{name} thread: {e}");
    }
}

fn options_dict<'a>(
    options: &'a [(&'a str, Value<'a>)],
) -> std::collections::HashMap<&'a str, &'a Value<'a>> {
    options.iter().map(|(k, v)| (*k, v)).collect()
}

/// An `O_PATH` fd for `path`, the form the portal takes: it names the file
/// or folder without opening its contents. An error for anything but a
/// file or a folder, followed through links.
fn open_path(path: &Path) -> std::io::Result<OwnedFd> {
    let file: File = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH)
        .open(path)?;
    let kind = file.metadata()?.file_type();
    if !kind.is_file() && !kind.is_dir() {
        return Err(std::io::Error::other("neither a file nor a folder"));
    }
    Ok(file.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStringExt;
    use std::process::{Child, Command, Stdio};
    use zbus::zvariant::OwnedValue;

    struct Bus(Child, String);

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    const BARE: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

    fn bus(session: bool, env: &[(&str, &Path)]) -> Option<(Bus, tempfile::TempDir)> {
        let dir = tempfile::tempdir().ok()?;
        let config = dir.path().join("bus.conf");
        std::fs::write(&config, BARE).ok()?;
        let mut cmd = Command::new("dbus-daemon");
        if session {
            cmd.arg("--session");
        } else {
            cmd.arg(format!("--config-file={}", config.display()));
        }
        let mut child = cmd
            .args(["--nofork", "--print-address"])
            .envs(env.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut line = String::new();
        BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        Some((Bus(child, line.trim().to_string()), dir))
    }

    fn connect(address: &str) -> Connection {
        zbus::blocking::connection::Builder::address(address)
            .expect("address")
            .build()
            .expect("connect")
    }

    #[derive(Default)]
    struct Seen {
        options: Vec<HashMap<String, OwnedValue>>,
        added: Vec<Vec<PathBuf>>,
        o_path: bool,
        stopped: Vec<String>,
    }

    struct Fake(Arc<Mutex<Seen>>);

    #[zbus::interface(name = "org.freedesktop.portal.FileTransfer")]
    impl Fake {
        #[zbus(property, name = "version")]
        fn version(&self) -> u32 {
            1
        }

        fn start_transfer(&self, options: HashMap<String, OwnedValue>) -> String {
            let mut seen = self.0.lock().unwrap();
            seen.options.push(options);
            seen.o_path = true;
            format!("key{}", seen.options.len())
        }

        fn add_files(
            &self,
            _key: &str,
            fds: Vec<zbus::zvariant::OwnedFd>,
            _options: HashMap<String, OwnedValue>,
        ) -> zbus::fdo::Result<()> {
            let mut seen = self.0.lock().unwrap();
            let mut paths = Vec::new();
            for fd in &fds {
                // SAFETY: `fd` is an open fd this call owns.
                let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
                seen.o_path &= flags & libc::O_PATH != 0;
                let link = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
                    .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
                if link.ends_with("refused") {
                    return Err(zbus::fdo::Error::AccessDenied("refused".into()));
                }
                paths.push(link);
            }
            seen.added.push(paths);
            Ok(())
        }

        fn retrieve_files(
            &self,
            key: &str,
            _options: HashMap<String, OwnedValue>,
        ) -> zbus::fdo::Result<Vec<String>> {
            if key != "key1" {
                return Err(zbus::fdo::Error::Failed("no such transfer".into()));
            }
            let seen = self.0.lock().unwrap();
            Ok(seen
                .added
                .iter()
                .flatten()
                .map(|p| p.to_string_lossy().into_owned())
                .collect())
        }

        fn stop_transfer(&self, key: String) {
            self.0.lock().unwrap().stopped.push(key);
        }
    }

    fn fake() -> Option<(Bus, Connection, Arc<Mutex<Seen>>)> {
        let (bus, _dir) = bus(false, &[])?;
        let seen = Arc::new(Mutex::new(Seen::default()));
        let portal = zbus::blocking::connection::Builder::address(bus.1.as_str())
            .unwrap()
            .serve_at(PATH, Fake(seen.clone()))
            .unwrap()
            .name(NAME)
            .unwrap()
            .build()
            .unwrap();
        Some((bus, portal, seen))
    }

    fn files(dir: &Path, names: impl IntoIterator<Item = String>) -> Vec<PathBuf> {
        names
            .into_iter()
            .map(|name| {
                let p = dir.join(name);
                std::fs::write(&p, b"x").unwrap();
                p
            })
            .collect()
    }

    #[test]
    fn dragged_files_go_as_path_fds_under_a_key_and_come_back_as_paths() {
        let Some((bus, _portal, seen)) = fake() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let paths = files(
            dir.path(),
            (0..FDS_PER_CALL + 4).map(|i| format!("f{i} x.txt")),
        );
        let ft = FileTransfer::on(connect(&bus.1)).expect("the interface answers");
        let key = ft.send(&paths).unwrap();
        assert_eq!(key, "key1");
        {
            let seen = seen.lock().unwrap();
            let opts = &seen.options[0];
            assert!(!bool::try_from(&opts["writable"]).unwrap(), "read-only");
            assert!(
                bool::try_from(&opts["autostop"]).unwrap(),
                "ends at retrieval"
            );
            let per_call: Vec<usize> = seen.added.iter().map(Vec::len).collect();
            assert_eq!(per_call, [FDS_PER_CALL, 4]);
            assert!(seen.o_path, "every fd is O_PATH");
        }
        assert_eq!(ft.retrieve(&key).unwrap(), paths);
        assert!(ft.retrieve("key9").is_err());
    }

    #[test]
    fn a_path_that_cannot_go_is_left_out_and_a_refusal_stops_the_transfer() {
        let Some((bus, _portal, seen)) = fake() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let ft = FileTransfer::on(connect(&bus.1)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(ft.send(&[dir.path().join("gone")]).is_err());
        assert!(seen.lock().unwrap().options.is_empty());
        let good = files(dir.path(), ["a".to_string(), "b".to_string()]);
        std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("dangling"))
            .unwrap();
        let fifo =
            std::ffi::CString::new(dir.path().join("fifo").into_os_string().into_vec()).unwrap();
        // SAFETY: `fifo` is a NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let mut paths = vec![dir.path().join("gone"), dir.path().join("dangling")];
        paths.push(good[0].clone());
        paths.push(dir.path().join("fifo"));
        paths.push(good[1].clone());
        paths.push(dir.path().to_path_buf());
        assert_eq!(ft.send(&paths).unwrap(), "key1");
        let mut want = good.clone();
        want.push(dir.path().to_path_buf());
        assert_eq!(seen.lock().unwrap().added, [want]);
        let paths = files(dir.path(), ["refused".to_string()]);
        assert!(ft.send(&paths).is_err());
        assert_eq!(seen.lock().unwrap().stopped, ["key2"]);
    }

    fn eventually(seen: &Mutex<Seen>, done: impl Fn(&Seen) -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if done(&seen.lock().unwrap()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn key_of(transfers: &Transfers, paths: &[PathBuf]) -> Option<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        transfers.send_key(paths.to_vec(), move |bytes| {
            let _ = tx.send(bytes);
        });
        rx.recv_timeout(Duration::from_secs(5))
            .ok()
            .and_then(|b| decode_key(&b))
    }

    #[test]
    fn a_drag_sends_a_few_keys_and_stops_the_untraded_ones_when_it_ends() {
        let Some((bus, _portal, seen)) = fake() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let ft = FileTransfer::on(connect(&bus.1)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let paths = files(dir.path(), ["a".to_string()]);
        assert!(!Transfers::on(None).offered());
        assert_eq!(key_of(&Transfers::on(None), &paths), None);
        let transfers = Transfers::on(Some(ft));
        assert!(transfers.offered());
        let keys: Vec<Option<String>> = (0..MAX_PER_DRAG + 1)
            .map(|_| key_of(&transfers, &paths))
            .collect();
        let want: Vec<Option<String>> = (1..=MAX_PER_DRAG)
            .map(|i| Some(format!("key{i}")))
            .chain([None])
            .collect();
        assert_eq!(keys, want, "past {MAX_PER_DRAG} a drag sends no key");
        assert!(seen.lock().unwrap().stopped.is_empty());
        drop(transfers);
        assert!(
            eventually(&seen, |s| s.stopped.len() == MAX_PER_DRAG),
            "every key is stopped once the drag ends"
        );
    }

    #[test]
    fn a_failed_connection_is_tried_again_after_a_while() {
        let now = Instant::now();
        assert!(due(&Link::Down(None), now));
        assert!(!due(&Link::Down(Some(now)), now));
        assert!(due(&Link::Down(Some(now)), now + RETRY));
        assert!(!due(&Link::Connecting, now));
    }

    #[test]
    fn no_portal_on_the_bus_is_no_file_transfer() {
        let Some((bus, _dir)) = bus(false, &[]) else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        assert!(FileTransfer::on(connect(&bus.1)).is_err());
    }

    #[test]
    fn a_key_goes_with_a_nul_and_comes_back_without() {
        assert_eq!(encode_key("5bd2"), b"5bd2\0");
        assert_eq!(decode_key(&encode_key("5bd2")).as_deref(), Some("5bd2"));
        assert_eq!(decode_key(b"5bd2").as_deref(), Some("5bd2"));
        assert_eq!(decode_key(b"5bd2\n\0\0").as_deref(), Some("5bd2"));
        for bad in [&b""[..], b"\0", b"a\0b", b"\xff"] {
            assert_eq!(decode_key(bad), None, "{bad:?}");
        }
    }

    #[test]
    #[ignore = "needs dbus-daemon, xdg-document-portal and FUSE"]
    fn the_document_portal_hands_a_program_outside_a_sandbox_its_files() {
        let home = tempfile::tempdir().unwrap();
        let runtime = home.path().join("run");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::set_permissions(
            &runtime,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let env = [
            ("HOME", home.path()),
            ("XDG_RUNTIME_DIR", runtime.as_path()),
            ("XDG_DATA_HOME", home.path()),
        ];
        let (bus, _dir) = bus(true, &env).expect("dbus-daemon");
        let paths = files(home.path(), ["a b.txt".to_string(), "c".to_string()]);
        let source = FileTransfer::on(connect(&bus.1)).expect("the portal answers");
        let key = source.send(&paths).unwrap();
        let target = FileTransfer::on(connect(&bus.1)).unwrap();
        assert_eq!(target.retrieve(&key).unwrap(), paths);
        assert!(target.retrieve(&key).is_err());
        let _ = Command::new("fusermount3")
            .args(["-u", "-z"])
            .arg(runtime.join("doc"))
            .status();
    }
}
