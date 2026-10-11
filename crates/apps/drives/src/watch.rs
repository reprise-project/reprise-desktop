//! The UDisks2 side: [`udisks::follow`] hands the app a whole [`Snapshot`] after
//! every change, and the calls the app asks for, each on its own thread (any can wait on polkit).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use reprisefs::udisks::{self, Error, Snapshot, Udisks};
use zeroize::Zeroizing;

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const LOGIN: &str = "org.freedesktop.login1";
const SESSION_IFACE: &str = "org.freedesktop.login1.Session";

#[derive(Debug)]
pub enum Msg {
    Snapshot(Snapshot),
    Locked(String),
    Arrived(crate::model::Arrival),
    Mounted {
        volume: String,
        result: Result<PathBuf, Error>,
    },
    Unlocked {
        volume: String,
        result: Result<String, Error>,
    },
    Removed {
        disk: String,
        result: Result<(), Error>,
    },
}

#[derive(Clone)]
struct Poster {
    tx: Sender<Msg>,
    waker: repriseapp::Waker,
}

impl Poster {
    fn post(&self, m: Msg) {
        if self.tx.send(m).is_ok() {
            self.waker.wake();
        }
    }
}

#[derive(Clone)]
pub struct Calls {
    udisks: Option<Udisks>,
    poster: Poster,
}

impl Calls {
    fn spawn(&self, name: &str, f: impl FnOnce(&Udisks, &Poster) + Send + 'static) {
        let Some(u) = self.udisks.clone() else {
            log::warn!("no system bus; {name} dropped");
            return;
        };
        let p = self.poster.clone();
        if let Err(e) = std::thread::Builder::new()
            .name(format!("reprise-drives-{name}"))
            .spawn(move || f(&u, &p))
        {
            log::warn!("could not start {name}: {e}");
        }
    }

    pub fn arrived(&self, arrival: crate::model::Arrival) {
        self.spawn("arrival", move |u, p| {
            if session_locked(u) {
                p.post(Msg::Locked(arrival.volume().to_string()));
            } else {
                p.post(Msg::Arrived(arrival));
            }
        });
    }

    pub fn mount(&self, volume: String) {
        self.spawn("mount", move |u, p| {
            let result = u.mount(&volume);
            p.post(Msg::Mounted { volume, result });
        });
    }

    pub fn unlock(&self, volume: String, passphrase: Zeroizing<String>) {
        self.spawn("unlock", move |u, p| {
            let result = u.unlock(&volume, &passphrase);
            p.post(Msg::Unlocked { volume, result });
        });
    }

    pub fn remove(&self, disk: String) {
        self.spawn("remove", move |u, p| {
            let snap = u.snapshot();
            let result = u.safely_remove(&snap, &disk);
            p.post(Msg::Removed { disk, result });
        });
    }
}

fn session_locked(u: &Udisks) -> bool {
    let conn = u.connection();
    let reply = conn.call_method(
        Some(LOGIN),
        "/org/freedesktop/login1/session/auto",
        Some(PROPERTIES),
        "Get",
        &(SESSION_IFACE, "LockedHint"),
    );
    reply
        .and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedValue>())
        .ok()
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

pub fn watch(waker: repriseapp::Waker) -> (Receiver<Msg>, Calls) {
    let (tx, rx) = channel();
    let poster = Poster { tx, waker };
    let calls = Calls {
        udisks: Udisks::system(),
        poster: poster.clone(),
    };
    udisks::follow(move |s| poster.post(Msg::Snapshot(s)));
    (rx, calls)
}
