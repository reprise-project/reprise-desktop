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
use zbus::object_server::SignalEmitter;
use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant, Variant};

type Shortcuts = Vec<(String, HashMap<String, OwnedVariant>)>;

/// The sessions one app may hold, well under the compositor's `MAX_SHORTCUTS`.
const MAX_SESSIONS_PER_APP: usize = 4;
const _: () = assert!(MAX_SESSIONS_PER_APP < MAX_SHORTCUTS);

pub(crate) struct ShortcutsSession {
    app: String,
    ids: Vec<String>,
    _handle: ShortcutsHandle,
    _relay: zruntime::Task<(), zruntime::Shared>,
}

pub struct GlobalShortcuts {
    pub sessions: Sessions,
    /// None when the compositor offered no `reprise_input_v1`; sessions
    /// then fail, as they do on a compositor without `create_shortcuts`.
    pub link: Option<Link>,
    pub frontend: crate::Frontend,
    pub config: Config,
}

/// config.toml, [`shortcutconf::read`] outside tests. Blocking.
pub type Config = fn() -> toml_edit::DocumentMut;

fn string_opt(opts: &HashMap<String, OwnedVariant>, key: &str) -> String {
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
    /// A first bind that adds no shortcuts: an empty OK, nothing asked or written,
    /// so the app is asked once it binds something.
    Nothing,
    /// The permit prompt, listing `new`: every shortcut on a first bind (`first`), else only the
    /// ids an allowed app has not bound before.
    Ask {
        first: bool,
        new: Vec<String>,
    },
    Write,
}

fn step(doc: &toml_edit::DocumentMut, app: &str, wanted: &[Wanted]) -> Step {
    if shortcutconf::is_denied(doc, app) {
        return Step::Refused;
    }
    let first = shortcutconf::entries(doc, app).is_empty();
    let new = shortcutconf::preview(doc, app, wanted);
    match (first, new.is_empty()) {
        (true, true) => Step::Nothing,
        (false, true) => Step::Write,
        (_, false) => Step::Ask { first, new },
    }
}

/// What an allowed app's bind writes after the prompt for the ids not in `had`, its entries:
/// all of `wanted` on Allow; on Deny the new ids with no key, so they are not asked about
/// again; with no answer only the ids it had, so the next bind asks again.
fn granted(had: &[Entry], wanted: &[Wanted], answer: Answer) -> Vec<Wanted> {
    let known = |w: &Wanted| had.iter().any(|e| e.id == w.id);
    match answer {
        Answer::Allow { .. } => wanted.to_vec(),
        Answer::Deny => (wanted.iter())
            .map(|w| Wanted {
                trigger: if known(w) {
                    w.trigger.clone()
                } else {
                    String::new()
                },
                ..w.clone()
            })
            .collect(),
        Answer::Dismissed => wanted.iter().filter(|w| known(w)).cloned().collect(),
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
                Variant::from(e.description.as_str())
                    .try_to_owned()
                    .expect("string"),
            );
            o.insert(
                "trigger_description".into(),
                Variant::from(trigger(&e.keys))
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
        Variant::from(described(entries))
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

/// `config` read in the blocking pool: the file may sit on a slow home.
async fn read(config: Config) -> toml_edit::DocumentMut {
    blocking::unblock(config).await
}

/// [`shortcutconf::update`] in the blocking pool, where waiting on the edit lock stalls no call.
async fn update<T: Send + 'static>(
    f: impl FnOnce(&mut toml_edit::DocumentMut) -> T + Send + 'static,
) -> Result<T, String> {
    blocking::unblock(move || shortcutconf::update(f)).await
}

/// Whether `app`, holding `open` sessions, may open one more: not when the user denied it, nor
/// past its share of the compositor's pool, which one app would otherwise drain for all.
fn may_open(doc: &toml_edit::DocumentMut, app: &str, open: usize) -> bool {
    !shortcutconf::is_denied(doc, app) && open < MAX_SESSIONS_PER_APP
}

fn sessions_of(sessions: &Sessions, app: &str) -> usize {
    (sessions.lock().unwrap().values())
        .filter(|s| matches!(&s.kind, SessionKind::GlobalShortcuts(g) if g.app == app))
        .count()
}

async fn current(config: Config, app: &str, ids: &[String]) -> Vec<Entry> {
    let all = shortcutconf::entries(&read(config).await, app);
    ids.iter()
        .filter_map(|id| all.iter().find(|e| &e.id == id).cloned())
        .collect()
}

impl GlobalShortcuts {
    /// BindShortcuts once its request is served: asks, then writes the bindings.
    async fn bind(
        &self,
        request: &crate::Served<'_>,
        key: &str,
        app: String,
        wanted: Vec<Wanted>,
        parent_window: String,
    ) -> (u32, Results) {
        let doc = read(self.config).await;
        let (first, items) = match step(&doc, &app, &wanted) {
            Step::Refused => return (RESPONSE_CANCELLED, Results::new()),
            Step::Nothing => return (RESPONSE_OK, results(&[])),
            Step::Ask { first, new } => (first, Some(new)),
            Step::Write => (false, None),
        };
        let answer = match items {
            None => Answer::Allow { remember: false },
            Some(items) => {
                let parent = crate::dialog::parent_handle(&parent_window);
                let id = app.clone();
                let session = Some((&self.sessions, key));
                let asked = request.ask(session, move |c| {
                    crate::permit::ask(crate::permit::Kind::Shortcuts, &id, items, false, parent, c)
                });
                asked.await.unwrap_or(Answer::Dismissed)
            }
        };
        if request.closed() {
            log::info!("GlobalShortcuts.BindShortcuts for {app:?}: closed");
            return (RESPONSE_CANCELLED, Results::new());
        }
        match answer {
            Answer::Allow { .. } => {}
            Answer::Deny if first => {
                log::info!("GlobalShortcuts.BindShortcuts for {app:?} denied");
                let denied = app.clone();
                if let Err(e) = update(move |d| shortcutconf::deny(d, &denied)).await {
                    log::warn!("GlobalShortcuts: recording the refusal: {e}");
                }
                return (RESPONSE_CANCELLED, Results::new());
            }
            Answer::Dismissed if first => {
                log::info!("GlobalShortcuts.BindShortcuts for {app:?}: no answer");
                return (RESPONSE_CANCELLED, Results::new());
            }
            Answer::Deny | Answer::Dismissed => {
                log::info!("GlobalShortcuts.BindShortcuts for {app:?}: new shortcuts {answer:?}");
            }
        }
        let bound = app.clone();
        let written = update(move |d| {
            let had = shortcutconf::entries(d, &bound);
            let wanted = granted(&had, &wanted, answer);
            let ids: Vec<String> = wanted.iter().map(|w| w.id.clone()).collect();
            let known = (wanted.iter()).any(|w| had.iter().any(|e| e.id == w.id));
            (ids, known, shortcutconf::bind(d, &bound, &wanted))
        });
        let (ids, known, entries) = match written.await {
            Ok(w) => w,
            Err(e) => {
                log::warn!("GlobalShortcuts: writing {app}'s shortcuts: {e}");
                return (RESPONSE_OTHER, Results::new());
            }
        };
        // A refusal that leaves the session none of the shortcuts it already had is Cancelled.
        if !answer.allowed() && !known {
            return (RESPONSE_CANCELLED, Results::new());
        }
        if let Some(d) = self.sessions.lock().unwrap().get_mut(key)
            && let SessionKind::GlobalShortcuts(g) = &mut d.kind
        {
            g.ids = ids;
        }
        (RESPONSE_OK, results(&entries))
    }
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
        _options: HashMap<String, Variant<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u32, Results)> {
        self.frontend.check(&hdr).await?;
        crate::check_session(&session_handle)?;
        // Bindings are kept per app id; a program without one has nowhere
        // to keep them.
        if app_id.is_empty() {
            log::warn!("GlobalShortcuts: refusing a program with no app id");
            return Ok((RESPONSE_OTHER, Results::new()));
        }
        let doc = read(self.config).await;
        if !may_open(&doc, &app_id, sessions_of(&self.sessions, &app_id)) {
            log::warn!(
                "GlobalShortcuts: refusing {app_id:?}: it is in shortcuts_denied or holds {MAX_SESSIONS_PER_APP} sessions"
            );
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
        let task = zruntime::spawn(relay(
            conn.clone(),
            self.sessions.clone(),
            path.clone(),
            h.events(),
            self.config,
        ));
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
        let (owner, key) = (crate::sender(&hdr), session_handle.as_str());
        if crate::frontend_left(conn, &self.frontend, &self.sessions, &owner, key).await {
            return Ok((RESPONSE_OTHER, Results::new()));
        }
        Ok((RESPONSE_OK, Results::new()))
    }

    /// Binds `shortcuts` for the session's app. The app's first bind asks
    /// the user; a Deny is kept, and later binds answer Cancelled without
    /// asking. Once allowed, a bind asks only about ids the app has not bound
    /// before (`granted`). A prompt closed without an answer asks again next
    /// time. A first bind that adds no shortcut answers OK with none and asks nothing.
    #[allow(clippy::too_many_arguments)]
    async fn bind_shortcuts(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        shortcuts: Shortcuts,
        parent_window: String,
        _options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, Results)> {
        let asker = self.frontend.check(&hdr).await?;
        let key = session_handle.as_str();
        let Some((app, _)) = session_of(&self.sessions, key) else {
            return Ok((RESPONSE_OTHER, Results::new()));
        };
        let wanted = wanted(shortcuts);
        log::info!(
            "GlobalShortcuts.BindShortcuts {key} for {app:?}: {:?}",
            wanted.iter().map(|w| &w.id).collect::<Vec<_>>()
        );
        let request = crate::Served::new(server, &handle, &asker).await?;
        let out = self.bind(&request, key, app, wanted, parent_window).await;
        request.end().await;
        Ok(out)
    }

    async fn list_shortcuts(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
    ) -> zbus::fdo::Result<(u32, Results)> {
        self.frontend.check(&hdr).await?;
        match session_of(&self.sessions, session_handle.as_str()) {
            Some((app, ids)) if ids.is_empty() => Ok((
                RESPONSE_OK,
                results(&shortcutconf::entries(&read(self.config).await, &app)),
            )),
            Some((app, ids)) => Ok((
                RESPONSE_OK,
                results(&current(self.config, &app, &ids).await),
            )),
            None => Ok((RESPONSE_OTHER, Results::new())),
        }
    }

    async fn configure_shortcuts(
        &self,
        _session_handle: ObjectPath<'_>,
        _parent_window: String,
        _options: HashMap<String, Variant<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
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
    config: Config,
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
                let shortcuts = described(&current(config, &app, &ids).await);
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
    fn a_denied_app_or_one_at_its_session_share_opens_no_session() {
        let doc: toml_edit::DocumentMut = "[portal]\nshortcuts_denied = [\"t\"]\n".parse().unwrap();
        assert!(may_open(&doc, "o", 0));
        assert!(may_open(&doc, "o", MAX_SESSIONS_PER_APP - 1));
        assert!(
            !may_open(&doc, "o", MAX_SESSIONS_PER_APP),
            "the share is full"
        );
        assert!(!may_open(&doc, "t", 0), "t is denied");
    }

    #[test]
    fn requested_shortcuts_read_their_description_and_trigger() {
        let mut opts = HashMap::new();
        opts.insert(
            "description".to_string(),
            OwnedVariant::try_from(Variant::from("Hold to talk")).unwrap(),
        );
        opts.insert(
            "preferred_trigger".to_string(),
            OwnedVariant::try_from(Variant::from("Pause")).unwrap(),
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

    fn w(id: &str, trigger: &str) -> Wanted {
        Wanted {
            id: id.into(),
            description: String::new(),
            trigger: trigger.into(),
        }
    }

    fn bound() -> toml_edit::DocumentMut {
        "[[global_shortcuts]]\napp = \"t\"\nid = \"talk\"\nkeys = \"Pause\"\n"
            .parse()
            .unwrap()
    }

    #[test]
    fn a_first_bind_of_nothing_asks_nothing() {
        let none = toml_edit::DocumentMut::new();
        assert_eq!(step(&none, "t", &[]), Step::Nothing, "nothing to ask about");
        assert_eq!(
            step(&none, "t", &[w("", "")]),
            Step::Nothing,
            "no id is added"
        );
        assert_eq!(
            step(&none, "t", &[w("talk", "")]),
            Step::Ask {
                first: true,
                new: vec!["talk: no key".into()]
            }
        );
        assert_eq!(
            step(&bound(), "t", &[]),
            Step::Write,
            "an allowed app binds"
        );
        let denied: toml_edit::DocumentMut =
            "[portal]\nshortcuts_denied = [\"t\"]\n".parse().unwrap();
        assert_eq!(step(&denied, "t", &[]), Step::Refused);
        assert_eq!(step(&denied, "t", &[w("talk", "")]), Step::Refused);
    }

    #[test]
    fn a_rebind_of_known_ids_asks_nothing() {
        assert_eq!(step(&bound(), "t", &[w("talk", "F5")]), Step::Write);
    }

    #[test]
    fn a_rebind_with_a_new_id_asks_about_that_id_only() {
        assert_eq!(
            step(&bound(), "t", &[w("talk", "Pause"), w("mute", "CTRL+m")]),
            Step::Ask {
                first: false,
                new: vec!["mute: Ctrl+m".into()]
            }
        );
    }

    fn rebind(answer: Answer) -> Vec<Entry> {
        let mut d = bound();
        let had = shortcutconf::entries(&d, "t");
        let g = granted(&had, &[w("talk", "F5"), w("mute", "CTRL+m")], answer);
        shortcutconf::bind(&mut d, "t", &g);
        shortcutconf::entries(&d, "t")
    }

    fn e(id: &str, keys: &str) -> Entry {
        Entry {
            id: id.into(),
            description: String::new(),
            keys: keys.into(),
        }
    }

    #[test]
    fn an_allowed_rebind_gives_the_new_id_its_key() {
        let got = rebind(Answer::Allow { remember: false });
        assert_eq!(got, [e("talk", "Pause"), e("mute", "Ctrl+m")]);
    }

    #[test]
    fn a_denied_rebind_keeps_the_old_set_and_adds_the_new_id_with_no_key() {
        let got = rebind(Answer::Deny);
        assert_eq!(got, [e("talk", "Pause"), e("mute", "")]);
    }

    #[test]
    fn an_unanswered_rebind_keeps_only_the_old_set() {
        assert_eq!(rebind(Answer::Dismissed), [e("talk", "Pause")]);
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

    static CONFIG: crate::tests::Gate = crate::tests::Gate::new();

    fn config_held_open() -> toml_edit::DocumentMut {
        CONFIG.wait();
        toml_edit::DocumentMut::new()
    }

    static BOUND_CONFIG: crate::tests::Gate = crate::tests::Gate::new();

    /// config.toml with `talk` bound for org.example.App, so binding it again asks nothing.
    fn bound_config_held_open() -> toml_edit::DocumentMut {
        BOUND_CONFIG.wait();
        let mut doc = toml_edit::DocumentMut::new();
        let talk = Wanted {
            id: "talk".into(),
            description: "Talk".into(),
            trigger: String::new(),
        };
        shortcutconf::bind(&mut doc, "org.example.App", &[talk]);
        doc
    }

    /// BindShortcuts of `talk` for org.example.App, reading config.toml through `config`, with a
    /// Close once the read waits on `gate`: the answer and whether its Request is still served.
    /// None without a dbus-daemon.
    fn bind_closed_during_the_read(
        config: Config,
        gate: &'static crate::tests::Gate,
    ) -> Option<(u32, Results, bool)> {
        use crate::inputlink::tests::{fake_compositor, link_on, request};
        let bus = crate::lockmode::tests::bus()?;
        let (ours, mut theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            fake_compositor(&mut theirs, 2, 10);
            while request(&mut theirs).is_ok() {}
        });
        let link = link_on(ours);
        Some(zbus::block_on(async {
            let sessions = Sessions::default();
            let (client, portal) = crate::tests::portal_with(&bus, |frontend| GlobalShortcuts {
                sessions: sessions.clone(),
                link: None,
                frontend,
                config,
            })
            .await;
            let key = format!("{}1_1/g", crate::SESSION_PREFIX);
            let shortcuts = ShortcutsSession {
                app: "org.example.App".into(),
                ids: Vec::new(),
                _handle: link.create_shortcuts("org.example.App").unwrap(),
                _relay: zruntime::spawn(async {}),
            };
            let owner = client.unique_name().unwrap().to_string();
            let data = SessionData::new(owner, SessionKind::GlobalShortcuts(shortcuts));
            sessions.lock().unwrap().insert(key.clone(), data);
            let handle = format!("{}1_1/b", crate::REQUEST_PREFIX);
            let bound = zruntime::spawn({
                let (client, portal, handle) = (client.clone(), portal.clone(), handle.clone());
                async move {
                    let mut talk = HashMap::new();
                    talk.insert("description", Variant::from("Talk"));
                    let none: HashMap<&str, Variant<'_>> = HashMap::new();
                    let handle = ObjectPath::try_from(handle.as_str()).unwrap();
                    let session = ObjectPath::try_from(key.as_str()).unwrap();
                    let body = (handle, session, vec![("talk", talk)], "", none);
                    let reply = crate::tests::call_portal(
                        &client,
                        &portal,
                        "GlobalShortcuts",
                        "BindShortcuts",
                        &body,
                    );
                    let reply = reply.await.unwrap();
                    reply.body().deserialize::<(u32, Results)>().unwrap()
                }
            });
            assert!(gate.reached().await, "the call reads config.toml");
            let closed = crate::tests::close_request(&client, &portal, &handle).await;
            assert!(closed.is_ok(), "the Close is answered: {closed:?}");
            gate.open();
            let (code, results) = bound.await.unwrap();
            let served = crate::tests::request_served(&portal, &handle).await;
            (code, results, served)
        }))
    }

    #[test]
    fn a_close_while_the_config_is_read_answers_cancelled_and_leaves_no_request() {
        let Some((code, results, served)) = bind_closed_during_the_read(config_held_open, &CONFIG)
        else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        assert_eq!(code, RESPONSE_CANCELLED);
        assert!(results.is_empty(), "nothing bound");
        assert!(!served, "no Request object stays behind");
    }

    #[test]
    fn a_close_before_a_bind_that_asks_nothing_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        reprisefs::configfile::set_path(Some(config.clone()));
        let out = bind_closed_during_the_read(bound_config_held_open, &BOUND_CONFIG);
        reprisefs::configfile::set_path(None);
        let Some((code, results, served)) = out else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        assert_eq!(code, RESPONSE_CANCELLED);
        assert!(results.is_empty(), "nothing bound");
        assert!(!config.exists(), "config.toml is not written");
        assert!(!served, "no Request object stays behind");
    }
}
