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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profiles {
    pub available: Vec<String>,
    pub active: String,
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
fn fold(props: &HashMap<String, OwnedVariant>) -> Profiles {
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
    Profiles { available, active }
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
    if !feed.send(fold(&store)) {
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
        if !feed.send(fold(&store)) {
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
                        if let Some(all) = all_props(&props, name).await
                            && tx.send(fold(&all)).is_ok()
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
        let p = fold(&props(
            &["power-saver", "balanced", "performance"],
            "balanced",
        ));
        assert_eq!(p.available, ["power-saver", "balanced", "performance"]);
        assert_eq!(p.active, "balanced");
    }

    #[test]
    fn fold_lists_only_what_the_daemon_offers() {
        let p = fold(&props(&["power-saver", "balanced"], "power-saver"));
        assert_eq!(p.available, ["power-saver", "balanced"]);
        assert_eq!(p.active, "power-saver");
        assert_eq!(
            fold(&HashMap::new()),
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
