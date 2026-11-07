//! The Remote Desktop page's switch and status: read off the loop while the page shows, again
//! every [`POLL`], and the switch, New Certificate and Disconnect carried out at once.

use crate::registry::Page;
use crate::{SettingsApp, loginhost, pages, registry, take};
use reprisebus::remote::{self, Status};
use reprisekit::controls::form::Form;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(2);

/// How long a switch turned on waits for the server to answer before showing what it found.
const START_WAIT: Duration = Duration::from_secs(2);

/// What the page shows: the unit's switch, what the server reports, this computer's address,
/// and whether the login manager runs remote logins, with its certificate's fingerprint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Remote {
    pub(crate) enabled: bool,
    pub(crate) status: Option<Status>,
    pub(crate) address: Option<String>,
    pub(crate) manager: bool,
    pub(crate) machine_fingerprint: Option<String>,
}

/// A read or an act off the loop: the state afterwards, and what to tell when the act failed.
pub(crate) struct Reading {
    pub(crate) remote: Remote,
    pub(crate) failed: Option<String>,
}

/// What a button or the switch asks of the server.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Act {
    Allow(bool),
    /// End the client at this address, as `Clients` lists it.
    Disconnect(String),
    NewCertificate,
}

/// Disconnect for the client selected on `page`, as `status` lists them.
pub(crate) fn disconnect(page: &Form, status: Option<&Status>) -> Option<Act> {
    pages::selected_client(page, status).map(|a| Act::Disconnect(a.into()))
}

/// The unit, the server and the login manager as they are now.
fn read() -> Remote {
    let status = remote::status();
    let manager = reprisebus::login::manager_runs();
    Remote {
        enabled: remote::enabled(),
        address: status.as_ref().and_then(|_| address()),
        status,
        manager,
        machine_fingerprint: manager.then(loginhost::machine_fingerprint).flatten(),
    }
}

/// This computer's address on the default route's interface: the source address the kernel
/// picks for a documentation address. Connecting a UDP socket sends nothing.
fn address() -> Option<String> {
    let probe = |bind: &str, to: &str| {
        let socket = UdpSocket::bind(bind).ok()?;
        socket.connect(to).ok()?;
        socket.local_addr().ok().map(|a| a.ip())
    };
    probe("0.0.0.0:0", "192.0.2.1:9")
        .or_else(|| probe("[::]:0", "[2001:db8::1]:9"))
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .map(|ip| ip.to_string())
}

fn act(act: Act) -> Reading {
    let failed = match act {
        Act::Allow(on) => remote::set_enabled(on).err().map(|why| {
            let to = if on { "turned on" } else { "turned off" };
            format!("Remote Desktop could not be {to}.\n\n{why}")
        }),
        Act::Disconnect(address) => remote::disconnect_client(&address)
            .err()
            .and_then(disconnect_failed),
        Act::NewCertificate => (remote::regenerate().err())
            .map(|why| format!("A new certificate could not be made.\n\n{why}")),
    };
    let started = Instant::now();
    let mut remote = read();
    while remote.enabled && remote.status.is_none() && started.elapsed() < START_WAIT {
        std::thread::sleep(Duration::from_millis(100));
        remote = read();
    }
    Reading { remote, failed }
}

/// What to tell when Disconnect failed; nothing when the client left before it was asked to.
fn disconnect_failed(why: String) -> Option<String> {
    (!why.contains("no client at"))
        .then(|| format!("The remote computer could not be disconnected.\n\n{why}"))
}

impl SettingsApp {
    /// Read the server off the loop when the page opens; [`SettingsApp::remote_done`] goes on
    /// polling while it stays open.
    pub(crate) fn ensure_remote(&mut self) {
        if registry::open(&self.sheet) == Page::RemoteDesktop && self.remote_job.is_none() {
            self.start_remote(|| Reading {
                remote: read(),
                failed: None,
            });
        }
    }

    /// Carry out `what` off the loop, the switch and buttons greyed meanwhile. A poll under
    /// way is dropped, since it may have read the state from before.
    pub(crate) fn remote_act(&mut self, what: Act) {
        if self.remote_acting {
            self.show_remote(self.selected_client());
            return;
        }
        let page = &mut self.sheet.pages[Page::RemoteDesktop.index()];
        for id in [
            pages::REMOTE_ALLOW,
            pages::REMOTE_REGENERATE,
            pages::REMOTE_DISCONNECT,
        ] {
            page.set_enabled(id, false);
        }
        self.remote_acting = true;
        self.start_remote(move || act(what));
    }

    pub(crate) fn remote_disconnect(&mut self) {
        let page = &self.sheet.pages[Page::RemoteDesktop.index()];
        if let Some(act) = disconnect(page, self.remote.status.as_ref()) {
            self.remote_act(act);
        }
    }

    pub(crate) fn remote_toggled(&mut self) {
        let on = self.sheet.pages[Page::RemoteDesktop.index()].checked(pages::REMOTE_ALLOW);
        self.remote_act(Act::Allow(on == Some(true)));
    }

    /// A read or an act finished: show it, and read again in [`POLL`] while the page is open.
    /// True when the page changed.
    pub(crate) fn remote_done(&mut self) -> bool {
        let Some(rx) = &self.remote_job else {
            return false;
        };
        let done = match take(rx) {
            Ok(None) => return false,
            Ok(Some(r)) => r,
            Err(()) => Reading {
                remote: self.remote.clone(),
                failed: None,
            },
        };
        self.remote_job = None;
        let acted = std::mem::take(&mut self.remote_acting);
        if let Some(why) = &done.failed {
            self.fail(why);
        }
        let changed = acted || done.remote != self.remote;
        let keep = self.selected_client();
        self.remote = done.remote;
        if changed {
            self.show_remote(keep);
        }
        self.ensure_account();
        if registry::open(&self.sheet) == Page::RemoteDesktop {
            self.start_remote(|| {
                std::thread::sleep(POLL);
                Reading {
                    remote: read(),
                    failed: None,
                }
            });
        }
        changed
    }

    /// Run `job` off the loop as the page's read or act.
    #[cfg(not(test))]
    fn start_remote(&mut self, job: impl FnOnce() -> Reading + Send + 'static) {
        self.remote_job = Some(self.spawn(job));
    }

    /// Tests feed `remote_job` themselves, so nothing reaches the session bus or systemd.
    #[cfg(test)]
    fn start_remote(&mut self, _: impl FnOnce() -> Reading + Send + 'static) {}

    /// The address of the client selected on the page, which shows `self.remote`.
    fn selected_client(&self) -> Option<String> {
        let page = &self.sheet.pages[Page::RemoteDesktop.index()];
        pages::selected_client(page, self.remote.status.as_ref()).map(str::to_owned)
    }

    /// Put what is known of the server on the page, the client at `keep` selected; the page is
    /// built again when the login manager started or stopped.
    pub(crate) fn show_remote(&mut self, keep: Option<String>) {
        let at = Page::RemoteDesktop.index();
        let r = &self.remote;
        let machine = r.manager.then(|| {
            r.machine_fingerprint
                .as_deref()
                .unwrap_or(pages::NOT_RUNNING)
        });
        if pages::managed(&self.sheet.pages[at]) != r.manager {
            let page = pages::remote_desktop(self.area(), None, false, machine);
            self.sheet.set_page(at, page);
            pages::write_form(&mut self.sheet.pages[at], &self.want);
            pages::follow(&mut self.sheet, &self.want);
        }
        let r = &self.remote;
        pages::show_remote(
            &mut self.sheet.pages[at],
            r.status.as_ref(),
            r.enabled,
            r.address.as_deref(),
            keep.as_deref(),
            machine,
        );
        self.show_account();
    }
}

#[cfg(test)]
mod tests {
    use super::disconnect_failed;

    #[test]
    fn disconnecting_a_client_that_already_left_tells_nothing() {
        assert_eq!(
            disconnect_failed("no client at 192.168.1.30:51234".into()),
            None
        );
        let told = disconnect_failed("the server does not answer".into());
        assert_eq!(
            told.as_deref(),
            Some("The remote computer could not be disconnected.\n\nthe server does not answer")
        );
    }
}
