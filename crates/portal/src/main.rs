//! reprise-portal: xdg-desktop-portal backend (ScreenCast, Screenshot, RemoteDesktop, InputCapture, GlobalShortcuts, Access,
//! FileChooser, AppChooser, Notification, Inhibit) as `org.freedesktop.impl.portal.desktop.reprise`; dialogs run in a child process (`dialog`).
//! It serves only while Reprise is the session's compositor (see `owner`).

mod access;
mod appchoose;
mod appchooser;
mod cast;
mod choices;
mod colorpick;
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
mod wayland;

use cast::Cast;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use wayland::{Target, Wayland};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{
    DeserializeDict, ObjectPath, OwnedObjectPath, OwnedValue, SerializeDict, Type, Value,
};

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.reprise";
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

/// The vendor tag in `restore_data`: ("reprise", 2, "output:<name>" or
/// "toplevel:<identifier>"). Version 2 tokens exist only for a grant the
/// user ticked Remember on; a version 1 token was issued without asking
/// and restores nothing.
const RESTORE_VENDOR: &str = "reprise";
const RESTORE_VERSION: u32 = 2;

/// Applications whose windows the picker never lists: the portals' own
/// surfaces, including this program's dialog.
const HIDDEN_APPS: [&str; 2] = ["reprise.portal", "xdg-desktop-portal-wlr"];

pub(crate) type Results = HashMap<String, OwnedValue>;

/// Casts start and stop one at a time: a client that shares again right
/// after closing would otherwise open the new capture session while the old
/// one is still stopping, and a compositor that renders an initial frame
/// only for the first session on a source would leave the new one waiting
/// for damage a static window never produces. Held inside the blocking pool
/// only, never across an await.
static CAST_TURN: Mutex<()> = Mutex::new(());

/// Stop a `Cast` off the executor: `Cast::stop` joins the capture
/// thread, which can wait a frame and PipeWire's teardown. Every path
/// that ends a cast comes through here so none of them stalls zbus.
async fn stop_cast(c: Cast) {
    blocking::unblock(move || {
        let _turn = CAST_TURN.lock().unwrap_or_else(|e| e.into_inner());
        c.stop()
    })
    .await;
}

/// The unique bus name a call came from; empty off a bus, which no
/// NameOwnerChanged ever names.
pub(crate) fn sender(hdr: &zbus::message::Header<'_>) -> String {
    hdr.sender().map(|s| s.to_string()).unwrap_or_default()
}

/// End a session already taken out of the map: cancel its dialog, stop
/// its cast, signal `Closed` and take its object off the bus. The EIS
/// client and the capture object go with `data`.
async fn end_session(conn: &zbus::Connection, key: &str, data: Option<SessionData>) {
    if let Some(mut s) = data {
        if let Some(c) = s.picking.take() {
            c.store(true, Ordering::SeqCst);
        }
        if let Some(c) = s.take_cast() {
            stop_cast(c).await;
        }
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

/// End every session and inhibition of a frontend that left the bus
/// (xdg-desktop-portal restarted or crashed), since nothing else would
/// ever close them.
async fn watch_frontends(conn: zbus::Connection, sessions: Sessions, inhibits: inhibit::Shared) {
    use futures_lite::StreamExt;
    let changes = match zbus::fdo::DBusProxy::new(&conn).await {
        Ok(p) => p.receive_name_owner_changed().await,
        Err(e) => Err(e),
    };
    let mut changes = match changes {
        Ok(c) => c,
        Err(e) => {
            log::warn!("NameOwnerChanged: {e}; sessions outlive their frontend");
            return;
        }
    };
    while let Some(sig) = changes.next().await {
        let Ok(args) = sig.args() else { continue };
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
) -> zbus::fdo::Result<()> {
    check_handle(handle, REQUEST_PREFIX)?;
    let request = Request {
        cancel,
        path: handle.to_string(),
    };
    server.at(handle.clone(), request).await?;
    Ok(())
}

async fn replace_session(sessions: &Sessions, key: String, data: SessionData) {
    let old = sessions.lock().unwrap().insert(key, data);
    if let Some(c) = old.and_then(|mut s| s.take_cast()) {
        stop_cast(c).await;
    }
}

#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct SelectSourcesOptions {
    types: Option<u32>,
    #[allow(dead_code)]
    multiple: Option<bool>,
    cursor_mode: Option<u32>,
    restore_data: Option<(String, u32, OwnedValue)>,
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
    restore_data: Option<(String, u32, OwnedValue)>,
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
/// cast Start made from it. On a RemoteDesktop session `types` stays 0
/// until ScreenCast.SelectSources runs on it.
#[derive(Default)]
struct CastState {
    types: u32,
    cursor_mode: u32,
    persist_mode: u32,
    restore: Option<Target>,
    cast: Option<Cast>,
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

    fn take_cast(&mut self) -> Option<Cast> {
        self.screencast_mut()?.cast.take()
    }
}

type Sessions = Arc<Mutex<HashMap<String, SessionData>>>;

struct ScreenCast {
    sessions: Sessions,
    source_types: u32,
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
        _options: HashMap<String, Value<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, Results)> {
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
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        options: SelectSourcesOptions,
    ) -> zbus::fdo::Result<(u32, Results)> {
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
        s.restore = options.restore_data.as_ref().and_then(restore_target);
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
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        _options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u32, StartResults)> {
        let key = session_handle.to_string();
        let parent = dialog::parent_handle(&parent_window);
        match pick_and_cast(&self.sessions, server, conn, &handle, &key, &app_id, parent).await {
            Ok(st) => Ok((
                RESPONSE_OK,
                StartResults {
                    streams: vec![st.stream],
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
    pub(crate) stream: (u32, StreamProps),
    pub(crate) persist_mode: u32,
    pub(crate) restore_data: Option<(String, u32, OwnedValue)>,
}

pub(crate) async fn pick_and_cast(
    sessions: &Sessions,
    server: &zbus::ObjectServer,
    conn: &zbus::Connection,
    handle: &ObjectPath<'_>,
    key: &str,
    app_id: &str,
    parent: Option<String>,
) -> Result<Started, u32> {
    let (types, cursor_mode, persist_mode, restore) = {
        let sessions = sessions.lock().unwrap();
        let Some(s) = sessions.get(key).and_then(SessionData::screencast) else {
            return Err(RESPONSE_OTHER);
        };
        (s.types, s.cursor_mode, s.persist_mode, s.restore.clone())
    };
    let cancel = Arc::new(AtomicBool::new(false));
    if let Err(e) = export_request(server, handle, cancel.clone()).await {
        log::warn!("Start: {e}");
        return Err(RESPONSE_OTHER);
    }
    if let Some(s) = sessions.lock().unwrap().get_mut(key) {
        s.picking = Some(cancel.clone());
    }
    let app = app_id.to_string();
    let chosen =
        blocking::unblock(move || choose(types, restore, &app, persist_mode > 0, parent, cancel))
            .await;
    let _ = server.remove::<Request, _>(handle.clone()).await;
    if let Some(s) = sessions.lock().unwrap().get_mut(key) {
        s.picking = None;
    }
    let (target, output_info, remember) = match chosen {
        Ok(Some(t)) => t,
        Ok(None) => return Err(RESPONSE_CANCELLED),
        Err(e) => {
            log::warn!("Start: {e}");
            return Err(RESPONSE_OTHER);
        }
    };
    let mode = cursor_mode_for(cursor_mode, &target);
    let t2 = target.clone();
    let cast = match blocking::unblock(move || {
        let _turn = CAST_TURN.lock().unwrap_or_else(|e| e.into_inner());
        Cast::start(t2, mode)
    })
    .await
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Start: {e}");
            return Err(RESPONSE_OTHER);
        }
    };
    let (source_type, position, mapping_id, size) = match (&target, output_info) {
        (Target::Output(name), Some(o)) => (
            SOURCE_MONITOR,
            Some((o.x, o.y)),
            Some(name.clone()),
            (o.width, o.height),
        ),
        (Target::Output(name), None) => (
            SOURCE_MONITOR,
            None,
            Some(name.clone()),
            (cast.width as i32, cast.height as i32),
        ),
        (Target::Toplevel(_), _) => (
            SOURCE_WINDOW,
            None,
            None,
            (cast.width as i32, cast.height as i32),
        ),
    };
    let stream = (
        cast.node_id,
        StreamProps {
            position,
            size,
            source_type,
            mapping_id,
            pipewire_serial: cast.serial,
        },
    );
    log::info!("Start: node {} for {:?}", cast.node_id, target);
    let persist_mode = granted_persist(persist_mode, remember);
    let restore_data = (persist_mode > 0).then(|| restore_value(&target));
    let mut cast = cast;
    let ended = cast.take_ended();
    let node_id = cast.node_id;
    // A second Start hands back the session's earlier cast, and a Close
    // during this one leaves the new cast unkept. Either stops off the
    // lock, since `Cast::stop` joins its thread.
    let (replaced, unkept) = match sessions
        .lock()
        .unwrap()
        .get_mut(key)
        .and_then(SessionData::screencast_mut)
    {
        Some(s) => (s.cast.replace(cast), None),
        None => (None, Some(cast)),
    };
    if let Some(old) = replaced {
        stop_cast(old).await;
    }
    if let Some(cast) = unkept {
        stop_cast(cast).await;
        return Err(RESPONSE_CANCELLED);
    }
    // The compositor can end the cast on its own (the window closed,
    // the output went); the frontend hears of it as a Closed session.
    if let Some(ended) = ended {
        conn.executor()
            .spawn(
                cast_ended(
                    conn.clone(),
                    sessions.clone(),
                    key.to_string(),
                    node_id,
                    ended,
                ),
                "cast-ended",
            )
            .detach();
    }
    Ok(Started {
        stream,
        persist_mode,
        restore_data,
    })
}

/// Wait for the cast under `key` to end and, if the session still holds
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
        let ours = map
            .get(&key)
            .and_then(|s| s.screencast()?.cast.as_ref())
            .is_some_and(|c| c.node_id == node_id);
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

fn restore_target(data: &(String, u32, OwnedValue)) -> Option<Target> {
    if data.0 != RESTORE_VENDOR || data.1 != RESTORE_VERSION {
        return None;
    }
    let s: String = <String>::try_from(data.2.clone()).ok()?;
    lastpick::decode(&s)
}

fn restore_value(t: &Target) -> (String, u32, OwnedValue) {
    let s = lastpick::encode(t);
    (
        RESTORE_VENDOR.to_string(),
        RESTORE_VERSION,
        Value::from(s).try_to_owned().expect("string"),
    )
}

/// Pick what to capture and whether to remember it: a remembered target
/// that still exists (remembered again), else the share dialog, modal on
/// `parent`, which offers Remember when `offer_remember`. Blocking; runs
/// off the bus thread.
fn choose(
    types: u32,
    restore: Option<Target>,
    app_id: &str,
    offer_remember: bool,
    parent: Option<String>,
    cancel: Arc<AtomicBool>,
) -> Result<Option<(Target, Option<wayland::OutputInfo>, bool)>, String> {
    let wl = Wayland::connect()?;
    let outputs = wl.outputs();
    let toplevels = wl.toplevels();
    drop(wl);
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
            let info = info_for(&t);
            return Ok(Some((t, info, true)));
        }
    }
    let mut entries = Vec::new();
    if types & SOURCE_MONITOR != 0 {
        for o in &outputs {
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
        for w in &toplevels {
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
    if entries.is_empty() {
        return Err("nothing to share".into());
    }
    let last = lastpick::load(app_id);
    let preselect = last.and_then(|t| entries.iter().position(|e| e.target == t));
    let Some((t, remember)) = picker::pick(
        &crate::remotedesktop::display_name(app_id),
        entries,
        preselect,
        offer_remember,
        parent,
        cancel,
    ) else {
        return Ok(None);
    };
    lastpick::save(app_id, &t);
    let info = info_for(&t);
    Ok(Some((t, info, remember)))
}

struct Request {
    cancel: Arc<AtomicBool>,
    path: String,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        log::info!("Request {} closed", self.path);
        self.cancel.store(true, Ordering::SeqCst);
    }
}

struct Session {
    path: OwnedObjectPath,
    sessions: Sessions,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(&self, #[zbus(connection)] conn: &zbus::Connection) -> zbus::fdo::Result<()> {
        log::info!("Session {} closed", self.path.as_str());
        let data = self.sessions.lock().unwrap().remove(self.path.as_str());
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

async fn serve(
    sessions: Sessions,
    source_types: u32,
    link: Option<inputlink::Link>,
    notes: notification::Shared,
    inhibits: inhibit::Shared,
) -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            PATH,
            ScreenCast {
                sessions: sessions.clone(),
                source_types,
            },
        )?
        .serve_at(PATH, screenshot::Screenshot)?
        .serve_at(
            PATH,
            remotedesktop::RemoteDesktop {
                sessions: sessions.clone(),
                link: link.clone(),
            },
        )?
        .serve_at(
            PATH,
            globalshortcuts::GlobalShortcuts {
                sessions: sessions.clone(),
                link: link.clone(),
            },
        )?
        .serve_at(PATH, inputcapture::InputCapture { sessions, link })?
        .serve_at(PATH, access::Access)?
        .serve_at(PATH, filechooser::FileChooser)?
        .serve_at(PATH, appchooser::AppChooser::default())?
        .serve_at(PATH, notification::Notification { book: notes })?
        .serve_at(
            PATH,
            inhibit::Inhibit {
                shared: inhibits.clone(),
            },
        )?
        .serve_at(PATH, inhibit::SessionEnd { shared: inhibits })?
        .build()
        .await
}

fn main() {
    // zbus logs every call it serves at info, with the whole message; RUST_LOG
    // still turns it back on.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,zbus=warn"))
        .init();
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
    if std::env::args().any(|a| a == "--pickcolor") {
        colorpick::run_dialog();
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
    // The compositor's input link backs RemoteDesktop, InputCapture and
    // GlobalShortcuts; without it those interfaces answer but grant nothing.
    let link = match inputlink::Link::start() {
        Ok(l) => Some(l),
        Err(e) => {
            log::warn!("input link: {e}; RemoteDesktop, InputCapture and GlobalShortcuts disabled");
            None
        }
    };
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    zbus::block_on(async move {
        let notes = notification::Shared::default();
        let (inhibits, idle) = inhibit::shared();
        let conn = serve(
            sessions.clone(),
            source_types,
            link,
            notes.clone(),
            inhibits.clone(),
        )
        .await
        .unwrap_or_else(|e| {
            log::error!("could not serve {BUS_NAME}: {e}");
            std::process::exit(1);
        });
        conn.executor()
            .spawn(notification::watch(conn.clone(), notes), "notifications")
            .detach();
        conn.executor()
            .spawn(inhibit::run_idle(conn.clone(), idle), "idle inhibitions")
            .detach();
        conn.executor()
            .spawn(
                inhibit::watch_lock(conn.clone(), inhibits.clone()),
                "screen lock",
            )
            .detach();
        owner::exit_with(compositor);
        watch_frontends(conn, sessions, inhibits).await;
        std::future::pending::<()>().await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
