//! One running copy of a program. The first start owns a well-known name
//! on the session bus and serves `org.freedesktop.Application` under it;
//! a later start hands its request to that copy ([`claim`]) and exits.
//! The later start's xdg-activation token and its by-key mark ride along as
//! platform data ([`Handoff`]), so the running copy can raise its window with
//! them.

use crate::mutex::lock;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::{OwnedValue, Value};

const INTERFACE: &str = "org.freedesktop.Application";
const TOKEN: &str = "activation-token";
const BY_KEY: &str = "reprise-keyboard-launch";

/// Set to `1` on a program the desktop started from the keyboard: its first window opens
/// with cues showing.
pub const KEYBOARD_LAUNCH_ENV: &str = "REPRISE_KEYBOARD_LAUNCH";

/// What a start hands the running copy: its xdg-activation token, and whether
/// it came from the keyboard, so the raised window shows its cues.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Handoff {
    pub token: Option<String>,
    pub by_key: bool,
}

impl Handoff {
    /// This process's own start, from `XDG_ACTIVATION_TOKEN` and [`KEYBOARD_LAUNCH_ENV`].
    pub fn from_env() -> Handoff {
        Handoff {
            token: std::env::var("XDG_ACTIVATION_TOKEN").ok(),
            by_key: std::env::var_os(KEYBOARD_LAUNCH_ENV).is_some_and(|v| v == "1"),
        }
    }

    /// As `org.freedesktop.Application` platform data: `activation-token`
    /// and `reprise-keyboard-launch`.
    pub fn platform_data(&self) -> HashMap<&'static str, Value<'static>> {
        let mut data = HashMap::new();
        if let Some(token) = &self.token {
            data.insert(TOKEN, Value::from(token.clone()));
        }
        if self.by_key {
            data.insert(BY_KEY, Value::from(true));
        }
        data
    }

    pub fn from_platform_data(data: &HashMap<String, OwnedValue>) -> Handoff {
        Handoff {
            token: data
                .get(TOKEN)
                .and_then(|v| <&str>::try_from(&**v).ok())
                .map(str::to_string),
            by_key: data
                .get(BY_KEY)
                .and_then(|v| bool::try_from(&**v).ok())
                .unwrap_or(false),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// An action and its string parameters (`ActivateAction`), None for a
    /// plain `Activate`.
    pub action: Option<(String, Vec<String>)>,
    pub handoff: Handoff,
}

/// What [`claim`] found.
pub enum Claim {
    First(Requests),
    /// Another copy runs and took the request: exit.
    Handed,
    /// No session bus, or no answer from the copy that owns the name: run
    /// as if alone.
    Alone,
}

type Wake = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

pub struct Requests {
    rx: Receiver<Request>,
    wake: Wake,
}

impl Requests {
    /// Call `wake` after each request from now on. One that came before
    /// waits in [`Requests::next`] without a call.
    pub fn wake_with(&self, wake: impl Fn() + Send + 'static) {
        *lock(&self.wake) = Some(Box::new(wake));
    }

    pub fn next(&self) -> Option<Request> {
        self.rx.try_recv().ok()
    }
}

/// Own `name`, or hand `action` (None: just raise the window) and this
/// start's [`Handoff`] to the copy that owns it.
pub fn claim(name: &str, action: Option<(String, Vec<String>)>) -> Claim {
    let conn = match crate::session() {
        Ok(c) => c,
        Err(e) => {
            log::info!("no session bus, so no single instance: {e}");
            return Claim::Alone;
        }
    };
    let path = path(name);
    let (tx, rx) = channel();
    let wake = Wake::default();
    // Served before the name is owned, so no caller finds the name without it.
    let app = Application {
        tx,
        wake: wake.clone(),
    };
    if let Err(e) = conn.object_server().at(path.as_str(), app) {
        log::warn!("could not serve {INTERFACE}: {e}");
        return Claim::Alone;
    }
    match conn.request_name_with_flags(name, RequestNameFlags::DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner) => return Claim::First(Requests { rx, wake }),
        Ok(_) | Err(zbus::Error::NameTaken) => {}
        Err(e) => {
            log::warn!("could not own {name}: {e}");
            return Claim::Alone;
        }
    }
    let _ = conn.object_server().remove::<Application, _>(path.as_str());
    let data = Handoff::from_env().platform_data();
    let sent = match action {
        Some((action, params)) => {
            let params: Vec<Value> = params.into_iter().map(Value::from).collect();
            conn.call_method(
                Some(name),
                path.as_str(),
                Some(INTERFACE),
                "ActivateAction",
                &(action, params, data),
            )
        }
        None => conn.call_method(
            Some(name),
            path.as_str(),
            Some(INTERFACE),
            "Activate",
            &(data,),
        ),
    };
    match sent {
        Ok(_) => Claim::Handed,
        Err(e) => {
            log::warn!("the running copy of {name} did not answer: {e}");
            Claim::Alone
        }
    }
}

/// The object path `org.freedesktop.Application` names for a bus name:
/// `/org/reprise/Settings` for `org.reprise.Settings`.
fn path(name: &str) -> String {
    format!("/{}", name.replace('.', "/"))
}

struct Application {
    tx: Sender<Request>,
    wake: Wake,
}

impl Application {
    fn send(&self, action: Option<(String, Vec<String>)>, data: &HashMap<String, OwnedValue>) {
        let handoff = Handoff::from_platform_data(data);
        if self.tx.send(Request { action, handoff }).is_ok()
            && let Some(wake) = &*lock(&self.wake)
        {
            wake();
        }
    }
}

#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, platform_data: HashMap<String, OwnedValue>) {
        self.send(None, &platform_data);
    }

    /// Files are not something a Reprise program opens this way; the
    /// window is raised.
    fn open(&self, _uris: Vec<String>, platform_data: HashMap<String, OwnedValue>) {
        self.send(None, &platform_data);
    }

    fn activate_action(
        &self,
        action_name: String,
        parameter: Vec<OwnedValue>,
        platform_data: HashMap<String, OwnedValue>,
    ) {
        let params = parameter
            .iter()
            .filter_map(|v| <&str>::try_from(&**v).ok().map(str::to_string))
            .collect();
        self.send(Some((action_name, params)), &platform_data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received(h: &Handoff) -> Handoff {
        let data = h
            .platform_data()
            .into_iter()
            .map(|(k, v)| (k.to_string(), OwnedValue::try_from(v).expect("owned")))
            .collect();
        Handoff::from_platform_data(&data)
    }

    #[test]
    fn a_handoff_survives_the_platform_data() {
        let h = Handoff {
            token: Some("t-1".into()),
            by_key: true,
        };
        assert_eq!(received(&h), h);
        let pointer = Handoff {
            token: Some("t-2".into()),
            by_key: false,
        };
        assert!(!pointer.platform_data().contains_key(BY_KEY));
        assert_eq!(received(&pointer), pointer);
        assert_eq!(received(&Handoff::default()), Handoff::default());
    }
}
