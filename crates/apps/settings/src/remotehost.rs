//! The Remote Desktop page's switch and status: read off the loop while the page shows, again
//! every [`POLL`], and the switch, New Certificate and Disconnect carried out at once.

use crate::registry::Page;
use crate::{SettingsApp, pages, registry, take};
use reprisebus::remote::{self, Status};
use std::net::UdpSocket;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(2);

/// How long a switch turned on waits for the server to answer before showing what it found.
const START_WAIT: Duration = Duration::from_secs(2);

/// What the page shows: the unit's switch, what the server reports, and this computer's address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Remote {
    pub(crate) enabled: bool,
    pub(crate) status: Option<Status>,
    pub(crate) address: Option<String>,
}

/// A read or an act off the loop: the state afterwards, and what to tell when the act failed.
pub(crate) struct Reading {
    pub(crate) remote: Remote,
    pub(crate) failed: Option<String>,
}

/// What a button or the switch asks of the server.
pub(crate) enum Act {
    Allow(bool),
    Disconnect,
    NewCertificate,
}

/// The unit and the server as they are now.
fn read() -> Remote {
    let status = remote::status();
    Remote {
        enabled: remote::enabled(),
        address: status.as_ref().and_then(|_| address()),
        status,
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
        Act::Disconnect => (remote::disconnect().err())
            .map(|why| format!("The remote computer could not be disconnected.\n\n{why}")),
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
            self.show_remote();
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
        self.remote = done.remote;
        if changed {
            self.show_remote();
        }
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

    /// Put what is known of the server on the page.
    pub(crate) fn show_remote(&mut self) {
        let r = &self.remote;
        pages::show_remote(
            &mut self.sheet.pages[Page::RemoteDesktop.index()],
            r.status.as_ref(),
            r.enabled,
            r.address.as_deref(),
        );
    }
}
