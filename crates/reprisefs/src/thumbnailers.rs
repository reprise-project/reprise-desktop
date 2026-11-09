//! The freedesktop `*.thumbnailer` programs and [`Runner::run`]: no shell, idle priority, inside
//! a [`Jail`], its process group killed past [`DEADLINE`].

use std::ffi::OsString;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// Thumbnailers running at once, counted across every Reprise process.
pub const SLOTS: usize = 2;

/// How long a thumbnailer may run before its process group is killed.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// The memory a jailed thumbnailer's scope may use, as systemd's `MemoryMax` reads it.
pub const MEMORY_MAX: &str = "512M";

/// The largest PNG a thumbnailer may leave, in bytes and on either side.
const MAX_PNG_BYTES: u64 = 16 << 20;
pub const MAX_SIDE: u32 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnailer {
    /// The file name without `.thumbnailer`: one in an earlier folder hides any later one.
    pub id: String,
    exec: String,
    try_exec: Option<String>,
    mimes: Vec<String>,
}

impl Thumbnailer {
    /// The `[Thumbnailer Entry]` of `text`; None without an `Exec` or a `MimeType`.
    pub fn parse(text: &str, id: &str) -> Option<Thumbnailer> {
        let kv = crate::apps::group_kv(text, "[Thumbnailer Entry]");
        let exec = kv.get("Exec")?.trim().to_string();
        let mimes: Vec<String> = kv
            .get("MimeType")?
            .split(';')
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| !m.is_empty())
            .collect();
        if crate::apps::exec_argv(&exec).is_empty() || mimes.is_empty() {
            return None;
        }
        let try_exec = kv.get("TryExec").map(|t| t.trim().to_string());
        Some(Thumbnailer {
            id: id.to_string(),
            exec,
            try_exec: try_exec.filter(|t| !t.is_empty()),
            mimes,
        })
    }

    /// A thumbnailer of Reprise's own that runs the command line `exec`, for no type.
    pub fn command(id: &str, exec: String) -> Thumbnailer {
        Thumbnailer {
            id: id.to_string(),
            exec,
            try_exec: None,
            mimes: Vec::new(),
        }
    }

    /// The program runs: its `TryExec`, else its first argument, is an executable file (by
    /// absolute path or on `PATH`).
    fn runnable(&self) -> bool {
        let program = self
            .try_exec
            .clone()
            .or_else(|| crate::apps::exec_argv(&self.exec).into_iter().next());
        program.is_some_and(|p| executable(&p))
    }

    /// The command line for a thumbnail of `input` fitted inside `size` px, written to `output`:
    /// `%i` is the input path, `%u` its URI, `%o` the output path, `%s` the size and `%%` a
    /// percent. Other field codes are dropped.
    pub fn argv(&self, input: &Path, output: &Path, size: u32) -> Vec<OsString> {
        crate::apps::split_exec(&self.exec, |code| match code {
            'i' => vec![input.as_os_str().to_os_string()],
            'u' => vec![crate::thumbnails::file_uri(input).into()],
            'o' => vec![output.as_os_str().to_os_string()],
            's' => vec![size.to_string().into()],
            _ => Vec::new(),
        })
    }
}

fn executable(program: &str) -> bool {
    resolve(program, std::env::var_os("PATH").as_deref()).is_some()
}

/// The executable file `program` names: itself when absolute, else the first on `path`.
fn resolve(program: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let is_exe = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let p = Path::new(program);
        return (p.is_absolute() && is_exe(p)).then(|| p.to_path_buf());
    }
    if program.is_empty() {
        return None;
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(program))
        .find(|p| p.is_absolute() && is_exe(p))
}

/// The runnable thumbnailers, the user's folder's first, then each of `$XDG_DATA_DIRS` in turn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Thumbnailers {
    list: Vec<Thumbnailer>,
}

impl Thumbnailers {
    /// Every `*.thumbnailer` in `dirs`, in name order within a folder; a name already seen in an
    /// earlier folder is skipped, and so is one whose program is not installed.
    pub fn load(dirs: &[PathBuf]) -> Thumbnailers {
        let mut seen = std::collections::HashSet::new();
        let mut list = Vec::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut files: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "thumbnailer"))
                .collect();
            files.sort();
            for path in files {
                let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !seen.insert(id.to_string()) {
                    continue;
                }
                let parsed = crate::apps::read_desktop_file(&path)
                    .ok()
                    .and_then(|text| Thumbnailer::parse(&text, id));
                list.extend(parsed.filter(Thumbnailer::runnable));
            }
        }
        Thumbnailers { list }
    }

    /// The thumbnailer for `mime`: the first naming it exactly, else the first naming its
    /// `major/*`.
    pub fn for_mime(&self, mime: &str) -> Option<&Thumbnailer> {
        let mime = mime.to_ascii_lowercase();
        let major = mime.split_once('/').map(|(m, _)| format!("{m}/*"));
        let names = |t: &&Thumbnailer, m: &str| t.mimes.iter().any(|x| x == m);
        (self.list.iter().find(|t| names(t, &mime)))
            .or_else(|| major.and_then(|m| self.list.iter().find(|t| names(t, &m))))
    }
}

/// `thumbnailers/` in `$XDG_DATA_HOME` (or `~/.local/share`), then in each of `$XDG_DATA_DIRS`.
pub fn dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    (home.into_iter())
        .chain(
            data_dirs
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        )
        .map(|d| d.join("thumbnailers"))
        .collect()
}

/// How long [`installed`] trusts its list before it stats the folders again.
const RESTAT: Duration = Duration::from_secs(2);

struct Installed {
    stamps: Vec<Option<SystemTime>>,
    checked: Instant,
    list: Arc<Thumbnailers>,
}

/// The thumbnailers in [`dirs`], read again when a folder appears, goes or changes.
pub fn installed() -> Arc<Thumbnailers> {
    static INSTALLED: Mutex<Option<Installed>> = Mutex::new(None);
    let mut held = reprisebus::mutex::lock(&INSTALLED);
    if let Some(i) = held.as_ref().filter(|i| i.checked.elapsed() < RESTAT) {
        return i.list.clone();
    }
    let dirs = dirs();
    let stamps: Vec<_> = (dirs.iter())
        .map(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok())
        .collect();
    match held.as_mut() {
        Some(i) if i.stamps == stamps => i.checked = Instant::now(),
        _ => {
            *held = Some(Installed {
                stamps,
                checked: Instant::now(),
                list: Arc::new(Thumbnailers::load(&dirs)),
            });
        }
    }
    held.as_ref().map(|i| i.list.clone()).unwrap_or_default()
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    /// Every slot stayed taken for the whole deadline: nothing ran, so try again later.
    Busy,
    /// The jail does not start here, or not around this file: nothing ran, so the file is not
    /// marked failed.
    NoJail(String),
    /// It ran and left no usable PNG, or was killed at the deadline.
    Failed(String),
}

/// How a thumbnailer is confined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Jail {
    /// bubblewrap inside a systemd user scope capped at [`MEMORY_MAX`]: no network, a new pid
    /// namespace, `/usr` and the input read-only, the scratch folder the one writable place.
    Bwrap {
        bwrap: PathBuf,
        systemd_run: PathBuf,
    },
    /// None: the program runs as the user. For tests of the run itself.
    Bare,
}

impl Jail {
    /// The bubblewrap jail when `bwrap` and `systemd-run` are on `path`.
    pub fn find(path: Option<&std::ffi::OsStr>) -> Option<Jail> {
        Some(Jail::Bwrap {
            bwrap: resolve("bwrap", path)?,
            systemd_run: resolve("systemd-run", path)?,
        })
    }

    /// The command line running `argv` here, reading `input` and writing in `scratch`.
    fn wrap(&self, argv: Vec<OsString>, input: &Path, scratch: &Path) -> Option<Vec<OsString>> {
        let Jail::Bwrap { bwrap, systemd_run } = self else {
            return Some(argv);
        };
        let (program, args) = argv.split_first()?;
        let program = resolve(program.to_str()?, std::env::var_os("PATH").as_deref())?;
        let mut out = vec![os(systemd_run)];
        out.extend(["--user", "--scope", "--quiet", "--collect"].map(os));
        out.extend([os("-p"), os(format!("MemoryMax={MEMORY_MAX}"))]);
        out.extend([os("-p"), os("MemorySwapMax=0"), os("--"), os(bwrap)]);
        out.extend(
            [
                "--unshare-all",
                "--die-with-parent",
                "--new-session",
                "--clearenv",
            ]
            .map(os),
        );
        out.extend(["--ro-bind", "/usr", "/usr"].map(os));
        for top in ["/bin", "/sbin", "/lib", "/lib64", "/lib32"] {
            match std::fs::symlink_metadata(top) {
                Ok(m) if m.is_symlink() => {
                    let to = std::fs::read_link(top).ok()?;
                    out.extend([os("--symlink"), os(to), os(top)]);
                }
                Ok(m) if m.is_dir() => out.extend(["--ro-bind", top, top].map(os)),
                _ => {}
            }
        }
        for etc in ["/etc/ld.so.cache", "/etc/fonts", "/var/cache/fontconfig"] {
            out.extend(["--ro-bind-try", etc, etc].map(os));
        }
        out.extend(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"].map(os));
        if !program.starts_with("/usr") {
            out.extend([os("--ro-bind"), os(&program), os(&program)]);
        }
        out.extend([os("--ro-bind"), os(input), os(input)]);
        out.extend([
            os("--bind"),
            os(scratch),
            os(scratch),
            os("--chdir"),
            os(scratch),
        ]);
        out.extend(["--setenv", "PATH", "/usr/bin"].map(os));
        out.extend([os("--setenv"), os("HOME"), os(scratch)]);
        for lang in ["LANG", "LC_ALL"] {
            if let Some(v) = std::env::var_os(lang) {
                out.extend([os("--setenv"), os(lang), v]);
            }
        }
        out.extend([os("--"), os(program)]);
        out.extend(args.iter().cloned());
        Some(out)
    }
}

fn os(s: impl AsRef<std::ffi::OsStr>) -> OsString {
    s.as_ref().to_os_string()
}

/// Where [`Runner::run`] takes its slots and makes its scratch folders, how long it waits, and
/// how it confines the thumbnailer.
pub struct Runner {
    pub slot_dir: PathBuf,
    pub scratch: PathBuf,
    pub deadline: Duration,
    pub jail: Jail,
    /// Whether the jail ran `true`, tried at the first run.
    starts: OnceLock<Result<(), String>>,
}

impl Runner {
    /// Slots and scratch folders in `dir`.
    pub fn new(dir: &Path, deadline: Duration, jail: Jail) -> Runner {
        Runner {
            slot_dir: dir.to_path_buf(),
            scratch: dir.to_path_buf(),
            deadline,
            jail,
            starts: OnceLock::new(),
        }
    }

    /// Slots and scratch folders in `$XDG_RUNTIME_DIR` (else the temp folder), each run in the
    /// bubblewrap [`Jail`]. None without bubblewrap: no thumbnailer runs unconfined.
    pub fn jailed() -> Option<Runner> {
        let jail = Jail::find(std::env::var_os("PATH").as_deref())?;
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(std::env::temp_dir);
        Some(Runner::new(&dir, DEADLINE, jail))
    }

    /// The PNG `t` makes of `input` fitted inside `size` px. Its header is checked (at most
    /// [`MAX_SIDE`] a side); decoding the pixels is the caller's.
    pub fn run(&self, t: &Thumbnailer, input: &Path, size: u32) -> Result<Vec<u8>, RunError> {
        let _slot = self.slot().ok_or(RunError::Busy)?;
        let scratch = Scratch::new(&self.scratch).map_err(|e| fail("no scratch folder", e))?;
        if let Err(why) = self
            .starts
            .get_or_init(|| self.jail_runs_true(&scratch.0, &scratch.0))
        {
            return Err(RunError::NoJail(why.clone()));
        }
        let output = scratch.0.join("thumbnail.png");
        let argv = (self.jail)
            .wrap(t.argv(input, &output, size), input, &scratch.0)
            .ok_or_else(|| RunError::Failed(format!("{}: no program to run", t.id)))?;
        let status = self.wait(&argv, &scratch.0).map_err(|e| fail(&t.id, e))?;
        if !status.success() {
            // bubblewrap's own setup (a bind it cannot make) fails before the thumbnailer runs.
            if let Err(why) = self.jail_runs_true(input, &scratch.0) {
                return Err(RunError::NoJail(why));
            }
            return Err(RunError::Failed(format!("{}: {status}", t.id)));
        }
        // The jail wrote this folder, so a symlink it left there is not followed out of it.
        let png = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(&output)
            .and_then(|f| crate::ops::read_opened(&f, MAX_PNG_BYTES))
            .map_err(|e| fail(&t.id, e))?;
        let header = png::Decoder::new(std::io::Cursor::new(&png))
            .read_info()
            .map_err(|e| fail(&t.id, e))?;
        let (w, h) = (header.info().width, header.info().height);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(RunError::Failed(format!("{}: a {w}x{h} PNG", t.id)));
        }
        Ok(png)
    }

    /// Whether the jail starts `true` around `input` and `scratch`; why not when it does not.
    fn jail_runs_true(&self, input: &Path, scratch: &Path) -> Result<(), String> {
        if self.jail == Jail::Bare {
            return Ok(());
        }
        let argv = (self.jail)
            .wrap(vec![os("true")], input, scratch)
            .ok_or("no true to run")?;
        match self.wait(&argv, scratch) {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!("bubblewrap or systemd-run: {status}")),
            Err(e) => Err(e),
        }
    }

    /// Run `argv` in `dir` at idle priority in a process group of its own, which is killed when
    /// it ends or at the deadline.
    fn wait(&self, argv: &[OsString], dir: &Path) -> Result<std::process::ExitStatus, String> {
        let (program, args) = argv.split_first().ok_or("no Exec")?;
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        // SAFETY: runs between fork and exec; setpriority and ioprio_set are plain system calls,
        // async-signal-safe, and nothing allocates.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, 19);
                libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, IOPRIO_IDLE);
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let group = child.id() as libc::pid_t;
        let end = Instant::now() + self.deadline;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < end => std::thread::sleep(POLL),
                waited => {
                    kill_group(group);
                    // A child stuck on a dead mount may outlive SIGKILL for a while.
                    std::thread::spawn(move || child.wait());
                    return Err(match waited {
                        Err(e) => e.to_string(),
                        _ => format!("no thumbnail within {:?}", self.deadline),
                    });
                }
            }
        };
        // Whatever it left running in its group goes with it.
        kill_group(group);
        Ok(status)
    }

    /// One of the [`SLOTS`] lock files, held until the file is dropped; None when all stay taken
    /// for the whole deadline.
    fn slot(&self) -> Option<std::fs::File> {
        let end = Instant::now() + self.deadline;
        loop {
            for n in 0..SLOTS {
                let path = self.slot_dir.join(format!("reprise-thumbnailer-{n}.lock"));
                let Ok(f) = std::fs::File::options()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .mode(0o600)
                    .open(&path)
                else {
                    continue;
                };
                use std::os::fd::AsRawFd;
                // SAFETY: `f` is an open file for the whole call.
                if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    return Some(f);
                }
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(POLL * 2);
        }
    }
}

const POLL: Duration = Duration::from_millis(25);
const IOPRIO_WHO_PROCESS: libc::c_int = 1;
/// `IOPRIO_CLASS_IDLE` in the class bits.
const IOPRIO_IDLE: libc::c_int = 3 << 13;

fn fail(what: &str, e: impl std::fmt::Display) -> RunError {
    RunError::Failed(format!("{what}: {e}"))
}

fn kill_group(group: libc::pid_t) {
    // SAFETY: a plain system call; the group may already be gone.
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
}

/// A private folder for one run, removed with what the thumbnailer left in it.
struct Scratch(PathBuf);

impl Scratch {
    fn new(parent: &Path) -> std::io::Result<Scratch> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = parent.join(format!("reprise-thumbnailer-{}-{n}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests;
