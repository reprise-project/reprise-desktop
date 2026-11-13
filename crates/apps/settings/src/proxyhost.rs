//! The Network page's proxy: read from its `environment.d` file when Settings opens; Apply writes
//! the file and, off the loop, hands the variables to the systemd user manager, so programs
//! started afterwards use them.

use crate::registry::Page;
use crate::{SettingsApp, pages, take};
use reprisefs::proxy;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

/// Hands a change to the user manager's environment ([`proxy::change`]). Blocking.
pub(crate) type Publish = fn(&proxy::Change) -> Result<(), String>;

pub(crate) struct Proxy {
    pub(crate) file: PathBuf,
    /// The file's variables as last read or written.
    pub(crate) vars: Vec<(String, String)>,
    /// The variables last handed to the manager: what the next change unsets when it stops
    /// setting them.
    pub(crate) sent: Vec<(String, String)>,
    pub(crate) loaded: proxy::Proxy,
    pub(crate) want: proxy::Proxy,
    pub(crate) publish: Publish,
    /// The manager call under way.
    pub(crate) job: Option<Receiver<Result<(), String>>>,
    /// An Apply came while the call ran: hand the manager the file again when it ends.
    pub(crate) again: bool,
    /// The sheet closes once the call ends.
    pub(crate) quit: bool,
}

impl Proxy {
    pub(crate) fn open(file: PathBuf, publish: Publish) -> Proxy {
        let vars = proxy::read(&file);
        let loaded = proxy::Proxy::from_vars(&vars);
        Proxy {
            file,
            sent: vars.clone(),
            vars,
            want: loaded.clone(),
            loaded,
            publish,
            job: None,
            again: false,
            quit: false,
        }
    }

    /// The user's file and manager; tests get a file that is never there and a manager that
    /// takes nothing.
    pub(crate) fn session() -> Proxy {
        if cfg!(test) {
            Proxy::open(PathBuf::from("/nonexistent/reprise-proxy.conf"), |_| Ok(()))
        } else {
            Proxy::open(proxy::path(), manager)
        }
    }
}

fn manager(c: &proxy::Change) -> Result<(), String> {
    reprisebus::units::set_user_environment(&c.unset, &c.set).map_err(|e| e.to_string())
}

/// The log line for a change the manager refused: the variables' names, never their values,
/// which may hold a password.
pub(crate) fn refused(c: &proxy::Change, why: &str) -> String {
    format!(
        "the user manager did not take {}: {why}",
        c.names().join(" ")
    )
}

impl SettingsApp {
    /// Take `p` as the proxy the file holds and show it.
    #[cfg(test)]
    pub(crate) fn take_proxy(&mut self, p: Proxy) {
        self.proxy = p;
        let page = &mut self.sheet.pages[Page::Network.index()];
        pages::show_proxy(page, &self.proxy.want);
    }

    /// A proxy row moved: read the rows back, and grey those of the other modes.
    pub(crate) fn proxy_changed(&mut self) {
        let page = &mut self.sheet.pages[Page::Network.index()];
        self.proxy.want = pages::read_proxy(page);
        self.sync_apply();
    }

    pub(crate) fn proxy_dirty(&self) -> bool {
        self.proxy.want != self.proxy.loaded
    }

    pub(crate) fn revert_proxy(&mut self) {
        self.proxy.want = self.proxy.loaded.clone();
        let page = &mut self.sheet.pages[Page::Network.index()];
        pages::show_proxy(page, &self.proxy.want);
    }

    /// Write the file and start setting the manager's variables. False when a field is wrong or
    /// the write failed; a field left wrong keeps the sheet dirty.
    pub(crate) fn apply_proxy(&mut self) -> bool {
        if !self.proxy_dirty() {
            return true;
        }
        let vars = match self.proxy.want.vars() {
            Ok(v) => v,
            Err(why) => {
                self.fail(&why);
                return false;
            }
        };
        if let Err(e) = proxy::write(&self.proxy.file, &vars) {
            log::warn!("write {}: {e}", self.proxy.file.display());
            self.fail("The proxy settings could not be written.");
            return false;
        }
        self.proxy.vars = (vars.into_iter())
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        self.proxy.loaded = self.proxy.want.clone();
        self.publish_proxy();
        true
    }

    /// Hand the manager what changed since it last heard, unless a call is still under way.
    fn publish_proxy(&mut self) {
        if self.proxy.job.is_some() {
            self.proxy.again = true;
            return;
        }
        let new: Vec<(&str, String)> = (self.proxy.vars.iter())
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        let change = proxy::change(&self.proxy.sent, &new);
        self.proxy.sent = self.proxy.vars.clone();
        if change.is_empty() {
            return;
        }
        let publish = self.proxy.publish;
        let job = move || (publish)(&change).map_err(|e| refused(&change, &e));
        self.proxy.job = Some(self.start_publish(job));
    }

    #[cfg(not(test))]
    fn start_publish(
        &mut self,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> Receiver<Result<(), String>> {
        self.spawn(job)
    }

    /// Tests run the call at once, against the `Publish` they gave.
    #[cfg(test)]
    fn start_publish(
        &mut self,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> Receiver<Result<(), String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(job());
        rx
    }

    /// The manager call ended: say when it failed, and send what changed meanwhile. Some(true)
    /// when the sheet waited on it to close.
    pub(crate) fn proxy_done(&mut self) -> Option<bool> {
        let done = match take(self.proxy.job.as_ref()?) {
            Ok(None) => return None,
            Ok(Some(r)) => r,
            Err(()) => Err("the user manager call ended without a result".into()),
        };
        self.proxy.job = None;
        if let Err(e) = done {
            log::warn!("{e}");
            if !self.proxy.quit {
                self.fail("The proxy settings were saved and take effect at the next log on; this session could not take them.");
            }
        }
        if std::mem::take(&mut self.proxy.again) {
            self.publish_proxy();
        }
        match self.proxy.job {
            Some(_) => None,
            None => Some(std::mem::take(&mut self.proxy.quit)),
        }
    }
}
