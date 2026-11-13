//! Connecting to a server: `gio mount` on its own thread, its prompts posted as [`Step`]s and the
//! answers written to its stdin. A password kept in the Secret Service is tried before asking.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use reprisebus::secrets::Secrets;
use zeroize::Zeroizing;

use super::conversation::{Conversation, Field, Prompt};
use super::{Address, gio_command, gio_error};

/// What gio prints when an anonymous try is turned away.
const ANONYMOUS_DENIED: &str = "Anonymous access denied";
/// The schemes whose servers may let a guest in: gvfs logs in anonymously first when asked to.
const GUEST_SCHEMES: &[&str] = &["smb", "ftp", "ftps", "dav", "davs"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    /// The server's own words.
    pub message: String,
    pub user: String,
    /// The user name may be changed; false when the address names the user.
    pub asks_user: bool,
    /// The box asks for a domain (SMB).
    pub domain: bool,
    /// The last answer was turned down.
    pub retry: bool,
    /// A Secret Service is there to remember the password.
    pub can_remember: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub message: String,
    pub choices: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failed {
    Cancelled,
    Error(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Login(Login),
    Question(Question),
    /// Mounted, with the address's FUSE folder; `None` for an SMB server, which has none.
    Done(Result<Option<PathBuf>, Failed>),
}

#[derive(Clone)]
pub struct Credentials {
    pub user: String,
    pub domain: String,
    pub password: Zeroizing<String>,
    pub remember: bool,
}

pub enum Answer {
    Login(Credentials),
    /// The index into [`Question::choices`].
    Choice(usize),
    Cancel,
}

/// A connection under way. Dropping it cancels it.
pub struct Connecting {
    answers: Sender<Answer>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Connecting {
    pub fn answer(&self, a: Answer) {
        let _ = self.answers.send(a);
    }

    pub fn cancel(&self) {
        if let Some(c) = reprisebus::mutex::lock(&self.child).as_mut() {
            let _ = c.kill();
        }
        let _ = self.answers.send(Answer::Cancel);
    }
}

impl Drop for Connecting {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Mounts `addr` on a thread of its own; `post` gets each prompt and then the outcome.
pub fn connect(addr: Address, post: impl FnMut(Step) + Send + 'static) -> Connecting {
    let (tx, rx) = std::sync::mpsc::channel();
    let child = Arc::new(Mutex::new(None));
    let slot = child.clone();
    let spawned = std::thread::Builder::new()
        .name("gio-mount".into())
        .spawn(move || run(addr, post, rx, slot));
    if let Err(e) = spawned {
        log::warn!("could not start a gio mount thread: {e}");
    }
    Connecting { answers: tx, child }
}

fn run(
    addr: Address,
    mut post: impl FnMut(Step),
    rx: Receiver<Answer>,
    child: Arc<Mutex<Option<Child>>>,
) {
    let secrets = Secrets::open();
    let stored = in_address(&addr).or_else(|| secrets.as_ref().and_then(|s| stored(s, &addr)));
    let mut talk = Talk {
        addr: &addr,
        stored,
        typed: None,
        asked: 0,
        can_remember: secrets.is_some(),
    };
    let mut anonymous = talk.stored.is_none()
        && addr.user.is_none()
        && GUEST_SCHEMES.contains(&addr.scheme.as_str());
    let outcome = loop {
        match talk.attempt(anonymous, &mut post, &rx, &child) {
            Err(Failed::Error(e)) if anonymous && e == ANONYMOUS_DENIED => anonymous = false,
            r => break r,
        }
    };
    let mounted = outcome.is_ok();
    let outcome = outcome.map(|()| {
        (!addr.is_server())
            .then(|| super::local_path(&addr.uri()))
            .flatten()
    });
    let outcome = match outcome {
        Ok(None) if !addr.is_server() => Err(Failed::Error(
            "The share is connected, but gvfs gives it no folder (is gvfsd-fuse running?)".into(),
        )),
        o => o,
    };
    post(Step::Done(outcome));
    // After the share opens: storing may wait on the keyring's own unlock prompt.
    if mounted
        && let (Some(s), Some(c)) = (&secrets, talk.typed.as_ref().filter(|c| c.remember))
        && let Err(e) = remember(s, &addr, c)
    {
        log::warn!("could not remember the password for {}: {e}", addr.host);
    }
}

struct Talk<'a> {
    addr: &'a Address,
    /// The address's password, else the Secret Service's for the server, until it is tried.
    stored: Option<Credentials>,
    /// What the person typed last.
    typed: Option<Credentials>,
    /// Password requests answered so far.
    asked: usize,
    can_remember: bool,
}

impl Talk<'_> {
    /// One `gio mount`, `-a` for a guest try.
    fn attempt(
        &mut self,
        anonymous: bool,
        post: &mut impl FnMut(Step),
        rx: &Receiver<Answer>,
        slot: &Arc<Mutex<Option<Child>>>,
    ) -> Result<(), Failed> {
        let Some(mut cmd) = gio_command() else {
            return Err(Failed::Error("gio is not installed.".into()));
        };
        cmd.arg("mount");
        if anonymous {
            cmd.arg("-a");
        }
        let mut child = cmd
            .arg("--")
            .arg(self.addr.uri())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Failed::Error(e.to_string()))?;
        let mut stdin = child.stdin.take().expect("piped");
        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        *reprisebus::mutex::lock(slot) = Some(child);
        let mut conv = Conversation::default();
        let mut current: Option<Credentials> = None;
        let mut cancelled = false;
        let mut buf = [0u8; 1024];
        loop {
            let n = match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let Some(prompt) = conv.feed(&buf[..n]) else {
                continue;
            };
            let reply = match prompt {
                Prompt::Field {
                    field,
                    default,
                    message,
                } => {
                    if message.is_some() || current.is_none() {
                        current = self.credentials(field, default, message, post, rx);
                    }
                    current.as_ref().map(|c| match field {
                        Field::User => c.user.clone(),
                        Field::Domain => c.domain.clone(),
                        Field::Password => c.password.to_string(),
                    })
                }
                Prompt::Choice { message, choices } => {
                    post(Step::Question(Question { message, choices }));
                    match rx.recv() {
                        Ok(Answer::Choice(i)) => Some((i + 1).to_string()),
                        _ => None,
                    }
                }
            };
            match reply {
                Some(text) => write_line(&mut stdin, &text),
                None => {
                    cancelled = true;
                    if let Some(c) = reprisebus::mutex::lock(slot).as_mut() {
                        let _ = c.kill();
                    }
                    break;
                }
            }
        }
        drop(stdin);
        let mut err = String::new();
        let _ = stderr.read_to_string(&mut err);
        let status = reprisebus::mutex::lock(slot).take().map(|mut c| c.wait());
        match status {
            _ if cancelled => Err(Failed::Cancelled),
            Some(Ok(s)) if s.success() => Ok(()),
            // Killed by `Connecting::cancel`.
            Some(Ok(s)) if std::os::unix::process::ExitStatusExt::signal(&s).is_some() => {
                Err(Failed::Cancelled)
            }
            _ => Err(Failed::Error(gio_error(&err))),
        }
    }

    /// The answers to a new password request: the stored password the first time, else the
    /// person's, `None` when they cancelled.
    fn credentials(
        &mut self,
        field: Field,
        default: Option<String>,
        message: Option<String>,
        post: &mut impl FnMut(Step),
        rx: &Receiver<Answer>,
    ) -> Option<Credentials> {
        if let Some(s) = self.stored.take() {
            self.asked += 1;
            return Some(s);
        }
        let login = Login {
            message: message.unwrap_or_default(),
            user: match field {
                Field::User => default.unwrap_or_default(),
                _ => self.addr.user.clone().unwrap_or_default(),
            },
            asks_user: field == Field::User,
            domain: self.addr.scheme == "smb",
            retry: self.asked > 0,
            can_remember: self.can_remember,
        };
        self.asked += 1;
        post(Step::Login(login));
        match rx.recv() {
            Ok(Answer::Login(c)) => {
                self.typed = Some(c.clone());
                Some(c)
            }
            _ => None,
        }
    }
}

/// gio reads one line per answer, so a line break inside one would answer the next prompt.
fn write_line(stdin: &mut ChildStdin, text: &str) {
    let mut line = Zeroizing::new(text.replace(['\n', '\r'], ""));
    line.push('\n');
    let _ = stdin
        .write_all(line.as_bytes())
        .and_then(|()| stdin.flush());
}

/// The password typed into the address, answered like one typed into the box.
fn in_address(addr: &Address) -> Option<Credentials> {
    Some(Credentials {
        user: addr.user.clone().unwrap_or_default(),
        domain: String::new(),
        password: addr.password.as_ref()?.0.clone(),
        remember: false,
    })
}

/// The password the Secret Service keeps for `addr`'s server.
fn stored(secrets: &Secrets, addr: &Address) -> Option<Credentials> {
    let attrs = addr.secret_attributes();
    let query: HashMap<&str, &str> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let found = secrets.lookup(&query)?;
    let get = |k: &str| found.attributes.get(k).cloned().unwrap_or_default();
    Some(Credentials {
        user: get("user"),
        domain: get("domain"),
        password: found.secret,
        remember: false,
    })
}

fn remember(secrets: &Secrets, addr: &Address, c: &Credentials) -> Result<(), String> {
    let mut attrs = addr.secret_attributes();
    if !c.user.is_empty() {
        attrs.insert("user", c.user.clone());
    }
    if !c.domain.is_empty() {
        attrs.insert("domain", c.domain.clone());
    }
    let label = match c.user.as_str() {
        "" => format!("{}://{}/", addr.scheme, addr.host),
        user => format!("{}://{user}@{}/", addr.scheme, addr.host),
    };
    let attrs: HashMap<&str, &str> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    secrets.store(&label, &attrs, &c.password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_in_the_address_answers_the_first_prompt() {
        let addr = Address::parse("smb://alice:pw@nas/share").unwrap();
        let c = in_address(&addr).unwrap();
        assert_eq!((c.user.as_str(), c.password.as_str()), ("alice", "pw"));
        assert!(!c.remember);
        assert!(in_address(&Address::parse("smb://alice@nas/share").unwrap()).is_none());
    }
}
