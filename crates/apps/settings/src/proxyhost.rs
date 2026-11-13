//! The Network page's proxy: read from its `environment.d` file when Settings opens; Apply writes
//! the file and hands the variables to the systemd user manager, so programs started afterwards
//! use them.

use crate::registry::Page;
use crate::{SettingsApp, pages};
use reprisefs::proxy;
use std::path::PathBuf;

/// Runs each `systemctl --user` argument list in order ([`proxy::manager_args`]).
pub(crate) type Publish = fn(&[Vec<String>]) -> Result<(), String>;

pub(crate) struct Proxy {
    pub(crate) file: PathBuf,
    /// The file's variables as last read or written: what the next Apply unsets when it stops
    /// setting them.
    pub(crate) vars: Vec<(String, String)>,
    pub(crate) loaded: proxy::Proxy,
    pub(crate) want: proxy::Proxy,
    pub(crate) publish: Publish,
}

impl Proxy {
    pub(crate) fn open(file: PathBuf, publish: Publish) -> Proxy {
        let vars = proxy::read(&file);
        let loaded = proxy::Proxy::from_vars(&vars);
        Proxy {
            file,
            vars,
            want: loaded.clone(),
            loaded,
            publish,
        }
    }

    /// The user's file and manager; tests get a file that is never there and a manager that
    /// takes nothing.
    pub(crate) fn session() -> Proxy {
        if cfg!(test) {
            Proxy::open(PathBuf::from("/nonexistent/reprise-proxy.conf"), |_| Ok(()))
        } else {
            Proxy::open(proxy::path(), systemctl)
        }
    }
}

pub(crate) fn systemctl(args: &[Vec<String>]) -> Result<(), String> {
    for a in args {
        let out = std::process::Command::new("systemctl")
            .arg("--user")
            .args(a)
            .output()
            .map_err(|e| format!("systemctl: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
    }
    Ok(())
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

    /// Write the file and set the manager's variables. False when a field is wrong or a write
    /// failed; a field left wrong keeps the sheet dirty.
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
        let args = proxy::manager_args(&self.proxy.vars, &vars);
        self.proxy.vars = (vars.into_iter())
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        self.proxy.loaded = self.proxy.want.clone();
        if let Err(e) = (self.proxy.publish)(&args) {
            log::warn!("systemctl --user {args:?}: {e}");
            self.fail("The proxy settings were saved and take effect at the next log on; this session could not take them.");
            return false;
        }
        true
    }
}
