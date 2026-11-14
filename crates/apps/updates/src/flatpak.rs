//! The Flatpak side: application and runtime updates in the system and user installation, through the `flatpak`
//! command (it does the pull, signature checks and polkit prompt itself). Output is read in the C locale.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};

use crate::model::{Install, Kind, Origin, Update};

const FLATPAK: &str = "flatpak";

pub fn present() -> bool {
    Command::new(FLATPAK)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn flag(i: Install) -> &'static str {
    match i {
        Install::System => "--system",
        Install::User => "--user",
    }
}

/// Run `flatpak ARGS` to its end: its output, or the last thing it said
/// on stderr.
fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new(FLATPAK)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Flatpak could not be started: {e}."))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(complaint(&String::from_utf8_lossy(&out.stderr)))
    }
}

fn complaint(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("Flatpak failed.");
    let line = line.strip_prefix("error:").map_or(line, str::trim_start);
    let mut s = line.to_string();
    if let Some(first) = s.get(..1) {
        s.replace_range(..1, &first.to_uppercase());
    }
    if !s.ends_with('.') {
        s.push('.');
    }
    s
}

/// What is installed, from `flatpak list --columns=ref,name,version`:
/// a ref without its `app/` or `runtime/` to its name and version.
fn parse_list(out: &str) -> HashMap<String, (String, String)> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let r = f.next()?.trim();
            let name = f.next().unwrap_or("").trim();
            let version = f.next().unwrap_or("").trim();
            (!r.is_empty()).then(|| (r.to_string(), (name.to_string(), version.to_string())))
        })
        .collect()
}

/// The updates in `flatpak remote-ls --updates
/// --columns=ref,name,version,branch,origin,download-size`, named and
/// versioned as `installed` has them where it does.
fn parse_updates(
    out: &str,
    install: Install,
    installed: &HashMap<String, (String, String)>,
) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            let r = *f.first()?;
            let (kind, bare) = r.split_once('/')?;
            let origin = match kind {
                "app" => Origin::App(install),
                "runtime" => Origin::Runtime(install),
                _ => return None,
            };
            let field = |i: usize| f.get(i).copied().unwrap_or("");
            let (now_name, now_version) = installed
                .get(bare)
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .unwrap_or(("", ""));
            let id = bare.split('/').next().unwrap_or(bare);
            let name = [now_name, field(1), id]
                .into_iter()
                .find(|n| !n.is_empty())
                .unwrap_or(id);
            let version = match field(2) {
                "" => field(3),
                v => v,
            };
            let summary = match (field(4), field(5)) {
                ("", _) => String::new(),
                (o, "") => format!("from {o}"),
                (o, size) => format!("from {o}, {size} to download"),
            };
            Some(Update {
                id: r.to_string(),
                name: name.to_string(),
                current: if field(2).is_empty() {
                    String::new()
                } else {
                    now_version.to_string()
                },
                version: version.to_string(),
                summary,
                kind: Kind::Normal,
                origin,
            })
        })
        .collect()
}

fn updates_in(install: Install) -> Result<Vec<Update>, String> {
    let installed = parse_list(&run(&[
        "list",
        flag(install),
        "--columns=ref,name,version",
    ])?);
    if installed.is_empty() {
        return Ok(Vec::new());
    }
    let out = run(&[
        "remote-ls",
        "--updates",
        flag(install),
        "--columns=ref,name,version,branch,origin,download-size",
    ])?;
    Ok(parse_updates(&out, install, &installed))
}

pub fn updates() -> Result<Vec<Update>, String> {
    let mut out = updates_in(Install::System)?;
    out.extend(updates_in(Install::User)?);
    Ok(out)
}

/// Follows `flatpak update -y` output: the numbered operation table, then one
/// `Updating 2/3… ███▌ 41%  3.9 MB/s` line per operation step. (`--noninteractive`
/// picks Flatpak's quiet transaction, which prints no percentages.)
#[derive(Default)]
struct Follow {
    /// The table's operations in order: id and download size in bytes (0 when not shown).
    ops: Vec<(String, u64)>,
    last: Option<(usize, u32)>,
}

impl Follow {
    /// The operation `line` reports on and the transaction's fraction done, when it moved.
    /// Operations weigh by download size where the table gave every one a size.
    fn line(&mut self, line: &str) -> Option<(&str, f64)> {
        let line = line.trim();
        if let Some(op) = table_row(line) {
            self.ops.push(op);
            return None;
        }
        let (k, n, percent) = op_progress(line)?;
        if self.last == Some((k, percent)) {
            return None;
        }
        self.last = Some((k, percent));
        let here = percent.min(100) as f64 / 100.0;
        let sized = self.ops.len() >= n && self.ops.iter().all(|o| o.1 > 0);
        let fraction = if sized {
            let total: u64 = self.ops.iter().map(|o| o.1).sum();
            let before: u64 = self.ops[..k - 1].iter().map(|o| o.1).sum();
            (before as f64 + here * self.ops[k - 1].1 as f64) / total as f64
        } else {
            ((k - 1) as f64 + here) / n as f64
        };
        let id = self.ops.get(k - 1).map_or("", |o| o.0.as_str());
        Some((id, fraction))
    }

    /// The most the transaction downloads, in bytes, once the table gave every operation a size.
    fn download(&self) -> Option<u64> {
        (!self.ops.is_empty() && self.ops.iter().all(|o| o.1 > 0))
            .then(|| self.ops.iter().map(|o| o.1).sum())
    }
}

/// A row of the operation table, ` 1.\t   \torg.example.App\tstable\tu\tflathub\t< 1.2 MB`,
/// as its id and download size.
fn table_row(line: &str) -> Option<(String, u64)> {
    let (num, rest) = line.split_once('.')?;
    num.parse::<usize>().ok()?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    let id = words.first()?;
    let size = words
        .windows(2)
        .rev()
        .find_map(|w| Some(w[0].parse::<f64>().ok()? * unit(w[1])? as f64))
        .unwrap_or(0.0);
    Some((id.to_string(), size as u64))
}

/// GLib's size units, as `g_format_size` writes them.
fn unit(s: &str) -> Option<u64> {
    Some(match s {
        "byte" | "bytes" => 1,
        "kB" => 1_000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "TB" => 1_000_000_000_000,
        _ => return None,
    })
}

/// An operation's progress line as its number, the operation count and its percentage:
/// `Updating 2/3… 41%`, or `Installing… 7%` when there is one operation. The first line
/// of an operation has no percentage and counts as 0%.
fn op_progress(line: &str) -> Option<(usize, usize, u32)> {
    let rest = ["Updating", "Installing", "Uninstalling"]
        .iter()
        .find_map(|w| line.strip_prefix(w))?;
    let (k, n) = if rest.starts_with('…') || rest.starts_with("...") {
        (1, 1)
    } else {
        let count = rest.strip_prefix(' ')?.split(['…', '.', ' ']).next()?;
        let (k, n) = count.split_once('/')?;
        (k.parse().ok()?, n.parse().ok()?)
    };
    if k == 0 || k > n {
        return None;
    }
    let percent = rest
        .split_whitespace()
        .rev()
        .find_map(|w| w.strip_suffix('%')?.parse().ok())
        .unwrap_or(0);
    Some((k, n, percent))
}

/// Update `refs` in `install`, calling `step` with the id of the operation running and the
/// fraction of the whole update done. Any related ref they need (a newer runtime, an
/// extension) comes along.
pub fn install(
    install: Install,
    refs: &[String],
    step: &mut dyn FnMut(&str, f64),
) -> Result<(), String> {
    if refs.is_empty() {
        return Ok(());
    }
    let mut args = vec!["update", flag(install), "-y"];
    args.extend(refs.iter().map(String::as_str));
    follow(&args, step, &mut |_| {}, &mut |_| {})
}

/// Run `flatpak ARGS`, a transaction answered yes to every question, calling `step` as in
/// [`install`], `started` with a [`Pidfd`] for the process and `sized` with the most it
/// downloads, in bytes, once its first operation starts.
pub fn follow(
    args: &[&str],
    step: &mut dyn FnMut(&str, f64),
    started: &mut dyn FnMut(Pidfd),
    sized: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let mut cmd = Command::new(FLATPAK);
    cmd.args(args).env("LC_ALL", "C");
    follow_command(cmd, step, started, sized)
}

/// A handle on a child process, opened before it is reaped, so a signal sent through it after
/// the child is gone fails instead of reaching a reused pid.
#[derive(Debug)]
pub struct Pidfd(OwnedFd);

impl Pidfd {
    fn open(pid: u32) -> Option<Self> {
        // SAFETY: pidfd_open takes a pid and flags, no pointers.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        // SAFETY: a non-negative return is a new descriptor owned here.
        (fd >= 0).then(|| Self(unsafe { OwnedFd::from_raw_fd(fd as RawFd) }))
    }

    /// Send SIGINT; ESRCH once the process is reaped.
    pub fn interrupt(&self) -> io::Result<()> {
        // SAFETY: pidfd_send_signal takes a pidfd owned here, a signal, a null info pointer (it
        // then acts like kill) and no flags.
        let sent = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.0.as_raw_fd(),
                libc::SIGINT,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if sent == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

fn follow_command(
    mut cmd: Command,
    step: &mut dyn FnMut(&str, f64),
    started: &mut dyn FnMut(Pidfd),
    sized: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Flatpak could not be started: {e}."))?;
    if let Some(fd) = Pidfd::open(child.id()) {
        started(fd);
    }
    // stderr is read on a thread of its own, so a chatty one never
    // fills its pipe while stdout is followed here.
    let err = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut e, &mut s);
            s
        })
    });
    if let Some(out) = child.stdout.take() {
        let mut follow = Follow::default();
        let mut told = false;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some((id, fraction)) = follow.line(&line) {
                step(id, fraction);
            }
            if !told && follow.last.is_some() {
                told = true;
                if let Some(bytes) = follow.download() {
                    sized(bytes);
                }
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let stderr = err.and_then(|t| t.join().ok()).unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(complaint(&stderr))
    }
}

/// The apps and runtimes `install` holds, as full refs (`app/ID/ARCH/BRANCH`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refs {
    pub apps: Vec<String>,
    pub runtimes: Vec<String>,
    /// The runtimes the apps run on, as `runtime/ID/ARCH/BRANCH`.
    pub used: Vec<String>,
}

/// `flatpak list --app --columns=ref,runtime` and `flatpak list --runtime --columns=ref`.
fn parse_refs(apps: &str, runtimes: &str) -> Refs {
    let rows = |out: &str| -> Vec<Vec<String>> {
        out.lines()
            .map(|l| {
                l.split('\t')
                    .map(|f| f.trim().to_string())
                    .collect::<Vec<_>>()
            })
            .filter(|f| f.first().is_some_and(|r| !r.is_empty()))
            .collect()
    };
    let mut refs = Refs::default();
    for row in rows(apps) {
        refs.apps.push(format!("app/{}", row[0]));
        if let Some(rt) = row.get(1).filter(|r| !r.is_empty()) {
            refs.used.push(format!("runtime/{rt}"));
        }
    }
    refs.runtimes = rows(runtimes)
        .into_iter()
        .map(|row| format!("runtime/{}", row[0]))
        .collect();
    refs
}

/// What `install` holds now; None when Flatpak cannot list it.
pub fn refs(install: Install) -> Option<Refs> {
    let apps = run(&["list", flag(install), "--app", "--columns=ref,runtime"]).ok()?;
    let runtimes = run(&["list", flag(install), "--runtime", "--columns=ref"]).ok()?;
    Some(parse_refs(&apps, &runtimes))
}

/// How long a cancelled job's installation holds still before it is read as final, and the most
/// that is waited.
const QUIET: Duration = Duration::from_secs(2);
const SETTLE_CAP: Duration = Duration::from_secs(30);

/// What `install` holds once it stops changing: flatpak-system-helper finishes a Deploy under
/// way after a cancelled client is gone. A user install deploys in the client itself.
pub fn settled_refs(install: Install) -> Option<Refs> {
    let root = (install == Install::System)
        .then(|| installations().into_iter().next())
        .flatten();
    let look = || {
        let deploying = root.as_deref().map(deploys_under_way).unwrap_or_default();
        (refs(install), deploying)
    };
    let (refs, _) = until_still(
        look,
        |(_, deploying)| deploying.is_empty(),
        Instant::now,
        std::thread::sleep,
    );
    refs
}

/// Call `look` every half second until it returns the same for [`QUIET`] and `idle` holds, or
/// [`SETTLE_CAP`] passes: the last look.
fn until_still<T: PartialEq>(
    mut look: impl FnMut() -> T,
    idle: impl Fn(&T) -> bool,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
) -> T {
    let start = now();
    let mut last = look();
    let mut since = start;
    loop {
        if now().duration_since(start) >= SETTLE_CAP {
            return last;
        }
        sleep(Duration::from_millis(500));
        let next = look();
        if next != last {
            since = now();
            last = next;
        } else if idle(&last) && now().duration_since(since) >= QUIET {
            return last;
        }
    }
}

/// The checkouts under way in installation `root`: recent `.CHECKSUM-XXXXXX` folders beside a
/// ref's deployments, renamed to `CHECKSUM` once whole.
fn deploys_under_way(root: &std::path::Path) -> Vec<PathBuf> {
    let recent = |p: &std::path::Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|e| e < 2 * SETTLE_CAP))
    };
    let mut found = Vec::new();
    for kind in ["app", "runtime"] {
        let bases = subdirs(&root.join(kind))
            .flat_map(|id| subdirs(&id))
            .flat_map(|arch| subdirs(&arch));
        for base in bases {
            found.extend(subdirs(&base).filter(|d| {
                d.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                    && recent(d)
            }));
        }
    }
    found
}

fn subdirs(dir: &std::path::Path) -> impl Iterator<Item = PathBuf> + use<> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
}

/// How a cancelled install or uninstall ended, from what was installed before and after it.
#[derive(Debug, PartialEq, Eq)]
pub enum Settled {
    /// An app came or went before the cancel reached Flatpak: the job is done.
    Done,
    /// Nothing changed for the app; these runtimes came along before the cancel and go again.
    Undo(Vec<String>),
}

/// A runtime goes again only when it came during the job, no app in `after` runs on it, and it
/// extends (`ID.Locale`, `ID.GL.default`) no ref that stays.
pub fn settle(before: &Refs, after: &Refs, installing: bool) -> Settled {
    let new = |a: &[String], b: &[String]| -> Vec<String> {
        a.iter().filter(|r| !b.contains(r)).cloned().collect()
    };
    let moved = if installing {
        new(&after.apps, &before.apps)
    } else {
        new(&before.apps, &after.apps)
    };
    if !moved.is_empty() {
        return Settled::Done;
    }
    if !installing {
        return Settled::Undo(Vec::new());
    }
    let id = |r: &str| r.split('/').nth(1).unwrap_or_default().to_string();
    let loose: Vec<String> = new(&after.runtimes, &before.runtimes)
        .into_iter()
        .filter(|r| !after.used.contains(r))
        .collect();
    let staying: Vec<String> = (after.apps.iter().chain(&after.runtimes))
        .filter(|r| !loose.contains(r))
        .map(|r| id(r))
        .collect();
    let extends = |r: &str| {
        let own = id(r);
        staying.iter().any(|s| own.starts_with(&format!("{s}.")))
    };
    Settled::Undo(loose.iter().filter(|r| !extends(r)).cloned().collect())
}

/// Make this process the reaper of what a stopped `flatpak` leaves running: the `revokefs-fuse`
/// that serves a system install's download outlives a killed client, and its mount with it.
pub fn adopt_orphans() {
    // SAFETY: prctl's PR_SET_CHILD_SUBREAPER takes plain integers.
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
}

/// The `revokefs-fuse` processes this process adopted ([`adopt_orphans`]) end, unmounting their
/// download folders, which are then removed. Later orphans go to the usual reaper again.
pub fn sweep_orphans() {
    // SAFETY: prctl's PR_SET_CHILD_SUBREAPER takes plain integers.
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 0, 0, 0, 0) };
    let me = std::process::id();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return;
    };
    for pid in dir
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        if stat_parent(&stat) != Some(("revokefs-fuse", me)) {
            continue;
        }
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        end_child(pid);
        if let Some(mount) = mount_of(&cmdline) {
            let _ = std::fs::remove_dir(&mount);
            if let Some(cache) = mount.parent().filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("flatpak-cache-"))
            }) {
                let _ = std::fs::remove_dir(cache);
            }
        }
    }
}

/// SIGTERM child `pid` (FUSE then unmounts and exits) and reap it, SIGKILL after two seconds.
fn end_child(pid: i32) {
    // SAFETY: kill and waitpid take plain integers and a null status pointer; `pid` is this
    // process's own child, unreaped until here, so it names no other process.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
        let end = Instant::now() + Duration::from_secs(2);
        while libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) == 0 {
            if Instant::now() >= end {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// `/proc/PID/stat`'s command name and parent pid.
fn stat_parent(stat: &str) -> Option<(&str, u32)> {
    let (open, close) = (stat.find('(')?, stat.rfind(')')?);
    let comm = stat.get(open + 1..close)?;
    let ppid = stat
        .get(close + 1..)?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some((comm, ppid))
}

/// The mount folder, the last argument, of a `revokefs-fuse` command line.
fn mount_of(cmdline: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let last = cmdline.split(|&b| b == 0).rfind(|a| !a.is_empty())?;
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(last));
    path.is_absolute().then_some(path)
}

/// An installed app as `flatpak info` describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// `app/ID/ARCH/BRANCH`.
    pub reference: String,
    pub version: String,
    /// The remote it came from.
    pub origin: String,
    /// "34.2 MB", as Flatpak writes it.
    pub size: String,
}

/// `flatpak info`'s `Key: value` lines.
fn parse_info(out: &str) -> Info {
    let mut info = Info::default();
    for line in out.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "Ref" => info.reference = v,
            "Version" => info.version = v,
            "Origin" => info.origin = v,
            "Installed Size" | "Installed" => info.size = v,
            _ => {}
        }
    }
    info
}

/// What `install` holds of app `id`.
pub fn info(install: Install, id: &str) -> Result<Info, String> {
    run(&["info", flag(install), id]).map(|out| parse_info(&out))
}

/// The system and user installations, where Flatpak looks for them.
pub fn installations() -> Vec<PathBuf> {
    let var = |k| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let system = var("FLATPAK_SYSTEM_DIR").unwrap_or_else(|| "/var/lib/flatpak".into());
    let user = var("FLATPAK_USER_DIR").or_else(|| {
        var("XDG_DATA_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".local/share")))
            .map(|d| d.join("flatpak"))
    });
    std::iter::once(system).chain(user).collect()
}

/// Flatpak touches an installation's `.changed` after every change to it, whoever made it.
const CHANGED: &str = ".changed";

enum Watched {
    Installation,
    /// The parent of an installation not made yet, with the installation's name in it.
    Parent(OsString),
}

/// The installations' `.changed` files under inotify.
pub struct Watch {
    inotify: Inotify,
    watched: Vec<(WatchDescriptor, Watched, PathBuf)>,
}

impl Watch {
    pub fn new(dirs: Vec<PathBuf>) -> io::Result<Watch> {
        let mut w = Watch {
            inotify: Inotify::init()?,
            watched: Vec::new(),
        };
        for dir in dirs {
            if !w.add(&dir) {
                log::info!("{} is not watched for Flatpak changes", dir.display());
            }
        }
        Ok(w)
    }

    /// Watch `dir`, or the parent it will be made in; false when neither is there.
    fn add(&mut self, dir: &std::path::Path) -> bool {
        let mut watches = self.inotify.watches();
        let mask =
            WatchMask::ATTRIB | WatchMask::CREATE | WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO;
        if let Ok(wd) = watches.add(dir, mask) {
            self.watched
                .push((wd, Watched::Installation, dir.to_path_buf()));
            return true;
        }
        let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
            return false;
        };
        match watches.add(parent, WatchMask::CREATE | WatchMask::MOVED_TO) {
            Ok(wd) => {
                let parent = Watched::Parent(name.to_os_string());
                self.watched.push((wd, parent, dir.to_path_buf()));
                true
            }
            Err(_) => false,
        }
    }

    /// Whether the events waiting tell of a change. An installation that appears is one, and is
    /// watched from then on.
    fn read(&mut self) -> io::Result<bool> {
        let mut buf = [0u8; 4096];
        let mut changed = false;
        let mut made = Vec::new();
        loop {
            let events = match self.inotify.read_events(&mut buf) {
                Ok(e) => e,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            for e in events {
                if e.mask.contains(EventMask::IGNORED) {
                    continue;
                }
                let Some((_, watched, dir)) = self.watched.iter().find(|(wd, ..)| *wd == e.wd)
                else {
                    continue;
                };
                match watched {
                    Watched::Installation => changed |= e.name == Some(CHANGED.as_ref()),
                    Watched::Parent(name) if e.name == Some(name.as_os_str()) => {
                        made.push((e.wd.clone(), dir.clone()));
                        changed = true;
                    }
                    Watched::Parent(_) => {}
                }
            }
        }
        for (wd, dir) in made {
            self.watched.retain(|(w, _, d)| !(*w == wd && *d == dir));
            if !self.watched.iter().any(|(w, ..)| *w == wd) {
                let _ = self.inotify.watches().remove(wd);
            }
            self.add(&dir);
        }
        Ok(changed)
    }

    /// Whether an event is waiting, after at most `timeout` (forever with `None`).
    fn wait(&self, timeout: Option<Duration>) -> io::Result<bool> {
        let mut fd = libc::pollfd {
            fd: self.inotify.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.map_or(-1, |t| {
            t.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32
        });
        // SAFETY: `fd` is one valid pollfd that outlives the call.
        match unsafe { libc::poll(&mut fd, 1, ms) } {
            n if n >= 0 => Ok(n > 0),
            _ => match io::Error::last_os_error() {
                e if e.kind() == io::ErrorKind::Interrupted => Ok(false),
                e => Err(e),
            },
        }
    }

    /// Blocks until a change, then until `settle` passes with no other: one call per burst.
    /// Returns when the last change of the burst was heard.
    pub fn next(&mut self, settle: Duration) -> io::Result<Instant> {
        while !(self.wait(None)? && self.read()?) {}
        burst_end(Instant::now(), settle, Instant::now, |left| {
            Ok(self.wait(Some(left))? && self.read()?)
        })
    }
}

/// When the burst whose first change was heard at `first` ends: `changed(left)` waits up to
/// `left` for another change and says whether one came.
fn burst_end(
    first: Instant,
    settle: Duration,
    mut now: impl FnMut() -> Instant,
    mut changed: impl FnMut(Duration) -> io::Result<bool>,
) -> io::Result<Instant> {
    let mut last = first;
    loop {
        let left = (last + settle).saturating_duration_since(now());
        if left.is_zero() {
            return Ok(last);
        }
        if changed(left)? {
            last = now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_after_the_process_is_reaped_reaches_no_process() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 0"]);
        let mut fd = None;
        let ran = follow_command(cmd, &mut |_, _| {}, &mut |f| fd = Some(f), &mut |_| {});
        assert_eq!(ran, Ok(()));
        let fd = fd.expect("a pidfd for the started process");
        let sent = fd.interrupt().map_err(|e| e.raw_os_error());
        assert_eq!(
            sent,
            Err(Some(libc::ESRCH)),
            "the reaped process is not signalled"
        );
    }

    const LIST: &str = "org.example.Editor/x86_64/stable\tEditor\t2.1.0\n\
        org.freedesktop.Platform.GL.default/x86_64/26.08-extra\tMesa (Extra)\t26.1.8\n\
        org.freedesktop.Platform/x86_64/26.08\tFreedesktop Platform\tfreedesktop-sdk-26.08.0\n";

    const UPDATES: &str = "app/org.example.Editor/x86_64/stable\teditor\t2.2.0\tstable\tflathub\t1.2 MB\n\
        runtime/org.freedesktop.Platform.GL.default/x86_64/26.08-extra\tdefault\t26.1.9\t26.08-extra\tflathub\t150.5 MB\n\
        runtime/org.freedesktop.Platform/x86_64/26.08\tfreedesktop platform\t\t26.08\tflathub\t281.7 MB\n";

    #[test]
    fn remote_updates_take_their_names_and_versions_from_what_is_installed() {
        let u = parse_updates(UPDATES, Install::System, &parse_list(LIST));
        assert_eq!(u.len(), 3);
        assert_eq!(u[0].origin, Origin::App(Install::System));
        assert_eq!(u[0].id, "app/org.example.Editor/x86_64/stable");
        assert_eq!(u[0].name, "Editor");
        assert_eq!(u[0].version_cell(), "2.1.0 → 2.2.0");
        assert_eq!(u[0].summary, "from flathub, 1.2 MB to download");
        assert_eq!(u[1].origin, Origin::Runtime(Install::System));
        assert_eq!(u[1].name, "Mesa (Extra)");
        assert_eq!(u[2].version_cell(), "26.08");
        assert_eq!(u[2].name, "Freedesktop Platform");
    }

    #[test]
    fn a_ref_not_listed_as_installed_goes_by_its_id() {
        let u = parse_updates(
            "app/org.example.Tool/x86_64/master\t\t\tmaster\ttest\t\n",
            Install::User,
            &HashMap::new(),
        );
        assert_eq!(u[0].name, "org.example.Tool");
        assert_eq!(u[0].version, "master");
        assert_eq!(u[0].summary, "from test");
    }

    /// `flatpak update -y` output, piped, for two operations.
    const UPDATE_OUT: &str = "Looking for updates…\n\n\n \
         1.\t   \torg.example.Platform\tstable\tu\ttest\t< 300.0 MB\n \
         2.\t   \torg.example.Editor\tstable\tu\ttest\t< 100.0 MB (partial)\n\n\n\
        Updating 1/2…\n\
        Updating 1/2… 0%  0 bytes/s\n\
        Updating 1/2… ████▌ 50%  3.9 MB/s  00:30\n\
        Updating 1/2… ████▌ 50%  4.1 MB/s  00:29\n\
        Updating 2/2…\n\
        Updating 2/2… ██████████ 100%  4.0 MB/s\n\
        Changes complete.\n";

    fn followed(out: &str) -> Vec<(String, f64)> {
        let mut f = Follow::default();
        out.lines()
            .filter_map(|l| f.line(l).map(|(id, x)| (id.to_string(), x)))
            .collect()
    }

    #[test]
    fn update_progress_runs_over_the_whole_transaction_by_download_size() {
        let steps = followed(UPDATE_OUT);
        let want = [
            ("org.example.Platform", 0.0),
            ("org.example.Platform", 0.375),
            ("org.example.Editor", 0.75),
            ("org.example.Editor", 1.0),
        ];
        assert_eq!(steps.len(), want.len(), "{steps:?}");
        for ((id, f), (wid, wf)) in steps.iter().zip(want) {
            assert_eq!(id, wid);
            assert!((f - wf).abs() < 1e-9, "{id} at {f}, expected {wf}");
        }
    }

    #[test]
    fn a_flatpakref_install_tells_its_download_size_once_it_starts() {
        let out = include_str!("../tests/fixtures/flatpak-install-ref.txt");
        let mut f = Follow::default();
        let mut lines = out.lines();
        for line in lines.by_ref() {
            f.line(line);
            if f.last.is_some() {
                break;
            }
        }
        assert_eq!(f.download(), Some(30_400_000), "the table's \"< 30.4 MB\"");
        let last = lines.filter_map(|l| f.line(l).map(|(_, x)| x)).last();
        assert_eq!(last, Some(1.0));
        assert_eq!(Follow::default().download(), None, "no table, no size");
    }

    fn refs_of(apps: &[&str], runtimes: &[&str], used: &[&str]) -> Refs {
        let v = |l: &[&str]| l.iter().map(|r| r.to_string()).collect();
        Refs {
            apps: v(apps),
            runtimes: v(runtimes),
            used: v(used),
        }
    }

    #[test]
    fn a_cancelled_job_is_done_only_when_its_app_came_or_went() {
        let before = refs_of(&["app/a/x86_64/stable"], &["runtime/p/x86_64/1"], &[]);
        let half = refs_of(
            &["app/a/x86_64/stable"],
            &["runtime/p/x86_64/1", "runtime/q.Locale/x86_64/2"],
            &[],
        );
        assert_eq!(
            settle(&before, &half, true),
            Settled::Undo(vec!["runtime/q.Locale/x86_64/2".into()])
        );
        let whole = refs_of(
            &["app/a/x86_64/stable", "app/k/x86_64/stable"],
            &["runtime/p/x86_64/1", "runtime/q/x86_64/2"],
            &["runtime/q/x86_64/2"],
        );
        assert_eq!(settle(&before, &whole, true), Settled::Done);
        assert_eq!(settle(&before, &before, false), Settled::Undo(Vec::new()));
        let gone = refs_of(&[], &["runtime/p/x86_64/1"], &[]);
        assert_eq!(settle(&before, &gone, false), Settled::Done);
    }

    #[test]
    fn a_cancel_undoes_only_runtimes_nothing_installed_runs_on_or_extends() {
        let before = refs_of(
            &[
                "app/org.example.Old/x86_64/stable",
                "app/org.example.Other/x86_64/stable",
            ],
            &[
                "runtime/org.freedesktop.Platform/x86_64/25.08",
                "runtime/org.gnome.Platform/x86_64/48",
            ],
            &[
                "runtime/org.freedesktop.Platform/x86_64/25.08",
                "runtime/org.gnome.Platform/x86_64/48",
            ],
        );
        let after = refs_of(
            &[
                "app/org.example.Old/x86_64/stable",
                "app/org.example.Other/x86_64/stable",
            ],
            &[
                "runtime/org.freedesktop.Platform/x86_64/25.08",
                "runtime/org.freedesktop.Platform.GL.default/x86_64/25.08",
                "runtime/org.gnome.Platform/x86_64/48",
                "runtime/org.gnome.Platform/x86_64/49",
                "runtime/org.gnome.Platform.Locale/x86_64/49",
                "runtime/org.kde.Platform/x86_64/6.10",
                "runtime/org.kde.Platform.Locale/x86_64/6.10",
                "runtime/org.example.New.Locale/x86_64/stable",
            ],
            &[
                "runtime/org.freedesktop.Platform/x86_64/25.08",
                "runtime/org.gnome.Platform/x86_64/49",
            ],
        );
        assert_eq!(
            settle(&before, &after, true),
            Settled::Undo(vec![
                "runtime/org.kde.Platform/x86_64/6.10".into(),
                "runtime/org.kde.Platform.Locale/x86_64/6.10".into(),
                "runtime/org.example.New.Locale/x86_64/stable".into(),
            ]),
            "the GNOME runtime an app updated alongside now runs on, its Locale and the GL \
             extension of a runtime already there stay"
        );
    }

    #[test]
    fn installed_refs_and_the_runtimes_apps_run_on_read_from_flatpak_list() {
        let apps = "org.example.Editor/x86_64/stable\torg.gnome.Platform/x86_64/49\n\
                    org.example.Bare/x86_64/master\t\n";
        let runtimes = "org.gnome.Platform/x86_64/49\n\
                        org.freedesktop.Platform.GL.default/x86_64/25.08\n\n";
        assert_eq!(
            parse_refs(apps, runtimes),
            refs_of(
                &[
                    "app/org.example.Editor/x86_64/stable",
                    "app/org.example.Bare/x86_64/master"
                ],
                &[
                    "runtime/org.gnome.Platform/x86_64/49",
                    "runtime/org.freedesktop.Platform.GL.default/x86_64/25.08"
                ],
                &["runtime/org.gnome.Platform/x86_64/49"],
            )
        );
    }

    /// `until_still` on a clock that moves only when it sleeps, over `looks` (the last repeats)
    /// with `busy` looks counting as not idle: what it returned and when.
    fn still(looks: &[u32], busy: &[u32]) -> (u32, Duration) {
        let t0 = Instant::now();
        let clock = std::cell::Cell::new(t0);
        let mut i = 0;
        let got = until_still(
            || {
                let v = looks[i.min(looks.len() - 1)];
                i += 1;
                v
            },
            |v| !busy.contains(v),
            || clock.get(),
            |d| clock.set(clock.get() + d),
        );
        (got, clock.get() - t0)
    }

    #[test]
    fn a_cancelled_job_is_read_once_the_installation_holds_still() {
        assert_eq!(
            still(&[1, 1, 2], &[]),
            (2, Duration::from_millis(3000)),
            "an app the helper deploys after the client is gone is waited for"
        );
        assert_eq!(still(&[1], &[]), (1, QUIET));
        assert_eq!(
            still(&[1, 1, 1, 1, 1, 1, 2], &[1]),
            (2, Duration::from_millis(5000)),
            "a deploy under way is waited out even while the refs hold still"
        );
        assert_eq!(
            still(&[1], &[1]),
            (1, SETTLE_CAP),
            "a deploy that never ends is given up on"
        );
    }

    #[test]
    fn an_adopted_revokefs_is_known_by_its_stat_and_mount() {
        let stat = "187681 (revokefs-fuse) S 4242 187681 187681 0 -1 4194624 107 0 0 0";
        assert_eq!(stat_parent(stat), Some(("revokefs-fuse", 4242)));
        assert_eq!(stat_parent("12 (a) b) R 7 12"), Some(("a) b", 7)));
        assert_eq!(stat_parent("garbage"), None);
        let cmdline = b"/usr/lib/revokefs-fuse\0-o\0uid=1000\0--socket=3\0\
                        /var/lib/flatpak/repo/tmp/flatpak-cache-GSCPY3\0\
                        /var/tmp/flatpak-cache-JDYOY3/org.kde.Platform-7LEPY3\0";
        assert_eq!(
            mount_of(cmdline),
            Some(PathBuf::from(
                "/var/tmp/flatpak-cache-JDYOY3/org.kde.Platform-7LEPY3"
            ))
        );
        assert_eq!(mount_of(b"x\0relative\0"), None);
    }

    #[test]
    fn operations_without_sizes_weigh_the_same() {
        let out = " 1.\t   \torg.example.Tool\tmaster\tu\ttest\n \
             2.\t   \torg.example.Ext\tmaster\ti\ttest\n\
            Installing 2/2… 40%\n";
        assert_eq!(followed(out), [("org.example.Ext".to_string(), 0.7)]);
    }

    #[test]
    fn a_lone_operation_reads_its_own_percentage() {
        assert_eq!(op_progress("Updating… ██ 12%  1.0 MB/s"), Some((1, 1, 12)));
        assert_eq!(op_progress("Installing 3/4…"), Some((3, 4, 0)));
        assert_eq!(op_progress("Updating to rebased version"), None);
        assert_eq!(op_progress("Updating appstream data"), None);
        assert_eq!(op_progress("Nothing to do."), None);
    }

    /// As Flatpak marks a change: makes `.changed`, or sets its times by path (`touch -c` does
    /// not open the file).
    fn mark(dir: &std::path::Path) {
        let path = dir.join(CHANGED);
        if !path.exists() {
            return std::fs::write(&path, "").unwrap();
        }
        let ok = Command::new("touch").arg("-c").arg(&path).status();
        assert!(ok.is_ok_and(|s| s.success()));
    }

    /// Runs `burst_end` on a clock moved only by its waits; `gaps` are the ms between changes.
    fn burst(settle: Duration, gaps: &[u64]) -> (Duration, Duration, usize) {
        let t0 = Instant::now();
        let clock = std::cell::Cell::new(t0);
        let mut gaps = gaps.iter().map(|&ms| Duration::from_millis(ms)).peekable();
        let end = burst_end(
            t0,
            settle,
            || clock.get(),
            |left| {
                let next = gaps.next_if(|&gap| gap <= left);
                clock.set(clock.get() + next.unwrap_or(left));
                Ok(next.is_some())
            },
        )
        .unwrap();
        (end - t0, clock.get() - t0, gaps.count())
    }

    #[test]
    fn a_burst_ends_at_its_last_change_once_settle_passes_without_another() {
        let settle = Duration::from_millis(200);
        let ms = Duration::from_millis;
        assert_eq!(burst(settle, &[60, 60]), (ms(120), ms(320), 0));
        assert_eq!(burst(settle, &[200]), (ms(200), ms(400), 0));
        assert_eq!(
            burst(settle, &[60, 201, 10]),
            (ms(60), ms(260), 2),
            "a change after settle is the next burst's"
        );
    }

    #[test]
    fn marks_waiting_are_one_change_and_an_installation_made_later_is_watched() {
        let root =
            std::env::temp_dir().join(format!("reprise-flatpak-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (system, user) = (root.join("system"), root.join("user"));
        std::fs::create_dir_all(&system).unwrap();
        mark(&system);
        let mut w = Watch::new(vec![system.clone(), user.clone()]).unwrap();

        std::fs::write(system.join("other"), "").unwrap();
        assert!(!w.read().unwrap(), "only .changed tells of a change");

        for _ in 0..3 {
            mark(&system);
        }
        w.next(Duration::from_millis(50)).unwrap();
        assert!(
            !w.wait(Some(Duration::ZERO)).unwrap(),
            "one call took every mark"
        );

        std::fs::create_dir(&user).unwrap();
        assert!(w.read().unwrap(), "a new installation is a change");
        mark(&user);
        assert!(w.read().unwrap(), "and is watched from then on");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_failure_says_its_last_line() {
        assert_eq!(
            complaint("Looking for updates…\nerror: Unable to load summary from remote flathub\n"),
            "Unable to load summary from remote flathub."
        );
        assert_eq!(complaint(""), "Flatpak failed.");
    }

    #[test]
    fn info_reads_the_version_origin_and_size() {
        let out = "Space Cadet Pinball - Game engine for Space Cadet Pinball\n\n\
            \x20         ID: com.github.k4zmu2a.spacecadetpinball\n\
            \x20        Ref: app/com.github.k4zmu2a.spacecadetpinball/x86_64/stable\n\
            \x20    Version: 2.1.0\n\
            \x20     Origin: flathub\n\
            Installed Size: 34.2 MB\n\
            \x20    Subject: Merge pull request #42 (8734c8ac3a5a)\n";
        assert_eq!(
            parse_info(out),
            Info {
                reference: "app/com.github.k4zmu2a.spacecadetpinball/x86_64/stable".into(),
                version: "2.1.0".into(),
                origin: "flathub".into(),
                size: "34.2 MB".into(),
            }
        );
    }
}
