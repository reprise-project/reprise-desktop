//! An activation NetworkManager accepted, followed until it settles: a join or VPN that fails
//! after the call returned still says why.

use super::snapshot::get_all;
use super::*;
use reprisekit::tr;

/// `NMActiveConnectionState`.
const ACTIVATED: u32 = 2;
const DEACTIVATED: u32 = 4;
/// `NMDeviceState`.
const DEVICE_FAILED: u32 = 120;

/// What an activation was for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Joining {
    Wifi(Ssid),
    /// Named by its active connection's `Id`.
    Vpn,
}

/// Why an activation ended without connecting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Why {
    /// The saved key or password was refused, or none was there to send.
    Secrets,
    Login,
    NotFound,
    NoAddress,
    Other,
}

/// One `StateChanged`: the active connection's `(state, reason)`, or the device's new state and
/// reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Seen {
    Active(u32, u32),
    Device(u32, u32),
}

/// One activation's signals folded in order. A Wi-Fi failure shows on the device (`FAILED` with
/// its reason) just before the active connection deactivates as "device disconnected".
#[derive(Debug, Default)]
pub(super) struct Settle {
    device_failed: Option<u32>,
}

impl Settle {
    /// None while the activation runs; `Some(None)` once it joined or was ended by a newer
    /// activation or the user; `Some(Some(why))` once it failed.
    pub(super) fn feed(&mut self, seen: Seen) -> Option<Option<Why>> {
        match seen {
            Seen::Device(DEVICE_FAILED, reason) => {
                self.device_failed = Some(reason);
                None
            }
            Seen::Active(ACTIVATED, _) => Some(None),
            Seen::Active(DEACTIVATED, reason) => Some(match self.device_failed {
                Some(r) => Some(device_why(r)),
                None => active_why(reason),
            }),
            Seen::Device(..) | Seen::Active(..) => None,
        }
    }
}

/// An `NMDeviceStateReason` the device failed with.
fn device_why(reason: u32) -> Why {
    match reason {
        7 => Why::Secrets,
        53 => Why::NotFound,
        5 | 6 | 15..=17 => Why::NoAddress,
        _ => Why::Other,
    }
}

/// An `NMActiveConnectionStateReason`; None for an end that is no failure (the user's own, a
/// device going away, a removed profile).
fn active_why(reason: u32) -> Option<Why> {
    match reason {
        9 => Some(Why::Secrets),
        10 => Some(Why::Login),
        5 => Some(Why::NoAddress),
        4 | 6..=8 | 12 | 13 => Some(Why::Other),
        _ => None,
    }
}

/// The balloon text for a failed activation of `name`.
pub(super) fn not_joined(joining: &Joining, name: &str, why: Why) -> String {
    match (why, joining) {
        (Why::Secrets, Joining::Wifi(_)) => tr!("nm-key-rejected", name = name),
        (Why::Secrets, Joining::Vpn) => {
            tr!("nm-no-saved-secret", name = name)
        }
        (Why::Login, _) => tr!("nm-login-rejected", name = name),
        (Why::NotFound, _) => tr!("nm-not-found", name = name),
        (Why::NoAddress, _) => tr!("nm-no-address", name = name),
        (Why::Other, _) => tr!("nm-could-not-connect", name = name),
    }
}

/// Follow the active connection at `active` (and `device`'s state, for a join) on a task of its
/// own; a failure comes back as a [`Wake::Ran`] carrying the balloon text, and a refused Wi-Fi
/// key also as [`NmMsg::KeyRefused`].
pub(super) fn follow(
    conn: &Connection,
    active: String,
    device: Option<String>,
    joining: Joining,
    wakes: async_channel::Sender<Wake>,
) {
    let conn = conn.clone();
    zruntime::spawn(async move {
        let Some((why, id)) = settle(&conn, &active, device.as_deref()).await else {
            return;
        };
        let name = match &joining {
            Joining::Wifi(ssid) => ssid.to_string(),
            Joining::Vpn => id,
        };
        log::info!("{name} did not connect: {why:?}");
        let reply = match (&joining, why) {
            (Joining::Wifi(ssid), Why::Secrets) => Some(Box::new(NmMsg::KeyRefused(ssid.clone()))),
            _ => None,
        };
        let _ = wakes
            .send(Wake::Ran {
                device,
                error: Some(not_joined(&joining, &name, why)),
                reply,
            })
            .await;
    })
    .detach();
}

/// How the activation at `active` failed and its `Id`; None once it joined, was ended without
/// failing, or was gone before it could be read.
async fn settle(conn: &Connection, active: &str, device: Option<&str>) -> Option<(Why, String)> {
    let watch = |path: &str, iface: &'static str| {
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .path(path)
            .interface(iface)
            .member("StateChanged")
            .build()
            .map(zbus::OwnedMatchRule::from)
    };
    let mut rules = Vec::new();
    for (path, iface) in [(Some(active), ACTIVE_IFACE), (device, DEVICE_IFACE)] {
        match path.map(|p| watch(p, iface)) {
            Some(Ok(rule)) => rules.push(rule),
            Some(Err(e)) => log::warn!("cannot follow {path:?}: {e}"),
            None => {}
        }
    }
    // Subscribed before the first read, so a change in between is seen.
    let mut signals = reprisebus::signals(conn, NAME, rules).await;
    let props = get_all(conn, active, ACTIVE_IFACE).await?;
    let id = text(&props, "Id");
    let mut settle = Settle::default();
    if let Some(done) = settle.feed(Seen::Active(uint(&props, "State"), 0)) {
        return done.map(|why| (why, id));
    }
    while let Some(signal) = signals.next().await {
        let reprisebus::Signal::Message(m) = signal else {
            return None;
        };
        let from_active = m.header().path().is_some_and(|p| p.as_str() == active);
        let seen = if from_active {
            m.body()
                .deserialize::<(u32, u32)>()
                .map(|(state, reason)| Seen::Active(state, reason))
        } else {
            m.body()
                .deserialize::<(u32, u32, u32)>()
                .map(|(state, _, reason)| Seen::Device(state, reason))
        };
        match seen.map(|s| settle.feed(s)) {
            Ok(Some(done)) => return done.map(|why| (why, id)),
            Ok(None) => {}
            Err(e) => log::debug!("an unreadable StateChanged: {e}"),
        }
    }
    None
}
