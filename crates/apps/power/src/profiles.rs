//! power-profiles-daemon over the system bus. The daemon is optional; without it the profiles are empty.

use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use zbus::OwnedVariant;
use zbus::{Connection, Proxy};

use crate::upower::{Feed, Run, follow};

/// Bus names with object paths, current first; older daemons own only
/// `net.hadess.PowerProfiles`. Each name is also the interface at its path.
const DAEMONS: [(&str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
    ),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// The profile the low battery saver holds.
pub const SAVER: &str = "power-saver";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profiles {
    pub available: Vec<String>,
    pub active: String,
    /// The daemon's unique bus name; a new one is a restarted daemon, which holds nothing.
    pub owner: String,
}

pub fn label(profile: &str) -> String {
    match profile {
        "power-saver" => "Power saver".into(),
        "balanced" => "Balanced".into(),
        "performance" => "Performance".into(),
        other => other.into(),
    }
}

/// The daemon's properties folded into `Profiles`: `Profiles` is an
/// `aa{sv}`, one dictionary per profile with its name under `Profile`.
fn fold(props: &HashMap<String, OwnedVariant>, owner: &str) -> Profiles {
    let available = props
        .get("Profiles")
        .and_then(|v| Vec::<HashMap<String, OwnedVariant>>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| String::try_from(p.get("Profile")?.clone()).ok())
        .collect();
    let active = props
        .get("ActiveProfile")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();
    Profiles {
        available,
        active,
        owner: owner.into(),
    }
}

type Changed = (String, HashMap<String, OwnedVariant>, Vec<String>);

async fn properties(
    conn: &Connection,
    name: &'static str,
    path: &'static str,
) -> Option<Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(name)
        .path(path)
        .interface(PROPERTIES)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()
}

async fn all_props(props: &Proxy<'_>, name: &str) -> Option<HashMap<String, OwnedVariant>> {
    props
        .call_method("GetAll", &(name,))
        .await
        .ok()?
        .body()
        .deserialize()
        .ok()
}

async fn find(
    conn: &Connection,
) -> Option<(
    &'static str,
    &'static str,
    Proxy<'static>,
    HashMap<String, OwnedVariant>,
)> {
    for (name, path) in DAEMONS {
        if let Some(props) = properties(conn, name, path).await
            && let Some(all) = all_props(&props, name).await
        {
            return Some((name, path, props, all));
        }
    }
    None
}

async fn run(feed: &mut Feed<Profiles>) -> Run {
    let Ok(conn) = Connection::system().await else {
        return feed.unavailable();
    };
    let Some((name, path, props, mut store)) = find(&conn).await else {
        return feed.unavailable_because(format_args!(
            "power-profiles-daemon is not answering; no profiles to show"
        ));
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(name)
        .path(path)
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .build();
    let rules = match rule {
        Ok(r) => vec![r.into()],
        Err(e) => {
            log::warn!("no match rule for {path}: {e}");
            Vec::new()
        }
    };
    let mut signals = reprisebus::signals(&conn, name, rules).await;
    // Read again after subscribing, so a change in between is not lost.
    if let Some(all) = all_props(&props, name).await {
        store = all;
    }
    let owner = reprisebus::name_owner(&conn, name)
        .await
        .unwrap_or_default();
    if !feed.send(fold(&store, &owner)) {
        return Run::Closed;
    }
    while let Some(signal) = signals.next().await {
        match signal {
            reprisebus::Signal::Message(m) => match m.body().deserialize::<Changed>() {
                Ok((_, changed, invalidated)) if invalidated.is_empty() => store.extend(changed),
                _ => {
                    if let Some(all) = all_props(&props, name).await {
                        store = all;
                    }
                }
            },
            reprisebus::Signal::OwnerChanged => {
                log::info!("power-profiles-daemon changed hands; reconnecting");
                break;
            }
        }
        if !feed.send(fold(&store, &owner)) {
            return Run::Closed;
        }
    }
    Run::Ended { delivered: true }
}

/// Picks a profile. A pick the daemon refuses sends the daemon's own
/// state back down the watch, so the app drops what it showed early.
#[derive(Clone)]
pub struct Setter {
    tx: Sender<Profiles>,
    waker: repriseapp::Waker,
}

impl Setter {
    pub fn set(&self, profile: String) {
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        let spawned = std::thread::Builder::new()
            .name("reprise-power-profile".into())
            .spawn(move || {
                zbus::block_on(async {
                    let Ok(conn) = Connection::system().await else {
                        return;
                    };
                    let Some((name, _, props, _)) = find(&conn).await else {
                        return;
                    };
                    let value = zbus::Variant::from(profile.as_str());
                    if let Err(e) = props
                        .call_method("Set", &(name, "ActiveProfile", value))
                        .await
                    {
                        log::warn!("power-profiles-daemon refused {profile}: {e}");
                        let owner = reprisebus::name_owner(&conn, name)
                            .await
                            .unwrap_or_default();
                        if let Some(all) = all_props(&props, name).await
                            && tx.send(fold(&all, &owner)).is_ok()
                        {
                            waker.wake();
                        }
                    }
                });
            });
        if let Err(e) = spawned {
            log::warn!("could not start the profile thread: {e}");
        }
    }
}

/// Takes and lets go of a hold on the power saver profile. The daemon switches back when the last hold
/// goes, and drops every hold when the user picks a profile or the holder leaves the bus.
pub struct Hold {
    tx: Sender<bool>,
    /// One per take, after `waker` wakes the app: whether the daemon holds power saver.
    answers: Receiver<bool>,
}

impl Hold {
    pub fn start(waker: repriseapp::Waker) -> Hold {
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let (answer, answers) = std::sync::mpsc::channel::<bool>();
        let spawned = std::thread::Builder::new()
            .name("reprise-power-hold".into())
            .spawn(move || {
                // The proxy keeps the connection, and with it the hold, alive.
                let mut held: Option<(Proxy<'static>, u32)> = None;
                for on in rx {
                    held = zbus::block_on(async {
                        match (on, held.take()) {
                            (true, None) => take_hold().await,
                            (false, Some((daemon, cookie))) => {
                                if let Err(e) =
                                    daemon.call_method("ReleaseProfile", &(cookie,)).await
                                {
                                    log::debug!("hold {cookie} was already gone: {e}");
                                }
                                None
                            }
                            (_, kept) => kept,
                        }
                    });
                    if on {
                        let _ = answer.send(held.is_some());
                        waker.wake();
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("could not start the profile hold thread: {e}");
        }
        Hold { tx, answers }
    }

    /// Take (`on`) or let go of the hold; false when the thread is gone, so no answer comes.
    pub fn set(&self, on: bool) -> bool {
        self.tx.send(on).is_ok()
    }

    /// The takes answered since the last call.
    pub fn answers(&self) -> Vec<bool> {
        self.answers.try_iter().collect()
    }
}

async fn take_hold() -> Option<(Proxy<'static>, u32)> {
    let conn = Connection::system().await.ok()?;
    let (name, path, _, _) = find(&conn).await?;
    let daemon: Proxy<'static> = zbus::proxy::Builder::new(&conn)
        .destination(name)
        .path(path)
        .interface(name)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;
    let cookie = daemon
        .call_method("HoldProfile", &(SAVER, "Low battery", "reprise.power"))
        .await
        .and_then(|m| m.body().deserialize::<u32>());
    match cookie {
        Ok(cookie) => {
            log::info!("holding {SAVER} for the low battery");
            Some((daemon, cookie))
        }
        Err(e) => {
            log::warn!("power-profiles-daemon refused to hold {SAVER}: {e}");
            None
        }
    }
}

pub fn watch(waker: repriseapp::Waker) -> (Receiver<Profiles>, Setter) {
    let (tx, rx) = std::sync::mpsc::channel();
    let setter = Setter {
        tx: tx.clone(),
        waker: waker.clone(),
    };
    follow("reprise-power-profiles", tx, waker, |feed| {
        zbus::block_on(run(feed))
    });
    (rx, setter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::Variant;

    fn profile(name: &str) -> HashMap<String, Variant<'static>> {
        HashMap::from([
            ("Profile".to_string(), Variant::from(name.to_string())),
            ("Driver".to_string(), Variant::from("placeholder")),
        ])
    }

    fn props(list: &[&str], active: &str) -> HashMap<String, OwnedVariant> {
        let list: Vec<_> = list.iter().map(|p| profile(p)).collect();
        HashMap::from([
            (
                "Profiles".to_string(),
                Variant::from(list).try_to_owned().unwrap(),
            ),
            (
                "ActiveProfile".to_string(),
                Variant::from(active.to_string()).try_to_owned().unwrap(),
            ),
        ])
    }

    #[test]
    fn fold_lists_the_profiles_in_the_daemons_order() {
        let p = fold(
            &props(&["power-saver", "balanced", "performance"], "balanced"),
            "",
        );
        assert_eq!(p.available, ["power-saver", "balanced", "performance"]);
        assert_eq!(p.active, "balanced");
    }

    #[test]
    fn fold_lists_only_what_the_daemon_offers() {
        let p = fold(&props(&["power-saver", "balanced"], "power-saver"), "");
        assert_eq!(p.available, ["power-saver", "balanced"]);
        assert_eq!(p.active, "power-saver");
        assert_eq!(
            fold(&HashMap::new(), ""),
            Profiles::default(),
            "no daemon, no profiles"
        );
    }

    #[test]
    fn profiles_have_readable_labels() {
        assert_eq!(label("power-saver"), "Power saver");
        assert_eq!(label("balanced"), "Balanced");
        assert_eq!(label("performance"), "Performance");
        assert_eq!(label("turbo"), "turbo", "an unknown profile keeps its name");
    }
}
