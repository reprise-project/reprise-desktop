//! Programs' progress, unread counts and urgency on their task buttons through
//! `com.canonical.Unity.LauncherEntry`: a program sends `Update(app_uri, properties)` and the
//! buttons of the sender process's windows draw `progress` while `progress-visible` holds; a sender
//! with no window of its own lights its app's buttons. `count` (while `count-visible`) badges every
//! button of the app, and `urgent` turning on flashes its windows' buttons. `progress-paused` is
//! Reprise's own key. A sender leaving the bus takes its entries.

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
    count: Option<u32>,
    count_visible: Option<bool>,
    urgent: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Entry {
    pid: u32,
    progress: f64,
    visible: bool,
    paused: bool,
    /// When `visible` last turned on, on the [`Entries`] clock.
    since: u64,
    count: u32,
    count_visible: bool,
    /// When an update last carried `count` or `count-visible`, on the [`Entries`] clock.
    counted: u64,
    urgent: bool,
}

impl Entry {
    fn badge(&self) -> Option<u32> {
        (self.count_visible && self.count > 0).then_some(self.count)
    }
}

/// What one message changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// A button's progress or badge may have changed.
    pub redraw: bool,
    /// The app key whose `urgent` just turned on.
    pub urgent: Option<String>,
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
            let applied = state.launcher_entries.apply(msg);
            if let Some(app) = applied.urgent {
                state.launcher_urgent(&app);
            }
            if applied.redraw {
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

    /// Take `msg` in.
    fn apply(&mut self, msg: Msg) -> Applied {
        match msg {
            Msg::Update {
                sender,
                pid,
                app,
                props,
            } => {
                self.clock += 1;
                let e = self.by_sender.entry((sender, app.clone())).or_default();
                let before = e.clone();
                e.pid = pid;
                e.progress = props.progress.unwrap_or(e.progress).clamp(0.0, 1.0);
                e.visible = props.visible.unwrap_or(e.visible);
                e.paused = props.paused.unwrap_or(e.paused);
                e.count = props.count.unwrap_or(e.count);
                e.count_visible = props.count_visible.unwrap_or(e.count_visible);
                e.urgent = props.urgent.unwrap_or(e.urgent);
                if e.visible && !before.visible {
                    e.since = self.clock;
                }
                let counts = props.count.is_some() || props.count_visible.is_some();
                if counts {
                    e.counted = self.clock;
                }
                let shown = |e: &Entry| e.visible.then_some((e.progress, e.paused));
                Applied {
                    redraw: shown(&before) != shown(e) || counts,
                    urgent: (e.urgent && !before.urgent).then_some(app),
                }
            }
            Msg::Gone(sender) => {
                let before = self.by_sender.len();
                self.by_sender.retain(|(s, _), _| *s != sender);
                Applied {
                    redraw: self.by_sender.len() != before,
                    urgent: None,
                }
            }
        }
    }

    /// True when some entry shows progress.
    pub fn any_shown(&self) -> bool {
        self.by_sender.values().any(|e| e.visible)
    }

    /// True when some entry shows a count.
    pub fn any_badge(&self) -> bool {
        self.by_sender.values().any(|e| e.badge().is_some())
    }

    /// The unread count for the app `keys` ([`app_keys`]) name: what the sender that last sent a
    /// count key for it says, while visible and above 0.
    pub fn badge(&self, keys: &[String]) -> Option<u32> {
        (self.by_sender.iter())
            .filter(|((_, app), _)| keys.contains(app))
            .max_by_key(|(_, e)| e.counted)
            .and_then(|(_, e)| e.badge())
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

/// The app keys entries for a window or pin of `app_id` (or the desktop file `desktop_id`) may come
/// under: the id itself and the desktop file `index` finds for it, lower case.
pub fn app_keys(app_id: &str, desktop_id: Option<&str>, index: &[AppEntry]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut add = |id: &str| {
        let key = id
            .strip_suffix(".desktop")
            .unwrap_or(id)
            .to_ascii_lowercase();
        if !key.is_empty() && !keys.contains(&key) {
            keys.push(key);
        }
    };
    add(app_id);
    if let Some(d) = desktop_id {
        add(d);
    }
    if !app_id.is_empty()
        && let Some(entry) = reprisefs::apps::entry_for_app_id(index, app_id, |_| true)
    {
        add(&entry.id);
    }
    keys
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
    Some((sender, app_key(&uri)?, props_of(props)))
}

fn props_of(props: HashMap<String, OwnedVariant>) -> Props {
    let mut p = Props::default();
    for (k, v) in props {
        match k.as_str() {
            "progress" => p.progress = f64::try_from(v).ok(),
            "progress-visible" => p.visible = bool::try_from(v).ok(),
            "progress-paused" => p.paused = bool::try_from(v).ok(),
            "count" => p.count = count_of(&v),
            "count-visible" => p.count_visible = bool::try_from(v).ok(),
            "urgent" => p.urgent = bool::try_from(v).ok(),
            _ => {}
        }
    }
    p
}

/// `count` as a count: the convention's int64, or any other integer a sender picks; a negative
/// one is 0.
fn count_of(v: &zbus::Variant<'_>) -> Option<u32> {
    use zbus::Variant as Value;
    let n: i128 = match v {
        Value::I64(n) => (*n).into(),
        Value::U64(n) => (*n).into(),
        Value::I32(n) => (*n).into(),
        Value::U32(n) => (*n).into(),
        Value::I16(n) => (*n).into(),
        Value::U16(n) => (*n).into(),
        Value::U8(n) => (*n).into(),
        _ => return None,
    };
    Some(u32::try_from(n.max(0)).unwrap_or(u32::MAX))
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
                ..Props::default()
            },
        }
    }

    fn props(sender: &str, app: &str, props: Props) -> Msg {
        Msg::Update {
            sender: sender.into(),
            pid: 1,
            app: app.into(),
            props,
        }
    }

    fn counted(count: Option<u32>, count_visible: Option<bool>) -> Props {
        Props {
            count,
            count_visible,
            ..Props::default()
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
            !e.apply(update(":1.5", 50, "app", Some(0.3), None)).redraw,
            "not visible yet"
        );
        assert!(e.apply(update(":1.5", 50, "app", None, Some(true))).redraw);
        assert_eq!(e.progress(None, "App", &[], no_windows), shown(0.3));
        assert!(
            !e.apply(Msg::Gone(":1.9".into())).redraw,
            "another sender left"
        );
        assert!(e.apply(Msg::Gone(":1.5".into())).redraw);
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

    #[test]
    fn count_count_visible_and_urgent_parse_from_their_variants() {
        use zbus::Variant as Value;
        let v = |val: Value<'static>| OwnedVariant::try_from(val).unwrap();
        let p = props_of(HashMap::from([
            ("count".to_string(), v(Value::I64(12))),
            ("count-visible".to_string(), v(Value::Bool(true))),
            ("urgent".to_string(), v(Value::Bool(true))),
            ("progress".to_string(), v(Value::F64(0.5))),
            ("unknown".to_string(), v(Value::Bool(true))),
        ]));
        assert_eq!(
            p,
            Props {
                progress: Some(0.5),
                count: Some(12),
                count_visible: Some(true),
                urgent: Some(true),
                ..Props::default()
            }
        );
        let count = |val| props_of(HashMap::from([("count".to_string(), v(val))])).count;
        assert_eq!(count(Value::I32(3)), Some(3), "an int32 count is taken too");
        assert_eq!(count(Value::U64(7)), Some(7));
        assert_eq!(count(Value::I64(-4)), Some(0), "a negative count is none");
        assert_eq!(count(Value::Str("5".into())), None);
        let wrong = props_of(HashMap::from([(
            "count-visible".to_string(),
            v(Value::I32(1)),
        )]));
        assert_eq!(wrong.count_visible, None, "a mistyped key is left out");
    }

    #[test]
    fn a_badge_shows_while_count_visible_and_above_zero_and_the_last_sender_wins() {
        let keys = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut e = Entries::default();
        e.apply(props(":1.5", "mail", counted(Some(3), None)));
        assert_eq!(e.badge(&keys(&["mail"])), None, "not visible yet");
        assert!(
            e.apply(props(":1.5", "mail", counted(None, Some(true))))
                .redraw
        );
        assert_eq!(e.badge(&keys(&["other", "mail"])), Some(3));
        assert!(e.any_badge());
        e.apply(props(":1.5", "mail", counted(Some(0), None)));
        assert_eq!(e.badge(&keys(&["mail"])), None, "0 shows no badge");
        e.apply(props(":1.5", "mail", counted(Some(4), None)));
        e.apply(props(":1.6", "mail", counted(Some(9), Some(true))));
        assert_eq!(
            e.badge(&keys(&["mail"])),
            Some(9),
            "the newer sender's count"
        );
        e.apply(Msg::Gone(":1.6".into()));
        assert_eq!(
            e.badge(&keys(&["mail"])),
            Some(4),
            "the other sender's count stays"
        );
        assert!(
            e.apply(props(":1.7", "mail", counted(None, Some(false))))
                .redraw
        );
        assert_eq!(
            e.badge(&keys(&["mail"])),
            None,
            "a newer sender hiding the count hides it"
        );
        assert!(
            !e.apply(props(":1.7", "mail", Props::default())).redraw,
            "an update without count keys leaves the badge alone"
        );
        assert_eq!(e.badge(&keys(&["chat"])), None);
    }

    #[test]
    fn urgent_reports_its_app_only_when_it_turns_on() {
        let mut e = Entries::default();
        let urgent = |on| Props {
            urgent: Some(on),
            ..Props::default()
        };
        let a = e.apply(props(":1.5", "chat", urgent(true)));
        assert_eq!(a.urgent.as_deref(), Some("chat"));
        assert!(!a.redraw, "urgency alone redraws nothing");
        assert_eq!(e.apply(props(":1.5", "chat", urgent(true))).urgent, None);
        assert_eq!(
            e.apply(props(":1.5", "chat", Props::default())).urgent,
            None
        );
        e.apply(props(":1.5", "chat", urgent(false)));
        assert_eq!(
            e.apply(props(":1.5", "chat", urgent(true)))
                .urgent
                .as_deref(),
            Some("chat"),
            "urgent again after clearing flashes again"
        );
    }

    #[test]
    fn app_keys_take_the_id_the_desktop_file_and_drop_duplicates() {
        assert_eq!(app_keys("Mail", None, &[]), ["mail"]);
        assert_eq!(
            app_keys("mail", Some("org.Mail.desktop"), &[]),
            ["mail", "org.mail"]
        );
        assert_eq!(app_keys("", Some("mail.desktop"), &[]), ["mail"]);
    }
}
