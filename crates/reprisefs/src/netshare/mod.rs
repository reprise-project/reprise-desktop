//! Network shares through gvfs: mounting with `gio mount`, the mounted shares from its mount
//! tracker, and browsing. Nothing here reads inside a share, so a dead server holds only its caller.

pub mod browse;
mod conversation;
pub mod mount;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use zbus::{OwnedObjectPath, OwnedVariant};

const DAEMON: &str = "org.gtk.vfs.Daemon";
const TRACKER_PATH: &str = "/org/gtk/vfs/mounttracker";
const TRACKER: &str = "org.gtk.vfs.MountTracker";
/// A burst of mounts and unmounts costs one read after this pause.
const SETTLE: Duration = Duration::from_millis(150);
/// How long a `gio info` or `gio mount -u` gets before it is killed.
const GIO_WAIT: Duration = Duration::from_secs(30);

/// The schemes Connect to Server takes. `ssh` is read as `sftp`.
pub const SCHEMES: &[&str] = &["smb", "sftp", "ftp", "ftps", "dav", "davs", "nfs", "afp"];

/// A server address: `scheme://[user@]host[:port][/path]`, or `\\host\share\path` for SMB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub scheme: String,
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    /// Starts with `/`, or is empty.
    pub path: String,
}

impl Address {
    /// `text` as an address: one of [`SCHEMES`] (case-insensitive) with a host, or a UNC path.
    pub fn parse(text: &str) -> Option<Address> {
        let text = text.trim();
        if let Some(unc) = text.strip_prefix("\\\\") {
            let unc = unc.replace('\\', "/");
            return Address::parse(&format!("smb://{unc}"));
        }
        let (scheme, rest) = text.split_once("://")?;
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "ssh" => "sftp".to_string(),
            s if SCHEMES.contains(&s) => s.to_string(),
            _ => return None,
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => rest.split_at(i),
            None => (rest, ""),
        };
        let (user, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u.to_string()).filter(|u| !u.is_empty()), h),
            None => (None, authority),
        };
        let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
            let (h, after) = v6.split_once(']')?;
            let port = match after.strip_prefix(':') {
                Some(p) => Some(p.parse().ok()?),
                None if after.is_empty() => None,
                None => return None,
            };
            (format!("[{h}]"), port)
        } else {
            match hostport.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), Some(p.parse().ok()?)),
                None => (hostport.to_string(), None),
            }
        };
        if host.is_empty() || host.contains(char::is_whitespace) {
            return None;
        }
        Some(Address {
            scheme,
            user,
            host,
            port,
            path: path.to_string(),
        })
    }

    pub fn uri(&self) -> String {
        let user = self
            .user
            .as_ref()
            .map_or(String::new(), |u| format!("{u}@"));
        let port = self.port.map_or(String::new(), |p| format!(":{p}"));
        let path = if self.path.is_empty() {
            "/"
        } else {
            &self.path
        };
        format!("{}://{user}{}{port}{path}", self.scheme, self.host)
    }

    /// An SMB share's name: the path's first segment.
    pub fn share(&self) -> Option<&str> {
        (self.scheme == "smb")
            .then(|| self.path.split('/').find(|s| !s.is_empty()))
            .flatten()
    }

    /// An SMB server with no share: it lists its shares in the Network place rather than
    /// opening as a folder.
    pub fn is_server(&self) -> bool {
        self.scheme == "smb" && self.share().is_none()
    }

    /// The Secret Service attributes a password for this server is kept and found under,
    /// in the network password schema other desktops use.
    fn secret_attributes(&self) -> HashMap<&'static str, String> {
        let mut a = HashMap::from([
            (
                "xdg:schema",
                "org.gnome.keyring.NetworkPassword".to_string(),
            ),
            ("protocol", self.scheme.clone()),
            ("server", self.host.clone()),
        ]);
        if let Some(port) = self.port {
            a.insert("port", port.to_string());
        }
        if let Some(user) = &self.user {
            a.insert("user", user.clone());
        }
        a
    }
}

/// A mounted share, as gvfs lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    /// gvfs's name for it: "public on nas", "user on host".
    pub name: String,
    /// The first name of gvfs's themed icon, `folder-remote` when it gives none.
    pub icon: String,
    /// Its FUSE folder under `$XDG_RUNTIME_DIR/gvfs`.
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// `gio` is installed, so Connect to Server can mount.
    pub gio: bool,
    pub shares: Vec<Share>,
}

impl Snapshot {
    pub fn share_at(&self, path: &Path) -> Option<&Share> {
        self.shares.iter().find(|s| s.path == path)
    }
}

fn gio() -> Option<&'static Path> {
    static GIO: OnceLock<Option<PathBuf>> = OnceLock::new();
    GIO.get_or_init(|| {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|d| d.join("gio"))
            .find(|p| p.is_absolute() && p.is_file())
    })
    .as_deref()
}

/// `gio` with untranslated prompts and errors, which [`mount`] and [`browse`] read.
fn gio_command() -> Option<Command> {
    let mut c = Command::new(gio()?);
    c.env("LC_ALL", "C.UTF-8");
    Some(c)
}

/// gio's error text without its `gio: URI: ` lead.
fn gio_error(stderr: &str) -> String {
    let last = stderr.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("");
    let text = last.strip_prefix("gio: ").unwrap_or(last);
    // The URI itself holds `:`, so the message starts after the last `: ` before any text.
    let text = match text.find(": ") {
        Some(i) if text[..i].contains("://") || text.starts_with('/') => &text[i + 2..],
        _ => text,
    };
    text.trim().to_string()
}

/// Runs `gio args`, killed after `wait`: its standard output, or its error.
fn run_gio(args: &[&str], wait: Duration) -> Result<String, String> {
    let Some(mut cmd) = gio_command() else {
        return Err("gio is not installed.".into());
    };
    let mut child = cmd
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut o = String::new();
        let _ = out.read_to_string(&mut o);
        let mut e = String::new();
        let _ = err.read_to_string(&mut e);
        (o, e)
    });
    let deadline = Instant::now() + wait;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let (out, err) = reader.join().unwrap_or_default();
    match status {
        Some(s) if s.success() => Ok(out),
        Some(_) => Err(gio_error(&err)),
        None => Err("The server did not answer.".into()),
    }
}

/// The FUSE folder gvfs gives `uri` once it is mounted.
pub fn local_path(uri: &str) -> Option<PathBuf> {
    let out = run_gio(&["info", "-a", "standard::name", "--", uri], GIO_WAIT).ok()?;
    out.lines()
        .find_map(|l| l.strip_prefix("local path: "))
        .map(PathBuf::from)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Unmount {
    /// Something has a file open on the share, or is waiting on a server that stopped answering.
    Busy,
    Error(String),
}

/// Unmounts the share whose FUSE folder is `path`; `force` drops what is still open on it.
pub fn unmount(path: &Path, force: bool) -> Result<(), Unmount> {
    let path = path
        .to_str()
        .ok_or_else(|| Unmount::Error("The share's folder name is not text.".into()))?;
    let args: &[&str] = match force {
        true => &["mount", "-f", "-u", "--", path],
        false => &["mount", "-u", "--", path],
    };
    run_gio(args, GIO_WAIT)
        .map(|_| ())
        .map_err(|e| match e.to_lowercase().contains("busy") {
            true => Unmount::Busy,
            false => Unmount::Error(e),
        })
}

/// The mounted shares on a thread of its own: `post` gets every share at once and again after
/// each burst of mounts and unmounts. Nothing is started when gvfs is not running.
pub fn follow(post: impl FnMut(Snapshot) + Send + 'static) {
    if let Err(e) = std::thread::Builder::new()
        .name("gvfs-mounts".into())
        .spawn(move || zbus::block_on(run(post)))
    {
        log::warn!("could not start the gvfs mounts thread: {e}");
    }
}

async fn run(mut post: impl FnMut(Snapshot)) {
    use futures_util::FutureExt;
    use futures_util::stream::StreamExt;

    let gio = gio().is_some();
    let conn = match reprisebus::session() {
        Ok(c) => c,
        Err(e) => {
            log::info!("no session bus ({e}); no network shares to follow");
            post(Snapshot {
                gio,
                ..Snapshot::default()
            });
            return;
        }
    };
    let mut rules = Vec::new();
    for member in ["Mounted", "Unmounted"] {
        match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(DAEMON)
            .interface(TRACKER)
            .member(member)
            .build()
        {
            Ok(r) => rules.push(r.into()),
            Err(e) => log::warn!("no match rule for {member}: {e}"),
        }
    }
    let mut signals = reprisebus::signals(&conn, DAEMON, rules).await;
    loop {
        post(Snapshot {
            gio,
            shares: read(&conn).await,
        });
        if signals.next().await.is_none() {
            log::warn!("the gvfs signal streams all ended");
            return;
        }
        async_io::Timer::after(SETTLE).await;
        while let Some(Some(_)) = signals.next().now_or_never() {}
    }
}

/// `(dbus_id, obj_path, display_name, stable_name, x_content_types, icon, symbolic_icon,
/// prefered_filename_encoding, user_visible, fuse_mountpoint, mount_spec, default_location)`.
type Mount = (
    String,
    OwnedObjectPath,
    String,
    String,
    String,
    String,
    String,
    String,
    bool,
    Vec<u8>,
    (Vec<u8>, HashMap<String, OwnedVariant>),
    Vec<u8>,
);

/// The shares gvfs has mounted, asked only of a daemon already running: listing mounts never
/// starts gvfs.
async fn read(conn: &zbus::Connection) -> Vec<Share> {
    if reprisebus::name_owner(conn, DAEMON).await.is_none() {
        return Vec::new();
    }
    let reply = conn
        .call_method(
            Some(DAEMON),
            TRACKER_PATH,
            Some(TRACKER),
            "ListMounts2",
            &(true,),
        )
        .await;
    match reply.and_then(|m| m.body().deserialize::<Vec<Mount>>()) {
        Ok(mounts) => shares(mounts),
        Err(e) => {
            log::info!("no gvfs mounts ({e})");
            Vec::new()
        }
    }
}

fn shares(mounts: Vec<Mount>) -> Vec<Share> {
    use std::os::unix::ffi::OsStringExt;
    let mut out: Vec<Share> = mounts
        .into_iter()
        .filter(|m| m.8)
        .filter_map(|m| {
            let fuse = m.9.split(|&b| b == 0).next().unwrap_or_default().to_vec();
            (!fuse.is_empty()).then(|| Share {
                name: m.2,
                icon: themed_icon(&m.5).unwrap_or("folder-remote").to_string(),
                path: PathBuf::from(std::ffi::OsString::from_vec(fuse)),
            })
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// The first name of a serialized themed icon: `. GThemedIcon folder-remote folder`.
fn themed_icon(icon: &str) -> Option<&str> {
    let mut words = icon.split(' ');
    (words.next() == Some(".") && words.next() == Some("GThemedIcon"))
        .then(|| words.next())
        .flatten()
        .filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Address {
        Address::parse(text).expect(text)
    }

    #[test]
    fn an_address_reads_its_scheme_user_host_port_and_path() {
        let a = at("SMB://alice@nas:1445/public/music");
        assert_eq!(a.scheme, "smb");
        assert_eq!(a.user.as_deref(), Some("alice"));
        assert_eq!(a.host, "nas");
        assert_eq!(a.port, Some(1445));
        assert_eq!(a.path, "/public/music");
        assert_eq!(a.uri(), "smb://alice@nas:1445/public/music");
        assert_eq!(a.share(), Some("public"));
        assert!(!a.is_server());
    }

    #[test]
    fn a_unc_path_and_ssh_read_as_smb_and_sftp() {
        assert_eq!(at(r"\\nas\public\music").uri(), "smb://nas/public/music");
        assert_eq!(at("ssh://host").uri(), "sftp://host/");
        assert_eq!(at("sftp://[fe80::1]:2222/srv").host, "[fe80::1]");
        assert_eq!(at("sftp://[fe80::1]:2222/srv").port, Some(2222));
    }

    #[test]
    fn an_smb_address_without_a_share_is_a_server() {
        assert!(at("smb://nas").is_server());
        assert!(at("smb://nas/").is_server());
        assert!(!at("sftp://nas/").is_server());
    }

    #[test]
    fn other_text_is_not_an_address() {
        for text in [
            "/home/u",
            "http://example.com",
            "smb://",
            "smb://host:port/x",
            "smb://a b/",
            "Computer",
        ] {
            assert_eq!(Address::parse(text), None, "{text}");
        }
    }

    #[test]
    fn a_password_is_kept_per_scheme_server_port_and_named_user() {
        let a = at("sftp://bob@host:2222/");
        let attrs = a.secret_attributes();
        assert_eq!(attrs["xdg:schema"], "org.gnome.keyring.NetworkPassword");
        assert_eq!(attrs["protocol"], "sftp");
        assert_eq!(attrs["server"], "host");
        assert_eq!(attrs["port"], "2222");
        assert_eq!(attrs["user"], "bob");
        let attrs = at("smb://nas/public").secret_attributes();
        assert!(!attrs.contains_key("user") && !attrs.contains_key("port"));
    }

    #[test]
    fn gio_errors_lose_the_program_and_the_uri() {
        assert_eq!(
            gio_error("gio: smb://localhost/private/: Anonymous access denied\n"),
            "Anonymous access denied"
        );
        assert_eq!(
            gio_error("gio: /run/user/1000/gvfs/smb-share:server=a,share=b: Device busy\n"),
            "Device busy"
        );
        assert_eq!(gio_error("Something broke\n"), "Something broke");
    }

    #[test]
    fn mounts_list_as_visible_shares_with_their_fuse_folder_and_icon() {
        let mount = |name: &str, visible: bool, fuse: &[u8]| -> Mount {
            (
                ":1.1".into(),
                OwnedObjectPath::try_from("/org/gtk/vfs/mount/1").unwrap(),
                name.into(),
                String::new(),
                String::new(),
                ". GThemedIcon network-server network".into(),
                String::new(),
                String::new(),
                visible,
                fuse.to_vec(),
                (Vec::new(), HashMap::new()),
                Vec::new(),
            )
        };
        let got = shares(vec![
            mount(
                "public on nas",
                true,
                b"/run/user/1000/gvfs/smb-share:server=nas,share=public\0",
            ),
            mount("hidden", false, b"/run/user/1000/gvfs/x\0"),
            mount("no folder", true, b"\0"),
        ]);
        assert_eq!(
            got,
            vec![Share {
                name: "public on nas".into(),
                icon: "network-server".into(),
                path: PathBuf::from("/run/user/1000/gvfs/smb-share:server=nas,share=public"),
            }]
        );
        assert_eq!(themed_icon("garbage"), None);
    }
}
