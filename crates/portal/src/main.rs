//! reprise-portal: xdg-desktop-portal backend (ScreenCast, Screenshot, RemoteDesktop, InputCapture, GlobalShortcuts, Access,
//! FileChooser, AppChooser, Notification, Inhibit, Background, Wallpaper) as `org.freedesktop.impl.portal.desktop.reprise`; dialogs run in a child process (`dialog`).
//! It serves only while Reprise is the session's compositor (see `owner`).

#[cfg(test)]
reprisebus::test_session!();

mod access;
mod appchoose;
mod appchooser;
mod background;
mod cast;
mod choices;
mod dialog;
mod filechooser;
mod filepick;
mod filters;
mod globalshortcuts;
mod inhibit;
mod inputcapture;
mod inputlink;
mod inputproto;
mod lastpick;
mod notification;
mod owner;
mod permit;
mod picker;
mod remotedesktop;
mod screenshot;
mod shortcutconf;
mod stills;
mod trusted;
mod wallpaper;
mod wayland;

use cast::Cast;
use reprisebus::mutex::lock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use wayland::{Target, Wayland};
use zbus::object_server::SignalEmitter;
use zbus::{
    DeserializeDict, ObjectPath, OwnedObjectPath, OwnedVariant, SerializeDict, Type, Variant,
};

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.reprise";
/// xdg-desktop-portal: the one peer that may call this backend.
const FRONTEND: &str = "org.freedesktop.portal.Desktop";
pub(crate) const PATH: &str = "/org/freedesktop/portal/desktop";
const SESSION_PREFIX: &str = "/org/freedesktop/portal/desktop/session/";
const REQUEST_PREFIX: &str = "/org/freedesktop/portal/desktop/request/";

const RESPONSE_OK: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_OTHER: u32 = 2;

const SOURCE_MONITOR: u32 = 1;
const SOURCE_WINDOW: u32 = 2;
const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
const CURSOR_METADATA: u32 = 4;

/// The vendor tag in `restore_data`: ("reprise", 3, "<app id>:<grant>"),
/// the grant being "output:<name>", "toplevel:<identifier>" or
/// "remote-desktop:<devices>[;<output or toplevel>]". A token restores only
/// for the app it was issued to, and only for a grant the user ticked
/// Remember on; tokens of earlier versions restore nothing.
const RESTORE_VENDOR: &str = "reprise";
const RESTORE_VERSION: u32 = 3;

/// Applications whose windows the picker never lists: the portals' own
/// surfaces, including this program's dialog.
const HIDDEN_APPS: [&str; 2] = ["reprise.portal", "xdg-desktop-portal-wlr"];

pub(crate) type Results = HashMap<String, OwnedVariant>;

/// Casts start and stop one at a time: a client that shares again right
/// after closing would otherwise open the new capture session while the old
/// one is still stopping, and a compositor that renders an initial frame
/// only for the first session on a source would leave the new one waiting
/// for damage a static window never produces. Held inside the blocking pool
/// only, never across an await.
static CAST_TURN: Mutex<()> = Mutex::new(());

/// Stop casts off the executor: `Cast::stop` joins the capture
/// thread, which can wait a frame and PipeWire's teardown. Every path
/// that ends a cast comes through here so none of them stalls zbus.
async fn stop_casts(casts: Vec<Cast>) {
    if casts.is_empty() {
        return;
    }
    blocking::unblock(move || {
        let _turn = lock(&CAST_TURN);
        casts.into_iter().for_each(Cast::stop)
    })
    .await;
}

/// The unique bus name a call came from; empty off a bus, which no
/// NameOwnerChanged ever names.
pub(crate) fn sender(hdr: &zbus::message::Header<'_>) -> String {
    hdr.sender().map(|s| s.to_string()).unwrap_or_default()
}

/// The unique name that owns `FRONTEND`, kept current by `watch_frontends`.
#[derive(Clone, Default)]
pub(crate) struct Frontend {
    owner: Arc<Mutex<Option<String>>>,
    conn: Arc<std::sync::OnceLock<zbus::Connection>>,
}

impl Frontend {
    fn set(&self, owner: Option<String>) {
        *lock(&self.owner) = owner;
    }

    /// AccessDenied unless the call came from xdg-desktop-portal: app ids,
    /// restore data and handles are taken on its word only.
    pub(crate) async fn check(&self, hdr: &zbus::message::Header<'_>) -> zbus::fdo::Result<()> {
        let cached = lock(&self.owner).clone();
        let sender = hdr.sender().map(|s| s.as_str());
        let conn = self.conn.get();
        let (owner, verdict) = check_refreshing(sender, cached.clone(), || async {
            match conn {
                Some(c) => reprisebus::name_owner(c, FRONTEND).await,
                None => None,
            }
        })
        .await;
        self.replace(&cached, owner);
        verdict
    }

    /// Take `owner` as the frontend's owner unless the cache moved off `cached` meanwhile: a
    /// NameOwnerChanged that `watch_frontends` stored is newer than the owner read before it.
    fn replace(&self, cached: &Option<String>, owner: Option<String>) {
        let mut held = lock(&self.owner);
        if *held == *cached {
            *held = owner;
        }
    }
}

/// `check_frontend` against `cached`, then once more against the owner `refresh` reads off the
/// bus: the frontend's NameOwnerChanged is not ordered against its first call. Returns the
/// owner the verdict used.
async fn check_refreshing<F: std::future::Future<Output = Option<String>>>(
    sender: Option<&str>,
    cached: Option<String>,
    refresh: impl FnOnce() -> F,
) -> (Option<String>, zbus::fdo::Result<()>) {
    if check_frontend(sender, cached.as_deref()).is_ok() {
        return (cached, Ok(()));
    }
    let owner = refresh().await;
    let verdict = check_frontend(sender, owner.as_deref());
    (owner, verdict)
}

/// AccessDenied unless the call came from `owner`.
pub(crate) fn check_owner(
    hdr: &zbus::message::Header<'_>,
    owner: Option<&str>,
) -> zbus::fdo::Result<()> {
    check_frontend(hdr.sender().map(|s| s.as_str()), owner)
}

fn check_frontend(sender: Option<&str>, owner: Option<&str>) -> zbus::fdo::Result<()> {
    if reprisebus::sent_by(sender, owner) {
        Ok(())
    } else {
        Err(zbus::fdo::Error::AccessDenied(format!(
            "{} may not call {BUS_NAME}",
            sender.unwrap_or("a peer without a name")
        )))
    }
}

/// End a session already taken out of the map: cancel its dialog, stop
/// its cast, signal `Closed` and take its object off the bus. The EIS
/// client and the capture object go with `data`.
async fn end_session(conn: &zbus::Connection, key: &str, data: Option<SessionData>) {
    if let Some(mut s) = data {
        if let Some(c) = s.picking.take() {
            c.store(true, Ordering::SeqCst);
        }
        stop_casts(s.take_casts()).await;
    }
    if let Ok(emitter) = SignalEmitter::new(conn, key) {
        let _ = Session::closed(&emitter).await;
    }
    let _ = conn.object_server().remove::<Session, _>(key).await;
}

fn take_owned_by(
    sessions: &mut HashMap<String, SessionData>,
    owner: &str,
) -> Vec<(String, SessionData)> {
    sessions.extract_if(|_, s| s.owner == owner).collect()
}

async fn owner_changes(conn: &zbus::Connection) -> zbus::Result<zbus::fdo::NameOwnerChangedStream> {
    zbus::fdo::DBusProxy::new(conn)
        .await?
        .receive_name_owner_changed()
        .await
}

/// Follow `FRONTEND` to its owner, and end every session and inhibition of
/// a frontend that left the bus (xdg-desktop-portal restarted or crashed),
/// since nothing else would ever close them.
async fn watch_frontends(
    conn: zbus::Connection,
    mut changes: zbus::fdo::NameOwnerChangedStream,
    frontend: Frontend,
    sessions: Sessions,
    inhibits: inhibit::Shared,
) {
    use futures_lite::StreamExt;
    while let Some(sig) = changes.next().await {
        let Ok(args) = sig.args() else { continue };
        if args.name() == FRONTEND {
            frontend.set(args.new_owner().as_ref().map(|o| o.to_string()));
        }
        if args.new_owner().is_some() {
            continue;
        }
        let gone = take_owned_by(&mut sessions.lock().unwrap(), args.name());
        for (key, data) in gone {
            log::info!("Session {key}: {} left the bus", args.name());
            end_session(&conn, &key, Some(data)).await;
        }
        inhibit::release_owned_by(&conn, &inhibits, args.name()).await;
    }
}

/// Err unless `path` names an object below `prefix`: a handle anywhere
/// else (`PATH`, say) would put our object over one we serve.
pub(crate) fn check_handle(path: &ObjectPath<'_>, prefix: &str) -> zbus::fdo::Result<()> {
    match path.as_str().strip_prefix(prefix) {
        Some(rest) if !rest.is_empty() => Ok(()),
        _ => Err(zbus::fdo::Error::InvalidArgs(format!(
            "{path} is not under {prefix}"
        ))),
    }
}

pub(crate) fn check_session(session_handle: &ObjectPath<'_>) -> zbus::fdo::Result<()> {
    check_handle(session_handle, SESSION_PREFIX)
}

/// Serve the `Request` object at `handle` while a dialog is up, so the
/// frontend can cancel it through `cancel`. Remove it with
/// `server.remove::<Request, _>` once the dialog is gone.
pub(crate) async fn export_request(
    server: &zbus::ObjectServer,
    handle: &ObjectPath<'_>,
    cancel: Arc<AtomicBool>,
    frontend: &Frontend,
) -> zbus::fdo::Result<()> {
    check_handle(handle, REQUEST_PREFIX)?;
    let request = Request {
        cancel,
        path: handle.to_string(),
        frontend: frontend.clone(),
    };
    server.at(handle.clone(), request).await?;
    Ok(())
}

async fn replace_session(sessions: &Sessions, key: String, data: SessionData) {
    let old = sessions.lock().unwrap().insert(key, data);
    if let Some(mut s) = old {
        stop_casts(s.take_casts()).await;
    }
}

#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct SelectSourcesOptions {
    types: Option<u32>,
    #[allow(dead_code)]
    multiple: Option<bool>,
    cursor_mode: Option<u32>,
    restore_data: Option<(String, u32, OwnedVariant)>,
    persist_mode: Option<u32>,
}

#[derive(SerializeDict, Type, Clone)]
#[zvariant(signature = "a{sv}")]
pub(crate) struct StreamProps {
    position: Option<(i32, i32)>,
    size: (i32, i32),
    source_type: u32,
    mapping_id: Option<String>,
    #[zvariant(rename = "pipewire-serial")]
    pipewire_serial: u64,
}

#[derive(SerializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct StartResults {
    streams: Vec<(u32, StreamProps)>,
    persist_mode: u32,
    restore_data: Option<(String, u32, OwnedVariant)>,
}

struct SessionData {
    /// The unique bus name of the frontend that created it: the session
    /// ends when that name leaves the bus.
    owner: String,
    /// Set while Start or BindShortcuts shows a dialog; Close flips it so
    /// the dialog goes away with the session.
    picking: Option<Arc<AtomicBool>>,
    kind: SessionKind,
}

/// The ScreenCast side of a session: what SelectSources chose and the
/// casts Start made from it, one per stream. On a RemoteDesktop session `types` stays 0
/// until ScreenCast.SelectSources runs on it.
#[derive(Default)]
struct CastState {
    types: u32,
    cursor_mode: u32,
    persist_mode: u32,
    restore: Option<Target>,
    casts: Vec<Cast>,
}

enum SessionKind {
    ScreenCast(CastState),
    RemoteDesktop(remotedesktop::RemoteSession),
    InputCapture(inputcapture::CaptureSession),
    GlobalShortcuts(globalshortcuts::ShortcutsSession),
}

impl SessionData {
    fn new(owner: String, kind: SessionKind) -> Self {
        SessionData {
            owner,
            picking: None,
            kind,
        }
    }

    fn screencast(&self) -> Option<&CastState> {
        match &self.kind {
            SessionKind::ScreenCast(c) => Some(c),
            SessionKind::RemoteDesktop(r) => Some(&r.screencast),
            SessionKind::InputCapture(_) | SessionKind::GlobalShortcuts(_) => None,
        }
    }

    fn screencast_mut(&mut self) -> Option<&mut CastState> {
        match &mut self.kind {
            SessionKind::ScreenCast(c) => Some(c),
            SessionKind::RemoteDesktop(r) => Some(&mut r.screencast),
            SessionKind::InputCapture(_) | SessionKind::GlobalShortcuts(_) => None,
        }
    }

    fn remote_mut(&mut self) -> Option<&mut remotedesktop::RemoteSession> {
        match &mut self.kind {
            SessionKind::RemoteDesktop(r) => Some(r),
            _ => None,
        }
    }

    fn capture(&self) -> Option<&inputcapture::CaptureSession> {
        match &self.kind {
            SessionKind::InputCapture(c) => Some(c),
            _ => None,
        }
    }

    fn capture_mut(&mut self) -> Option<&mut inputcapture::CaptureSession> {
        match &mut self.kind {
            SessionKind::InputCapture(c) => Some(c),
            _ => None,
        }
    }

    fn take_casts(&mut self) -> Vec<Cast> {
        (self.screencast_mut()).map_or_else(Vec::new, |s| std::mem::take(&mut s.casts))
    }
}

type Sessions = Arc<Mutex<HashMap<String, SessionData>>>;

struct ScreenCast {
    sessions: Sessions,
    source_types: u32,
    frontend: Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        self.source_types
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED | CURSOR_METADATA
    }

    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: HashMap<String, Variant<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, Results)> {
        self.frontend.check(&hdr).await?;
        check_session(&session_handle)?;
        log::info!("CreateSession {} for {app_id:?}", session_handle.as_str());
        let key = session_handle.to_string();
        replace_session(
            &self.sessions,
            key.clone(),
            SessionData::new(
                sender(&hdr),
                SessionKind::ScreenCast(CastState {
                    types: SOURCE_MONITOR,
                    cursor_mode: CURSOR_HIDDEN,
                    ..Default::default()
                }),
            ),
        )
        .await;
        server
            .at(
                session_handle.clone(),
                Session {
                    path: session_handle.clone().into(),
                    sessions: self.sessions.clone(),
                },
            )
            .await?;
        Ok((RESPONSE_OK, Results::new()))
    }

    async fn select_sources(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        options: SelectSourcesOptions,
    ) -> zbus::fdo::Result<(u32, Results)> {
        self.frontend.check(&hdr).await?;
        let mut sessions = self.sessions.lock().unwrap();
        let Some(s) = sessions
            .get_mut(session_handle.as_str())
            .and_then(SessionData::screencast_mut)
        else {
            return Ok((RESPONSE_OTHER, Results::new()));
        };
        s.types = options.types.unwrap_or(SOURCE_MONITOR) & self.source_types;
        s.cursor_mode = options.cursor_mode.unwrap_or(CURSOR_HIDDEN);
        s.persist_mode = options.persist_mode.unwrap_or(0);
        s.restore = (options.restore_data.as_ref()).and_then(|d| restore_target(d, &app_id));
        log::info!(
            "SelectSources types={} cursor={} persist={} restore={:?}",
            s.types,
            s.cursor_mode,
            s.persist_mode,
            s.restore
        );
        if s.types == 0 {
            return Ok((RESPONSE_OTHER, Results::new()));
        }
        Ok((RESPONSE_OK, Results::new()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn start(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        _options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u32, StartResults)> {
        self.frontend.check(&hdr).await?;
        let key = session_handle.to_string();
        let parent = dialog::parent_handle(&parent_window);
        match pick_and_cast(
            &self.sessions,
            &self.frontend,
            server,
            conn,
            &handle,
            &key,
            &app_id,
            parent,
            Pick::Ask,
        )
        .await
        {
            Ok(st) => Ok((
                RESPONSE_OK,
                StartResults {
                    streams: st.streams,
                    persist_mode: st.persist_mode,
                    restore_data: st.restore_data,
                },
            )),
            Err(code) => Ok((code, empty_start())),
        }
    }
}

/// The persist mode Start grants: what the app asked for (at most 2,
/// until revoked) when the user ticked Remember, else 0.
pub(crate) fn granted_persist(requested: u32, remember: bool) -> u32 {
    if remember { requested.min(2) } else { 0 }
}

pub(crate) struct Started {
    pub(crate) streams: Vec<(u32, StreamProps)>,
    /// The target the restore token names; None unless persisted.
    pub(crate) remembered: Option<Target>,
    pub(crate) persist_mode: u32,
    pub(crate) restore_data: Option<(String, u32, OwnedVariant)>,
}

/// How Start picks what to capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// A remembered target, else the share dialog: one stream.
    Ask,
    /// The remote desktop server's pick, with no dialog and not remembered: [`every_screen`], a
    /// stream each. The compositor tells its capture apart (`Wayland::connect_remote_capture`).
    All,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn pick_and_cast(
    sessions: &Sessions,
    frontend: &Frontend,
    server: &zbus::ObjectServer,
    conn: &zbus::Connection,
    handle: &ObjectPath<'_>,
    key: &str,
    app_id: &str,
    parent: Option<String>,
    pick: Pick,
) -> Result<Started, u32> {
    let (types, cursor_mode, persist_mode, restore) = {
        let sessions = sessions.lock().unwrap();
        let Some(s) = sessions.get(key).and_then(SessionData::screencast) else {
            return Err(RESPONSE_OTHER);
        };
        (s.types, s.cursor_mode, s.persist_mode, s.restore.clone())
    };
    let cancel = Arc::new(AtomicBool::new(false));
    if let Err(e) = export_request(server, handle, cancel.clone(), frontend).await {
        log::warn!("Start: {e}");
        return Err(RESPONSE_OTHER);
    }
    if let Some(s) = sessions.lock().unwrap().get_mut(key) {
        s.picking = Some(cancel.clone());
    }
    let app = app_id.to_string();
    let chosen = blocking::unblock(move || {
        choose(types, restore, &app, persist_mode > 0, parent, cancel, pick)
    })
    .await;
    let _ = server.remove::<Request, _>(handle.clone()).await;
    if let Some(s) = sessions.lock().unwrap().get_mut(key) {
        s.picking = None;
    }
    let chosen = match chosen {
        Ok(Some(c)) => c,
        Ok(None) => return Err(RESPONSE_CANCELLED),
        Err(e) => {
            log::warn!("Start: {e}");
            return Err(RESPONSE_OTHER);
        }
    };
    let targets: Vec<_> = chosen.targets.iter().map(|(t, _)| t.clone()).collect();
    let remote = pick == Pick::All;
    let mut casts = match blocking::unblock(move || {
        let _turn = lock(&CAST_TURN);
        start_each(
            &targets,
            |t| Cast::start(t.clone(), cursor_mode_for(cursor_mode, t), remote),
            Cast::stop,
        )
    })
    .await
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Start: {e}");
            return Err(RESPONSE_OTHER);
        }
    };
    let streams = (casts.iter().zip(&chosen.targets))
        .map(|(c, (t, info))| {
            log::info!("Start: node {} for {t:?}", c.node_id);
            stream_for(c.node_id, c.serial, t, info.as_ref(), (c.width, c.height))
        })
        .collect();
    let persist_mode = granted_persist(persist_mode, chosen.remembered.is_some());
    let remembered = chosen.remembered.filter(|_| persist_mode > 0);
    let restore_data = (remembered.as_ref()).map(|t| restore_data(app_id, &lastpick::encode(t)));
    let ended: Vec<_> = (casts.iter_mut())
        .filter_map(|c| Some((c.node_id, c.take_ended()?)))
        .collect();
    // A second Start hands back the session's earlier casts, and a Close
    // during this one leaves the new casts unkept. Either stops off the
    // lock, since `Cast::stop` joins its thread.
    let (replaced, unkept) = match sessions
        .lock()
        .unwrap()
        .get_mut(key)
        .and_then(SessionData::screencast_mut)
    {
        Some(s) => (std::mem::replace(&mut s.casts, casts), None),
        None => (Vec::new(), Some(casts)),
    };
    stop_casts(replaced).await;
    if let Some(casts) = unkept {
        stop_casts(casts).await;
        return Err(RESPONSE_CANCELLED);
    }
    // The compositor can end a cast on its own (the window closed,
    // the output went); the frontend hears of it as a Closed session.
    for (node_id, ended) in ended {
        zruntime::spawn(cast_ended(
            conn.clone(),
            sessions.clone(),
            key.to_string(),
            node_id,
            ended,
        ))
        .detach();
    }
    Ok(Started {
        streams,
        remembered,
        persist_mode,
        restore_data,
    })
}

/// Start a cast for each target in turn, or none: when one fails, the ones started before it stop.
fn start_each<C>(
    targets: &[Target],
    mut start: impl FnMut(&Target) -> Result<C, String>,
    stop: impl FnMut(C),
) -> Result<Vec<C>, String> {
    let mut started = Vec::with_capacity(targets.len());
    for t in targets {
        match start(t) {
            Ok(c) => started.push(c),
            Err(e) => {
                started.into_iter().for_each(stop);
                return Err(e);
            }
        }
    }
    Ok(started)
}

/// The Start result for cast `node` of `target`: an output's logical position and size, else the
/// cast's own size in pixels.
fn stream_for(
    node: u32,
    serial: u64,
    target: &Target,
    output: Option<&wayland::OutputInfo>,
    cast_size: (u32, u32),
) -> (u32, StreamProps) {
    let pixels = (cast_size.0 as i32, cast_size.1 as i32);
    let (source_type, position, mapping_id, size) = match (target, output) {
        (Target::Output(name), Some(o)) => (
            SOURCE_MONITOR,
            Some((o.x, o.y)),
            Some(name.clone()),
            (o.width, o.height),
        ),
        (Target::Output(name), None) => (SOURCE_MONITOR, None, Some(name.clone()), pixels),
        (Target::Toplevel(_), _) => (SOURCE_WINDOW, None, None, pixels),
    };
    (
        node,
        StreamProps {
            position,
            size,
            source_type,
            mapping_id,
            pipewire_serial: serial,
        },
    )
}

/// Wait for cast `node_id` under `key` to end and, if the session still holds
/// it (the compositor stopped it, not a Close from the frontend),
/// close the session the way `Session::close` would: signal `Closed`
/// and take the object off the bus. A Close from the frontend removes
/// the session first, so the thread's end then finds nothing to do.
async fn cast_ended(
    conn: zbus::Connection,
    sessions: Sessions,
    key: String,
    node_id: u32,
    ended: std::sync::mpsc::Receiver<()>,
) {
    blocking::unblock(move || {
        let _ = ended.recv();
    })
    .await;
    let gone = {
        let mut map = sessions.lock().unwrap();
        let ours = (map.get(&key).and_then(SessionData::screencast))
            .is_some_and(|s| s.casts.iter().any(|c| c.node_id == node_id));
        ours.then(|| map.remove(&key)).flatten()
    };
    let Some(data) = gone else {
        return;
    };
    log::info!("Session {key}: the compositor stopped the cast");
    end_session(&conn, &key, Some(data)).await;
}

/// The cursor mode to capture with: metadata only for an output (a
/// window has no cursor session; it falls back to hidden, which is what
/// the mode shows without a cursor session).
fn cursor_mode_for(requested: u32, target: &Target) -> cast::CursorMode {
    if requested & CURSOR_METADATA != 0 && matches!(target, Target::Output(_)) {
        cast::CursorMode::Metadata
    } else if requested & CURSOR_EMBEDDED != 0 {
        cast::CursorMode::Embedded
    } else {
        cast::CursorMode::Hidden
    }
}

fn empty_start() -> StartResults {
    StartResults {
        streams: Vec::new(),
        persist_mode: 0,
        restore_data: None,
    }
}

/// The output or window a restore token names, for `app_id` only.
fn restore_target(data: &(String, u32, OwnedVariant), app_id: &str) -> Option<Target> {
    let grant = restored_grant(data, app_id)?;
    match remotedesktop::parse_grant(grant) {
        Some((_, target)) => target,
        None => lastpick::decode(grant),
    }
}

pub(crate) fn restore_data(app_id: &str, grant: &str) -> (String, u32, OwnedVariant) {
    (
        RESTORE_VENDOR.to_string(),
        RESTORE_VERSION,
        Variant::from(format!("{app_id}:{grant}"))
            .try_to_owned()
            .expect("string"),
    )
}

/// The grant in one of our current tokens issued to `app_id`. An app id
/// holds no `:`, so the first one ends it.
pub(crate) fn restored_grant<'a>(
    data: &'a (String, u32, OwnedVariant),
    app_id: &str,
) -> Option<&'a str> {
    if data.0 != RESTORE_VENDOR || data.1 != RESTORE_VERSION {
        return None;
    }
    let (app, grant) = as_str(&data.2)?.split_once(':')?;
    (app == app_id).then_some(grant)
}

fn as_str<'a>(v: &'a Variant<'a>) -> Option<&'a str> {
    match v {
        Variant::Str(s) => Some(s.as_str()),
        Variant::Variant(inner) => as_str(inner),
        _ => None,
    }
}

/// What Start captures: the targets in stream order with each output's info, and the target
/// Remember was ticked for.
struct Chosen {
    targets: Vec<(Target, Option<wayland::OutputInfo>)>,
    remembered: Option<Target>,
}

/// Pick what to capture ([`choose_from`]); the share dialog is modal on `parent` and offers
/// Remember when `offer_remember`. Blocking; runs off the bus thread.
fn choose(
    types: u32,
    restore: Option<Target>,
    app_id: &str,
    offer_remember: bool,
    parent: Option<String>,
    cancel: Arc<AtomicBool>,
    pick: Pick,
) -> Result<Option<Chosen>, String> {
    let wl = Wayland::connect()?;
    let outputs = wl.outputs();
    let toplevels = wl.toplevels();
    drop(wl);
    choose_from(pick, types, &outputs, &toplevels, restore, |entries| {
        let last = lastpick::load(app_id);
        let preselect = last.and_then(|t| entries.iter().position(|e| e.target == t));
        let (t, remember) = picker::pick(
            picker::Purpose::Share,
            &crate::remotedesktop::display_name(app_id),
            entries,
            preselect,
            offer_remember,
            parent,
            cancel,
        )?;
        lastpick::save(app_id, &t);
        Some((t, remember))
    })
}

/// For [`Pick::All`] [`every_screen`]; for [`Pick::Ask`] a remembered target that still exists
/// (remembered again), else what `dialog` picks from the share dialog's rows. None when the
/// dialog is cancelled.
fn choose_from(
    pick: Pick,
    types: u32,
    outputs: &[wayland::OutputInfo],
    toplevels: &[wayland::ToplevelInfo],
    restore: Option<Target>,
    dialog: impl FnOnce(Vec<picker::Entry>) -> Option<(Target, bool)>,
) -> Result<Option<Chosen>, String> {
    if pick == Pick::All {
        let screens = every_screen(outputs);
        if screens.is_empty() {
            return Err("no output to share".into());
        }
        let targets = (screens.into_iter())
            .map(|o| (Target::Output(o.name.clone()), Some(o)))
            .collect();
        return Ok(Some(Chosen {
            targets,
            remembered: None,
        }));
    }
    let info_for = |t: &Target| match t {
        Target::Output(n) => outputs.iter().find(|o| &o.name == n).cloned(),
        Target::Toplevel(_) => None,
    };
    if let Some(t) = restore {
        let exists = match &t {
            Target::Output(n) => outputs.iter().any(|o| &o.name == n),
            Target::Toplevel(i) => toplevels.iter().any(|w| &w.identifier == i),
        };
        if exists {
            return Ok(Some(Chosen {
                targets: vec![(t.clone(), info_for(&t))],
                remembered: Some(t),
            }));
        }
    }
    let entries = share_entries(types, outputs, toplevels);
    if entries.is_empty() {
        return Err("nothing to share".into());
    }
    let Some((t, remember)) = dialog(entries) else {
        return Ok(None);
    };
    Ok(Some(Chosen {
        targets: vec![(t.clone(), info_for(&t))],
        remembered: remember.then_some(t),
    }))
}

/// The outputs a trusted session shares, in screen order: the primary ([`origin_output`]) first,
/// then top to bottom and left to right. An output at a negative position is left out, since the
/// compositor's EIS gives it no region, unless that leaves none.
fn every_screen(outputs: &[wayland::OutputInfo]) -> Vec<wayland::OutputInfo> {
    let placed: Vec<_> = (outputs.iter())
        .filter(|o| o.x >= 0 && o.y >= 0)
        .cloned()
        .collect();
    let mut shared = if placed.is_empty() {
        outputs.to_vec()
    } else {
        placed
    };
    let primary = origin_output(&shared).map(|o| o.name.clone());
    shared.sort_by_key(|o| (Some(&o.name) != primary.as_ref(), o.y, o.x));
    shared
}

/// The output whose top-left is the layout origin, else the top-left-most: the topmost, and of
/// those the leftmost.
fn origin_output(outputs: &[wayland::OutputInfo]) -> Option<&wayland::OutputInfo> {
    (outputs.iter().find(|o| (o.x, o.y) == (0, 0)))
        .or_else(|| outputs.iter().min_by_key(|o| (o.y, o.x)))
}

/// The share dialog's rows: the outputs and the windows `types` asks for.
pub(crate) fn share_entries(
    types: u32,
    outputs: &[wayland::OutputInfo],
    toplevels: &[wayland::ToplevelInfo],
) -> Vec<picker::Entry> {
    let mut entries = Vec::new();
    if types & SOURCE_MONITOR != 0 {
        for o in outputs {
            entries.push(picker::Entry {
                label: if o.description.is_empty() {
                    format!("Screen {} ({}x{})", o.name, o.width, o.height)
                } else {
                    format!("{} ({}x{})", o.description, o.width, o.height)
                },
                icon: picker::SCREEN_ICON.into(),
                screen: true,
                target: Target::Output(o.name.clone()),
            });
        }
    }
    if types & SOURCE_WINDOW != 0 {
        let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
        for w in toplevels {
            // A toplevel with no title is a hidden helper surface (the
            // gtk portal keeps one), and our own dialogs are not on offer.
            if w.identifier.is_empty()
                || w.title.is_empty()
                || HIDDEN_APPS.contains(&w.app_id.as_str())
            {
                continue;
            }
            entries.push(picker::Entry {
                label: w.title.clone(),
                icon: reprisefs::apps::icon_for_app_id(&apps, &w.app_id)
                    .unwrap_or("")
                    .to_string(),
                screen: false,
                target: Target::Toplevel(w.identifier.clone()),
            });
        }
    }
    entries
}

struct Request {
    cancel: Arc<AtomicBool>,
    path: String,
    frontend: Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    async fn close(&self, #[zbus(header)] hdr: zbus::message::Header<'_>) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        log::info!("Request {} closed", self.path);
        self.cancel.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct Session {
    path: OwnedObjectPath,
    sessions: Sessions,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let data = {
            let mut sessions = self.sessions.lock().unwrap();
            let owner = sessions.get(self.path.as_str()).map(|s| s.owner.as_str());
            check_owner(&hdr, owner)?;
            sessions.remove(self.path.as_str())
        };
        log::info!("Session {} closed", self.path.as_str());
        end_session(conn, self.path.as_str(), data).await;
        Ok(())
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

fn probe_dmabuf(mut wl: Wayland) -> Result<(), String> {
    let out = wl.outputs().into_iter().next().ok_or("no output")?;
    let session = wl.open_session(&Target::Output(out.name.clone()), false)?;
    let c = wl.constraints();
    println!(
        "session {}x{} shm {:?} dmabuf device {:?}",
        c.width, c.height, c.shm_formats, c.dmabuf_device
    );
    for f in &c.dmabuf_formats {
        let mods: Vec<String> = f.modifiers.iter().map(|m| format!("{m:#x}")).collect();
        println!(
            "dmabuf format {:#x} modifiers [{}]",
            f.fourcc,
            mods.join(", ")
        );
    }
    let gbm = wl.open_gbm()?;
    println!("gbm device {}", gbm.path.display());
    let f = c
        .dmabuf_formats
        .iter()
        .find(|f| wayland::shm_format_of_fourcc(f.fourcc).is_some())
        .ok_or("no dmabuf format the portal streams")?;
    let buf = wl.dmabuf_buffer(&gbm, c.width, c.height, f.fourcc, &f.modifiers)?;
    println!(
        "allocated {:#x} modifier {:#x}, {} plane(s), stride {}",
        buf.fourcc,
        buf.modifier,
        buf.planes.len(),
        buf.planes[0].stride
    );
    wl.capture(&session, &buf, std::time::Duration::from_secs(2))
        .map_err(|e| format!("capture failed: {e:?}"))?;
    let px = buf.read()?;
    let mut distinct = std::collections::HashSet::new();
    for p in px.as_chunks::<4>().0 {
        distinct.insert(u32::from_ne_bytes(*p));
        if distinct.len() > 64 {
            break;
        }
    }
    println!(
        "captured {} bytes, {} distinct pixel values{}",
        px.len(),
        distinct.len(),
        if distinct.len() > 64 { "+" } else { "" }
    );
    Ok(())
}

/// Serve every interface on a new session-bus connection; `BUS_NAME` is
/// requested once `frontend` is known.
async fn serve(
    sessions: Sessions,
    source_types: u32,
    link: Option<inputlink::Link>,
    notes: notification::Shared,
    inhibits: inhibit::Shared,
    frontend: &Frontend,
) -> zbus::Result<zbus::Connection> {
    let frontend = || frontend.clone();
    zbus::connection::Builder::session()
        .serve_at(
            PATH,
            ScreenCast {
                sessions: sessions.clone(),
                source_types,
                frontend: frontend(),
            },
        )
        .serve_at(
            PATH,
            screenshot::Screenshot {
                frontend: frontend(),
                link: link.clone(),
            },
        )
        .serve_at(
            PATH,
            remotedesktop::RemoteDesktop {
                sessions: sessions.clone(),
                link: link.clone(),
                frontend: frontend(),
            },
        )
        .serve_at(
            PATH,
            globalshortcuts::GlobalShortcuts {
                sessions: sessions.clone(),
                link: link.clone(),
                frontend: frontend(),
            },
        )
        .serve_at(
            PATH,
            inputcapture::InputCapture {
                sessions,
                link: link.clone(),
                frontend: frontend(),
            },
        )
        .serve_at(
            PATH,
            access::Access {
                frontend: frontend(),
            },
        )
        .serve_at(
            PATH,
            filechooser::FileChooser {
                frontend: frontend(),
            },
        )
        .serve_at(PATH, appchooser::AppChooser::new(frontend()))
        .serve_at(
            PATH,
            notification::Notification {
                book: notes,
                frontend: frontend(),
            },
        )
        .serve_at(
            PATH,
            inhibit::Inhibit {
                shared: inhibits.clone(),
                frontend: frontend(),
            },
        )
        .serve_at(PATH, inhibit::SessionEnd { shared: inhibits })
        .serve_at(
            PATH,
            background::Background {
                frontend: frontend(),
                link,
            },
        )
        .serve_at(
            PATH,
            wallpaper::Wallpaper {
                frontend: frontend(),
            },
        )
        .build()
        .await
}

fn main() {
    // zbus logs every call it serves at info, with the whole message; RUST_LOG
    // still turns it back on.
    reprisebus::journal::init_logger("info,zbus=warn");
    // A panic in a handler can leave a process that holds the bus name and answers nothing;
    // exiting lets the unit restart.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        log::error!("{info}");
        std::process::exit(101);
    }));
    if std::env::args().any(|a| a == "--pick") {
        picker::run_dialog();
        return;
    }
    if std::env::args().any(|a| a == "--permit") {
        permit::run_dialog();
        return;
    }
    if std::env::args().any(|a| a == "--access") {
        access::run_dialog();
        return;
    }
    if std::env::args().any(|a| a == "--file") {
        filepick::run_dialog();
        return;
    }
    if std::env::args().any(|a| a == "--appchoose") {
        appchoose::run_dialog();
        return;
    }
    if std::env::args().any(|a| a == "--probe") {
        match Wayland::connect() {
            Ok(wl) => {
                println!(
                    "outputs: {}  toplevels: {}",
                    wl.can_capture_outputs(),
                    wl.can_capture_toplevels()
                );
                for o in wl.outputs() {
                    println!(
                        "output {:?} {:?} {}x{} at {},{}",
                        o.name, o.description, o.width, o.height, o.x, o.y
                    );
                }
                for t in wl.toplevels() {
                    println!("toplevel {:?} {:?} {:?}", t.identifier, t.app_id, t.title);
                }
                if let Err(e) = probe_dmabuf(wl) {
                    println!("dmabuf: {e}");
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    // Serve only a session whose compositor is Reprise, and only as long
    // as it runs (see `owner`). Refusing is no failure of the unit: it
    // exits cleanly without taking the bus name.
    let compositor = match owner::connect() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("{e}; not serving outside a Reprise session");
            owner::release_frontend();
            return;
        }
    };
    let source_types = match Wayland::connect() {
        Ok(wl) => {
            let mut t = 0;
            if wl.can_capture_outputs() {
                t |= SOURCE_MONITOR;
            }
            if wl.can_capture_toplevels() {
                t |= SOURCE_WINDOW;
            }
            t
        }
        Err(e) => {
            log::error!("{e}");
            std::process::exit(1);
        }
    };
    if source_types == 0 {
        log::error!("the compositor offers no ext-image-copy-capture sources");
        std::process::exit(1);
    }
    log::info!("serving {BUS_NAME}: source types {source_types}");
    // The compositor's input link backs RemoteDesktop, InputCapture,
    // GlobalShortcuts, Background and PickColor; without it those interfaces answer
    // but grant nothing.
    let link = match inputlink::Link::start() {
        Ok(l) => Some(l),
        Err(e) => {
            log::warn!(
                "input link: {e}; RemoteDesktop, InputCapture, GlobalShortcuts, Background and PickColor disabled"
            );
            None
        }
    };
    let background_link = link.clone();
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    zbus::block_on(async move {
        let notes = notification::Shared::default();
        let (inhibits, idle) = inhibit::shared();
        let frontend = Frontend::default();
        let fail = |e: zbus::Error| -> ! {
            log::error!("could not serve {BUS_NAME}: {e}");
            std::process::exit(1);
        };
        let conn = serve(
            sessions.clone(),
            source_types,
            link,
            notes.clone(),
            inhibits.clone(),
            &frontend,
        )
        .await
        .unwrap_or_else(|e| fail(e));
        let changes = owner_changes(&conn).await.unwrap_or_else(|e| fail(e));
        let _ = frontend.conn.set(conn.clone());
        frontend.set(reprisebus::name_owner(&conn, FRONTEND).await);
        conn.request_name(BUS_NAME)
            .await
            .unwrap_or_else(|e| fail(e));
        zruntime::spawn(notification::watch(conn.clone(), notes)).detach();
        zruntime::spawn(inhibit::run_idle(conn.clone(), idle)).detach();
        zruntime::spawn(inhibit::watch_lock(conn.clone(), inhibits.clone())).detach();
        if let Some(link) = background_link {
            zruntime::spawn(background::announce(conn.clone(), link)).detach();
        }
        owner::exit_with(compositor);
        watch_frontends(conn, changes, frontend, sessions, inhibits).await;
        std::future::pending::<()>().await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(name: &str, x: i32, y: i32) -> wayland::OutputInfo {
        wayland::OutputInfo {
            name: name.into(),
            x,
            y,
            width: 1920,
            height: 1080,
            ..Default::default()
        }
    }

    fn no_dialog(_: Vec<picker::Entry>) -> Option<(Target, bool)> {
        panic!("a trusted start shows no dialog")
    }

    fn shared(outputs: &[wayland::OutputInfo]) -> Vec<String> {
        let chosen = choose_from(Pick::All, SOURCE_MONITOR, outputs, &[], None, no_dialog)
            .expect("an output to share")
            .expect("not cancelled");
        let names = chosen.targets.iter().map(|(t, _)| match t {
            Target::Output(n) => n.clone(),
            Target::Toplevel(i) => panic!("a trusted start shares no window, got {i}"),
        });
        names.collect()
    }

    #[test]
    fn a_trusted_start_shares_every_output_in_screen_order() {
        let outputs = [at("DP-2", 1920, 0), at("DP-1", 0, 0)];
        let chosen = choose_from(Pick::All, SOURCE_MONITOR, &outputs, &[], None, no_dialog)
            .unwrap()
            .unwrap();
        assert_eq!(chosen.remembered, None, "a trusted start remembers nothing");
        let streams: Vec<_> = (chosen.targets.iter().zip(1..))
            .map(|((t, info), node)| stream_for(node, u64::from(node), t, info.as_ref(), (2, 2)))
            .collect();
        let nodes: Vec<_> = streams.iter().map(|s| s.0).collect();
        assert_eq!(nodes, [1, 2]);
        let positions: Vec<_> = streams.iter().map(|s| s.1.position).collect();
        assert_eq!(positions, [Some((0, 0)), Some((1920, 0))]);
        let ids: Vec<_> = streams.iter().map(|s| s.1.mapping_id.clone()).collect();
        assert_eq!(ids, [Some("DP-1".into()), Some("DP-2".into())]);
        for (_, s) in &streams {
            assert_eq!(s.size, (1920, 1080), "the logical size");
            assert_eq!(s.source_type, SOURCE_MONITOR);
        }
        let staggered = [at("DP-3", 0, 1080), at("DP-2", 1920, 0), at("DP-1", 0, 0)];
        assert_eq!(
            shared(&staggered),
            ["DP-1", "DP-2", "DP-3"],
            "top, then left"
        );
    }

    #[test]
    fn an_untrusted_start_still_shares_one_stream() {
        let outputs = [at("DP-1", 0, 0), at("DP-2", 1920, 0)];
        let mut offered = Vec::new();
        let chosen = choose_from(Pick::Ask, SOURCE_MONITOR, &outputs, &[], None, |entries| {
            offered = entries.into_iter().map(|e| e.target).collect();
            Some((Target::Output("DP-2".into()), false))
        })
        .unwrap()
        .unwrap();
        let both = [Target::Output("DP-1".into()), Target::Output("DP-2".into())];
        assert_eq!(offered, both, "the dialog offers every screen");
        assert_eq!(chosen.targets.len(), 1);
        assert_eq!(chosen.targets[0].0, Target::Output("DP-2".into()));
        assert_eq!(
            chosen.targets[0].1.as_ref().map(|o| (o.x, o.y)),
            Some((1920, 0))
        );
        assert_eq!(chosen.remembered, None, "Remember was not ticked");

        let restore = Some(Target::Output("DP-1".into()));
        let chosen = choose_from(Pick::Ask, SOURCE_MONITOR, &outputs, &[], restore, no_dialog)
            .unwrap()
            .unwrap();
        assert_eq!(
            chosen.targets.len(),
            1,
            "a restored grant is one stream too"
        );
        assert_eq!(chosen.remembered, Some(Target::Output("DP-1".into())));
    }

    #[test]
    fn outputs_at_negative_positions_are_left_out() {
        let outputs = [
            at("DP-3", -1920, 0),
            at("DP-2", 1920, 0),
            at("DP-4", 0, -1080),
            at("DP-1", 0, 0),
        ];
        assert_eq!(shared(&outputs), ["DP-1", "DP-2"]);
    }

    #[test]
    fn every_output_is_shared_when_none_lies_at_a_non_negative_position() {
        let outputs = [at("DP-1", -1920, 0), at("DP-2", -3840, 0)];
        assert_eq!(shared(&outputs), ["DP-2", "DP-1"]);
        let none = choose_from(Pick::All, SOURCE_MONITOR, &[], &[], None, no_dialog);
        assert!(none.is_err(), "no output is an error, not an empty grant");
    }

    #[test]
    fn a_cast_that_fails_to_start_stops_the_ones_started_before_it() {
        let targets: Vec<_> = ["DP-1", "DP-2", "DP-3"]
            .map(|n| Target::Output(n.into()))
            .into();
        let mut started = Vec::new();
        let mut stopped = Vec::new();
        let result = start_each(
            &targets,
            |t| {
                started.push(t.clone());
                match t {
                    Target::Output(n) if n == "DP-2" => Err("no capture".to_string()),
                    t => Ok(t.clone()),
                }
            },
            |c| stopped.push(c),
        );
        assert_eq!(result, Err("no capture".to_string()));
        assert_eq!(started, targets[..2], "DP-3 never starts");
        assert_eq!(stopped, targets[..1], "DP-1 stops again");
        let all = start_each(&targets, |t| Ok::<_, String>(t.clone()), |_| {});
        assert_eq!(all.as_deref(), Ok(&targets[..]));
    }

    #[test]
    fn the_primary_screen_is_the_origin_output_else_the_top_left_most() {
        let name = |outputs: &[wayland::OutputInfo]| origin_output(outputs).map(|o| o.name.clone());
        let beside = [at("DP-2", 1920, 0), at("DP-1", 0, 0)];
        assert_eq!(name(&beside).as_deref(), Some("DP-1"), "the origin output");
        let moved = [at("HEADLESS-1", 1920, 0)];
        assert_eq!(
            name(&moved).as_deref(),
            Some("HEADLESS-1"),
            "the only output, away from the origin"
        );
        let staggered = [at("DP-1", 0, 200), at("DP-2", 1920, 0), at("DP-3", 3840, 0)];
        assert_eq!(
            name(&staggered).as_deref(),
            Some("DP-2"),
            "the topmost, then the leftmost"
        );
        let left = [at("DP-1", 0, 0), at("DP-2", -1920, 0)];
        assert_eq!(
            name(&left).as_deref(),
            Some("DP-1"),
            "the origin output wins over one further left"
        );
        assert_eq!(name(&[]), None);
    }

    #[test]
    fn persistence_needs_a_ticked_remember_and_caps_at_until_revoked() {
        assert_eq!(granted_persist(2, false), 0);
        assert_eq!(granted_persist(1, false), 0);
        assert_eq!(granted_persist(0, true), 0);
        assert_eq!(granted_persist(1, true), 1);
        assert_eq!(granted_persist(2, true), 2);
        assert_eq!(granted_persist(7, true), 2);
    }

    #[test]
    fn handles_must_lie_below_the_frontends_prefixes() {
        let path = |p| ObjectPath::try_from(p).unwrap();
        let session = path("/org/freedesktop/portal/desktop/session/1_42/t0");
        let request = path("/org/freedesktop/portal/desktop/request/1_42/t0");
        assert!(check_session(&session).is_ok());
        assert!(check_handle(&request, REQUEST_PREFIX).is_ok());
        assert!(check_session(&request).is_err());
        assert!(check_handle(&session, REQUEST_PREFIX).is_err());
        assert!(check_session(&path(PATH)).is_err());
        assert!(check_session(&path("/org/freedesktop/portal/desktop/session")).is_err());
        assert!(check_session(&path("/org/freedesktop/portal/desktop/sessions/x")).is_err());
    }

    #[test]
    fn a_restore_token_restores_only_for_its_app() {
        let screen = Target::Output("DP-1".into());
        let token = restore_data("org.example.A", &lastpick::encode(&screen));
        assert_eq!(
            restore_target(&token, "org.example.A"),
            Some(screen.clone())
        );
        assert_eq!(restore_target(&token, "org.example.B"), None);
        assert_eq!(restore_target(&token, ""), None);
        let unbound = (
            RESTORE_VENDOR.to_string(),
            2,
            Variant::from("output:DP-1").try_to_owned().unwrap(),
        );
        assert_eq!(
            restore_target(&unbound, "output"),
            None,
            "an older token re-asks"
        );
        let remote = restore_data("org.example.A", "remote-desktop:3;output:DP-1");
        assert_eq!(restore_target(&remote, "org.example.A"), Some(screen));
    }

    #[test]
    fn a_sender_other_than_the_known_owner_is_checked_against_the_bus_once() {
        use futures_lite::future::block_on;
        let asked = std::cell::Cell::new(0);
        let refresh = |owner: Option<&'static str>| {
            let asked = &asked;
            move || {
                asked.set(asked.get() + 1);
                async move { owner.map(String::from) }
            }
        };
        let (owner, verdict) = block_on(check_refreshing(
            Some(":1.42"),
            None,
            refresh(Some(":1.42")),
        ));
        assert!(verdict.is_ok(), "the owner arrived after the cache");
        assert_eq!(owner.as_deref(), Some(":1.42"));
        assert_eq!(asked.get(), 1);
        let (_, verdict) = block_on(check_refreshing(
            Some(":1.42"),
            Some(":1.42".into()),
            refresh(None),
        ));
        assert!(verdict.is_ok());
        assert_eq!(asked.get(), 1, "a known owner asks the bus nothing");
        let (owner, verdict) = block_on(check_refreshing(
            Some(":1.7"),
            Some(":1.42".into()),
            refresh(Some(":1.42")),
        ));
        assert!(verdict.is_err(), "another peer is still refused");
        assert_eq!(owner.as_deref(), Some(":1.42"));
        assert_eq!(asked.get(), 2);
    }

    #[test]
    fn an_owner_read_off_the_bus_does_not_replace_a_newer_one_the_watch_stored() {
        let f = Frontend::default();
        f.set(Some(":1.50".into()));
        f.replace(&Some(":1.42".into()), Some(":1.43".into()));
        assert_eq!(lock(&f.owner).as_deref(), Some(":1.50"));
        f.replace(&Some(":1.50".into()), Some(":1.51".into()));
        assert_eq!(lock(&f.owner).as_deref(), Some(":1.51"));
    }

    #[test]
    fn only_the_frontends_unique_name_may_call() {
        assert!(check_frontend(Some(":1.42"), Some(":1.42")).is_ok());
        for (sender, owner) in [
            (Some(":1.43"), Some(":1.42")),
            (Some(":1.42"), None),
            (None, Some(":1.42")),
        ] {
            assert!(matches!(
                check_frontend(sender, owner),
                Err(zbus::fdo::Error::AccessDenied(_))
            ));
        }
    }

    #[test]
    fn reprise_portal_lists_every_portal_interface_the_crate_defines() {
        const PREFIX: &str = "#[zbus::interface(name = \"org.freedesktop.impl.portal.";
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut defined = std::collections::BTreeSet::new();
        for f in std::fs::read_dir(src).unwrap().flatten() {
            let text = std::fs::read_to_string(f.path()).unwrap();
            for line in text.lines() {
                if let Some(rest) = line.trim().strip_prefix(PREFIX)
                    && let Some((name, _)) = rest.split_once('"')
                    && !["Request", "Session"].contains(&name)
                {
                    defined.insert(format!("org.freedesktop.impl.portal.{name}"));
                }
            }
        }
        let file = include_str!("../../../data/portal/reprise.portal");
        let listed: std::collections::BTreeSet<String> = file
            .lines()
            .find_map(|l| l.strip_prefix("Interfaces="))
            .expect("an Interfaces= line")
            .split(';')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        assert_eq!(listed, defined);
    }

    #[test]
    fn a_frontend_leaving_takes_only_its_own_sessions() {
        let mut map = HashMap::new();
        for (key, owner) in [("a", ":1.5"), ("b", ":1.9"), ("c", ":1.5")] {
            let kind = SessionKind::ScreenCast(CastState::default());
            let data = SessionData::new(owner.into(), kind);
            map.insert(key.to_string(), data);
        }
        let mut gone: Vec<String> = take_owned_by(&mut map, ":1.5")
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        gone.sort();
        assert_eq!(gone, ["a", "c"]);
        assert_eq!(map.keys().collect::<Vec<_>>(), ["b"]);
        assert!(take_owned_by(&mut map, ":1.7").is_empty());
    }
}
