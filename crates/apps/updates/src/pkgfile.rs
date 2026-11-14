//! What a package file holds, read with the system's own tools in the C locale (`pacman -Qip`,
//! `dpkg-deb --field`), and an Arch package installed through `pkexec pacman -U`.

use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `.flatpakref`: an app in a repository.
    FlatpakRef,
    /// `.flatpak`: a single-file bundle.
    FlatpakBundle,
    /// `.pkg.tar.zst` (or `.xz`), for pacman.
    Pacman,
    Deb,
}

/// What `path`'s name says it is.
pub fn kind_of(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    Some(if name.ends_with(".flatpakref") {
        Kind::FlatpakRef
    } else if name.ends_with(".flatpak") {
        Kind::FlatpakBundle
    } else if name.ends_with(".pkg.tar.zst") || name.ends_with(".pkg.tar.xz") {
        Kind::Pacman
    } else if name.ends_with(".deb") {
        Kind::Deb
    } else {
        return None;
    })
}

/// A package file copied into a fresh folder only this user can open, which is deleted when the
/// copy drops. The sheet reads and installs the copy, so a file swapped after it opened changes
/// nothing.
#[derive(Debug)]
pub struct Copy {
    dir: PathBuf,
    /// The copy, under the file's own name.
    pub file: PathBuf,
    /// A detached `NAME.sig` lay beside the file and was copied with it.
    pub signed: bool,
}

impl Copy {
    /// The folder holding the copy.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The room a copy leaves free on `$XDG_RUNTIME_DIR`, a tmpfs in RAM that every session service
/// writes to.
const RUNTIME_MARGIN: u64 = 64 << 20;

/// Where a private copy of a `size`-byte file goes: `runtime` when it fits with
/// [`RUNTIME_MARGIN`] to spare by `free`, else `disk`.
pub fn copy_parent(
    runtime: &Path,
    disk: &Path,
    size: u64,
    free: impl Fn(&Path) -> Option<u64>,
) -> PathBuf {
    let fits = free(runtime).is_some_and(|f| size.saturating_add(RUNTIME_MARGIN) <= f);
    if fits { runtime } else { disk }.to_path_buf()
}

/// The bytes an unprivileged writer may still use on `dir`'s file system.
pub fn free_bytes(dir: &Path) -> Option<u64> {
    let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c` is NUL-terminated and `st` is written by statvfs before it is read.
    if unsafe { libc::statvfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs returned 0, so it filled `st`.
    let st = unsafe { st.assume_init() };
    Some(st.f_bavail.saturating_mul(st.f_frsize))
}

/// Copy the regular file `path`, and its `.sig` if there is one, into a new 0700 folder in
/// `parent`.
pub fn private_copy(path: &Path, parent: &Path) -> io::Result<Copy> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let dir = fresh_dir(parent)?;
    let mut copy = Copy {
        file: dir.join(name),
        dir,
        signed: false,
    };
    copy_regular(path, &copy.file)?;
    let sig = |p: &Path| {
        let mut s = p.as_os_str().to_owned();
        s.push(".sig");
        PathBuf::from(s)
    };
    copy.signed = copy_regular(&sig(path), &sig(&copy.file)).is_ok();
    Ok(copy)
}

fn fresh_dir(parent: &Path) -> io::Result<PathBuf> {
    use std::hash::{BuildHasher, Hasher};
    use std::os::unix::fs::DirBuilderExt;
    let mut tries = 0;
    loop {
        let tag = std::hash::RandomState::new().build_hasher().finish();
        let dir = parent.join(format!("reprise-install-{tag:016x}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tries < 16 => tries += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Copy `from` into the new file `to`; a FIFO or device is refused before it is read.
fn copy_regular(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut src = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(from)?;
    if !src.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut dst = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)?;
    io::copy(&mut src, &mut dst)?;
    Ok(())
}

/// A package file's name, version, installed size in bytes and who built it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub name: String,
    pub version: String,
    pub size: Option<u64>,
    pub packager: String,
}

/// A size as `pacman` or GLib writes it ("812.18 KiB", "34.2 MB" with a no-break space), in bytes.
pub fn size_bytes(text: &str) -> Option<u64> {
    let (n, unit) = text.trim().split_once(char::is_whitespace)?;
    let n: f64 = n.parse().ok()?;
    let mul = match unit.trim() {
        "B" | "bytes" | "byte" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "kB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        _ => return None,
    };
    Some((n * mul) as u64)
}

/// `pacman -Qip`'s `Key  : value` lines.
fn parse_pacman(out: &str) -> Facts {
    let mut f = Facts::default();
    for line in out.lines() {
        let Some((k, v)) = line.split_once(" : ") else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "Name" => f.name = v.into(),
            "Version" => f.version = v.into(),
            "Installed Size" => f.size = size_bytes(v),
            "Packager" => f.packager = v.into(),
            _ => {}
        }
    }
    f
}

/// `dpkg-deb --field`'s `Key: value` lines; `Installed-Size` is in KiB.
fn parse_deb(out: &str) -> Facts {
    let mut f = Facts::default();
    for line in out.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k {
            "Package" => f.name = v.into(),
            "Version" => f.version = v.into(),
            "Installed-Size" => f.size = v.parse::<u64>().ok().map(|k| k * 1024),
            "Maintainer" => f.packager = v.into(),
            _ => {}
        }
    }
    f
}

fn output(program: &str, args: &[&OsStr]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| format!("{program} is not installed, so Reprise cannot read this package."))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().map(str::trim).rfind(|l| !l.is_empty());
        Err(line.unwrap_or("The package could not be read.").to_string())
    }
}

/// Read package file `path` of `kind` (`Pacman` or `Deb`).
pub fn read(path: &Path, kind: Kind) -> Result<Facts, String> {
    let p = path.as_os_str();
    let os = OsStr::new;
    let facts = match kind {
        Kind::Pacman => parse_pacman(&output("pacman", &[os("-Qip"), os("--"), p])?),
        Kind::Deb => {
            let fields = ["Package", "Version", "Installed-Size", "Maintainer"];
            let mut args = vec![os("--field"), p];
            args.extend(fields.map(os));
            parse_deb(&output("dpkg-deb", &args)?)
        }
        Kind::FlatpakRef | Kind::FlatpakBundle => return Err("Not a package file.".into()),
    };
    if facts.name.is_empty() {
        return Err("The package does not name itself.".into());
    }
    Ok(facts)
}

/// Whether `pacman -U` checks a package file's `.sig`: false under `LocalFileSigLevel = Never`
/// (`pacman-conf` prints the resolved level), or when `pacman-conf` cannot say.
pub fn local_sigs_checked() -> bool {
    output("pacman-conf", &[OsStr::new("LocalFileSigLevel")]).is_ok_and(|o| sigs_checked(&o))
}

/// `pacman-conf LocalFileSigLevel`'s output names a level that checks package signatures.
pub(crate) fn sigs_checked(level: &str) -> bool {
    let mut words = level.split_whitespace().peekable();
    words.peek().is_some() && !words.any(|w| matches!(w, "Never" | "PackageNever"))
}

/// The packages `pacman -U path` would install, by name: the file's and those it pulls in.
pub fn pacman_targets(path: &Path) -> Result<Vec<String>, String> {
    let os = OsStr::new;
    let out = output(
        "pacman",
        &[
            os("-Up"),
            os("--print-format"),
            os("%n"),
            os("--"),
            path.as_os_str(),
        ],
    )?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// Why `pkexec pacman -U` did not install.
#[derive(Debug, PartialEq, Eq)]
pub enum PacmanError {
    /// pkexec's password prompt was dismissed.
    Dismissed,
    Failed(String),
}

/// pkexec's exit status when its prompt was dismissed.
const PKEXEC_DISMISSED: i32 = 126;

/// `line` without terminal escapes (colours, cursor moves, titles) or other control characters.
fn plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters, then one final byte in @..~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// "checking package integrity" as "Checking package integrity".
fn sentence(text: &str) -> String {
    let mut s = text.trim().to_string();
    if let Some(first) = s.get(..1) {
        s.replace_range(..1, &first.to_uppercase());
    }
    s
}

/// `(2/3) rest` as 2, 3 and `rest`.
fn counted(line: &str) -> Option<(usize, usize, &str)> {
    let (count, rest) = line.strip_prefix('(')?.split_once(") ")?;
    let (k, n) = count.split_once('/')?;
    Some((k.trim().parse().ok()?, n.trim().parse().ok()?, rest))
}

/// The bar's shares: downloads (when there are any), then pacman's checks, then installing.
const DOWNLOAD_SHARE: f64 = 0.4;
const CHECK_SHARE: f64 = 0.2;
/// pacman's checks before it installs, in order.
const CHECKS: [&str; 5] = [
    "checking keys in keyring",
    "checking package integrity",
    "loading package files",
    "checking for file conflicts",
    "checking available disk space",
];

/// Follows `pacman -U` on a terminal: download bars (` foo-1-1-x86_64  9.5 KiB ... [---]  89%`,
/// `Total (1/2) ... 40%`), check bars, then `(2/3) installing foo [---]  45%` and the hooks'
/// `(1/3) Arming ConditionNeedsUpdate...`. Without a terminal only the `...` steps come.
#[derive(Default)]
struct PacmanFollow {
    /// Packages are downloaded first.
    downloads: bool,
    /// A `Total` line drives the download share, so single files no longer do.
    totals: bool,
    /// The bar's fraction; it never goes back.
    done: f64,
    /// What was last reported: the bar in thousandths and the step.
    shown: (Option<u32>, Option<String>),
}

impl PacmanFollow {
    /// Follow the lines in `pending` that a CR or LF ended, leaving the rest there.
    fn feed(&mut self, pending: &mut Vec<u8>, step: &mut dyn FnMut(f64, Option<String>)) {
        while let Some(end) = pending.iter().position(|&b| b == b'\r' || b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            if let Some((f, s)) = self.line(&String::from_utf8_lossy(&line)) {
                step(f, s);
            }
        }
    }

    /// The bar's fraction, and the step to show when it changed, when either moved with `line`.
    fn line(&mut self, line: &str) -> Option<(f64, Option<String>)> {
        let line = plain(line);
        let line = line.trim();
        if line.starts_with("Total Download Size:") {
            self.downloads = true;
            return None;
        }
        let doing = match bar(line) {
            Some((label, percent)) => self.bar_step(label, percent),
            None if line.starts_with("::") => return None,
            None => {
                let step = line.strip_suffix("...")?;
                Some(sentence(counted(step).map_or(step, |(_, _, rest)| rest)))
            }
        };
        let doing = doing.filter(|d| self.shown.1.as_ref() != Some(d));
        let permille = Some((self.done * 1000.0).round() as u32);
        if doing.is_none() && self.shown.0 == permille {
            return None;
        }
        self.shown.0 = permille;
        if doing.is_some() {
            self.shown.1.clone_from(&doing);
        }
        Some((self.done, doing))
    }

    /// Move the bar for a bar line labelled `label` at `percent`; the step it names, if any.
    fn bar_step(&mut self, label: &str, percent: u32) -> Option<String> {
        let here = f64::from(percent.min(100)) / 100.0;
        let downloads = if self.downloads { DOWNLOAD_SHARE } else { 0.0 };
        let mut advance = |to: f64| self.done = self.done.max(to.clamp(0.0, 1.0));
        if let Some((k, n, what)) = counted(label) {
            let installs = [
                "installing",
                "upgrading",
                "reinstalling",
                "downgrading",
                "removing",
            ];
            if let Some(i) = CHECKS.iter().position(|c| what.starts_with(c)) {
                let share = (i as f64 + here) / CHECKS.len() as f64;
                advance(downloads + CHECK_SHARE * share);
            } else if installs.iter().any(|w| what.starts_with(w)) && k > 0 && k <= n {
                let base = downloads + CHECK_SHARE;
                advance(base + (1.0 - base) * ((k - 1) as f64 + here) / n as f64);
            }
            return Some(sentence(what));
        }
        if !label.split_whitespace().any(|w| w.ends_with("/s")) {
            return None;
        }
        self.downloads = true;
        if label.starts_with("Total ") {
            self.totals = true;
            self.done = self.done.max(DOWNLOAD_SHARE * here);
            return None;
        }
        if !self.totals {
            self.done = self.done.max(DOWNLOAD_SHARE * here);
        }
        let file = label.split_whitespace().next()?;
        Some(format!("Downloading {file}"))
    }
}

/// A progress bar line, `label [bar]  45%`, as its label and percentage.
fn bar(line: &str) -> Option<(&str, u32)> {
    let rest = line.strip_suffix('%')?;
    let percent = rest.rsplit(char::is_whitespace).next()?.parse().ok()?;
    let (label, _) = rest.rsplit_once('[')?;
    Some((label.trim(), percent))
}

/// A pseudo-terminal pair (master, slave), close-on-exec and `cols` wide, so pacman draws its bars
/// and leaves its names whole.
fn open_pty(cols: u16) -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut m, mut s) = (-1, -1);
    let size = libc::winsize {
        ws_row: 24,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty writes two descriptors into the two locals; the name and termios pointers
    // may be null and the window size outlives the call.
    let rc = unsafe {
        libc::openpty(
            &mut m,
            &mut s,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors were just opened and are owned here.
    let fds = unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) };
    for fd in [&fds.0, &fds.1] {
        // SAFETY: fcntl takes a descriptor owned above and plain flags.
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Ok(fds)
}

/// Install `path` with `pkexec pacman -U`, as root once polkit allows it, calling `step` with the
/// bar's fraction and each step pacman reports. pacman writes to a pseudo-terminal so it draws its
/// progress bars, or to a pipe, with steps only, where none opens.
pub fn pacman_install(
    path: &Path,
    step: &mut dyn FnMut(f64, Option<String>),
) -> Result<(), PacmanError> {
    use std::io::Read;
    let pty = open_pty(200)
        .inspect_err(|e| log::warn!("no terminal for pacman: {e}"))
        .ok();
    let (mut child, master) = {
        let mut cmd = Command::new("pkexec");
        cmd.args(["pacman", "-U", "--noconfirm", "--color", "never", "--"])
            .arg(path)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::piped());
        let master = match pty {
            Some((master, slave)) => {
                cmd.stdout(Stdio::from(slave));
                Some(master)
            }
            None => {
                cmd.stdout(Stdio::piped());
                None
            }
        };
        // cmd drops its copy of the slave here, so reading the master ends when pacman exits.
        let child = cmd
            .spawn()
            .map_err(|e| PacmanError::Failed(format!("pkexec could not be started: {e}.")))?;
        (child, master)
    };
    // stderr drains on its own thread, so a chatty one never fills its pipe.
    let err = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    let out: Option<Box<dyn Read>> = match master {
        Some(m) => Some(Box::new(std::fs::File::from(m))),
        None => child.stdout.take().map(|o| Box::new(o) as Box<dyn Read>),
    };
    if let Some(mut out) = out {
        let mut follow = PacmanFollow::default();
        let mut buf = [0u8; 4096];
        let mut pending = Vec::new();
        loop {
            let n = match out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // A pty master reads EIO once the last slave closes.
                Err(_) => break,
            };
            pending.extend_from_slice(&buf[..n]);
            follow.feed(&mut pending, step);
        }
    }
    let status = child
        .wait()
        .map_err(|e| PacmanError::Failed(e.to_string()))?;
    let stderr = err.and_then(|t| t.join().ok()).unwrap_or_default();
    match status.code() {
        Some(0) => Ok(()),
        Some(PKEXEC_DISMISSED) => Err(PacmanError::Dismissed),
        _ => {
            let line = stderr.lines().map(str::trim).rfind(|l| !l.is_empty());
            let line = line.map(|l| l.strip_prefix("error: ").unwrap_or(l));
            Err(PacmanError::Failed(
                line.unwrap_or("pacman did not install the package.")
                    .to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_says_what_kind_of_file_it_is() {
        let k = |n: &str| kind_of(Path::new(n));
        assert_eq!(k("/d/foot-1.28.0-2-x86_64.pkg.tar.zst"), Some(Kind::Pacman));
        assert_eq!(k("/d/old-1-1-any.pkg.tar.xz"), Some(Kind::Pacman));
        assert_eq!(k("/d/hello_2.10-3_amd64.DEB"), Some(Kind::Deb));
        assert_eq!(k("/d/org.example.App.flatpakref"), Some(Kind::FlatpakRef));
        assert_eq!(k("/d/org.example.App.flatpak"), Some(Kind::FlatpakBundle));
        assert_eq!(k("/d/backup.tar.zst"), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("reprise-pkgfile-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_private_copy_keeps_what_the_file_held_when_it_opened() {
        use std::os::unix::fs::PermissionsExt;
        let from = scratch("copy-from");
        let to = scratch("copy-to");
        let pkg = from.join("foot-1.28.0-2-x86_64.pkg.tar.zst");
        std::fs::write(&pkg, "first").unwrap();
        let copy = private_copy(&pkg, &to).unwrap();
        std::fs::write(&pkg, "swapped").unwrap();
        assert_eq!(copy.file.file_name(), pkg.file_name());
        assert_eq!(std::fs::read_to_string(&copy.file).unwrap(), "first");
        assert!(!copy.signed);
        let dir = copy.file.parent().unwrap().to_path_buf();
        assert_eq!(dir.parent(), Some(to.as_path()));
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
        drop(copy);
        assert!(!dir.exists(), "the copy's folder goes with it");

        std::fs::write(from.join("foot-1.28.0-2-x86_64.pkg.tar.zst.sig"), "sig").unwrap();
        let signed = private_copy(&pkg, &to).unwrap();
        assert!(signed.signed);
        let sig = signed
            .file
            .with_file_name("foot-1.28.0-2-x86_64.pkg.tar.zst.sig");
        assert_eq!(std::fs::read_to_string(sig).unwrap(), "sig");
        drop(signed);

        let fifo = from.join("pipe.deb");
        let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo reads the NUL-terminated path only during the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let refused = private_copy(&fifo, &to).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            std::fs::read_dir(&to).unwrap().count(),
            0,
            "no folder left behind"
        );
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    #[test]
    fn a_copy_that_would_crowd_the_runtime_dir_goes_to_the_disk() {
        let (runtime, disk) = (Path::new("/run/user/1000"), Path::new("/var/tmp"));
        let room = |free: u64| move |_: &Path| Some(free);
        let gib = 1 << 30;
        assert_eq!(copy_parent(runtime, disk, 10 << 20, room(gib)), runtime);
        assert_eq!(copy_parent(runtime, disk, gib, room(gib)), disk);
        assert_eq!(
            copy_parent(runtime, disk, gib - (32 << 20), room(gib)),
            disk,
            "the margin stays free"
        );
        assert_eq!(copy_parent(runtime, disk, 1, |_| None), disk, "no statvfs");
        assert!(free_bytes(&std::env::temp_dir()).is_some());
    }

    /// What following `out` reports: the bar's fraction and each new step.
    fn followed(out: &[u8]) -> Vec<(f64, Option<String>)> {
        let mut seen = Vec::new();
        let mut pending = out.to_vec();
        PacmanFollow::default().feed(&mut pending, &mut |f, s| seen.push((f, s)));
        seen
    }

    fn steps(seen: &[(f64, Option<String>)]) -> Vec<&str> {
        seen.iter().filter_map(|(_, s)| s.as_deref()).collect()
    }

    /// The bar's fraction when `step` was first shown.
    fn at(seen: &[(f64, Option<String>)], step: &str) -> f64 {
        let i = seen.iter().position(|(_, s)| s.as_deref() == Some(step));
        seen[i.unwrap_or_else(|| panic!("{step} is shown"))].0
    }

    fn rises(seen: &[(f64, Option<String>)]) -> bool {
        seen.windows(2).all(|w| w[0].0 <= w[1].0)
    }

    #[test]
    fn pacman_steps_read_as_sentences() {
        let seen = followed(
            b"installing htop...\nchecking package integrity...\n\
              :: Processing package changes...\nPackages (1) htop-3.5.3-1\n\
              (1/3) Arming ConditionNeedsUpdate...\n",
        );
        assert_eq!(
            steps(&seen),
            [
                "Installing htop",
                "Checking package integrity",
                "Arming ConditionNeedsUpdate"
            ]
        );
    }

    #[test]
    fn a_local_package_fills_the_bar_through_the_checks_and_the_install() {
        let seen = followed(include_bytes!("../tests/fixtures/pacman-local.txt"));
        let shown = steps(&seen);
        assert_eq!(
            shown[..3],
            [
                "Loading packages",
                "Resolving dependencies",
                "Looking for conflicting packages"
            ]
        );
        assert!(shown.contains(&"Checking package integrity"), "{shown:?}");
        assert!(
            !shown.iter().any(|s| s.starts_with("Downloading")),
            "{shown:?}"
        );
        assert!(rises(&seen), "the bar never goes back: {seen:?}");
        assert_eq!(at(&seen, "Checking keys in keyring"), 0.0);
        let installing = at(&seen, "Installing reprise-g5-sample");
        assert!(
            (installing - 0.2).abs() < 1e-9,
            "the install starts after the checks: {installing}"
        );
        assert_eq!(seen.last().map(|l| l.0), Some(1.0));
        assert_eq!(
            shown.last(),
            Some(&"Performing snapper post snapshots for the following configurations")
        );
    }

    #[test]
    fn a_package_with_a_download_fills_the_first_share_downloading() {
        let seen = followed(include_bytes!(
            "../tests/fixtures/pacman-download-candy.txt"
        ));
        let shown = steps(&seen);
        assert!(rises(&seen), "{seen:?}");
        assert!((at(&seen, "Downloading sl-5.05-6-x86_64") - 0.4 * 0.89).abs() < 1e-9);
        assert!(
            (at(&seen, "Checking keys in keyring") - 0.4).abs() < 1e-9,
            "{seen:?}"
        );
        assert!(at(&seen, "Installing sl") >= 0.6, "{seen:?}");
        assert!(
            (at(&seen, "Installing reprise-g5-sample") - 0.8).abs() < 1e-9,
            "the second of two installs starts half way through the last share: {seen:?}"
        );
        assert_eq!(seen.last().map(|l| l.0), Some(1.0));
        assert!(
            shown
                .iter()
                .all(|s| !s.contains('\u{1b}') && !s.contains('[')),
            "{shown:?}"
        );
    }

    #[test]
    fn lines_that_do_not_parse_move_nothing() {
        let seen = followed(
            b"(x/y) installing a [##] 50%\n(3/2) installing b [#] 50%\n[\n%\n] 7%\n\
              \x1b[1;33m\x1b]0;t\x07\n==> root: 12\n:: Proceed with installation? [Y/n] \n",
        );
        assert!(seen.iter().all(|(f, _)| *f == 0.0), "{seen:?}");
        assert_eq!(steps(&seen), ["Installing b"]);
    }

    #[test]
    fn escapes_are_dropped() {
        assert_eq!(
            plain("\x1b[1F a \x1b[1;33mC\x1b[m]\x1b]3008;x=1\x1b\\ 9%\0"),
            " a C] 9%"
        );
    }

    #[test]
    fn sizes_read_in_either_units() {
        assert_eq!(size_bytes("812.18 KiB"), Some(831_672));
        assert_eq!(size_bytes("34.2\u{a0}MB"), Some(34_200_000));
        assert_eq!(size_bytes("2.00 MiB"), Some(2 * 1024 * 1024));
        assert_eq!(size_bytes("lots"), None);
    }

    #[test]
    fn pacman_and_dpkg_name_the_package() {
        let pacman = "Name            : foot\n\
                      Version         : 1.28.0-2\n\
                      Description     : Fast, lightweight, and minimalistic Wayland terminal\n\
                      Installed Size  : 812.18 KiB\n\
                      Packager        : CachyOS <admin@cachyos.org>\n\
                      Build Date      : Tue 01 Sep 2026 10:00:00 AM UTC\n";
        assert_eq!(
            parse_pacman(pacman),
            Facts {
                name: "foot".into(),
                version: "1.28.0-2".into(),
                size: Some(831_672),
                packager: "CachyOS <admin@cachyos.org>".into(),
            }
        );
        let deb = "Package: hello\nVersion: 2.10-3\nInstalled-Size: 280\n\
                   Maintainer: Santiago Vila <sanvila@debian.org>\n";
        assert_eq!(
            parse_deb(deb),
            Facts {
                name: "hello".into(),
                version: "2.10-3".into(),
                size: Some(280 * 1024),
                packager: "Santiago Vila <sanvila@debian.org>".into(),
            }
        );
    }
}
