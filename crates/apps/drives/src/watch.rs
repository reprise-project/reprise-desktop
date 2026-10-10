//! The UDisks2 side: a thread following its signals that hands the app a whole [`Snapshot`] after
//! every change, and the calls the app asks for, each on its own thread (any can wait on polkit).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::stream::StreamExt;
use reprisefs::udisks::{self, Error, Objects, Snapshot, Udisks};
use zbus::Connection;
use zeroize::Zeroizing;

const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const LOGIN: &str = "org.freedesktop.login1";
const SESSION_IFACE: &str = "org.freedesktop.login1.Session";

const SIGNALS: &[&str] = &["InterfacesAdded", "InterfacesRemoved", "PropertiesChanged"];

/// A mount point appears a moment after the signal that announced the
/// device; a burst of signals costs one read after this pause.
const SETTLE: Duration = Duration::from_millis(150);

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
            let path = match &arrival {
                crate::model::Arrival::Mount(v) | crate::model::Arrival::Unlock(v) => v.clone(),
            };
            if session_locked(u) {
                p.post(Msg::Locked(path));
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
    if let Err(e) = std::thread::Builder::new()
        .name("reprise-drives-udisks".into())
        .spawn(move || zbus::block_on(run(poster)))
    {
        log::warn!("could not start the UDisks2 thread: {e}");
    }
    (rx, calls)
}

async fn run(poster: Poster) {
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::info!("no system bus ({e}); no drives to follow");
            poster.post(Msg::Snapshot(Snapshot::default()));
            return;
        }
    };
    let mut rules = Vec::new();
    for member in SIGNALS {
        match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(udisks::NAME)
            .and_then(|b| b.member(*member))
        {
            Ok(b) => rules.push(b.build().into()),
            Err(e) => log::warn!("no match rule for {member}: {e}"),
        }
    }
    // Subscribed before the first read, so no change slips between.
    let mut signals = reprisebus::signals(&conn, udisks::NAME, rules).await;
    loop {
        poster.post(Msg::Snapshot(read(&conn).await));
        if signals.next().await.is_none() {
            log::warn!("the UDisks2 signal streams all ended");
            return;
        }
        async_io::Timer::after(SETTLE).await;
        while let Some(Some(_)) = signals.next().now_or_never() {}
    }
}

async fn read(conn: &Connection) -> Snapshot {
    let reply = conn
        .call_method(
            Some(udisks::NAME),
            udisks::ROOT,
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        )
        .await;
    match reply.and_then(|m| m.body().deserialize::<Objects>()) {
        Ok(objects) => udisks::fold(&objects),
        Err(e) => {
            log::info!("no UDisks2 objects ({e})");
            Snapshot::default()
        }
    }
}
