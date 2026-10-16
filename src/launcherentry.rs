//! Programs' progress on their task buttons through `com.canonical.Unity.LauncherEntry`: a program
//! sends `Update(app_uri, properties)` and its windows' buttons draw `progress` while
//! `progress-visible` holds. `progress-paused` is Reprise's own key. A sender leaving the bus
//! takes its entries with it.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use reprisefs::apps::AppEntry;
use reprisekit::taskbar::TaskProgress;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::channel::{self, Event, Sender};
use zbus::zvariant::OwnedValue;

use crate::compositor::State;

const IFACE: &str = "com.canonical.Unity.LauncherEntry";
const RETRY: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Update {
        sender: String,
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
    sender: String,
    progress: f64,
    visible: bool,
    paused: bool,
}

/// The entries by app key: the desktop file's id without `.desktop`, lower case.
#[derive(Default)]
pub struct Entries {
    by_app: HashMap<String, Entry>,
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

    /// Take `msg` in; true when a button's progress changed.
    fn apply(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Update { sender, app, props } => {
                let e = self.by_app.entry(app).or_default();
                let before = e.clone();
                e.sender = sender;
                e.progress = props.progress.unwrap_or(e.progress).clamp(0.0, 1.0);
                e.visible = props.visible.unwrap_or(e.visible);
                e.paused = props.paused.unwrap_or(e.paused);
                let shown = |e: &Entry| e.visible.then_some((e.progress, e.paused));
                shown(&before) != shown(e)
            }
            Msg::Gone(sender) => {
                let before = self.by_app.len();
                self.by_app.retain(|_, e| e.sender != sender);
                self.by_app.len() != before
            }
        }
    }

    /// The progress of the program behind a window of `app_id`: its entry under the app id itself,
    /// else under the desktop file `index` finds for it.
    pub fn progress(&self, app_id: &str, index: &[AppEntry]) -> Option<TaskProgress> {
        if self.by_app.is_empty() || app_id.is_empty() {
            return None;
        }
        let e = self.by_app.get(&app_id.to_ascii_lowercase()).or_else(|| {
            let entry = reprisefs::apps::entry_for_app_id(index, app_id, |_| true)?;
            self.by_app
                .get(&app_key(&format!("application://{}", entry.id))?)
        })?;
        e.visible.then_some(TaskProgress {
            fraction: e.progress,
            paused: e.paused,
        })
    }
}

fn update_of(msg: &zbus::Message) -> Option<Msg> {
    let sender = msg.header().sender()?.to_string();
    let (uri, props) = msg
        .body()
        .deserialize::<(String, HashMap<String, OwnedValue>)>()
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
    Some(Msg::Update {
        sender,
        app: app_key(&uri)?,
        props: p,
    })
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
        .interface(IFACE)?
        .member("Update")?
        .build();
    let updates = zbus::MessageStream::for_match_rule(rule, &conn, None)
        .await?
        .filter_map(|m| std::future::ready(m.ok().as_ref().and_then(update_of)));
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let gone = dbus.receive_name_owner_changed().await?.filter_map(|s| {
        std::future::ready(s.args().ok().and_then(|a| {
            let name = a.name().to_string();
            (name.starts_with(':') && a.new_owner().is_none()).then_some(Msg::Gone(name))
        }))
    });
    let mut all = stream::select(updates.boxed(), gone.boxed());
    while let Some(msg) = all.next().await {
        if tx.send(msg).is_err() {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(sender: &str, app: &str, progress: Option<f64>, visible: Option<bool>) -> Msg {
        Msg::Update {
            sender: sender.into(),
            app: app.into(),
            props: Props {
                progress,
                visible,
                paused: None,
            },
        }
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
            !e.apply(update(":1.5", "app", Some(0.3), None)),
            "not visible yet"
        );
        assert!(e.apply(update(":1.5", "app", None, Some(true))));
        assert_eq!(
            e.progress("App", &[]),
            Some(TaskProgress {
                fraction: 0.3,
                paused: false
            })
        );
        assert!(!e.apply(Msg::Gone(":1.9".into())), "another sender left");
        assert!(e.apply(Msg::Gone(":1.5".into())));
        assert_eq!(e.progress("app", &[]), None);
    }
}
