//! `org.freedesktop.impl.portal.RemoteDesktop` v2 over libei: input granted by the permit dialog,
//! or without it to the enabled server (`trusted`); keyboard and pointer Notify use reprise_keys_v1.

use crate::inputlink::{Link, LockScreenHandle, Owner, Pointer, live};
use crate::inputproto::reprise_input_v1::Device;
use crate::wayland::Target;
use crate::{
    CastState, RESPONSE_CANCELLED, RESPONSE_OK, RESPONSE_OTHER, Request, SessionData, SessionKind,
    Sessions,
};
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::{ObjectPath, OwnedVariant, SerializeDict, Type, Variant};

pub const DEVICE_KEYBOARD: u32 = Device::Keyboard as u32;
pub const DEVICE_POINTER: u32 = Device::Pointer as u32;
const RESTORE_TAG: &str = "remote-desktop";

#[derive(Default)]
pub(crate) struct RemoteSession {
    devices: u32,
    persist: u32,
    restored: bool,
    granted: bool,
    /// Started for the remote desktop server without a dialog (`crate::trusted`).
    trusted: bool,
    /// The app id the frontend created the session for.
    pub(crate) app_id: String,
    eis: Option<crate::inputlink::EisHandle>,
    keys: Option<crate::inputlink::KeysHandle>,
    pub(crate) screencast: CastState,
    /// Held while the session casts lock mode's screens (`crate::lockmode`).
    pub(crate) lock: Option<Arc<crate::inputlink::LockScreenHandle>>,
    /// The balloon a start from a remembered grant showed.
    pub(crate) notice: Option<crate::remotenotice::Shown>,
}

pub struct RemoteDesktop {
    pub sessions: Sessions,
    /// None when the compositor offered no `reprise_input_v1`; every method then fails rather than
    /// pretending to grant.
    pub link: Option<Link>,
    pub frontend: crate::Frontend,
    pub notices: crate::remotenotice::Notices,
    pub ask: Ask,
}

/// The permit dialog, [`crate::permit::ask`] outside tests.
pub type Ask = fn(
    crate::permit::Kind,
    &str,
    Vec<String>,
    bool,
    Option<String>,
    Arc<AtomicBool>,
) -> crate::permit::Answer;

#[derive(SerializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct StartResults {
    devices: u32,
    clipboard_enabled: bool,
    persist_mode: u32,
    restore_data: Option<(String, u32, OwnedVariant)>,
    streams: Vec<(u32, crate::StreamProps)>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.RemoteDesktop")]
impl RemoteDesktop {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[zbus(property)]
    fn available_device_types(&self) -> u32 {
        DEVICE_KEYBOARD | DEVICE_POINTER
    }

    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: HashMap<String, Variant<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        self.frontend.check(&hdr).await?;
        crate::check_session(&session_handle)?;
        if live(&self.link).is_none() {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        }
        log::info!(
            "RemoteDesktop.CreateSession {} for {app_id:?}",
            session_handle.as_str()
        );
        crate::replace_session(
            &self.sessions,
            session_handle.to_string(),
            SessionData::new(
                crate::sender(&hdr),
                SessionKind::RemoteDesktop(RemoteSession {
                    devices: DEVICE_KEYBOARD | DEVICE_POINTER,
                    app_id: app_id.clone(),
                    ..Default::default()
                }),
            ),
        )
        .await;
        server
            .at(
                session_handle.clone(),
                crate::Session {
                    path: session_handle.clone().into(),
                    sessions: self.sessions.clone(),
                },
            )
            .await?;
        Ok((RESPONSE_OK, HashMap::new()))
    }

    async fn select_devices(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        options: HashMap<String, Variant<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        self.frontend.check(&hdr).await?;
        if live(&self.link).is_none() {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        }
        let mut sessions = self.sessions.lock().unwrap();
        let Some(s) = sessions
            .get_mut(session_handle.as_str())
            .and_then(SessionData::remote_mut)
        else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        if let Some(t) = options.get("types").and_then(|v| u32::try_from(v).ok()) {
            s.devices = t & (DEVICE_KEYBOARD | DEVICE_POINTER);
        }
        if let Some(p) = options
            .get("persist_mode")
            .and_then(|v| u32::try_from(v).ok())
        {
            s.persist = p;
        }
        let restored = (options.get("restore_data"))
            .and_then(|v| our_grant(v, &app_id))
            .filter(|_| !self.notices.forgotten(&app_id));
        s.restored = restored.is_some();
        // The frontend passes a token only here, so the picture it remembers is restored here too.
        if let Some((granted, target)) = restored {
            s.devices &= granted;
            s.screencast.restore = target;
        }
        log::info!(
            "RemoteDesktop.SelectDevices devices={} persist={} restored={}",
            s.devices,
            s.persist,
            s.restored
        );
        Ok((RESPONSE_OK, HashMap::new()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn start(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u32, StartResults)> {
        self.frontend.check(&hdr).await?;
        if live(&self.link).is_none() {
            return Ok((RESPONSE_OTHER, empty()));
        }
        let key = session_handle.to_string();
        let parent = crate::dialog::parent_handle(&parent_window);
        let caller = crate::trusted::session_caller(conn, session_handle.as_str()).await;
        let trust = match caller {
            Some(_) => {
                use crate::trusted::{HANDOVER_UNIT, UNIT, unit_facts};
                crate::trusted::start_trusted(
                    &app_id,
                    caller,
                    unit_facts(conn, UNIT).await.as_ref(),
                    unit_facts(conn, HANDOVER_UNIT).await.as_ref(),
                )
            }
            None => None,
        };
        let trusted = trust.is_some();
        let (devices, restored, mut requested) = {
            let mut sessions = self.sessions.lock().unwrap();
            let Some(s) = sessions.get_mut(&key).and_then(SessionData::remote_mut) else {
                return Ok((RESPONSE_OTHER, empty()));
            };
            if trusted {
                s.devices = DEVICE_KEYBOARD | DEVICE_POINTER;
            }
            s.trusted = trusted;
            (s.devices, s.restored, s.persist)
        };
        if let Some(p) = options
            .get("persist_mode")
            .and_then(|v| u32::try_from(v).ok())
        {
            requested = p;
        }
        if trusted {
            log::info!("RemoteDesktop.Start for {app_id:?}: trusted");
            requested = 0;
        }
        let mut remember = restored;
        if !restored && !trusted {
            let cancel = Arc::new(AtomicBool::new(false));
            crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
            if let Some(s) = self.sessions.lock().unwrap().get_mut(&key) {
                s.picking = Some(cancel.clone());
            }
            let id = app_id.clone();
            let parent = parent.clone();
            let ask = self.ask;
            let answer = blocking::unblock(move || {
                ask(
                    crate::permit::Kind::RemoteControl(crate::permit::Devices::from_mask(devices)),
                    &id,
                    Vec::new(),
                    requested > 0,
                    parent,
                    cancel,
                )
            })
            .await;
            let _ = server.remove::<Request, _>(handle.clone()).await;
            match self.sessions.lock().unwrap().get_mut(&key) {
                Some(s) => s.picking = None,
                None => return Ok((RESPONSE_CANCELLED, empty())),
            }
            match answer {
                crate::permit::Answer::Allow { remember: r } => {
                    remember = r;
                    if r {
                        self.notices.remembered(&app_id);
                    }
                }
                crate::permit::Answer::Deny | crate::permit::Answer::Dismissed => {
                    log::info!("RemoteDesktop.Start for {app_id:?} denied");
                    return Ok((RESPONSE_CANCELLED, empty()));
                }
            }
        }
        let persist = crate::granted_persist(requested, remember);
        // The frontend refuses a persist mode on ScreenCast.SelectSources in a remote desktop
        // session: the share picker offers Remember when this grant is remembered.
        if let Some(s) =
            (self.sessions.lock().unwrap().get_mut(&key)).and_then(SessionData::remote_mut)
        {
            s.screencast.persist_mode = persist;
        }
        // A session that also went through ScreenCast.SelectSources gets its picture here: the
        // frontend expects `streams` from this Start.
        let types = (self.sessions.lock().unwrap().get_mut(&key))
            .and_then(SessionData::remote_mut)
            .map(|s| s.screencast.types);
        let mut streams = Vec::new();
        let mut kept = None;
        let mut locked = None;
        match types {
            None => return Ok((RESPONSE_CANCELLED, empty())),
            Some(0) => {}
            Some(_) => {
                if let Some(trust) = trust {
                    match crate::lockmode::begin(conn, caller, trust, live(&self.link)).await {
                        Ok(l) => locked = l,
                        Err(crate::lockmode::NotBegun) => return Ok((RESPONSE_OTHER, empty())),
                    }
                }
                let pick = if trusted {
                    crate::lockmode::pick(locked.as_ref())
                } else {
                    crate::Pick::Ask
                };
                match crate::pick_and_cast(
                    &self.sessions,
                    &self.frontend,
                    server,
                    conn,
                    &handle,
                    &key,
                    &app_id,
                    parent,
                    pick,
                )
                .await
                {
                    Ok(st) => {
                        streams = st.streams;
                        kept = st.remembered;
                    }
                    Err(code) => return Ok((code, empty())),
                }
            }
        }
        log::info!(
            "RemoteDesktop.Start for {app_id:?}: devices {devices}, persist {persist}, streams {}",
            streams.len()
        );
        let follow = locked.map(|l| l.hold(conn.clone(), self.sessions.clone(), key.clone()));
        match (self.sessions.lock().unwrap().get_mut(&key)).and_then(SessionData::remote_mut) {
            Some(s) => {
                let (lock, follow) = follow.unzip();
                s.grant(live(&self.link), lock);
                if let Some(follow) = follow {
                    zruntime::spawn(follow).detach();
                }
            }
            None => return Ok((RESPONSE_CANCELLED, empty())),
        }
        if restored && !trusted {
            let sessions = self.sessions.clone();
            let show = (self.notices.clone()).show(conn.clone(), sessions, key, app_id.clone());
            zruntime::spawn(show).detach();
        }
        let restore_data =
            (persist > 0).then(|| crate::restore_data(&app_id, &grant(devices, kept.as_ref())));
        Ok((
            RESPONSE_OK,
            StartResults {
                devices,
                clipboard_enabled: false,
                persist_mode: persist,
                restore_data,
                streams,
            },
        ))
    }

    #[zbus(name = "ConnectToEIS")]
    async fn connect_to_eis(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: HashMap<String, Variant<'_>>,
    ) -> zbus::fdo::Result<zbus::OwnedFd> {
        self.frontend.check(&hdr).await?;
        let theirs = self.connect_sender(session_handle.as_str())?;
        log::info!("RemoteDesktop.ConnectToEIS for {app_id:?}");
        Ok(theirs.into())
    }

    async fn notify_pointer_motion(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        dx: f64,
        dy: f64,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        finite(&[dx, dy])?;
        self.with_pointer(&session_handle, |_| Ok(Pointer::Motion(dx, dy)))
    }
    async fn notify_pointer_motion_absolute(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        stream: u32,
        x: f64,
        y: f64,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        finite(&[x, y])?;
        self.with_pointer(&session_handle, |s| {
            match s.screencast.casts.iter().find(|c| c.node_id == stream) {
                Some(c) => match &c.target {
                    Target::Output(name) => Ok(Pointer::MotionAbsolute(name.clone(), x, y)),
                    Target::Toplevel(_) => Err(invalid("a window stream takes no absolute motion")),
                },
                None => Err(invalid("no such stream in the session")),
            }
        })
    }
    async fn notify_pointer_button(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        button: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        let button = u32::try_from(button).map_err(|_| invalid("a negative button"))?;
        self.with_pointer(&session_handle, |_| Ok(Pointer::Button(button, state != 0)))
    }
    async fn notify_pointer_axis(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        dx: f64,
        dy: f64,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        finite(&[dx, dy])?;
        self.with_pointer(&session_handle, |_| Ok(Pointer::Axis(dx, dy)))
    }
    async fn notify_pointer_axis_discrete(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        axis: u32,
        steps: i32,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        let vertical = match axis {
            0 => true,
            1 => false,
            _ => return Err(invalid("axis is 0 or 1")),
        };
        self.with_pointer(&session_handle, |_| Ok(Pointer::Wheel { vertical, steps }))
    }
    async fn notify_keyboard_keycode(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        keycode: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        self.with_keys(&session_handle, DEVICE_KEYBOARD, |k| {
            k.key(keycode as u32, state != 0)
        })
    }
    async fn notify_keyboard_keysym(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        keysym: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr).await?;
        self.with_keys(&session_handle, DEVICE_KEYBOARD, |k| {
            k.keysym(keysym as u32, state != 0)
        })
    }
    async fn notify_touch_down(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        _stream: u32,
        _slot: u32,
        _x: f64,
        _y: f64,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_touch_motion(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        _stream: u32,
        _slot: u32,
        _x: f64,
        _y: f64,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_touch_up(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        _slot: u32,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
}

impl RemoteSession {
    /// Granted, holding `lock` while it casts lock mode, with a keys object from `link` if the
    /// keyboard or the pointer was granted and lock mode has not already ended.
    pub(crate) fn grant(&mut self, link: Option<&Link>, lock: Option<Arc<LockScreenHandle>>) {
        self.granted = true;
        self.lock = lock;
        if self.devices != 0 && !self.lock_ended() {
            self.keys = link.and_then(|l| l.create_keys(self.owner()));
        }
    }

    /// Whose session this is, so the compositor names the app while it holds input.
    fn owner(&self) -> Owner {
        match self.trusted {
            true => Owner::Server,
            false => Owner::App(self.app_id.clone()),
        }
    }

    /// A session whose lock mode ended takes no new input objects.
    fn lock_ended(&self) -> bool {
        self.lock.as_ref().is_some_and(|l| l.ended())
    }
}

impl RemoteDesktop {
    /// ConnectToEIS for `session`: our end goes to the compositor, theirs is returned.
    pub(crate) fn connect_sender(&self, session: &str) -> zbus::fdo::Result<OwnedFd> {
        let Some(link) = live(&self.link) else {
            return Err(zbus::fdo::Error::Failed("no input link".into()));
        };
        let mut sessions = self.sessions.lock().unwrap();
        let Some(s) = (sessions.get_mut(session))
            .and_then(SessionData::remote_mut)
            .filter(|s| s.granted)
        else {
            return Err(zbus::fdo::Error::Failed("no such session".into()));
        };
        if s.lock_ended() {
            return Err(zbus::fdo::Error::Failed("lock mode ended".into()));
        }
        let (ours, theirs) = crate::inputlink::socketpair()?;
        s.eis = Some(link.connect_sender(ours, s.devices, s.owner()));
        Ok(theirs)
    }

    /// Run `f` on the keys object of `session_handle`, a session granted `device`.
    fn with_keys(
        &self,
        session_handle: &ObjectPath<'_>,
        device: u32,
        f: impl FnOnce(&crate::inputlink::KeysHandle),
    ) -> zbus::fdo::Result<()> {
        self.with_session(session_handle, device, |s| {
            f(s.keys.as_ref().expect("with_session checked the keys"));
            Ok(())
        })
    }

    /// Send the pointer request `input` makes of `session_handle`, a session granted the pointer.
    fn with_pointer(
        &self,
        session_handle: &ObjectPath<'_>,
        input: impl FnOnce(&RemoteSession) -> zbus::fdo::Result<Pointer>,
    ) -> zbus::fdo::Result<()> {
        if !live(&self.link).is_some_and(Link::has_pointer) {
            return Err(zbus::fdo::Error::NotSupported(
                "the compositor takes no pointer without libei; use ConnectToEIS".into(),
            ));
        }
        self.with_session(session_handle, DEVICE_POINTER, |s| {
            let input = input(s)?;
            s.keys
                .as_ref()
                .expect("with_session checked the keys")
                .pointer(input);
            Ok(())
        })
    }

    fn with_session(
        &self,
        session_handle: &ObjectPath<'_>,
        device: u32,
        f: impl FnOnce(&RemoteSession) -> zbus::fdo::Result<()>,
    ) -> zbus::fdo::Result<()> {
        if live(&self.link).is_none() {
            return Err(zbus::fdo::Error::Failed("no input link".into()));
        }
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get_mut(session_handle.as_str())
            .and_then(SessionData::remote_mut)
            .filter(|s| s.devices & device != 0 && s.keys.is_some());
        let name = if device == DEVICE_POINTER {
            "pointer"
        } else {
            "keyboard"
        };
        match session {
            Some(s) => f(s),
            None => Err(zbus::fdo::Error::Failed(format!(
                "no session granted the {name}"
            ))),
        }
    }
}

fn unsupported() -> zbus::fdo::Error {
    zbus::fdo::Error::NotSupported("use ConnectToEIS".into())
}

fn invalid(why: &str) -> zbus::fdo::Error {
    zbus::fdo::Error::InvalidArgs(why.into())
}

fn finite(values: &[f64]) -> zbus::fdo::Result<()> {
    match values.iter().all(|v| v.is_finite()) {
        true => Ok(()),
        false => Err(invalid("a coordinate is not finite")),
    }
}

fn empty() -> StartResults {
    StartResults {
        devices: 0,
        clipboard_enabled: false,
        persist_mode: 0,
        restore_data: None,
        streams: Vec::new(),
    }
}

/// The grant a RemoteDesktop token carries: the devices, and the picture
/// when the user also ticked Remember in the share picker.
fn grant(devices: u32, target: Option<&Target>) -> String {
    match target {
        Some(t) => format!("{RESTORE_TAG}:{devices};{}", crate::lastpick::encode(t)),
        None => format!("{RESTORE_TAG}:{devices}"),
    }
}

/// The devices and picture of a RemoteDesktop grant; None for any other.
pub(crate) fn parse_grant(grant: &str) -> Option<(u32, Option<Target>)> {
    let rest = grant.strip_prefix(RESTORE_TAG)?.strip_prefix(':')?;
    let (mask, target) = match rest.split_once(';') {
        Some((mask, t)) => (mask, Some(crate::lastpick::decode(t)?)),
        None => (rest, None),
    };
    let mask = mask.parse::<u32>().ok()? & (DEVICE_KEYBOARD | DEVICE_POINTER);
    Some((mask, target))
}

fn our_grant(v: &Variant<'_>, app_id: &str) -> Option<(u32, Option<Target>)> {
    let data = <(String, u32, OwnedVariant)>::try_from(v.try_to_owned().ok()?).ok()?;
    parse_grant(crate::restored_grant(&data, app_id)?)
}

pub(crate) fn display_name(app_id: &str) -> String {
    if app_id.is_empty() {
        return String::new();
    }
    app_entry(app_id).map_or_else(|| app_id.to_string(), |a| a.name)
}

/// The desktop entry of `app_id`, best first ([`reprisefs::apps::DesktopId::for_app_id`]).
pub(crate) fn app_entry(app_id: &str) -> Option<reprisefs::apps::AppEntry> {
    let mut apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
    let at = (reprisefs::apps::DesktopId::for_app_id(app_id).iter())
        .find_map(|id| apps.iter().position(|a| a.id == id.as_str()))?;
    Some(apps.swap_remove(at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputlink::tests::{fake_compositor, link_on, request, send};
    use crate::{RESTORE_VENDOR, restore_data};
    use std::os::unix::net::UnixStream;
    use std::sync::Mutex;
    use std::time::Duration;

    const APP: &str = "org.example.Remote";

    /// A v9 compositor that ends each lock-mode object `console_unlocked` on its layout.
    fn unlocking_compositor(mut theirs: UnixStream) {
        const LOCK_SCREEN: u32 = 8;
        const SET_LAYOUT: u32 = 0;
        const ENDED: u32 = 2;
        const CONSOLE_UNLOCKED: u32 = 1;
        std::thread::spawn(move || {
            let input = fake_compositor(&mut theirs, 2, 9);
            let timeout = Some(Duration::from_secs(5));
            theirs.set_read_timeout(timeout).unwrap();
            let mut lock = None;
            while let Ok((object, opcode, body)) = request(&mut theirs) {
                match (object, opcode) {
                    (o, LOCK_SCREEN) if o == input => {
                        lock = Some(u32::from_ne_bytes(body[..4].try_into().unwrap()));
                    }
                    (o, SET_LAYOUT) if Some(o) == lock => {
                        send(&mut theirs, o, ENDED, &CONSOLE_UNLOCKED.to_ne_bytes());
                    }
                    _ => {}
                }
            }
        });
    }

    #[test]
    fn a_session_whose_lock_mode_ended_gets_no_keys_and_no_eis() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        unlocking_compositor(theirs);
        let link = link_on(ours);
        let lock = Arc::new(link.lock_screen(&[(0, 0, 1280, 720, 100)]).unwrap());
        let ended = zbus::block_on(lock.events().recv());
        assert!(matches!(ended, Ok(crate::inputlink::LockEvent::Ended(_))));
        assert!(lock.ended());
        let sessions = Sessions::default();
        let both = DEVICE_KEYBOARD | DEVICE_POINTER;
        for (key, lock) in [("/ended", Some(lock)), ("/shown", None)] {
            let mut session = RemoteSession {
                devices: both,
                ..RemoteSession::default()
            };
            session.grant(Some(&link), lock);
            let data = SessionData::new(":1.7".into(), SessionKind::RemoteDesktop(session));
            sessions.lock().unwrap().insert(key.into(), data);
        }
        let portal = RemoteDesktop {
            sessions: sessions.clone(),
            link: Some(link),
            frontend: crate::Frontend::default(),
            notices: Default::default(),
            ask: never_asked,
        };
        let keys = |key: &str| {
            let mut map = sessions.lock().unwrap();
            let s = map.get_mut(key).and_then(SessionData::remote_mut).unwrap();
            s.keys.is_some()
        };
        assert!(!keys("/ended"), "no keys object once lock mode ended");
        assert!(portal.connect_sender("/ended").is_err());
        assert!(keys("/shown"), "a session without lock mode gets its keys");
        assert!(portal.connect_sender("/shown").is_ok(), "and its sender");
    }

    fn never_asked(
        _: crate::permit::Kind,
        _: &str,
        _: Vec<String>,
        _: bool,
        _: Option<String>,
        _: Arc<AtomicBool>,
    ) -> crate::permit::Answer {
        crate::permit::Answer::Deny
    }

    fn allowed(
        _: crate::permit::Kind,
        _: &str,
        _: Vec<String>,
        _: bool,
        _: Option<String>,
        _: Arc<AtomicBool>,
    ) -> crate::permit::Answer {
        crate::permit::Answer::Allow { remember: true }
    }

    fn allowed_once(
        _: crate::permit::Kind,
        _: &str,
        _: Vec<String>,
        _: bool,
        _: Option<String>,
        _: Arc<AtomicBool>,
    ) -> crate::permit::Answer {
        crate::permit::Answer::Allow { remember: false }
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Note {
        app_name: String,
        replaces_id: u32,
        summary: String,
        actions: Vec<String>,
        urgency: Option<u8>,
        desktop_entry: Option<String>,
        expire_timeout: i32,
    }

    type Noted = Arc<Mutex<Vec<Note>>>;
    type Closed = Arc<Mutex<Vec<u32>>>;

    struct FakeNotifications(Noted, Closed);

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl FakeNotifications {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            app_name: String,
            replaces_id: u32,
            _app_icon: String,
            summary: String,
            _body: String,
            actions: Vec<String>,
            hints: HashMap<String, OwnedVariant>,
            expire_timeout: i32,
        ) -> u32 {
            let mut noted = self.0.lock().unwrap();
            noted.push(Note {
                app_name,
                replaces_id,
                summary,
                actions,
                urgency: hints.get("urgency").and_then(|v| u8::try_from(v).ok()),
                desktop_entry: (hints.get("desktop-entry"))
                    .and_then(|v| String::try_from(v.try_clone().ok()?).ok()),
                expire_timeout,
            });
            match replaces_id {
                0 => 40 + noted.len() as u32,
                id => id,
            }
        }

        fn close_notification(&self, id: u32) {
            self.1.lock().unwrap().push(id);
        }
    }

    /// The `remote-desktop` table: token to data.
    type Table = Arc<Mutex<HashMap<String, OwnedVariant>>>;

    struct FakeStore(Table);

    #[zbus::interface(name = "org.freedesktop.impl.portal.PermissionStore")]
    impl FakeStore {
        fn list(&self, table: String) -> Vec<String> {
            match table.as_str() {
                "remote-desktop" => self.0.lock().unwrap().keys().cloned().collect(),
                _ => Vec::new(),
            }
        }

        fn lookup(
            &self,
            table: String,
            id: String,
        ) -> zbus::fdo::Result<(HashMap<String, Vec<String>>, OwnedVariant)> {
            let table = (table == "remote-desktop").then(|| self.0.lock().unwrap());
            match table.as_ref().and_then(|t| t.get(&id)) {
                Some(data) => Ok((HashMap::new(), data.clone())),
                None => Err(zbus::fdo::Error::Failed("no such entry".into())),
            }
        }

        fn delete(&self, table: String, id: String) -> zbus::fdo::Result<()> {
            let gone = table == "remote-desktop" && self.0.lock().unwrap().remove(&id).is_some();
            match gone {
                true => Ok(()),
                false => Err(zbus::fdo::Error::Failed("no such entry".into())),
            }
        }
    }

    /// The portal's RemoteDesktop and ScreenCast with `ask` for its dialog, a notification server
    /// and a permission store holding `table`, on a private bus.
    struct Desk {
        _bus: crate::lockmode::tests::Bus,
        client: zbus::Connection,
        portal: zbus::Connection,
        fakes: zbus::Connection,
        sessions: Sessions,
        noted: Noted,
        closed: Closed,
        table: Table,
        /// Each request on a keys object: opcode and body.
        keys_heard: KeysHeard,
    }

    type KeysHeard = Arc<Mutex<Vec<(u32, Vec<u8>)>>>;

    impl Desk {
        async fn on(bus: crate::lockmode::tests::Bus, ask: Ask, table: &[(&str, &str)]) -> Desk {
            Desk::at(10, bus, ask, table).await
        }

        /// As [`Desk::on`], with the compositor's `reprise_input_v1` at `version`.
        async fn at(
            version: u32,
            bus: crate::lockmode::tests::Bus,
            ask: Ask,
            table: &[(&str, &str)],
        ) -> Desk {
            let (ours, mut theirs) = UnixStream::pair().unwrap();
            let keys_heard = KeysHeard::default();
            let heard = keys_heard.clone();
            std::thread::spawn(move || {
                const CREATE_KEYS: u32 = 4;
                const CREATE_APP_KEYS: u32 = 10;
                let input = fake_compositor(&mut theirs, 2, version);
                let mut keys = Vec::new();
                while let Ok((object, opcode, body)) = request(&mut theirs) {
                    if object == input && matches!(opcode, CREATE_KEYS | CREATE_APP_KEYS) {
                        keys.push(u32::from_ne_bytes(body[..4].try_into().unwrap()));
                    } else if keys.contains(&object) {
                        heard.lock().unwrap().push((opcode, body));
                    }
                }
            });
            let (noted, closed) = (Noted::default(), Closed::default());
            let table: Table = Arc::new(Mutex::new(
                (table.iter())
                    .map(|(app, id)| {
                        let data =
                            restore_data(app, &grant(DEVICE_KEYBOARD | DEVICE_POINTER, None));
                        (id.to_string(), Variant::from(data).try_to_owned().unwrap())
                    })
                    .collect(),
            ));
            let fakes = zbus::connection::Builder::address(bus.1.as_str())
                .name("org.freedesktop.Notifications")
                .serve_at(
                    "/org/freedesktop/Notifications",
                    FakeNotifications(noted.clone(), closed.clone()),
                )
                .build()
                .await
                .unwrap();
            fakes
                .request_name("org.freedesktop.impl.portal.PermissionStore")
                .await
                .unwrap();
            fakes
                .object_server()
                .at(
                    "/org/freedesktop/impl/portal/PermissionStore",
                    FakeStore(table.clone()),
                )
                .await
                .unwrap();
            let client = crate::lockmode::tests::connect(&bus).await;
            let frontend = crate::Frontend::default();
            frontend.set(client.unique_name().map(|n| n.to_string()));
            let sessions = Sessions::default();
            let notices = crate::remotenotice::Notices::default();
            let portal = zbus::connection::Builder::address(bus.1.as_str())
                .serve_at(
                    crate::PATH,
                    RemoteDesktop {
                        sessions: sessions.clone(),
                        link: Some(link_on(ours)),
                        frontend: frontend.clone(),
                        notices: notices.clone(),
                        ask,
                    },
                )
                .serve_at(
                    crate::PATH,
                    crate::ScreenCast {
                        sessions: sessions.clone(),
                        source_types: crate::SOURCE_MONITOR | crate::SOURCE_WINDOW,
                        frontend: frontend.clone(),
                        link: None,
                    },
                )
                .build()
                .await
                .unwrap();
            let _ = frontend.conn.set(portal.clone());
            let watch = crate::remotenotice::watch(portal.clone(), sessions.clone(), notices);
            zruntime::spawn(watch).detach();
            Desk {
                _bus: bus,
                client,
                portal,
                fakes,
                sessions,
                noted,
                closed,
                table,
                keys_heard,
            }
        }

        async fn call<B, R>(&self, method: &str, body: &B) -> R
        where
            B: serde::Serialize + zbus::zvariant::DynamicType,
            R: serde::de::DeserializeOwned + zbus::zvariant::Type,
        {
            self.call_on("RemoteDesktop", method, body).await
        }

        async fn call_on<B, R>(&self, iface: &str, method: &str, body: &B) -> R
        where
            B: serde::Serialize + zbus::zvariant::DynamicType,
            R: serde::de::DeserializeOwned + zbus::zvariant::Type,
        {
            (self.try_call(iface, method, body).await)
                .unwrap()
                .body()
                .deserialize()
                .unwrap()
        }

        async fn try_call<B>(
            &self,
            iface: &str,
            method: &str,
            body: &B,
        ) -> zbus::Result<zbus::Message>
        where
            B: serde::Serialize + zbus::zvariant::DynamicType,
        {
            let dest = self.portal.unique_name().unwrap().to_string();
            let iface = format!("org.freedesktop.impl.portal.{iface}");
            let iface = Some(iface.as_str());
            (self.client)
                .call_method(Some(dest.as_str()), crate::PATH, iface, method, body)
                .await
        }

        /// The keys object's requests, once `n` have come or after 5 s.
        async fn keys_heard(&self, n: usize) -> Vec<(u32, Vec<u8>)> {
            for _ in 0..500 {
                if self.keys_heard.lock().unwrap().len() >= n {
                    break;
                }
                async_io::Timer::after(Duration::from_millis(10)).await;
            }
            self.keys_heard.lock().unwrap().clone()
        }

        /// CreateSession, SelectDevices (with `restore` for a remembered grant) and Start for
        /// `app_id` as session `n`; Start's response code.
        async fn start(&self, n: u32, app_id: &str, restore: bool) -> u32 {
            let grant = restore.then(|| grant(3, None));
            let (handle, path) = self.select(n, app_id, grant.as_deref()).await;
            let none: HashMap<&str, Variant<'_>> = HashMap::new();
            let (code, _): (u32, HashMap<String, OwnedVariant>) = self
                .call("Start", &(&handle, &path, app_id, "", &none))
                .await;
            code
        }

        /// CreateSession and SelectDevices with persist mode 2 and a token for `grant`, if
        /// any, for `app_id` as session `n`; the request and session paths.
        async fn select(
            &self,
            n: u32,
            app_id: &str,
            grant: Option<&str>,
        ) -> (ObjectPath<'static>, ObjectPath<'static>) {
            let session = format!("/org/freedesktop/portal/desktop/session/1_99/s{n}");
            let request = format!("/org/freedesktop/portal/desktop/request/1_99/r{n}");
            let handle = ObjectPath::try_from(request).unwrap();
            let path = ObjectPath::try_from(session).unwrap();
            let none: HashMap<&str, Variant<'_>> = HashMap::new();
            let (code, _): (u32, HashMap<String, OwnedVariant>) = self
                .call("CreateSession", &(&handle, &path, app_id, &none))
                .await;
            assert_eq!(code, RESPONSE_OK);
            let mut options: HashMap<&str, Variant<'_>> = HashMap::new();
            options.insert("types", Variant::U32(DEVICE_KEYBOARD | DEVICE_POINTER));
            options.insert("persist_mode", Variant::U32(2));
            if let Some(grant) = grant {
                options.insert("restore_data", token(app_id, grant));
            }
            let (code, _): (u32, HashMap<String, OwnedVariant>) = self
                .call("SelectDevices", &(&handle, &path, app_id, &options))
                .await;
            assert_eq!(code, RESPONSE_OK);
            (handle, path)
        }

        /// Session `n`'s picture state: the target to restore and the persist mode the share
        /// picker sees.
        fn cast_state(&self, n: u32) -> (Option<Target>, u32) {
            let key = format!("/org/freedesktop/portal/desktop/session/1_99/s{n}");
            let map = self.sessions.lock().unwrap();
            let s = map.get(&key).and_then(SessionData::screencast).unwrap();
            (s.restore.clone(), s.persist_mode)
        }

        /// Close session `n` as the frontend does.
        async fn close(&self, n: u32) {
            let session = format!("/org/freedesktop/portal/desktop/session/1_99/s{n}");
            let dest = self.portal.unique_name().unwrap().to_string();
            let iface = Some("org.freedesktop.impl.portal.Session");
            (self.client)
                .call_method(Some(dest.as_str()), session.as_str(), iface, "Close", &())
                .await
                .unwrap();
        }

        /// Whether session `n` holds its balloon's id, within 5 s.
        async fn noticed(&self, n: u32) -> bool {
            let key = format!("/org/freedesktop/portal/desktop/session/1_99/s{n}");
            for _ in 0..500 {
                let held = (self.sessions.lock().unwrap().get_mut(&key))
                    .and_then(SessionData::remote_mut)
                    .is_some_and(|s| s.notice.is_some());
                if held {
                    return true;
                }
                async_io::Timer::after(Duration::from_millis(10)).await;
            }
            false
        }

        /// The balloons closed so far, once `n` have or after 5 s.
        async fn closes(&self, n: usize) -> Vec<u32> {
            for _ in 0..500 {
                if self.closed.lock().unwrap().len() >= n {
                    break;
                }
                async_io::Timer::after(Duration::from_millis(10)).await;
            }
            self.closed.lock().unwrap().clone()
        }

        fn session(&self, n: u32) -> Option<bool> {
            let key = format!("/org/freedesktop/portal/desktop/session/1_99/s{n}");
            let mut map = self.sessions.lock().unwrap();
            map.get_mut(&key)
                .and_then(SessionData::remote_mut)
                .map(|s| s.restored)
        }

        /// The balloons so far, once `n` have come or after 5 s.
        async fn balloons(&self, n: usize) -> Vec<Note> {
            for _ in 0..500 {
                if self.noted.lock().unwrap().len() >= n {
                    break;
                }
                async_io::Timer::after(Duration::from_millis(10)).await;
            }
            self.noted.lock().unwrap().clone()
        }
    }

    /// Critical and from Reprise, not the app, so neither the app's mute nor Do Not Disturb holds
    /// it, and it stays up past a lock for 30 s.
    fn controlled(app_id: &str) -> Note {
        Note {
            app_name: "Reprise".to_string(),
            replaces_id: 0,
            summary: format!("{app_id} is controlling this computer"),
            actions: vec!["forget".to_string(), "Stop and forget".to_string()],
            urgency: Some(2),
            desktop_entry: None,
            expire_timeout: 30_000,
        }
    }

    #[test]
    fn an_app_keeps_one_balloon_which_closes_with_its_last_session() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let desk = Desk::on(bus, never_asked, &[]).await;
            assert_eq!(desk.start(1, APP, true).await, RESPONSE_OK);
            assert!(desk.noticed(1).await);
            assert_eq!(desk.start(2, APP, true).await, RESPONSE_OK);
            assert!(desk.noticed(2).await);
            let second = Note {
                replaces_id: 41,
                ..controlled(APP)
            };
            assert_eq!(
                desk.balloons(2).await[1],
                second,
                "the restart reuses the balloon"
            );
            desk.close(1).await;
            async_io::Timer::after(Duration::from_millis(200)).await;
            let none: [u32; 0] = [];
            assert_eq!(
                *desk.closed.lock().unwrap(),
                none,
                "the second session still has it"
            );
            desk.close(2).await;
            assert_eq!(desk.closes(1).await, [41]);
        });
    }

    #[test]
    fn a_start_from_a_remembered_grant_shows_one_critical_balloon_from_reprise() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let desk = Desk::on(bus, never_asked, &[]).await;
            assert_eq!(desk.start(1, APP, true).await, RESPONSE_OK, "no dialog");
            assert_eq!(desk.balloons(1).await, [controlled(APP)]);
            async_io::Timer::after(Duration::from_millis(200)).await;
            assert_eq!(desk.balloons(2).await.len(), 1, "exactly one balloon");
        });
    }

    #[test]
    fn a_start_the_dialog_allowed_shows_no_balloon() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let desk = Desk::on(bus, allowed, &[]).await;
            assert_eq!(desk.start(1, APP, false).await, RESPONSE_OK);
            let other = "org.example.Other";
            assert_eq!(desk.start(2, other, true).await, RESPONSE_OK);
            assert_eq!(
                desk.balloons(1).await,
                [controlled(other)],
                "only the remembered start shows a balloon"
            );
        });
    }

    #[test]
    fn stop_and_forget_ends_the_session_and_deletes_the_grant() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let other = "org.example.Other";
            let desk = Desk::on(bus, never_asked, &[(APP, "token-a"), (other, "token-b")]).await;
            assert_eq!(desk.start(1, APP, true).await, RESPONSE_OK);
            assert!(desk.noticed(1).await);
            assert_eq!(desk.start(3, APP, true).await, RESPONSE_OK);
            assert!(desk.noticed(3).await);
            let emitter = zbus::object_server::SignalEmitter::new(
                &desk.fakes,
                "/org/freedesktop/Notifications",
            )
            .unwrap();
            for _ in 0..500 {
                if desk.session(1).is_none() && desk.session(3).is_none() {
                    break;
                }
                // What the server sends for a button: the click, then the balloon closing.
                let server = "org.freedesktop.Notifications";
                let click = (41u32, "forget");
                (emitter.emit(server, "ActionInvoked", &click))
                    .await
                    .unwrap();
                (emitter.emit(server, "NotificationClosed", &(41u32, 2u32)))
                    .await
                    .unwrap();
                async_io::Timer::after(Duration::from_millis(10)).await;
            }
            assert_eq!(desk.session(1), None, "the session ended");
            assert_eq!(desk.session(3), None, "and every other session of the app");
            for _ in 0..500 {
                if !desk.table.lock().unwrap().contains_key("token-a") {
                    break;
                }
                async_io::Timer::after(Duration::from_millis(10)).await;
            }
            let left: Vec<String> = desk.table.lock().unwrap().keys().cloned().collect();
            assert_eq!(left, ["token-b"], "only the app's own grant is deleted");
            assert_eq!(desk.start(2, APP, true).await, RESPONSE_CANCELLED);
            assert_eq!(
                desk.session(2),
                Some(false),
                "a token the frontend still holds restores nothing and the dialog asks"
            );
        });
    }

    #[test]
    fn a_remembered_grant_restores_its_screen_past_select_sources() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let desk = Desk::on(bus, never_asked, &[]).await;
            let screen = Target::Output("DP-1".into());
            let remembered = grant(DEVICE_KEYBOARD | DEVICE_POINTER, Some(&screen));
            let (handle, path) = desk.select(1, APP, Some(&remembered)).await;
            assert_eq!(desk.cast_state(1).0, Some(screen.clone()));
            let mut options: HashMap<&str, Variant<'_>> = HashMap::new();
            options.insert("types", Variant::U32(crate::SOURCE_MONITOR));
            let (code, _): (u32, HashMap<String, OwnedVariant>) = desk
                .call_on(
                    "ScreenCast",
                    "SelectSources",
                    &(&handle, &path, APP, &options),
                )
                .await;
            assert_eq!(code, RESPONSE_OK);
            assert_eq!(
                desk.cast_state(1).0,
                Some(screen),
                "SelectSources without a token keeps the screen"
            );
        });
    }

    #[test]
    fn the_share_picker_offers_remember_when_the_grant_is_remembered() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let desk = Desk::on(bus, allowed, &[]).await;
            assert_eq!(desk.start(1, APP, false).await, RESPONSE_OK);
            assert_eq!(desk.cast_state(1).1, 2, "Remember ticked in the dialog");
            assert_eq!(desk.start(2, APP, true).await, RESPONSE_OK);
            assert_eq!(desk.cast_state(2).1, 2, "a restored grant");
            let desk_once =
                Desk::on(crate::lockmode::tests::bus().unwrap(), allowed_once, &[]).await;
            assert_eq!(desk_once.start(1, APP, false).await, RESPONSE_OK);
            assert_eq!(desk_once.cast_state(1).1, 0, "allowed without Remember");
        });
    }

    fn error_name(r: zbus::Result<zbus::Message>) -> String {
        match r {
            Err(zbus::Error::FDO(e)) => zbus::DBusError::name(&*e).to_string(),
            other => panic!("expected a D-Bus error, got {other:?}"),
        }
    }

    #[test]
    fn a_granted_pointer_moves_clicks_and_scrolls_through_notify() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let desk = Desk::at(12, bus, allowed, &[]).await;
            assert_eq!(desk.start(1, APP, false).await, RESPONSE_OK);
            let s =
                ObjectPath::try_from("/org/freedesktop/portal/desktop/session/1_99/s1").unwrap();
            let none: HashMap<&str, Variant<'_>> = HashMap::new();
            let rd = "RemoteDesktop";
            let calls = [
                desk.try_call(rd, "NotifyPointerMotion", &(&s, &none, 1.5f64, -2.0f64))
                    .await,
                desk.try_call(rd, "NotifyPointerButton", &(&s, &none, 0x110i32, 1u32))
                    .await,
                desk.try_call(rd, "NotifyPointerAxis", &(&s, &none, 0.0f64, 10.0f64))
                    .await,
                desk.try_call(rd, "NotifyPointerAxisDiscrete", &(&s, &none, 1u32, -1i32))
                    .await,
            ];
            assert!(calls.iter().all(Result::is_ok), "{calls:?}");
            let words = |ws: &[i32]| ws.iter().flat_map(|w| w.to_ne_bytes()).collect::<Vec<u8>>();
            assert_eq!(
                desk.keys_heard(4).await,
                [
                    (3, words(&[384, -512])),
                    (5, words(&[0x110, 1])),
                    (6, words(&[0, 2560])),
                    (7, words(&[1, -1])),
                ],
                "motion, button, axis and wheel on the keys object"
            );
            let absolute = (&s, &none, 77u32, 10.0f64, 10.0f64);
            let r = desk
                .try_call(rd, "NotifyPointerMotionAbsolute", &absolute)
                .await;
            assert_eq!(
                error_name(r),
                "org.freedesktop.DBus.Error.InvalidArgs",
                "no such stream"
            );
            let key = s.to_string();
            (desk.sessions.lock().unwrap().get_mut(&key))
                .and_then(SessionData::remote_mut)
                .unwrap()
                .devices = DEVICE_KEYBOARD;
            let r = desk
                .try_call(rd, "NotifyPointerMotion", &(&s, &none, 1.0f64, 1.0f64))
                .await;
            assert_eq!(
                error_name(r),
                "org.freedesktop.DBus.Error.Failed",
                "keyboard only"
            );
        });
    }

    #[test]
    fn before_v12_notify_pointer_is_not_supported() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let desk = Desk::at(11, bus, allowed, &[]).await;
            assert_eq!(desk.start(1, APP, false).await, RESPONSE_OK);
            let s =
                ObjectPath::try_from("/org/freedesktop/portal/desktop/session/1_99/s1").unwrap();
            let none: HashMap<&str, Variant<'_>> = HashMap::new();
            let motion = (&s, &none, 1.0f64, 1.0f64);
            let r = desk
                .try_call("RemoteDesktop", "NotifyPointerMotion", &motion)
                .await;
            assert_eq!(error_name(r), "org.freedesktop.DBus.Error.NotSupported");
        });
    }

    fn token(app_id: &str, grant: &str) -> Variant<'static> {
        Variant::from(restore_data(app_id, grant))
    }

    #[test]
    fn only_our_own_restore_data_skips_the_dialog() {
        let ours = token(APP, &grant(DEVICE_POINTER, None));
        assert_eq!(our_grant(&ours, APP), Some((DEVICE_POINTER, None)));
        let unasked = Variant::from((
            RESTORE_VENDOR.to_string(),
            2u32,
            Variant::from(format!("{RESTORE_TAG}:{DEVICE_POINTER}"))
                .try_to_owned()
                .unwrap(),
        ));
        assert_eq!(our_grant(&unasked, APP), None);
        let theirs = Variant::from((
            "someone-else".to_string(),
            crate::RESTORE_VERSION,
            Variant::from(RESTORE_TAG).try_to_owned().unwrap(),
        ));
        assert_eq!(our_grant(&theirs, APP), None);
        assert_eq!(our_grant(&token(APP, "output:Virtual-1"), APP), None);
    }

    #[test]
    fn a_grant_restores_only_for_the_app_it_was_issued_to() {
        let ours = token(APP, &grant(DEVICE_KEYBOARD, None));
        assert_eq!(our_grant(&ours, APP), Some((DEVICE_KEYBOARD, None)));
        assert_eq!(our_grant(&ours, "org.example.Other"), None);
        assert_eq!(our_grant(&ours, ""), None);
    }

    #[test]
    fn a_grant_carries_the_picture_it_was_remembered_with() {
        let screen = Target::Output("DP-1".into());
        let both = grant(DEVICE_KEYBOARD | DEVICE_POINTER, Some(&screen));
        assert_eq!(
            parse_grant(&both),
            Some((DEVICE_KEYBOARD | DEVICE_POINTER, Some(screen)))
        );
        assert_eq!(
            parse_grant(&grant(DEVICE_POINTER, None)),
            Some((DEVICE_POINTER, None))
        );
        assert_eq!(parse_grant("output:DP-1"), None);
        assert_eq!(parse_grant("remote-desktop:3;junk"), None);
    }
}
