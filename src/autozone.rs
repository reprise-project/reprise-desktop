//! Automatic time zone (`[time_zone] automatic`): a thread asks geoclue where the machine is and
//! sets the zone of the nearest town (`tzcity`) through timedated; docs/programs.md has the pace.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::stream::{BoxStream, SelectAll, StreamExt};
use smithay::reexports::calloop::{
    LoopHandle,
    channel::{Event, channel},
};
use zbus::{OwnedObjectPath, OwnedVariant, Variant};

use crate::compositor::State;

const HOUR: u64 = 3600;
/// How long a look that found no place holds off the next trigger.
const RETRY: u64 = 300;
/// How long geoclue gets to find the machine.
const FIX_WITHIN: Duration = Duration::from_secs(60);

const GEOCLUE: &str = "org.freedesktop.GeoClue2";
const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
const MANAGER: &str = "org.freedesktop.GeoClue2.Manager";
const CLIENT: &str = "org.freedesktop.GeoClue2.Client";
const LOCATION: &str = "org.freedesktop.GeoClue2.Location";
/// geoclue's `GCLUE_ACCURACY_LEVEL_CITY`.
const CITY: u32 = 4;
/// The desktop id the location pin names while a look runs: Date and Time's.
const DESKTOP_ID: &str = "reprise-datetime";

const TIMEDATE: &str = "org.freedesktop.timedate1";
const TIMEDATE_PATH: &str = "/org/freedesktop/timedate1";

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

#[derive(Default)]
pub struct AutoZone {
    wake: Option<async_channel::Sender<Wake>>,
    turns: Turns,
}

/// Counts the switch's turns on and off, so a look started before a turn sets nothing.
#[derive(Clone, Default)]
struct Turns(Arc<AtomicU64>);

impl Turns {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    fn still(&self, turn: u64) -> bool {
        self.now() == turn
    }
}

#[derive(Debug, PartialEq)]
enum Wake {
    /// `[time_zone] automatic` with `[privacy] location` allowing it.
    On(bool),
    Resumed,
    Network,
    Due,
}

impl AutoZone {
    /// Start the thread, reporting a zone it set on `handle`'s loop; `on` as for [`Wake::On`].
    pub fn start(handle: &LoopHandle<'static, State>, on: bool) -> AutoZone {
        let (tx, rx) = channel::<String>();
        let inserted = handle.insert_source(rx, |event, _, state| {
            if let Event::Msg(zone) = event {
                log::info!("time zone: set to {zone} from the location");
                reprisefs::locale::refresh_zone();
                state.redraw_taskbars();
                crate::nightlight::update(state);
            }
        });
        if let Err(e) = inserted {
            log::warn!("time zone: channel: {e}");
            return AutoZone::default();
        }
        let (wake, wakes) = async_channel::unbounded();
        let _ = wake.try_send(Wake::On(on));
        let turns = Turns::default();
        let theirs = turns.clone();
        let spawned = std::thread::Builder::new()
            .name("auto-timezone".into())
            .spawn(move || zbus::block_on(run(wakes, tx, theirs)));
        if let Err(e) = spawned {
            log::warn!("time zone: no thread: {e}");
            return AutoZone::default();
        }
        AutoZone {
            wake: Some(wake),
            turns,
        }
    }

    fn send(&self, w: Wake) {
        // Ahead of the message: a look in flight reads the turn before its next one is queued.
        if matches!(w, Wake::On(_)) {
            self.turns.0.fetch_add(1, Ordering::SeqCst);
        }
        if let Some(tx) = &self.wake {
            let _ = tx.try_send(w);
        }
    }
}

/// Whether the config turns the automatic zone on.
pub fn wanted(config: &crate::config::Config) -> bool {
    config.auto_time_zone && config.location
}

pub fn config_changed(state: &mut State) {
    state.auto_zone.send(Wake::On(wanted(&state.config)));
}

pub fn resumed(state: &mut State) {
    state.auto_zone.send(Wake::Resumed);
}

/// When the next look is due, in seconds since boot.
#[derive(Debug, Default, PartialEq)]
struct Pace {
    on: bool,
    /// A trigger before this waits for it: an hour after a look, `RETRY` after a missed one.
    rest_until: Option<u64>,
    due: Option<u64>,
}

impl Pace {
    fn wake(&mut self, w: &Wake, now: u64) {
        match *w {
            Wake::On(on) => {
                self.on = on;
                self.due = on.then_some(now);
            }
            Wake::Resumed | Wake::Network if self.on => {
                let at = self.rest_until.map_or(now, |r| r.max(now));
                self.due = Some(self.due.map_or(at, |d| d.min(at)));
            }
            Wake::Resumed | Wake::Network | Wake::Due => {}
        }
    }

    /// True when a look is due at `now`; [`Pace::done`] follows it.
    fn take(&mut self, now: u64) -> bool {
        if self.due.is_none_or(|d| d > now) {
            return false;
        }
        self.due = None;
        true
    }

    fn done(&mut self, missed: bool, now: u64) {
        self.rest_until = Some(now + if missed { RETRY } else { HOUR });
    }
}

/// Seconds since boot, suspend included, so a night asleep counts toward the hour.
fn boot_secs() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    ts.tv_sec.max(0) as u64
}

async fn run(
    wakes: async_channel::Receiver<Wake>,
    tx: smithay::reexports::calloop::channel::Sender<String>,
    turns: Turns,
) {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("time zone: no system bus ({e}); the zone is not set automatically");
            return;
        }
    };
    let mut events: SelectAll<BoxStream<'static, Wake>> = SelectAll::new();
    events.push(wakes.boxed());
    // NetworkManager's signals queue on a connection of their own: unread during a look, they
    // would stall the look's replies behind them.
    if let (Ok(rule), Ok(watch)) = (properties_rule(NM_PATH), zbus::Connection::system().await) {
        let network = reprisebus::signals(&watch, NM, vec![rule.into()]).await;
        events.push(
            network
                .filter_map(|s| async move { network_moved(&s) })
                .boxed(),
        );
    }
    let mut pace = Pace::default();
    loop {
        let now = boot_secs();
        if pace.take(now) {
            let turn = turns.now();
            let looked = look(&conn, || turns.still(turn)).await;
            pace.done(matches!(looked, Err(Miss::Retry(_))), boot_secs());
            match looked {
                Ok(Some(zone)) => {
                    let _ = tx.send(zone);
                }
                Ok(None) => {}
                Err(Miss::Retry(e) | Miss::Refused(e)) => log::info!("time zone: {e}"),
            }
            continue;
        }
        let next = match pace.due {
            Some(at) => {
                let wait = Duration::from_secs(at.saturating_sub(now));
                let due = async {
                    async_io::Timer::after(wait).await;
                    Some(Wake::Due)
                };
                futures_lite::future::or(events.next(), due).await
            }
            None => events.next().await,
        };
        let Some(w) = next else {
            return;
        };
        let now = boot_secs();
        let before = pace.due;
        pace.wake(&w, now);
        if let Some(at) = pace.due.filter(|_| pace.due != before) {
            log::info!(
                "time zone: {w:?}, next look in {} s",
                at.saturating_sub(now)
            );
        }
    }
}

fn properties_rule(path: &'static str) -> zbus::Result<zbus::MatchRule<'static>> {
    zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .path(path)
        .build()
}

/// NetworkManager came up or took a new primary connection.
fn network_moved(s: &reprisebus::Signal) -> Option<Wake> {
    let msg = match s {
        reprisebus::Signal::OwnerChanged => return Some(Wake::Network),
        reprisebus::Signal::Message(m) => m,
    };
    let (iface, props, _) = msg
        .body()
        .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
        .ok()?;
    let primary = props.get("PrimaryConnection")?;
    let path = OwnedObjectPath::try_from(primary.try_clone().ok()?).ok()?;
    connected(&iface, path.as_str()).then_some(Wake::Network)
}

fn connected(iface: &str, primary: &str) -> bool {
    iface == NM && primary != "/"
}

/// Why a look set no zone.
#[derive(Debug)]
enum Miss {
    /// No place or no timedated: the next trigger may look again after `RETRY`.
    Retry(String),
    /// polkit or tzdata said no: it stays no for the hour.
    Refused(String),
}

/// Find the machine and set the nearest zone while `still_on`; Some(zone) when it changed.
async fn look(
    conn: &zbus::Connection,
    still_on: impl Fn() -> bool,
) -> Result<Option<String>, Miss> {
    let at = locate(conn)
        .await
        .map_err(|e| Miss::Retry(format!("geoclue gave no location: {e}")))?;
    let zone = crate::tzcity::nearest(at).ok_or(Miss::Refused("tzdata lists no zones".into()))?;
    let current: String = timedated_zone(conn)
        .await
        .map_err(|e| Miss::Retry(format!("timedated does not answer: {e}")))?;
    if current == zone {
        log::info!("time zone: {zone} stays");
        return Ok(None);
    }
    if !still_on() {
        log::info!("time zone: turned off or on during the look; {zone} is not set");
        return Ok(None);
    }
    let set = conn
        .call_method(
            Some(TIMEDATE),
            TIMEDATE_PATH,
            Some(TIMEDATE),
            "SetTimezone",
            &(zone.as_str(), false),
        )
        .await;
    match set {
        Ok(_) => Ok(Some(zone)),
        Err(e) if needs_password(&e) => Err(Miss::Refused(format!(
            "polkit does not let {current} change to {zone} without a password; an \
             administrator has to allow org.freedesktop.timedate1.set-timezone"
        ))),
        Err(e) => Err(Miss::Retry(format!("timedated did not set {zone}: {e}"))),
    }
}

fn needs_password(e: &zbus::Error) -> bool {
    reprisebus::error::name(e).is_some_and(|n| {
        n == "org.freedesktop.DBus.Error.AccessDenied"
            || n == "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
    })
}

/// A fresh geoclue client at city accuracy: started, read once, deleted.
async fn locate(conn: &zbus::Connection) -> zbus::Result<(f64, f64)> {
    let reply = conn
        .call_method(
            Some(GEOCLUE),
            MANAGER_PATH,
            Some(MANAGER),
            "CreateClient",
            &(),
        )
        .await?;
    let client: OwnedObjectPath = reply.body().deserialize()?;
    let at = located(conn, &client).await;
    let _ = conn
        .call_method(
            Some(GEOCLUE),
            MANAGER_PATH,
            Some(MANAGER),
            "DeleteClient",
            &(&client,),
        )
        .await;
    at
}

async fn located(conn: &zbus::Connection, client: &OwnedObjectPath) -> zbus::Result<(f64, f64)> {
    set(conn, client, "DesktopId", Variant::from(DESKTOP_ID)).await?;
    set(conn, client, "RequestedAccuracyLevel", Variant::from(CITY)).await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(CLIENT)
        .member("LocationUpdated")
        .path(client.clone())
        .build()?;
    // geoclue sends `LocationUpdated` to the client's connection alone.
    let mut updates = reprisebus::owner_signals(conn, GEOCLUE, vec![rule.into()]).await;
    conn.call_method(Some(GEOCLUE), client, Some(CLIENT), "Start", &())
        .await?;
    let fix = async {
        while let Some(s) = updates.next().await {
            if let reprisebus::Signal::Message(m) = s
                && let Ok((_, new)) = m.body().deserialize::<(OwnedObjectPath, OwnedObjectPath)>()
            {
                return Some(new);
            }
        }
        None
    };
    let late = async {
        async_io::Timer::after(FIX_WITHIN).await;
        None
    };
    let found = futures_lite::future::or(fix, late).await;
    let _ = conn
        .call_method(Some(GEOCLUE), client, Some(CLIENT), "Stop", &())
        .await;
    let Some(location) = found else {
        return Err(zbus::Error::Failure(format!(
            "no fix within {} s",
            FIX_WITHIN.as_secs()
        )));
    };
    let lat: f64 = property(conn, GEOCLUE, location.as_str(), LOCATION, "Latitude").await?;
    let lon: f64 = property(conn, GEOCLUE, location.as_str(), LOCATION, "Longitude").await?;
    Ok((lat, lon))
}

async fn set(
    conn: &zbus::Connection,
    client: &OwnedObjectPath,
    name: &str,
    value: Variant<'_>,
) -> zbus::Result<()> {
    conn.call_method(
        Some(GEOCLUE),
        client,
        Some(PROPERTIES),
        "Set",
        &(CLIENT, name, value),
    )
    .await
    .map(|_| ())
}

async fn property<T>(
    conn: &zbus::Connection,
    dest: &str,
    path: &str,
    iface: &str,
    name: &str,
) -> zbus::Result<T>
where
    T: TryFrom<OwnedVariant>,
    T::Error: Into<zbus::Error>,
{
    let reply = conn
        .call_method(Some(dest), path, Some(PROPERTIES), "Get", &(iface, name))
        .await?;
    let v: OwnedVariant = reply.body().deserialize()?;
    T::try_from(v).map_err(Into::into)
}

async fn timedated_zone(conn: &zbus::Connection) -> zbus::Result<String> {
    property(conn, TIMEDATE, TIMEDATE_PATH, TIMEDATE, "Timezone").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_on_looks_at_once_and_triggers_wait_out_the_hour() {
        let mut p = Pace::default();
        p.wake(&Wake::Network, 10);
        assert!(!p.take(10), "off: a network change does nothing");
        p.wake(&Wake::On(true), 10);
        assert!(p.take(10));
        p.done(false, 10);
        p.wake(&Wake::Resumed, 600);
        assert_eq!(p.due, Some(10 + HOUR), "the hour runs from the last look");
        assert!(!p.take(600));
        p.wake(&Wake::Network, 700);
        assert_eq!(p.due, Some(10 + HOUR), "one look covers both");
        assert!(p.take(10 + HOUR));
        p.done(false, 10 + HOUR);
        p.wake(&Wake::Resumed, 20_000);
        assert!(p.take(20_000), "past the hour a resume looks at once");
        p.done(false, 20_000);
        p.wake(&Wake::On(false), 20_001);
        p.wake(&Wake::Network, 30_000);
        assert!(!p.take(30_000));
        p.wake(&Wake::On(true), 30_001);
        assert!(p.take(30_001), "turning on again does not wait");
    }

    #[test]
    fn a_look_that_found_no_place_lets_a_trigger_look_after_five_minutes() {
        let mut p = Pace::default();
        p.wake(&Wake::Resumed, 0);
        p.wake(&Wake::On(true), 0);
        assert!(p.take(0));
        p.done(true, 60);
        p.wake(&Wake::Network, 90);
        assert_eq!(p.due, Some(60 + RETRY), "not the hour");
        assert!(p.take(60 + RETRY));
        p.done(false, 60 + RETRY);
        p.wake(&Wake::Network, 60 + 2 * RETRY);
        assert_eq!(
            p.due,
            Some(60 + RETRY + HOUR),
            "a found place spends the hour"
        );
    }

    #[test]
    fn turning_the_switch_leaves_a_look_in_flight_behind() {
        let zone = AutoZone::default();
        let turn = zone.turns.now();
        zone.send(Wake::Resumed);
        zone.send(Wake::Network);
        assert!(zone.turns.still(turn), "triggers are not turns");
        zone.send(Wake::On(false));
        assert!(!zone.turns.still(turn), "turned off: the look sets nothing");
        let turn = zone.turns.now();
        zone.send(Wake::On(true));
        assert!(
            !zone.turns.still(turn),
            "turned on again: a fresh look follows"
        );
    }

    #[test]
    fn a_refusal_wanting_a_password_is_told_apart() {
        let e = |n: &str| reprisebus::error::reply(n, "no");
        assert!(needs_password(&e(
            "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
        )));
        assert!(needs_password(&e(
            "org.freedesktop.DBus.Error.AccessDenied"
        )));
        assert!(!needs_password(&e("org.freedesktop.timedate1.Failed")));
    }

    #[test]
    fn only_a_new_primary_connection_moves_the_network() {
        assert!(connected(
            NM,
            "/org/freedesktop/NetworkManager/ActiveConnection/3"
        ));
        assert!(!connected(NM, "/"), "disconnected");
        assert!(!connected(
            "org.freedesktop.NetworkManager.Device",
            "/org/freedesktop/NetworkManager/ActiveConnection/3"
        ));
    }
}
