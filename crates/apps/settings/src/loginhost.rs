//! The Login Manager page's host: reads the machine settings when the page first opens, and
//! saves them on Apply through `pkexec reprise-login-settings`, off the loop.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use repriselogin::machine::{MACHINE_CONFIG, MachineConfig};
use repriselogin::users::Source;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::Receiver;

/// The helper that replaces `MACHINE_CONFIG` with the TOML on its stdin, as root.
const HELPER: &str = "/usr/lib/reprise/reprise-login-settings";
const PKEXEC: &str = "/usr/bin/pkexec";
/// The Remote Desktop front's certificate fingerprint, while it runs.
const FINGERPRINT: &str = "/run/reprise-login-rdp/fingerprint";

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

/// A save under way and the settings it sent.
pub(crate) struct Saving {
    sent: MachineConfig,
    rx: Receiver<Result<MachineConfig, String>>,
    /// An OK, Cancel or close waits on it.
    pub(crate) quit: bool,
    /// Cancel or a close: the sheet closes when it ends, saved or not, and nothing follows it.
    discard: bool,
}

const NOT_SAVED: &str = "The login manager's settings could not be saved.";

/// Save `cfg` through `run`, which hands the TOML to the helper's stdin and returns how it
/// exited; the message to show when it did not save.
pub(crate) fn apply(
    cfg: &MachineConfig,
    run: &dyn Fn(&str) -> io::Result<Output>,
) -> Result<(), String> {
    let out = run(&cfg.to_toml()).map_err(|e| format!("{NOT_SAVED}\n\n{e}"))?;
    let why = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
    let with_why = |head: &str| match why.is_empty() {
        true => head.to_string(),
        false => format!("{head}\n\n{why}"),
    };
    // The helper's exits: 0 saved, 1 the write failed, 2 refused (a reason per line);
    // pkexec's 126 and 127: not authorized, or the prompt dismissed.
    match out.status.code() {
        Some(0) => Ok(()),
        Some(2) => Err(with_why("The login manager refused the settings:")),
        Some(126 | 127) => Err("The change was not authorized.".into()),
        _ => Err(with_why(NOT_SAVED)),
    }
}

/// `pkexec` running [`HELPER`] with `toml` on its stdin.
pub(crate) fn pkexec(toml: &str) -> io::Result<Output> {
    let mut child = Command::new(PKEXEC)
        .arg(HELPER)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let wrote = child.stdin.take().map(|mut s| s.write_all(toml.as_bytes()));
    let out = child.wait_with_output()?;
    match wrote {
        // A refusal before the helper read its input closes the pipe; its exit tells why.
        Some(Err(e)) if out.status.success() => Err(e),
        _ => Ok(out),
    }
}

/// The machine settings in `path`; a missing or unreadable file gives the defaults.
fn read(path: &Path) -> MachineConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log::warn!("{}: {e}", path.display());
            }
            String::new()
        }
    };
    let (config, warnings) = MachineConfig::parse(&text);
    for w in warnings {
        log::warn!("{}: {w}", path.display());
    }
    config
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
        if self.login.is_some() || registry::open(&self.sheet) != Page::LoginManager {
            return;
        }
        let loaded = read(&self.login_files.config);
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

    /// Save the page's changes off the loop, unless a save is under way.
    pub(crate) fn apply_login(&mut self) {
        if !self.login_dirty() || self.login_job.is_some() {
            return;
        }
        let Some(l) = &self.login else {
            return;
        };
        let (cfg, run, path) = (
            l.want.clone(),
            self.login_run,
            self.login_files.config.clone(),
        );
        let sent = cfg.clone();
        let rx = self.login_spawn(move || apply(&cfg, &run).map(|()| read(&path)));
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

    /// A save finished: show the file as it now reads, or why it was not saved. Edits made
    /// while it ran stay, and go in the next save. True when the sheet closes.
    pub(crate) fn login_done(&mut self) -> Option<bool> {
        let done = match take(&self.login_job.as_ref()?.rx) {
            Ok(None) => return None,
            Ok(Some(r)) => r,
            Err(()) => Err(NOT_SAVED.to_string()),
        };
        let s = self.login_job.take()?;
        let saved = match done {
            Ok(now) => {
                if let Some(l) = &mut self.login {
                    if l.want == s.sent {
                        l.want = now.clone();
                    }
                    l.loaded = now;
                }
                self.show_login();
                true
            }
            Err(why) => {
                if !(s.quit && s.discard) {
                    self.fail(&why);
                }
                false
            }
        };
        if saved && !s.discard {
            self.apply_login();
            if let Some(next) = &mut self.login_job {
                next.quit = s.quit;
            }
        }
        self.sync_apply();
        Some(s.quit && self.login_job.is_none() && (saved || s.discard))
    }

    /// Run `job` off the loop; tests run it at once.
    fn login_spawn<T: Send + 'static>(
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
