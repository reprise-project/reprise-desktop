//! The battery as UPower reports it: a store fed by the daemon's display device over the system bus.

use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use zbus::zvariant::OwnedValue;
use zbus::{Connection, Proxy};

const NAME: &str = "org.freedesktop.UPower";
const PATH: &str = "/org/freedesktop/UPower";
const DEVICE_IFACE: &str = "org.freedesktop.UPower.Device";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    Charging,
    Discharging,
    Full,
    #[default]
    Unknown,
}

impl State {
    /// UPower's `State` codes: 1 charging, 2 discharging, 3 empty, 4 fully charged, 5 pending charge, 6 pending discharge.
    pub fn from_code(code: u32) -> State {
        match code {
            1 | 5 => State::Charging,
            2 | 3 | 6 => State::Discharging,
            4 => State::Full,
            _ => State::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Battery {
    pub state: State,
    pub percent: u8,
    /// Seconds, 0 when unknown.
    pub time_to_empty: u64,
    pub time_to_full: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Power {
    pub on_battery: bool,
    pub battery: Option<Battery>,
}

fn fold(on_battery: bool, props: &HashMap<String, OwnedValue>) -> Power {
    let present = props
        .get("IsPresent")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    let kind = props
        .get("Type")
        .and_then(|v| u32::try_from(v.clone()).ok())
        .unwrap_or(0);
    // 2 is UPower's battery type; the display device reports 2 when it
    // stands for one.
    if !present || kind != 2 {
        return Power {
            on_battery,
            battery: None,
        };
    }
    let num = |k: &str| props.get(k).and_then(|v| f64::try_from(v.clone()).ok());
    let int = |k: &str| props.get(k).and_then(|v| i64::try_from(v.clone()).ok());
    Power {
        on_battery,
        battery: Some(Battery {
            state: props
                .get("State")
                .and_then(|v| u32::try_from(v.clone()).ok())
                .map(State::from_code)
                .unwrap_or_default(),
            percent: num("Percentage").unwrap_or(0.0).round().clamp(0.0, 100.0) as u8,
            time_to_empty: int("TimeToEmpty").unwrap_or(0).max(0) as u64,
            time_to_full: int("TimeToFull").unwrap_or(0).max(0) as u64,
        }),
    }
}

async fn proxy(
    conn: &Connection,
    path: String,
    iface: &'static str,
) -> zbus::Result<Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(NAME)?
        .path(path)?
        .interface(iface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

async fn all_props(props: &Proxy<'_>) -> Option<HashMap<String, OwnedValue>> {
    props
        .call_method("GetAll", &(DEVICE_IFACE,))
        .await
        .ok()?
        .body()
        .deserialize()
        .ok()
}

type Changed = (String, HashMap<String, OwnedValue>, Vec<String>);

enum Wake {
    Device(Option<Changed>),
    Daemon(Option<Changed>),
    Owner,
}

#[derive(Default)]
struct Store {
    on_battery: bool,
    device: HashMap<String, OwnedValue>,
}

impl Store {
    fn power(&self) -> Power {
        fold(self.on_battery, &self.device)
    }

    fn device_changed(&mut self, changed: HashMap<String, OwnedValue>) {
        self.device.extend(changed);
    }

    fn daemon_changed(&mut self, body: Option<&Changed>) -> bool {
        let Some((_, changed, invalidated)) = body else {
            return true;
        };
        if let Some(b) = changed
            .get("OnBattery")
            .and_then(|v| bool::try_from(v.clone()).ok())
        {
            self.on_battery = b;
        }
        invalidated.iter().any(|k| k == "OnBattery")
    }
}

async fn on_battery(upower: &Proxy<'_>) -> Option<bool> {
    upower.get_property::<bool>("OnBattery").await.ok()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Run {
    Ended { delivered: bool },
    Closed,
}

pub(crate) struct Feed<T> {
    tx: Sender<T>,
    waker: repriseapp::Waker,
    last: Option<T>,
    /// Unavailable since the last run that delivered; the reason is
    /// logged once per outage, not on every retry.
    down: bool,
}

impl<T: Clone + Default + PartialEq> Feed<T> {
    pub(crate) fn send(&mut self, p: T) -> bool {
        if self.last.as_ref() == Some(&p) {
            return true;
        }
        if self.tx.send(p.clone()).is_err() {
            return false;
        }
        self.last = Some(p);
        self.waker.wake();
        true
    }

    /// [`Feed::unavailable`], logging `why` unless the outage was logged.
    pub(crate) fn unavailable_because(&mut self, why: std::fmt::Arguments) -> Run {
        if self.outage_starts() {
            log::info!("{why}");
        }
        self.unavailable()
    }

    fn outage_starts(&mut self) -> bool {
        !std::mem::replace(&mut self.down, true)
    }

    /// Note how a run ended: one that delivered ends the outage.
    fn ended(&mut self, delivered: bool) {
        if delivered {
            self.down = false;
        }
    }

    pub(crate) fn unavailable(&mut self) -> Run {
        if self.send(T::default()) {
            Run::Ended { delivered: false }
        } else {
            Run::Closed
        }
    }
}

async fn run(feed: &mut Feed<Power>) -> Run {
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            return feed
                .unavailable_because(format_args!("no system bus ({e}); no battery to show"));
        }
    };
    let Ok(upower) = proxy(&conn, PATH.into(), NAME).await else {
        return feed.unavailable();
    };
    let device: String = match upower.call_method("GetDisplayDevice", &()).await {
        Ok(reply) => match reply
            .body()
            .deserialize::<zbus::zvariant::OwnedObjectPath>()
        {
            Ok(p) => p.as_str().to_string(),
            Err(_) => return feed.unavailable(),
        },
        Err(e) => {
            return feed
                .unavailable_because(format_args!("UPower unavailable ({e}); no battery to show"));
        }
    };
    // Only UPower's own broadcasts get through (see `reprisebus::signals`).
    let mut rules = Vec::new();
    for path in [device.as_str(), PATH] {
        match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .and_then(|b| b.path(path))
            .and_then(|b| b.interface(PROPERTIES))
            .and_then(|b| b.member("PropertiesChanged"))
        {
            Ok(b) => rules.push(b.build().into()),
            Err(e) => log::warn!("no match rule for {path}: {e}"),
        }
    }
    let body = |m: &zbus::Message| m.body().deserialize::<Changed>().ok();
    let daemon = |m: &zbus::Message| m.header().path().is_some_and(|p| p.as_str() == PATH);
    let mut streams = reprisebus::signals(&conn, NAME, rules)
        .await
        .filter_map(|s| async move {
            match s {
                reprisebus::Signal::Message(m) if daemon(&m) => Some(Wake::Daemon(body(&m))),
                reprisebus::Signal::Message(m) => Some(Wake::Device(body(&m))),
                reprisebus::Signal::OwnerChanged => Some(Wake::Owner),
            }
        })
        .boxed();
    let Ok(device_props) = proxy(&conn, device, PROPERTIES).await else {
        return feed.unavailable();
    };
    let mut store = Store {
        on_battery: on_battery(&upower).await.unwrap_or(false),
        device: all_props(&device_props).await.unwrap_or_default(),
    };
    if !feed.send(store.power()) {
        return Run::Closed;
    }
    while let Some(wake) = streams.next().await {
        match wake {
            // The signal carries what moved; only an invalidated key, or
            // a body that did not parse, is worth a round trip.
            Wake::Device(Some((_, changed, invalidated))) if invalidated.is_empty() => {
                store.device_changed(changed);
            }
            Wake::Device(_) => {
                if let Some(props) = all_props(&device_props).await {
                    store.device = props;
                }
            }
            Wake::Daemon(body) => {
                if store.daemon_changed(body.as_ref())
                    && let Some(b) = on_battery(&upower).await
                {
                    store.on_battery = b;
                }
            }
            // A restarted daemon is read afresh by the next run.
            Wake::Owner => {
                log::info!("UPower changed hands; reconnecting");
                break;
            }
        }
        if !feed.send(store.power()) {
            return Run::Closed;
        }
    }
    Run::Ended { delivered: true }
}

const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);

fn next_pause(pause: Duration, delivered: bool) -> Duration {
    if delivered {
        RETRY_MIN
    } else {
        (pause * 2).min(RETRY_MAX)
    }
}

pub fn watch(waker: repriseapp::Waker) -> Receiver<Power> {
    let (tx, rx) = channel();
    follow("reprise-power-upower", tx, waker, |feed| {
        zbus::block_on(run(feed))
    });
    rx
}

pub(crate) fn follow<T: Clone + Default + PartialEq + Send + 'static>(
    name: &str,
    tx: Sender<T>,
    waker: repriseapp::Waker,
    mut run: impl FnMut(&mut Feed<T>) -> Run + Send + 'static,
) {
    if let Err(e) = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let mut feed = Feed {
                tx,
                waker,
                last: None,
                down: false,
            };
            let mut pause = RETRY_MIN;
            while let Run::Ended { delivered } = run(&mut feed) {
                feed.ended(delivered);
                pause = next_pause(pause, delivered);
                std::thread::sleep(pause);
            }
        })
    {
        log::warn!("could not start the {name} thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn props(entries: Vec<(&str, Value<'static>)>) -> HashMap<String, OwnedValue> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
    }

    #[test]
    fn state_codes_map_to_the_four_states() {
        assert_eq!(State::from_code(1), State::Charging);
        assert_eq!(State::from_code(2), State::Discharging);
        assert_eq!(State::from_code(4), State::Full);
        assert_eq!(State::from_code(5), State::Charging);
        assert_eq!(State::from_code(6), State::Discharging);
        assert_eq!(State::from_code(0), State::Unknown);
    }

    #[test]
    fn fold_reads_a_battery() {
        let p = fold(
            true,
            &props(vec![
                ("IsPresent", Value::from(true)),
                ("Type", Value::from(2u32)),
                ("State", Value::from(2u32)),
                ("Percentage", Value::from(45.4f64)),
                ("TimeToEmpty", Value::from(7800i64)),
                ("TimeToFull", Value::from(0i64)),
            ]),
        );
        assert_eq!(
            p,
            Power {
                on_battery: true,
                battery: Some(Battery {
                    state: State::Discharging,
                    percent: 45,
                    time_to_empty: 7800,
                    time_to_full: 0,
                }),
            }
        );
    }

    #[test]
    fn a_signal_patches_the_kept_properties() {
        let mut store = Store {
            on_battery: false,
            device: props(vec![
                ("IsPresent", Value::from(true)),
                ("Type", Value::from(2u32)),
                ("State", Value::from(1u32)),
                ("Percentage", Value::from(40.0f64)),
                ("TimeToFull", Value::from(600i64)),
            ]),
        };
        store.device_changed(props(vec![("Percentage", Value::from(41.0f64))]));
        let b = store.power().battery.expect("still a battery");
        assert_eq!(
            (b.percent, b.state, b.time_to_full),
            (41, State::Charging, 600)
        );
        let daemon = |changed, invalidated: &[&str]| {
            (
                String::from(NAME),
                props(changed),
                invalidated.iter().map(|k| k.to_string()).collect(),
            )
        };
        let lid = daemon(vec![("LidIsClosed", Value::from(true))], &[]);
        assert!(!store.daemon_changed(Some(&lid)), "nothing to read again");
        assert!(!store.on_battery, "no OnBattery in it");
        let plugged = daemon(vec![("OnBattery", Value::from(true))], &[]);
        assert!(!store.daemon_changed(Some(&plugged)));
        assert!(store.on_battery);
    }

    #[test]
    fn a_daemon_signal_reads_on_battery_again_only_when_it_lacks_the_value() {
        let mut store = Store::default();
        assert!(store.daemon_changed(None), "unparsed body");
        let invalidated = (String::from(NAME), HashMap::new(), vec!["OnBattery".into()]);
        assert!(store.daemon_changed(Some(&invalidated)));
        let other = (
            String::from(NAME),
            HashMap::new(),
            vec!["LidIsClosed".into()],
        );
        assert!(
            !store.daemon_changed(Some(&other)),
            "another key invalidated"
        );
    }

    #[test]
    fn the_retry_pause_doubles_to_a_minute_and_resets_on_a_live_run() {
        let mut p = RETRY_MIN;
        let mut seen = Vec::new();
        for _ in 0..7 {
            p = next_pause(p, false);
            seen.push(p.as_secs());
        }
        assert_eq!(seen, [4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(next_pause(p, true), RETRY_MIN);
    }

    #[test]
    fn the_feed_sends_a_power_only_when_it_changed() {
        let (tx, rx) = channel();
        let mut feed = Feed {
            tx,
            waker: repriseapp::Waker::noop(),
            last: None,
            down: false,
        };
        let on_battery = Power {
            on_battery: true,
            battery: None,
        };
        assert!(feed.send(Power::default()));
        assert!(feed.send(Power::default()));
        assert!(feed.send(on_battery.clone()));
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [Power::default(), on_battery]
        );
        drop(rx);
        assert!(!feed.send(Power::default()), "the receiver is gone");
    }

    #[test]
    fn an_outage_is_logged_once_until_a_run_delivers() {
        let (tx, _rx) = channel::<Power>();
        let mut feed = Feed {
            tx,
            waker: repriseapp::Waker::noop(),
            last: None,
            down: false,
        };
        assert!(feed.outage_starts());
        feed.ended(false);
        assert!(!feed.outage_starts(), "a retry that fails again is quiet");
        feed.ended(true);
        assert!(feed.outage_starts(), "a new outage after a live run");
    }

    #[test]
    fn an_absent_display_device_is_no_battery() {
        let p = fold(
            false,
            &props(vec![
                ("IsPresent", Value::from(false)),
                ("Type", Value::from(0u32)),
            ]),
        );
        assert_eq!(p.battery, None);
        assert!(!p.on_battery);
    }
}
