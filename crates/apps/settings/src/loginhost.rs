//! The Login Manager page's host: reads the machine settings when the page first opens, and
//! saves them on Apply through `pkexec reprise-login-settings`, off the loop.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use repriselogin::machine::{MACHINE_CONFIG, MachineConfig};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

/// The accounts the login screen lists, by name: a uid in `login_defs`' `UID_MIN..=UID_MAX`
/// (1000 to 60000 when unset) and a login shell `shells` names.
fn listed(passwd: &str, shells: &str, login_defs: &str) -> Vec<String> {
    let (mut min, mut max) = (1000, 60000);
    for line in login_defs.lines() {
        let mut words = line.split_whitespace();
        match (words.next(), words.next().and_then(|v| v.parse().ok())) {
            (Some("UID_MIN"), Some(v)) => min = v,
            (Some("UID_MAX"), Some(v)) => max = v,
            _ => {}
        }
    }
    let shells: Vec<&str> = shells
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .collect();
    let mut names: Vec<String> = passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let (name, uid, shell) = (*f.first()?, f.get(2)?.parse::<u32>().ok()?, *f.get(6)?);
            let shell = if shell.is_empty() { "/bin/sh" } else { shell };
            let login =
                shells.contains(&shell) && !shell.ends_with("nologin") && !shell.ends_with("false");
            (login && (min..=max).contains(&uid) && !name.starts_with(['+', '-']))
                .then(|| name.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// [`listed`] from this machine's files; none under test.
fn users() -> Vec<String> {
    if cfg!(test) {
        return Vec::new();
    }
    let text = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    listed(
        &text("/etc/passwd"),
        &text("/etc/shells"),
        &text("/etc/login.defs"),
    )
}

impl SettingsApp {
    /// Read the machine settings the first time the page opens.
    pub(crate) fn ensure_login(&mut self) {
        if self.login.is_some() || registry::open(&self.sheet) != Page::LoginManager {
            return;
        }
        let loaded = read(&self.login_files.config);
        self.login = Some(Login {
            want: loaded.clone(),
            loaded,
            users: users(),
            fingerprint: fingerprint(&self.login_files.fingerprint),
        });
        self.show_login();
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
        self.start_login(move || apply(&cfg, &run).map(|()| read(&path)));
    }

    pub(crate) fn revert_login(&mut self) {
        if let Some(l) = &mut self.login {
            l.want = l.loaded.clone();
        }
        self.show_login();
    }

    /// A save finished: show the file as it now reads, or why it was not saved. True when the
    /// sheet closes, which an OK or a close waited on.
    pub(crate) fn login_done(&mut self) -> Option<bool> {
        let rx = self.login_job.as_ref()?;
        let done = match take(rx) {
            Ok(None) => return None,
            Ok(Some(r)) => r,
            Err(()) => Err(NOT_SAVED.to_string()),
        };
        self.login_job = None;
        let quit = std::mem::take(&mut self.login_quit);
        let saved = match done {
            Ok(now) => {
                if let Some(l) = &mut self.login {
                    l.loaded = now.clone();
                    l.want = now;
                }
                self.show_login();
                true
            }
            Err(why) => {
                self.fail(&why);
                false
            }
        };
        self.sync_apply();
        Some(saved && quit)
    }

    #[cfg(not(test))]
    fn start_login(
        &mut self,
        job: impl FnOnce() -> Result<MachineConfig, String> + Send + 'static,
    ) {
        self.login_job = Some(self.spawn(job));
    }

    /// Tests run the save at once, through the `login_run` they set.
    #[cfg(test)]
    fn start_login(
        &mut self,
        job: impl FnOnce() -> Result<MachineConfig, String> + Send + 'static,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(job());
        self.login_job = Some(rx);
    }
}

#[cfg(test)]
mod tests {
    use super::listed;

    #[test]
    fn the_listed_users_have_a_login_shell_and_a_uid_in_the_range() {
        let passwd = "root:x:0:0::/root:/bin/bash\n\
                      zoe:x:1001:1001::/home/zoe:/bin/zsh\n\
                      anna:x:1000:1000:Anna:/home/anna:/bin/bash\n\
                      svc:x:1002:1002::/:/usr/bin/nologin\n\
                      old:x:1003:1003::/home/old:\n\
                      far:x:70000:70000::/home/far:/bin/bash\n\
                      +nis:x:1004:1004::/:/bin/bash\n";
        let shells = "# login shells\n/bin/bash\n/bin/zsh\n/usr/bin/nologin\n/bin/sh\n";
        assert_eq!(
            listed(passwd, shells, "UID_MIN 1000\nUID_MAX 60000\n"),
            ["anna", "old", "zoe"]
        );
        assert_eq!(listed(passwd, shells, "UID_MIN 1001\n"), ["old", "zoe"]);
    }
}
