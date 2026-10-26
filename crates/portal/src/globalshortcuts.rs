//! `org.freedesktop.impl.portal.GlobalShortcuts` version 2. A session holds the compositor's
//! `reprise_shortcuts_v1` for the app, so its `[[global_shortcuts]]` chords are live while the
//! session is open. The permit answer is kept in config.toml (`shortcutconf`).

use crate::inputlink::{Link, MAX_SHORTCUTS, ShortcutEvent, ShortcutsHandle, live};
use crate::permit::Answer;
use crate::shortcutconf::{self, Entry, Wanted};
use crate::{
    RESPONSE_CANCELLED, RESPONSE_OK, RESPONSE_OTHER, Results, SessionData, SessionKind, Sessions,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

type Shortcuts = Vec<(String, HashMap<String, OwnedValue>)>;

pub(crate) struct ShortcutsSession {
    app: String,
    ids: Vec<String>,
    _handle: ShortcutsHandle,
    _relay: zbus::Task<()>,
}

pub struct GlobalShortcuts {
    pub sessions: Sessions,
    /// None when the compositor offered no `reprise_input_v1`; sessions
    /// then fail, as they do on a compositor without `create_shortcuts`.
    pub link: Option<Link>,
    pub frontend: crate::Frontend,
}

fn string_opt(opts: &HashMap<String, OwnedValue>, key: &str) -> String {
    opts.get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}

fn wanted(shortcuts: Shortcuts) -> Vec<Wanted> {
    shortcuts
        .into_iter()
        .map(|(id, o)| Wanted {
            description: string_opt(&o, "description"),
            trigger: string_opt(&o, "preferred_trigger"),
            id,
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Refused,
    /// A first bind of no shortcuts: an empty OK, nothing asked or written,
    /// so the app is asked once it binds something.
    Nothing,
    Ask,
    Write,
}

fn step(doc: &toml_edit::DocumentMut, app: &str, wanted: &[Wanted]) -> Step {
    if shortcutconf::is_denied(doc, app) {
        Step::Refused
    } else if !shortcutconf::entries(doc, app).is_empty() {
        Step::Write
    } else if wanted.is_empty() {
        Step::Nothing
    } else {
        Step::Ask
    }
}

fn trigger(keys: &str) -> &str {
    let live =
        repriseconf::keybind::check(keys).is_ok() && !repriseconf::keybind::modifier_only(keys);
    if live { keys } else { "" }
}

fn described(entries: &[Entry]) -> Shortcuts {
    entries
        .iter()
        .map(|e| {
            let mut o = HashMap::new();
            o.insert(
                "description".into(),
                Value::from(e.description.as_str())
                    .try_to_owned()
                    .expect("string"),
            );
            o.insert(
                "trigger_description".into(),
                Value::from(trigger(&e.keys))
                    .try_to_owned()
                    .expect("string"),
            );
            (e.id.clone(), o)
        })
        .collect()
}

fn results(entries: &[Entry]) -> Results {
    let mut r = Results::new();
    r.insert(
        "shortcuts".into(),
        Value::from(described(entries))
            .try_to_owned()
            .expect("shortcuts"),
    );
    r
}

fn session_of(sessions: &Sessions, session: &str) -> Option<(String, Vec<String>)> {
    let s = sessions.lock().unwrap();
    match &s.get(session)?.kind {
        SessionKind::GlobalShortcuts(g) => Some((g.app.clone(), g.ids.clone())),
        _ => None,
    }
}

fn current(app: &str, ids: &[String]) -> Vec<Entry> {
    let all = shortcutconf::entries(&shortcutconf::read(), app);
    ids.iter()
        .filter_map(|id| all.iter().find(|e| &e.id == id).cloned())
        .collect()
}

#[zbus::interface(name = "org.freedesktop.impl.portal.GlobalShortcuts")]
impl GlobalShortcuts {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: HashMap<String, Value<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u32, Results)> {
        self.frontend.check(&hdr)?;
        crate::check_session(&session_handle)?;
        // Bindings are kept per app id; a program without one has nowhere
        // to keep them.
        if app_id.is_empty() {
            log::warn!("GlobalShortcuts: refusing a program with no app id");
            return Ok((RESPONSE_OTHER, Results::new()));
        }
        let Some(link) = live(&self.link).filter(|l| l.has_shortcuts()) else {
            log::warn!(
                "GlobalShortcuts: refusing {app_id:?}: the compositor's input link is gone or makes no shortcuts objects"
            );
            return Ok((RESPONSE_OTHER, Results::new()));
        };
        log::info!(
            "GlobalShortcuts.CreateSession {} for {app_id:?}",
            session_handle.as_str()
        );
        let Some(h) = link.create_shortcuts(&app_id) else {
            log::warn!(
                "GlobalShortcuts: refusing {app_id:?}: {MAX_SHORTCUTS} sessions are open, the most the compositor keeps"
            );
            return Ok((RESPONSE_OTHER, Results::new()));
        };
        let path: OwnedObjectPath = session_handle.clone().into();
        // Relay the compositor's events as portal signals for as long as
        // the session lives; the task dies with the session's data.
        let task = conn.executor().spawn(
            relay(
                conn.clone(),
                self.sessions.clone(),
                path.clone(),
                h.events(),
            ),
            "global-shortcuts-relay",
        );
        crate::replace_session(
            &self.sessions,
            session_handle.to_string(),
            SessionData::new(
                crate::sender(&hdr),
                SessionKind::GlobalShortcuts(ShortcutsSession {
                    app: app_id,
                    ids: Vec::new(),
                    _handle: h,
                    _relay: task,
                }),
            ),
        )
        .await;
        server
            .at(
                session_handle.clone(),
                crate::Session {
                    path,
                    sessions: self.sessions.clone(),
                },
            )
            .await?;
        Ok((RESPONSE_OK, Results::new()))
    }

    /// Binds `shortcuts` for the session's app. The app's first bind asks
    /// the user; a Deny is kept, and later binds answer Cancelled without
    /// asking. A prompt closed without an answer asks again next time. A
    /// first bind of no shortcuts answers OK with none and asks nothing.
    #[allow(clippy::too_many_arguments)]
    async fn bind_shortcuts(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        shortcuts: Shortcuts,
        parent_window: String,
        _options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, Results)> {
        self.frontend.check(&hdr)?;
        let key = session_handle.as_str();
        let Some((app, _)) = session_of(&self.sessions, key) else {
            return Ok((RESPONSE_OTHER, Results::new()));
        };
        let wanted = wanted(shortcuts);
        log::info!(
            "GlobalShortcuts.BindShortcuts {key} for {app:?}: {:?}",
            wanted.iter().map(|w| &w.id).collect::<Vec<_>>()
        );
        let doc = shortcutconf::read();
        let first = match step(&doc, &app, &wanted) {
            Step::Refused => return Ok((RESPONSE_CANCELLED, Results::new())),
            Step::Nothing => return Ok((RESPONSE_OK, results(&[]))),
            Step::Ask => true,
            Step::Write => false,
        };
        if first {
            // The Request object lets the frontend cancel the prompt; a
            // Session.Close while it is up cancels it too.
            let cancel = Arc::new(AtomicBool::new(false));
            crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
            if let Some(s) = self.sessions.lock().unwrap().get_mut(key) {
                s.picking = Some(cancel.clone());
            }
            let items = shortcutconf::preview(&doc, &wanted);
            let parent = crate::dialog::parent_handle(&parent_window);
            let id = app.clone();
            let answer = blocking::unblock(move || {
                crate::permit::ask(
                    crate::permit::Kind::Shortcuts,
                    &id,
                    items,
                    false,
                    parent,
                    cancel,
                )
            })
            .await;
            let _ = server.remove::<crate::Request, _>(handle.clone()).await;
            if let Some(s) = self.sessions.lock().unwrap().get_mut(key) {
                s.picking = None;
            }
            match answer {
                Answer::Allow { .. } => {}
                Answer::Deny => {
                    log::info!("GlobalShortcuts.BindShortcuts for {app:?} denied");
                    if let Err(e) = shortcutconf::update(|d| shortcutconf::deny(d, &app)) {
                        log::warn!("GlobalShortcuts: recording the refusal: {e}");
                    }
                    return Ok((RESPONSE_CANCELLED, Results::new()));
                }
                Answer::Dismissed => {
                    log::info!("GlobalShortcuts.BindShortcuts for {app:?}: no answer");
                    return Ok((RESPONSE_CANCELLED, Results::new()));
                }
            }
        }
        let entries = match shortcutconf::update(|d| shortcutconf::bind(d, &app, &wanted)) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("GlobalShortcuts: writing {app}'s shortcuts: {e}");
                return Ok((RESPONSE_OTHER, Results::new()));
            }
        };
        if let Some(d) = self.sessions.lock().unwrap().get_mut(key)
            && let SessionKind::GlobalShortcuts(g) = &mut d.kind
        {
            g.ids = wanted.iter().map(|w| w.id.clone()).collect();
        }
        Ok((RESPONSE_OK, results(&entries)))
    }

    async fn list_shortcuts(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
    ) -> zbus::fdo::Result<(u32, Results)> {
        self.frontend.check(&hdr)?;
        match session_of(&self.sessions, session_handle.as_str()) {
            Some((app, ids)) if ids.is_empty() => Ok((
                RESPONSE_OK,
                results(&shortcutconf::entries(&shortcutconf::read(), &app)),
            )),
            Some((app, ids)) => Ok((RESPONSE_OK, results(&current(&app, &ids)))),
            None => Ok((RESPONSE_OTHER, Results::new())),
        }
    }

    async fn configure_shortcuts(
        &self,
        _session_handle: ObjectPath<'_>,
        _parent_window: String,
        _options: HashMap<String, Value<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr)?;
        match std::process::Command::new("reprise-settings")
            .args(["--page", "keyboard"])
            .spawn()
        {
            // Reap it when it exits; the portal outlives it.
            Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
            Err(e) => log::warn!("GlobalShortcuts: reprise-settings: {e}"),
        }
        Ok(())
    }

    #[zbus(signal)]
    async fn activated(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        shortcut_id: String,
        timestamp: u64,
        options: Results,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn deactivated(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        shortcut_id: String,
        timestamp: u64,
        options: Results,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn shortcuts_changed(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        shortcuts: Shortcuts,
    ) -> zbus::Result<()>;
}

/// The compositor's events for one session as its signals, with the
/// session path first. A key change reaches a session only once it has
/// bound something. Ends when the session's handle drops.
async fn relay(
    conn: zbus::Connection,
    sessions: Sessions,
    session: OwnedObjectPath,
    events: async_channel::Receiver<ShortcutEvent>,
) {
    let Ok(emitter) = SignalEmitter::new(&conn, crate::PATH) else {
        return;
    };
    while let Ok(ev) = events.recv().await {
        let r = match ev {
            ShortcutEvent::Activated { id, time } => {
                GlobalShortcuts::activated(
                    &emitter,
                    session.clone(),
                    id,
                    time.into(),
                    Results::new(),
                )
                .await
            }
            ShortcutEvent::Deactivated { id, time } => {
                GlobalShortcuts::deactivated(
                    &emitter,
                    session.clone(),
                    id,
                    time.into(),
                    Results::new(),
                )
                .await
            }
            ShortcutEvent::Changed => {
                let Some((app, ids)) = session_of(&sessions, session.as_str()) else {
                    continue;
                };
                if ids.is_empty() {
                    continue;
                }
                let shortcuts = described(&current(&app, &ids));
                GlobalShortcuts::shortcuts_changed(&emitter, session.clone(), shortcuts).await
            }
        };
        if let Err(e) = r {
            log::warn!("GlobalShortcuts signal: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_shortcuts_read_their_description_and_trigger() {
        let mut opts = HashMap::new();
        opts.insert(
            "description".to_string(),
            OwnedValue::try_from(Value::from("Hold to talk")).unwrap(),
        );
        opts.insert(
            "preferred_trigger".to_string(),
            OwnedValue::try_from(Value::from("Pause")).unwrap(),
        );
        let got = wanted(vec![("talk".into(), opts), ("bare".into(), HashMap::new())]);
        assert_eq!(
            got[0],
            Wanted {
                id: "talk".into(),
                description: "Hold to talk".into(),
                trigger: "Pause".into()
            }
        );
        assert_eq!(
            got[1],
            Wanted {
                id: "bare".into(),
                description: String::new(),
                trigger: String::new()
            }
        );
    }

    #[test]
    fn bound_shortcuts_carry_their_trigger_description() {
        let out = described(&[Entry {
            id: "talk".into(),
            description: "Hold to talk".into(),
            keys: "Pause".into(),
        }]);
        assert_eq!(out[0].0, "talk");
        assert_eq!(
            String::try_from(out[0].1["trigger_description"].clone()).unwrap(),
            "Pause"
        );
        assert_eq!(
            String::try_from(out[0].1["description"].clone()).unwrap(),
            "Hold to talk"
        );
    }

    #[test]
    fn keys_the_compositor_leaves_unassigned_have_no_trigger_description() {
        let e = |keys: &str| Entry {
            id: "x".into(),
            description: String::new(),
            keys: keys.into(),
        };
        let out = described(&[e("Ctrl+Shift_L"), e("Super+Nope"), e(""), e("Pause")]);
        let trigger = |i: usize| String::try_from(out[i].1["trigger_description"].clone()).unwrap();
        assert_eq!(trigger(0), "", "a modifier-only chord never fires");
        assert_eq!(trigger(1), "", "a key the layout does not name");
        assert_eq!(trigger(2), "");
        assert_eq!(trigger(3), "Pause");
    }

    #[test]
    fn a_first_bind_of_nothing_asks_nothing() {
        let w = |id: &str| Wanted {
            id: id.into(),
            description: String::new(),
            trigger: String::new(),
        };
        let none = toml_edit::DocumentMut::new();
        assert_eq!(step(&none, "t", &[]), Step::Nothing, "nothing to ask about");
        assert_eq!(step(&none, "t", &[w("talk")]), Step::Ask);
        let bound: toml_edit::DocumentMut =
            "[[global_shortcuts]]\napp = \"t\"\nid = \"talk\"\nkeys = \"\"\n"
                .parse()
                .unwrap();
        assert_eq!(step(&bound, "t", &[]), Step::Write, "an allowed app binds");
        assert_eq!(step(&bound, "t", &[w("talk")]), Step::Write);
        let denied: toml_edit::DocumentMut =
            "[portal]\nshortcuts_denied = [\"t\"]\n".parse().unwrap();
        assert_eq!(step(&denied, "t", &[]), Step::Refused);
        assert_eq!(step(&denied, "t", &[w("talk")]), Step::Refused);
    }

    #[test]
    fn results_carry_the_shortcuts_as_a_list_of_id_and_properties() {
        let r = results(&[Entry {
            id: "talk".into(),
            description: String::new(),
            keys: "Pause".into(),
        }]);
        assert_eq!(r["shortcuts"].value_signature().to_string(), "a(sa{sv})");
    }
}
