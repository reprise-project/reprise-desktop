//! One running copy of a program. The first start owns a well-known name
//! on the session bus and serves `org.freedesktop.Application` under it;
//! a later start hands its request to that copy ([`claim`]) and exits.
//! The later start's xdg-activation token rides along as the
//! `activation-token` platform data, so the running copy can raise its
//! window with it.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::{OwnedValue, Value};

const INTERFACE: &str = "org.freedesktop.Application";
const TOKEN: &str = "activation-token";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// An action and its string parameters (`ActivateAction`), None for a
    /// plain `Activate`.
    pub action: Option<(String, Vec<String>)>,
    pub token: Option<String>,
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
        *self.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(wake));
    }

    pub fn next(&self) -> Option<Request> {
        self.rx.try_recv().ok()
    }
}

/// Own `name`, or hand `action` (None: just raise the window) and this
/// start's `XDG_ACTIVATION_TOKEN` to the copy that owns it.
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
    let mut data = HashMap::new();
    if let Ok(token) = std::env::var("XDG_ACTIVATION_TOKEN") {
        data.insert(TOKEN, Value::from(token));
    }
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
        let token = data
            .get(TOKEN)
            .and_then(|v| <&str>::try_from(&**v).ok())
            .map(str::to_string);
        if self.tx.send(Request { action, token }).is_ok()
            && let Some(wake) = &*self.wake.lock().unwrap_or_else(|e| e.into_inner())
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
