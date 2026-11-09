//! The Login Manager page's host: reads the machine settings when the page first opens, and
//! saves them on Apply through `pkexec reprise-login-settings`, off the loop.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use repriselogin::front::FINGERPRINT;
use repriselogin::machine::{MACHINE_CONFIG, MachineConfig, file_tag, save_request};
use repriselogin::users::Source;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// The helper that replaces `MACHINE_CONFIG` with the TOML on its stdin, as root.
const HELPER: &str = "/usr/lib/reprise/reprise-login-settings";
const PKEXEC: &str = "/usr/bin/pkexec";
/// How often the open page reads the front's fingerprint again.
const POLL: Duration = Duration::from_secs(2);

/// The files the page reads.
#[derive(Clone, Debug)]
pub(crate) struct Files {
    pub(crate) config: PathBuf,
    pub(crate) fingerprint: PathBuf,
}

impl Default for Files {
    fn default() -> Self {
        Files {
            config: MACHINE_CONFIG.into(),
            fingerprint: FINGERPRINT.into(),
        }
    }
}

/// The machine settings as read and as the page has them; `users` are those it offers for
/// automatic logon.
pub(crate) struct Login {
    pub(crate) loaded: MachineConfig,
    pub(crate) want: MachineConfig,
    users: Vec<String>,
    fingerprint: Option<String>,
}

/// How a save ended; each but `Failed` carries the file as it reads after.
enum Ended {
    Saved(Read),
    Failed(String),
    /// The file no longer read as the page loaded it, so nothing was saved.
    Changed(Read),
}

type Read = Result<MachineConfig, String>;

/// A save under way and the settings it sent.
pub(crate) struct Saving {
    sent: MachineConfig,
    rx: Receiver<Ended>,
    /// An OK, Cancel or close waits on it.
    pub(crate) quit: bool,
    /// Cancel or a close: the sheet closes when it ends, saved or not, and nothing follows it.
    discard: bool,
}

const NOT_SAVED: &str = "The login manager's settings could not be saved.";
const NOT_AUTHORIZED: &str = "The change was not authorized.";
const CHANGED: &str = "Your changes were not saved: the login manager's settings changed since \
                       this page opened. The page now shows the new settings; make your changes \
                       again.";

/// Why [`apply`] saved nothing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unsaved {
    /// The file is no longer the one tagged `tag`.
    Changed,
    /// The message to show.
    Failed(String),
}

/// Save `cfg` through `run`, which hands the save request to the helper's stdin and returns how
/// it exited. The helper replaces only a file that still has `tag`.
pub(crate) fn apply(
    cfg: &MachineConfig,
    tag: &str,
    run: &dyn Fn(&str) -> io::Result<Output>,
) -> Result<(), Unsaved> {
    let out = run(&save_request(tag, &cfg.to_toml()))
        .map_err(|e| Unsaved::Failed(format!("{NOT_SAVED}\n\n{e}")))?;
    let why = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
    let with_why = |head: &str| match why.is_empty() {
        true => head.to_string(),
        false => format!("{head}\n\n{why}"),
    };
    // The helper's exits: 0 saved, 1 the write failed, 2 refused (a reason per line), 3 the file
    // changed; pkexec's 126: the prompt dismissed, 127: polkit said "Not authorized" or pkexec
    // failed.
    let why = match out.status.code() {
        Some(0) => return Ok(()),
        Some(3) => return Err(Unsaved::Changed),
        Some(2) => with_why("The login manager refused the settings:"),
        Some(126) => NOT_AUTHORIZED.into(),
        Some(127) if why.contains("Not authorized") => NOT_AUTHORIZED.into(),
        Some(127) => with_why(NOT_AUTHORIZED),
        _ => with_why(NOT_SAVED),
    };
    Err(Unsaved::Failed(why))
}

/// `pkexec` running [`HELPER`] with `input` on its stdin.
pub(crate) fn pkexec(input: &str) -> io::Result<Output> {
    run_helper(Path::new(HELPER), input)
}

/// `pkexec` running `helper`; a missing helper is named without asking for a password.
pub(crate) fn run_helper(helper: &Path, input: &str) -> io::Result<Output> {
    if !helper.exists() {
        let why = format!("{} is not installed.", helper.display());
        return Err(io::Error::new(io::ErrorKind::NotFound, why));
    }
    let mut child = Command::new(PKEXEC)
        .arg(helper)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let wrote = child
        .stdin
        .take()
        .map(|mut s| s.write_all(input.as_bytes()));
    let out = child.wait_with_output()?;
    match wrote {
        // A refusal before the helper read its input closes the pipe; its exit tells why.
        Some(Err(e)) if out.status.success() => Err(e),
        _ => Ok(out),
    }
}

/// The machine settings in `path`, the defaults when it is missing; the message to show when
/// it could not be read or holds anything the page would not save back.
fn read(path: &Path) -> Read {
    read_tagged(path).map(|(config, _)| config)
}

/// [`read`], with the file's tag for the save that follows.
fn read_tagged(path: &Path) -> Result<(MachineConfig, String), String> {
    let at = path.display();
    let utf8 = |b: Vec<u8>| match String::from_utf8(b) {
        Ok(t) => Ok(Some(t)),
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e.utf8_error())),
    };
    let text = match std::fs::read(path).and_then(utf8) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            log::warn!("{at}: {e}");
            return Err(format!(
                "The login manager's settings in {at} could not be read, so this page doesn't \
                 save over them.\n\n{e}"
            ));
        }
    };
    let tag = file_tag(text.as_deref().map(str::as_bytes));
    let (config, warnings) = MachineConfig::parse(text.as_deref().unwrap_or_default());
    if warnings.is_empty() {
        return Ok((config, tag));
    }
    log::warn!("{at}: {}", warnings.join("; "));
    Err(format!(
        "The login manager's settings in {at} have values this page can't use, so it doesn't \
         save over them. Correct or remove these lines as root, then press Apply again:\n\n{}",
        warnings.join("\n")
    ))
}

/// The Remote Desktop front's fingerprint; `None` while it is not running.
pub(crate) fn machine_fingerprint() -> Option<String> {
    fingerprint(Path::new(FINGERPRINT))
}

fn fingerprint(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.trim().to_string()).filter(|t| !t.is_empty())
}

/// The accounts the login screen lists, read through NSS; none under test.
pub(crate) fn machine_users() -> Source {
    match cfg!(test) {
        true => Source::default(),
        false => Source::read(),
    }
}

impl SettingsApp {
    /// Read the machine settings the first time the page opens, and the accounts off the loop.
    pub(crate) fn ensure_login(&mut self) {
        if registry::open(&self.sheet) != Page::LoginManager {
            return;
        }
        if self.login.is_some() {
            return self.poll_fingerprint();
        }
        let loaded = read(&self.login_files.config).unwrap_or_else(|why| {
            self.fail(&why);
            MachineConfig::default()
        });
        self.login = Some(Login {
            want: loaded.clone(),
            loaded,
            users: Vec::new(),
            fingerprint: fingerprint(&self.login_files.fingerprint),
        });
        self.show_login();
        let source = self.login_source;
        self.login_users =
            Some(self.login_spawn(move || source().listed().into_iter().map(|e| e.name).collect()));
        self.poll_fingerprint();
    }

    /// Read the front's fingerprint again in [`POLL`] unless a read is under way or the page
    /// is closed. Tests read it at once.
    fn poll_fingerprint(&mut self) {
        if self.login_poll.is_some() || registry::open(&self.sheet) != Page::LoginManager {
            return;
        }
        let path = self.login_files.fingerprint.clone();
        self.login_poll = Some(self.login_spawn(move || {
            if !cfg!(test) {
                std::thread::sleep(POLL);
            }
            fingerprint(&path)
        }));
    }

    /// A poll read the fingerprint: show it if it changed, and poll again. True when the page
    /// changed.
    pub(crate) fn login_poll_done(&mut self) -> bool {
        let Some(rx) = &self.login_poll else {
            return false;
        };
        let read = take(rx);
        if matches!(read, Ok(None)) {
            return false;
        }
        self.login_poll = None;
        let changed = match (&mut self.login, read) {
            (Some(l), Ok(Some(now))) if l.fingerprint != now => {
                l.fingerprint = now;
                true
            }
            _ => false,
        };
        if changed {
            self.show_login();
        }
        self.poll_fingerprint();
        changed
    }

    /// The accounts are in: offer them for automatic logon. True when the page changed.
    pub(crate) fn login_users_done(&mut self) -> bool {
        let Some(rx) = &self.login_users else {
            return false;
        };
        let users = match take(rx) {
            Ok(None) => return false,
            Ok(Some(users)) => users,
            Err(()) => Vec::new(),
        };
        self.login_users = None;
        if let Some(l) = &mut self.login {
            l.users = users;
        }
        self.show_login();
        true
    }

    fn show_login(&mut self) {
        let Some(l) = &self.login else {
            return;
        };
        pages::show_login(
            &mut self.sheet.pages[Page::LoginManager.index()],
            &l.want,
            &l.users,
            l.fingerprint.as_deref(),
        );
    }

    pub(crate) fn login_dirty(&self) -> bool {
        self.login.as_ref().is_some_and(|l| l.want != l.loaded)
    }

    /// Control `id` on the page moved.
    pub(crate) fn login_changed(&mut self, id: &str) {
        let page = &mut self.sheet.pages[Page::LoginManager.index()];
        if let Some(l) = &mut self.login {
            pages::read_login(page, id, &mut l.want);
        }
        pages::follow_login(page);
        self.sync_apply();
    }

    /// Save the page's changes off the loop unless a save is under way. The save rereads the
    /// file first and sends nothing when it no longer reads as the page loaded it; the helper
    /// refuses a file changed after that reread.
    pub(crate) fn apply_login(&mut self) {
        if !self.login_dirty() || self.login_job.is_some() {
            return;
        }
        let Some(l) = &self.login else {
            return;
        };
        let (cfg, loaded, run, path) = (
            l.want.clone(),
            l.loaded.clone(),
            self.login_run,
            self.login_files.config.clone(),
        );
        let sent = cfg.clone();
        let rx = self.login_spawn(move || match read_tagged(&path) {
            Ok((now, tag)) if now == loaded => match apply(&cfg, &tag, &run) {
                Ok(()) => Ended::Saved(read(&path)),
                Err(Unsaved::Changed) => Ended::Changed(read(&path)),
                Err(Unsaved::Failed(why)) => Ended::Failed(why),
            },
            now => Ended::Changed(now.map(|(config, _)| config)),
        });
        self.login_job = Some(Saving {
            sent,
            rx,
            quit: false,
            discard: false,
        });
    }

    /// Drop the page's unsaved changes; a save under way still lands.
    pub(crate) fn revert_login(&mut self) {
        if let Some(l) = &mut self.login {
            l.want = match &mut self.login_job {
                Some(s) => {
                    s.discard = true;
                    s.sent.clone()
                }
                None => l.loaded.clone(),
            };
        }
        self.show_login();
    }

    /// The sheet is closing without OK: a save under way ends it saved or not.
    pub(crate) fn close_login(&mut self) {
        if let Some(s) = &mut self.login_job {
            s.discard = true;
        }
    }

    /// A save ended: show the file as it now reads, or why nothing was saved. Edits made while it
    /// ran stay for the next save; a file changed since the page loaded it replaces them.
    pub(crate) fn login_done(&mut self) -> Option<bool> {
        let done = match take(&self.login_job.as_ref()?.rx) {
            Ok(None) => return None,
            Ok(Some(r)) => r,
            Err(()) => Ended::Failed(NOT_SAVED.to_string()),
        };
        let s = self.login_job.take()?;
        let (saved, why) = match done {
            Ended::Saved(now) => {
                let unedited = self.login.as_ref().is_some_and(|l| l.want == s.sent);
                self.login_reread(Ok(now.unwrap_or_else(|_| s.sent.clone())), unedited);
                (true, None)
            }
            Ended::Failed(why) => (false, Some(why)),
            Ended::Changed(now) => {
                let why = now.as_ref().err().cloned().unwrap_or(CHANGED.into());
                self.login_reread(now, true);
                (false, Some(why))
            }
        };
        if let Some(why) = why
            && !(s.quit && s.discard)
        {
            self.fail(&why);
        }
        if saved && !s.discard {
            self.apply_login();
            if let Some(next) = &mut self.login_job {
                next.quit = s.quit;
            }
        }
        self.sync_apply();
        Some(s.quit && self.login_job.is_none() && (saved || s.discard))
    }

    /// Take the file as it now reads as the page's loaded settings, and as its edits too when
    /// `replace`; an unreadable file leaves both.
    fn login_reread(&mut self, now: Read, replace: bool) {
        if let Some(l) = &mut self.login {
            if let Ok(now) = now {
                if replace {
                    l.want = now.clone();
                }
                l.loaded = now;
            }
            l.fingerprint = fingerprint(&self.login_files.fingerprint);
        }
        self.show_login();
    }

    /// Run `job` off the loop; tests run it at once.
    pub(crate) fn login_spawn<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Receiver<T> {
        if cfg!(test) {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(job());
            return rx;
        }
        self.spawn(job)
    }
}
