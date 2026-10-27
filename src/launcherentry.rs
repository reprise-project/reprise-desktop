//! Programs' progress on their task buttons through `com.canonical.Unity.LauncherEntry`: a program
//! sends `Update(app_uri, properties)` and the buttons of the sender process's windows draw
//! `progress` while `progress-visible` holds; a sender with no window of its own lights its app's
//! buttons. `progress-paused` is Reprise's own key. A sender leaving the bus takes its entries.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use reprisefs::apps::AppEntry;
use reprisekit::taskbar::TaskProgress;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::channel::{self, Event, Sender};
use zbus::OwnedVariant;

use crate::compositor::State;

const IFACE: &str = "com.canonical.Unity.LauncherEntry";
const RETRY: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Update {
        sender: String,
        pid: u32,
        app: String,
        props: Props,
    },
    Gone(String),
}

/// The keys of one `Update`; a key left out keeps its last value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Props {
    progress: Option<f64>,
    visible: Option<bool>,
    paused: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Entry {
    pid: u32,
    progress: f64,
    visible: bool,
    paused: bool,
    /// When `visible` last turned on, on the [`Entries`] clock.
    since: u64,
}

/// The entries by sender and app key (the desktop file's id without `.desktop`, lower case), so
/// two processes of one program keep their own.
#[derive(Default)]
pub struct Entries {
    by_sender: HashMap<(String, String), Entry>,
    clock: u64,
}

/// The app key of `uri` (`application://<id>.desktop`).
fn app_key(uri: &str) -> Option<String> {
    let id = uri.strip_prefix("application://")?;
    let id = id.strip_suffix(".desktop").unwrap_or(id);
    (!id.is_empty()).then(|| id.to_ascii_lowercase())
}

impl Entries {
    pub fn start(handle: &LoopHandle<'static, State>) -> Entries {
        let (tx, rx) = channel::channel::<Msg>();
        let inserted = handle.insert_source(rx, |event, _, state| {
            let Event::Msg(msg) = event else {
                return;
            };
            if state.launcher_entries.apply(msg) {
                state.redraw_taskbars();
            }
        });
        if let Err(e) = inserted {
            log::warn!("launcher entries: channel: {e}");
            return Entries::default();
        }
        let spawned = std::thread::Builder::new()
            .name("launcher-entry".into())
            .spawn(move || zbus::block_on(watch(tx)));
        if let Err(e) = spawned {
            log::warn!("launcher entries: no thread: {e}");
        }
        Entries::default()
    }

    /// Take `msg` in; true when a button's progress may have changed.
    fn apply(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Update {
                sender,
                pid,
                app,
                props,
            } => {
                self.clock += 1;
                let e = self.by_sender.entry((sender, app)).or_default();
                let before = e.clone();
                e.pid = pid;
                e.progress = props.progress.unwrap_or(e.progress).clamp(0.0, 1.0);
                e.visible = props.visible.unwrap_or(e.visible);
                e.paused = props.paused.unwrap_or(e.paused);
                if e.visible && !before.visible {
                    e.since = self.clock;
                }
                let shown = |e: &Entry| e.visible.then_some((e.progress, e.paused));
                shown(&before) != shown(e)
            }
            Msg::Gone(sender) => {
                let before = self.by_sender.len();
                self.by_sender.retain(|(s, _), _| *s != sender);
                self.by_sender.len() != before
            }
        }
    }

    /// True when some entry shows progress.
    pub fn any_shown(&self) -> bool {
        self.by_sender.values().any(|e| e.visible)
    }

    /// The progress for a window of `app_id` whose client is process `pid`: what that process
    /// sent, else what a process owning no window (`owns_window` false) sent for the app id itself
    /// or the desktop file `index` finds for it. Of several, the one shown last wins.
    pub fn progress(
        &self,
        pid: Option<u32>,
        app_id: &str,
        index: &[AppEntry],
        owns_window: impl Fn(u32) -> bool,
    ) -> Option<TaskProgress> {
        let shown = || self.by_sender.iter().filter(|(_, e)| e.visible);
        let own = pid.and_then(|p| latest(shown().filter(|(_, e)| e.pid == p)));
        let e = own.or_else(|| {
            if app_id.is_empty() {
                return None;
            }
            let mut apps = vec![app_id.to_ascii_lowercase()];
            let mut windowless = shown().filter(|(_, e)| !owns_window(e.pid)).peekable();
            windowless.peek()?;
            if let Some(entry) = reprisefs::apps::entry_for_app_id(index, app_id, |_| true) {
                apps.extend(app_key(&format!("application://{}", entry.id)));
            }
            latest(windowless.filter(|((_, app), _)| apps.contains(app)))
        })?;
        Some(TaskProgress {
            fraction: e.progress,
            paused: e.paused,
        })
    }
}

fn latest<'a, K>(entries: impl Iterator<Item = (K, &'a Entry)>) -> Option<&'a Entry> {
    entries.map(|(_, e)| e).max_by_key(|e| e.since)
}

/// The sender, app key and keys of an `Update`.
fn update_of(msg: &zbus::Message) -> Option<(String, String, Props)> {
    let sender = msg.header().sender()?.to_string();
    let (uri, props) = msg
        .body()
        .deserialize::<(String, HashMap<String, OwnedVariant>)>()
        .ok()?;
    let mut p = Props::default();
    for (k, v) in props {
        match k.as_str() {
            "progress" => p.progress = f64::try_from(v).ok(),
            "progress-visible" => p.visible = bool::try_from(v).ok(),
            "progress-paused" => p.paused = bool::try_from(v).ok(),
            _ => {}
        }
    }
    Some((sender, app_key(&uri)?, p))
}

enum Seen {
    Update(zbus::Message),
    Gone(String),
}

/// The thread: follow every `Update` on the session bus, and every unique name leaving it. A bus
/// not there yet is tried again every [`RETRY`].
async fn watch(tx: Sender<Msg>) {
    loop {
        if let Err(e) = follow(&tx).await {
            log::info!("launcher entries: stopped ({e}); trying again");
        }
        async_io::Timer::after(RETRY).await;
    }
}

async fn follow(tx: &Sender<Msg>) -> zbus::Result<()> {
    let conn = zbus::Connection::session().await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(IFACE)
        .member("Update")
        .build()?;
    let updates = zbus::MessageStream::for_match_rule(rule, &conn, None)
        .await?
        .filter_map(|m| std::future::ready(m.ok().map(Seen::Update)));
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let gone = dbus.receive_name_owner_changed().await?.filter_map(|s| {
        std::future::ready(s.args().ok().and_then(|a| {
            let name = a.name().to_string();
            (name.starts_with(':') && a.new_owner().is_none()).then_some(Seen::Gone(name))
        }))
    });
    let mut all = stream::select(updates.boxed(), gone.boxed());
    // Each sender's pid, asked once. A sender whose pid cannot be had has left the bus, so its
    // updates are dropped rather than kept with no Gone to come.
    let mut pids: HashMap<String, u32> = HashMap::new();
    while let Some(seen) = all.next().await {
        let msg = match seen {
            Seen::Gone(name) => match pids.remove(&name) {
                Some(_) => Msg::Gone(name),
                None => continue,
            },
            Seen::Update(m) => {
                let Some((sender, app, props)) = update_of(&m) else {
                    continue;
                };
                let pid = match pids.get(&sender) {
                    Some(&pid) => pid,
                    None => {
                        let Ok(name) = zbus::names::BusName::try_from(sender.as_str()) else {
                            continue;
                        };
                        match dbus.get_connection_unix_process_id(name).await {
                            Ok(pid) => *pids.entry(sender.clone()).or_insert(pid),
                            Err(e) => {
                                log::debug!("launcher entries: no pid for {sender}: {e}");
                                continue;
                            }
                        }
                    }
                };
                Msg::Update {
                    sender,
                    pid,
                    app,
                    props,
                }
            }
        };
        if tx.send(msg).is_err() {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(
        sender: &str,
        pid: u32,
        app: &str,
        progress: Option<f64>,
        visible: Option<bool>,
    ) -> Msg {
        Msg::Update {
            sender: sender.into(),
            pid,
            app: app.into(),
            props: Props {
                progress,
                visible,
                paused: None,
            },
        }
    }

    fn shown(fraction: f64) -> Option<TaskProgress> {
        Some(TaskProgress {
            fraction,
            paused: false,
        })
    }

    fn no_windows(_: u32) -> bool {
        false
    }

    #[test]
    fn the_app_key_is_the_desktop_id_without_its_suffix() {
        assert_eq!(
            app_key("application://reprise.Explorer.desktop").as_deref(),
            Some("reprise.explorer")
        );
        assert_eq!(app_key("file:///x.desktop"), None);
        assert_eq!(app_key("application://"), None);
    }

    #[test]
    fn progress_shows_while_visible_keeps_left_out_keys_and_leaves_with_its_sender() {
        let mut e = Entries::default();
        assert!(
            !e.apply(update(":1.5", 50, "app", Some(0.3), None)),
            "not visible yet"
        );
        assert!(e.apply(update(":1.5", 50, "app", None, Some(true))));
        assert_eq!(e.progress(None, "App", &[], no_windows), shown(0.3));
        assert!(!e.apply(Msg::Gone(":1.9".into())), "another sender left");
        assert!(e.apply(Msg::Gone(":1.5".into())));
        assert_eq!(e.progress(None, "app", &[], no_windows), None);
    }

    #[test]
    fn two_processes_of_one_app_each_light_their_own_window() {
        let mut e = Entries::default();
        e.apply(update(":1.5", 50, "app", Some(0.2), Some(true)));
        e.apply(update(":1.6", 60, "app", Some(0.7), Some(true)));
        e.apply(update(":1.5", 50, "app", Some(0.3), None));
        let owns = |p: u32| p == 50 || p == 60;
        assert_eq!(e.progress(Some(50), "app", &[], owns), shown(0.3));
        assert_eq!(e.progress(Some(60), "app", &[], owns), shown(0.7));
        assert_eq!(
            e.progress(Some(70), "app", &[], owns),
            None,
            "a third window of the app shows neither process's job"
        );

        e.apply(update(":1.6", 60, "app", None, Some(false)));
        assert_eq!(e.progress(Some(60), "app", &[], owns), None);
        assert_eq!(
            e.progress(Some(50), "app", &[], owns),
            shown(0.3),
            "one clearing leaves the other's bar"
        );
        e.apply(Msg::Gone(":1.5".into()));
        assert_eq!(e.progress(Some(50), "app", &[], owns), None);
    }

    #[test]
    fn a_sender_with_no_window_lights_its_apps_windows_and_the_last_shown_wins() {
        let mut e = Entries::default();
        e.apply(update(":1.7", 70, "app", Some(0.2), Some(true)));
        e.apply(update(":1.8", 80, "app", Some(0.6), Some(true)));
        e.apply(update(":1.7", 70, "app", Some(0.25), None));
        e.apply(update(":1.9", 90, "other", Some(0.9), Some(true)));
        let owns = |p: u32| p == 50;
        assert_eq!(
            e.progress(Some(50), "app", &[], owns),
            shown(0.6),
            "the job shown last, however often the other sends"
        );
        e.apply(update(":1.8", 80, "app", None, Some(false)));
        assert_eq!(e.progress(Some(50), "app", &[], owns), shown(0.25));
        assert_eq!(
            e.progress(Some(50), "app", &[], |p| p == 50 || p == 70),
            None,
            "a sender that owns a window lights only its own"
        );
    }
}
