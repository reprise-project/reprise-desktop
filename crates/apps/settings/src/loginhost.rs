//! The Login Manager page's host: reads the machine settings when the page first opens, and
//! saves them on Apply through `pkexec reprise-login-settings`, off the loop.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use repriseapp::heads::Head;
use repriselogin::front::FINGERPRINT;
use repriselogin::machine::{
    Background, LoginMode, LoginOutput, MACHINE_CONFIG, MAX_OUTPUTS, MAX_PICTURE, MachineConfig,
    Transform, decode_picture, file_tag, picture_magic, save_request,
};
use repriselogin::users::Source;
use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// The helper that replaces `MACHINE_CONFIG` with the TOML on its stdin, as root.
const HELPER: &str = "/usr/lib/reprise/reprise-login-settings";
const PKEXEC: &str = "/usr/bin/pkexec";
/// How often the open page reads the front's fingerprint again.
const POLL: Duration = Duration::from_secs(2);
/// How often a save looks whether pkexec has exited.
const EXIT_POLL: Duration = Duration::from_millis(50);

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
    /// A picture browsed for the background, sent with the next save.
    pub(crate) picked: Option<PathBuf>,
}

impl Login {
    /// The pictures the background combo offers, `picked` naming the browsed one.
    fn picture<'a>(&self, picked: Option<&'a str>) -> pages::LoginPicture<'a> {
        pages::LoginPicture {
            picked,
            saved: self.loaded.login.background == Some(Background::Picture),
        }
    }

    fn picked_name(&self) -> Option<String> {
        let p = self.picked.as_deref()?;
        Some(
            p.file_name()
                .unwrap_or(p.as_os_str())
                .to_string_lossy()
                .into_owned(),
        )
    }
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
    sent_picture: Option<PathBuf>,
    rx: Receiver<Ended>,
    withdraw: Withdraw,
    /// An OK, Cancel or close waits on it.
    pub(crate) quit: bool,
    /// Cancel or a close: the sheet closes when it ends, saved or not, and nothing follows it.
    discard: bool,
    /// Apply or OK was pressed while it ran: the edits made since are saved after it.
    again: bool,
}

/// The pkexec a save runs, which Cancel or a close stops while its password prompt is up.
#[derive(Clone, Default)]
pub(crate) struct Withdraw(Arc<Mutex<Pkexec>>);

#[derive(Default)]
struct Pkexec {
    child: Option<Child>,
    stopped: bool,
}

impl Withdraw {
    /// Kills the save's pkexec, or keeps it from starting. pkexec keeps the user's real uid
    /// until polkit lets it in, so the kill lands while the prompt is up; later the save lands.
    pub(crate) fn stop(&self) {
        let mut p = self.lock();
        p.stopped = true;
        if let Some(c) = p.child.as_mut() {
            let _ = c.kill();
        }
    }

    fn lock(&self) -> MutexGuard<'_, Pkexec> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Runs the helper with a save request on its stdin, as [`pkexec`] does.
pub(crate) type Run = fn(&[u8], &Withdraw) -> io::Result<Output>;

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

/// Save `cfg` and a new background `picture` through `run`, which hands the save request to the
/// helper's stdin and returns how it exited. The helper replaces only a file that still has `tag`.
pub(crate) fn apply(
    cfg: &MachineConfig,
    picture: Option<&[u8]>,
    tag: &str,
    run: &dyn Fn(&[u8]) -> io::Result<Output>,
) -> Result<(), Unsaved> {
    let out = run(&save_request(tag, &cfg.to_toml(), picture))
        .map_err(|e| Unsaved::Failed(format!("{NOT_SAVED}\n\n{e}")))?;
    let why = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
    let with_why = |head: &str| match why.is_empty() {
        true => head.to_string(),
        false => format!("{head}\n\n{why}"),
    };
    // The helper's exits are in docs/programs.md; pkexec's 126 is a dismissed prompt, 127 a
    // refusal or its own failure.
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
pub(crate) fn pkexec(input: &[u8], withdraw: &Withdraw) -> io::Result<Output> {
    run_helper(Path::new(PKEXEC), Path::new(HELPER), input, withdraw)
}

/// `pkexec` running `helper`, which `withdraw` can stop; a missing helper is named without
/// asking for a password.
pub(crate) fn run_helper(
    pkexec: &Path,
    helper: &Path,
    input: &[u8],
    withdraw: &Withdraw,
) -> io::Result<Output> {
    if !helper.exists() {
        let why = format!("{} is not installed.", helper.display());
        return Err(io::Error::new(io::ErrorKind::NotFound, why));
    }
    let (stdin, stderr) = {
        let mut p = withdraw.lock();
        if p.stopped {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The save was withdrawn.",
            ));
        }
        let mut child = Command::new(pkexec)
            .arg(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let pipes = (child.stdin.take(), child.stderr.take());
        p.child = Some(child);
        pipes
    };
    let wrote = stdin.map(|mut s| s.write_all(input));
    let status = loop {
        let mut held = withdraw.lock();
        if let Some(status) = held
            .child
            .as_mut()
            .map(Child::try_wait)
            .transpose()?
            .flatten()
        {
            held.child.take();
            break status;
        }
        drop(held);
        std::thread::sleep(EXIT_POLL);
    };
    let mut why = Vec::new();
    if let Some(mut e) = stderr {
        e.read_to_end(&mut why)?;
    }
    let out = Output {
        status,
        stdout: Vec::new(),
        stderr: why,
    };
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

/// The picture at `path`, read as the user so the helper never opens a path of theirs, and
/// decoded as the helper will, so one it refuses is named before the password is asked.
fn read_picture(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path)
        .and_then(|f| f.take(MAX_PICTURE as u64 + 1).read_to_end(&mut bytes));
    if let Err(e) = read {
        return Err(format!("{NOT_SAVED}\n\n{}: {e}", path.display()));
    }
    match decode_picture(&bytes) {
        Ok(_) => Ok(bytes),
        Err(why) => Err(format!(
            "{NOT_SAVED}\n\n{} can't be the login screen's background: {why}.",
            path.display()
        )),
    }
}

/// `heads` as the login screen's outputs, `primary` the name of the primary one: each runs the
/// mode, scale, transform and place it runs now.
pub(crate) fn login_outputs(heads: &[Head], primary: Option<&str>) -> Vec<LoginOutput> {
    (heads.iter().take(MAX_OUTPUTS))
        .map(|h| LoginOutput {
            name: h.name.clone(),
            mode: h.current_mode().map(|m| LoginMode {
                width: m.width.max(1) as u32,
                height: m.height.max(1) as u32,
                refresh_mhz: (m.refresh_mhz > 0).then_some(m.refresh_mhz as u32),
            }),
            quarters: (h.scale * 4.0).round().clamp(4.0, 16.0) as u8,
            transform: Transform::parse(h.transform.name()).unwrap_or_default(),
            position: Some(h.position),
            primary: Some(h.name.as_str()) == primary,
        })
        .collect()
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
            picked: None,
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
        let picked = l.picked_name();
        pages::show_login(
            &mut self.sheet.pages[Page::LoginManager.index()],
            &l.want,
            &l.users,
            l.fingerprint.as_deref(),
            l.picture(picked.as_deref()),
        );
    }

    pub(crate) fn login_dirty(&self) -> bool {
        (self.login.as_ref()).is_some_and(|l| l.want != l.loaded || l.picked.is_some())
    }

    /// Control `id` on the page moved; the background combo's rows follow its pick.
    pub(crate) fn login_changed(&mut self, id: &str) {
        let page = &mut self.sheet.pages[Page::LoginManager.index()];
        if let Some(l) = &mut self.login {
            if id == pages::LOGIN_BACKGROUND {
                let picked = l.picked_name();
                let picture = l.picture(picked.as_deref());
                pages::read_login_background(page, &mut l.want, picture);
                if l.want.login.background != Some(Background::Picture) {
                    l.picked = None;
                }
            } else {
                pages::read_login(page, id, &mut l.want);
            }
        }
        pages::follow_login(page);
        if id == pages::LOGIN_BACKGROUND {
            self.show_login();
        }
        self.sync_apply();
    }

    /// Use My Display Settings: the login screen is to run each output as the session runs it now.
    pub(crate) fn login_use_displays(&mut self) {
        let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
        let (layouts, config) = (read(&self.display.layouts), read(&self.path));
        let primary =
            crate::outputwrite::session_primary(&self.display.connected, &layouts, &config);
        let outputs = login_outputs(self.display.model.heads(), primary.as_deref());
        if outputs.is_empty() {
            return self.fail("No screens are listed, so there are no display settings to use.");
        }
        if let Some(l) = &mut self.login {
            l.want.outputs = outputs;
        }
        self.show_login();
        self.sync_apply();
    }

    /// Reset: each output back to its preferred mode at 100% on the login screen.
    pub(crate) fn login_reset_displays(&mut self) {
        if let Some(l) = &mut self.login {
            l.want.outputs.clear();
        }
        self.show_login();
        self.sync_apply();
    }

    /// Browse... picked `path` for the login screen's background, sent with the next save.
    pub(crate) fn login_picture(&mut self, path: PathBuf) {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let mut head = [0; 8];
        let read = std::fs::File::open(&path).and_then(|mut f| {
            let len = f.metadata()?.len();
            let n = f.read(&mut head)?;
            Ok((len, n))
        });
        match read {
            Ok((len, _)) if len > MAX_PICTURE as u64 => {
                return self.fail(&format!(
                    "'{name}' is larger than {} MB, too large for the login screen.",
                    MAX_PICTURE >> 20
                ));
            }
            Ok((_, n)) if !picture_magic(&head[..n]) => {
                return self.fail(&format!(
                    "'{name}' is not a PNG, JPEG or BMP picture, which the login screen shows."
                ));
            }
            Ok(_) => {}
            Err(e) => return self.fail(&format!("Cannot open '{name}': {e}.")),
        }
        if let Some(l) = &mut self.login {
            l.want.login.background = Some(Background::Picture);
            l.picked = Some(path);
        }
        self.show_login();
        self.sync_apply();
    }

    /// Save the page's changes off the loop unless a save is under way. The save rereads the
    /// file first and sends nothing when it no longer reads as the page loaded it; the helper
    /// refuses a file changed after that reread. False when the page holds settings it refuses.
    pub(crate) fn apply_login(&mut self) -> bool {
        if !self.login_dirty() {
            return true;
        }
        if let Some(s) = &mut self.login_job {
            s.again = true;
            return true;
        }
        let Some(l) = &self.login else {
            return true;
        };
        let (_, warnings) = MachineConfig::parse(&l.want.to_toml());
        if !warnings.is_empty() {
            self.fail(&format!("{NOT_SAVED}\n\n{}", warnings.join("\n")));
            return false;
        }
        let (cfg, loaded, run, path) = (
            l.want.clone(),
            l.loaded.clone(),
            self.login_run,
            self.login_files.config.clone(),
        );
        let (sent, sent_picture) = (cfg.clone(), l.picked.clone());
        let picked = l.picked.clone();
        let withdraw = Withdraw::default();
        let w = withdraw.clone();
        let rx = self.login_spawn(move || {
            let picture = match picked.as_deref().map(read_picture).transpose() {
                Ok(p) => p,
                Err(why) => return Ended::Failed(why),
            };
            match read_tagged(&path) {
                Ok((now, tag)) if now == loaded => {
                    match apply(&cfg, picture.as_deref(), &tag, &|input| run(input, &w)) {
                        Ok(()) => Ended::Saved(read(&path)),
                        Err(Unsaved::Changed) => Ended::Changed(read(&path)),
                        Err(Unsaved::Failed(why)) => Ended::Failed(why),
                    }
                }
                now => Ended::Changed(now.map(|(config, _)| config)),
            }
        });
        self.login_job = Some(Saving {
            sent,
            sent_picture,
            rx,
            withdraw,
            quit: false,
            discard: false,
            again: false,
        });
        true
    }

    /// Drop the page's unsaved changes, and a save under way while its prompt is up.
    pub(crate) fn revert_login(&mut self) {
        if let Some(l) = &mut self.login {
            l.picked = None;
            l.want = match &mut self.login_job {
                Some(s) => {
                    s.discard = true;
                    s.withdraw.stop();
                    s.sent.clone()
                }
                None => l.loaded.clone(),
            };
        }
        self.show_login();
    }

    /// The sheet is closing without OK: a save under way is stopped while its prompt is up, and
    /// ends it saved or not.
    pub(crate) fn close_login(&mut self) {
        if let Some(s) = &mut self.login_job {
            s.discard = true;
            s.withdraw.stop();
        }
    }

    /// A save ended: show the file as it now reads, or why nothing was saved. Edits made while it
    /// ran are saved after it when Apply or OK was pressed meanwhile, else left for Apply; a file
    /// changed since the page loaded it replaces them.
    pub(crate) fn login_done(&mut self) -> Option<bool> {
        let done = match take(&self.login_job.as_ref()?.rx) {
            Ok(None) => return None,
            Ok(Some(r)) => r,
            Err(()) => Ended::Failed(NOT_SAVED.to_string()),
        };
        let s = self.login_job.take()?;
        let (saved, why) = match done {
            Ended::Saved(now) => {
                let unedited = (self.login.as_ref())
                    .is_some_and(|l| l.want == s.sent && l.picked == s.sent_picture);
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
        if saved && !s.discard && (s.again || s.quit) {
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
                    l.picked = None;
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
