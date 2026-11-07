//! Your Remote Desktop password through the login manager's account socket: its status on the
//! Remote Desktop page and the Set... and Clear... dialogs, one request each, off the loop.

use crate::rdpassworddlg::{self, Purpose, RdPasswordDialog};
use crate::registry::Page;
use crate::{SettingsApp, pages, take};
use repriseapp::keys::form_input;
use repriseapp::{Ctx, Event, Reply};
use repriselogin::account::{ACCOUNT_SOCKET, AccountRefusal, AccountReply, AccountRequest};
use repriselogin::frame::{read_msg, write_msg};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// How long a reply may take; a set or clear waits for the login password's check.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);

/// `req` sent over one connection to the socket at `path`, and the one reply read back.
pub(crate) fn ask(
    path: &Path,
    req: &AccountRequest,
    timeout: Duration,
) -> Result<AccountReply, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut s = UnixStream::connect(path).map_err(|e| at(&e))?;
    s.set_read_timeout(Some(timeout)).map_err(|e| at(&e))?;
    s.set_write_timeout(Some(timeout)).map_err(|e| at(&e))?;
    write_msg(&mut s, req).map_err(|e| at(&e))?;
    read_msg(&mut s).map_err(|e| at(&e))
}

/// The socket Settings asks; none under test, which give their own.
pub(crate) fn socket() -> Option<PathBuf> {
    (!cfg!(test)).then(|| ACCOUNT_SOCKET.into())
}

type Asked = Receiver<Result<AccountReply, String>>;

/// The reply once it is in; a job that ended without one is a failure.
fn landed(rx: &Asked) -> Option<Result<AccountReply, String>> {
    match take(rx) {
        Ok(got) => got,
        Err(()) => Some(Err("the request ended without a reply".into())),
    }
}

/// The password's status as the daemon last told it, and the requests under way.
#[derive(Default)]
pub(crate) struct Account {
    pub(crate) set: Option<bool>,
    asked: bool,
    status: Option<Asked>,
    change: Option<(Purpose, Asked)>,
}

impl SettingsApp {
    /// Ask for the status once while the login manager runs remote logins; forget it when it
    /// stops.
    pub(crate) fn ensure_account(&mut self) {
        if !self.remote.manager {
            self.account = Account::default();
            return;
        }
        let Some(path) = self.account_socket.clone() else {
            return;
        };
        if self.account.asked {
            return;
        }
        self.account.asked = true;
        self.account.status =
            Some(self.login_spawn(move || ask(&path, &AccountRequest::Status, TIMEOUT)));
    }

    pub(crate) fn show_account(&mut self) {
        pages::show_rd_password(
            &mut self.sheet.pages[Page::RemoteDesktop.index()],
            self.account.set,
        );
    }

    pub(crate) fn open_rd_password(&mut self, purpose: Purpose) -> Reply {
        self.rd_password = Some(RdPasswordDialog::new(purpose));
        Reply::redraw_all()
    }

    pub(crate) fn rd_password_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.rd_password.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => rdpassworddlg::Outcome::Cancel,
                Event::Blink => {
                    d.form.blink(ctx.caret_on);
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        self.rd_password_outcome(out)
    }

    /// Carry out what the dialog asked. A request runs off the loop with the dialog open; a
    /// Cancel drops the dialog and its fields, and a request under way still lands.
    pub(crate) fn rd_password_outcome(&mut self, out: rdpassworddlg::Outcome) -> Reply {
        match out {
            rdpassworddlg::Outcome::None => Reply::none(),
            rdpassworddlg::Outcome::Redraw => Reply::redraw(),
            rdpassworddlg::Outcome::Cancel => {
                self.rd_password = None;
                Reply::redraw_all()
            }
            rdpassworddlg::Outcome::Send(req) => {
                let (Some(path), Some(d)) = (self.account_socket.clone(), &self.rd_password) else {
                    if let Some(d) = &mut self.rd_password {
                        d.refused(AccountRefusal::Failed);
                    }
                    return Reply::redraw();
                };
                let purpose = d.purpose;
                let rx = self.login_spawn(move || ask(&path, &req, TIMEOUT));
                self.account.change = Some((purpose, rx));
                Reply::redraw()
            }
        }
    }

    /// A status or a change came back: show it on the page, or in the dialog when refused. True
    /// when anything changed.
    pub(crate) fn account_done(&mut self) -> bool {
        let mut changed = false;
        if let Some(got) = self.account.status.as_ref().and_then(landed) {
            self.account.status = None;
            match got {
                Ok(AccountReply::Status { remote_password }) => {
                    self.account.set = Some(remote_password);
                    changed = true;
                }
                Ok(other) => log::warn!("account status: answered {other:?}"),
                Err(e) => log::warn!("account status: {e}"),
            }
        }
        let done =
            (self.account.change.as_ref()).and_then(|(purpose, rx)| Some((*purpose, landed(rx)?)));
        if let Some((purpose, got)) = done {
            self.account.change = None;
            changed = true;
            let refused = match got {
                Ok(AccountReply::Done) => {
                    self.account.set = Some(purpose == Purpose::Set);
                    self.rd_password = None;
                    None
                }
                Ok(AccountReply::Refused { reason }) => Some(reason),
                Ok(other) => {
                    log::warn!("Remote Desktop password: answered {other:?}");
                    Some(AccountRefusal::Failed)
                }
                Err(e) => {
                    log::warn!("Remote Desktop password: {e}");
                    Some(AccountRefusal::Failed)
                }
            };
            if let (Some(r), Some(d)) = (refused, &mut self.rd_password) {
                d.refused(r);
            }
        }
        if changed {
            self.show_account();
        }
        changed
    }
}
