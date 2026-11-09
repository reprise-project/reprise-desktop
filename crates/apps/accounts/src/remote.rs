//! Your Remote Desktop password through the login manager's account socket: what the Password
//! page's section reads of it, and the create, change and remove requests its dialogs send, each
//! off the loop.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use reprisebus::login;
use reprisekit::controls::remotepassword::{Access, Action, Answer, Facts, Manager};
use repriselogin::account::{
    ACCOUNT_SOCKET, AccountRefusal, AccountReply, AccountRequest, RemoteAccess,
};
use repriselogin::frame::{read_msg, write_msg};

/// How long a create, change or remove may take: past the login manager's own bound, its 5 s
/// group look-up and 25 s password check, with room for storing the password.
pub const TIMEOUT: Duration = Duration::from_secs(40);
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// `req` sent over one connection to the socket at `path`, and the one reply read back.
pub fn ask(path: &Path, req: &AccountRequest, timeout: Duration) -> Result<AccountReply, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut s = UnixStream::connect(path).map_err(|e| at(&e))?;
    s.set_read_timeout(Some(timeout)).map_err(|e| at(&e))?;
    s.set_write_timeout(Some(timeout)).map_err(|e| at(&e))?;
    write_msg(&mut s, req).map_err(|e| at(&e))?;
    read_msg(&mut s).map_err(|e| at(&e))
}

/// Where a read looks; tests give their own.
#[derive(Clone)]
pub struct Sources {
    pub socket: PathBuf,
    pub manager: fn() -> login::Manager,
    /// How long a read waits after the login manager did not answer.
    pub retry: Duration,
}

impl Default for Sources {
    /// This computer's; under test no login manager, so nothing reaches the bus.
    fn default() -> Sources {
        Sources {
            socket: ACCOUNT_SOCKET.into(),
            manager: if cfg!(test) {
                || login::Manager::Missing
            } else {
                login::manager
            },
            retry: Duration::from_secs(5),
        }
    }
}

/// What one read found: everything [`Facts`] holds but whose account is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub manager: Manager,
    /// What the login manager answered; `Allowed` until it answers.
    pub access: Access,
}

impl Found {
    fn idle(manager: Manager) -> Found {
        Found {
            manager,
            access: Access::Allowed,
        }
    }

    pub fn silent(&self) -> bool {
        self.manager == Manager::Running(Answer::Silent)
    }
}

/// The login manager's state and, while it runs, its answer on your Remote Desktop password.
/// A failed ask logs at `failed`.
pub fn read(src: &Sources, failed: log::Level) -> Found {
    let silent = |why: String| {
        log::log!(failed, "account status: {why}");
        (Answer::Silent, Access::Allowed)
    };
    let (answer, access) = match (src.manager)() {
        login::Manager::Missing => return Found::idle(Manager::Missing),
        login::Manager::Stopped => return Found::idle(Manager::Stopped),
        login::Manager::Running => {
            match ask(&src.socket, &AccountRequest::Status, STATUS_TIMEOUT) {
                Ok(AccountReply::Status {
                    remote_password,
                    access,
                }) => (Answer::Has(remote_password), access_of(access)),
                Ok(other) => silent(format!("answered {other:?}")),
                Err(e) => silent(e),
            }
        }
    };
    Found {
        manager: Manager::Running(answer),
        access,
    }
}

fn access_of(access: RemoteAccess) -> Access {
    match access {
        RemoteAccess::Allowed => Access::Allowed,
        RemoteAccess::Off => Access::RemoteLoginsOff,
        RemoteAccess::NotInGroup { group } => Access::NotInGroup(group),
        RemoteAccess::Root => Access::Root,
    }
}

type Asked = Receiver<Result<AccountReply, String>>;

/// A create, change or remove under way, sent from the dialog of generation `from`.
pub struct Change {
    pub action: Action,
    pub from: u64,
    pub rx: Asked,
}

/// What is known of your Remote Desktop password, and the read and change under way.
#[derive(Default)]
pub struct Remote {
    pub found: Option<Found>,
    pub reading: Option<Receiver<Found>>,
    pub change: Option<Change>,
    /// The generation the next dialog opens with.
    pub opened: u64,
}

/// What a finished change did.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    Done,
    Refused(AccountRefusal),
}

impl Remote {
    /// The section's facts for an account that is `yours`; `None` until the first read lands.
    pub fn facts(&self, yours: bool) -> Option<Facts> {
        let f = self.found.as_ref()?;
        Some(Facts {
            manager: f.manager,
            yours,
            access: f.access.clone(),
            busy: self.change.is_some(),
        })
    }

    /// The read, once it is in.
    pub fn read_landed(&mut self) -> Option<&Found> {
        let got = match self.reading.as_ref()?.try_recv() {
            Ok(found) => found,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.reading = None;
                return None;
            }
        };
        self.reading = None;
        self.found = Some(got);
        self.found.as_ref()
    }

    /// The change, once its reply is in, with the dialog generation that sent it. A `Done`
    /// shows at once, a remove leaving you none and a create or change one, and outdates a read
    /// still under way.
    pub fn change_landed(&mut self) -> Option<(u64, Ended)> {
        let c = self.change.as_ref()?;
        let got = match c.rx.try_recv() {
            Ok(got) => got,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the request ended without a reply".into()),
        };
        let Change { action, from, .. } = self.change.take()?;
        let ended = match got {
            Ok(AccountReply::Done) => {
                if let Some(Found {
                    manager: m @ Manager::Running(_),
                    ..
                }) = &mut self.found
                {
                    *m = Manager::Running(Answer::Has(action != Action::Remove));
                }
                self.reading = None;
                Ended::Done
            }
            Ok(AccountReply::Refused { reason }) => Ended::Refused(reason),
            Ok(other) => {
                log::warn!("Remote Desktop password: answered {other:?}");
                Ended::Refused(AccountRefusal::Failed)
            }
            Err(e) => {
                log::warn!("Remote Desktop password: {e}");
                Ended::Refused(AccountRefusal::Failed)
            }
        };
        Some((from, ended))
    }
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
