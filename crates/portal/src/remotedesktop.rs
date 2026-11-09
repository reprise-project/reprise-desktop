//! `org.freedesktop.impl.portal.RemoteDesktop` v2 over libei: input granted by the permit dialog,
//! or without it to the enabled server (`trusted`); keycode and keysym Notify use reprise_keys_v1.

use crate::inputlink::{Link, LockScreenHandle, Owner, live};
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
    app_id: String,
    eis: Option<crate::inputlink::EisHandle>,
    keys: Option<crate::inputlink::KeysHandle>,
    pub(crate) screencast: CastState,
    /// Held while the session casts lock mode's screens (`crate::lockmode`).
    pub(crate) lock: Option<Arc<crate::inputlink::LockScreenHandle>>,
}

pub struct RemoteDesktop {
    pub sessions: Sessions,
    /// None when the compositor offered no `reprise_input_v1`; every method then fails rather than
    /// pretending to grant.
    pub link: Option<Link>,
    pub frontend: crate::Frontend,
}

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
        let restored = (options.get("restore_data")).and_then(|v| our_grant(v, &app_id));
        if let Some(granted) = restored {
            s.devices &= granted;
        }
        s.restored = restored.is_some();
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
            let answer = blocking::unblock(move || {
                crate::permit::ask(
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
                crate::permit::Answer::Allow { remember: r } => remember = r,
                crate::permit::Answer::Deny | crate::permit::Answer::Dismissed => {
                    log::info!("RemoteDesktop.Start for {app_id:?} denied");
                    return Ok((RESPONSE_CANCELLED, empty()));
                }
            }
        }
        let persist = crate::granted_persist(requested, remember);
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
        _s: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        _dx: f64,
        _dy: f64,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_pointer_motion_absolute(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        _stream: u32,
        _x: f64,
        _y: f64,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_pointer_button(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        _button: i32,
        _state: u32,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_pointer_axis(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        _dx: f64,
        _dy: f64,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_pointer_axis_discrete(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Variant<'_>>,
        _axis: u32,
        _steps: i32,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
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
        self.with_keys(&session_handle, |k| k.key(keycode as u32, state != 0))
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
        self.with_keys(&session_handle, |k| k.keysym(keysym as u32, state != 0))
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
    /// keyboard was granted and lock mode has not already ended.
    pub(crate) fn grant(&mut self, link: Option<&Link>, lock: Option<Arc<LockScreenHandle>>) {
        self.granted = true;
        self.lock = lock;
        if self.devices & DEVICE_KEYBOARD != 0 && !self.lock_ended() {
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

    fn with_keys(
        &self,
        session_handle: &ObjectPath<'_>,
        f: impl FnOnce(&crate::inputlink::KeysHandle),
    ) -> zbus::fdo::Result<()> {
        if live(&self.link).is_none() {
            return Err(zbus::fdo::Error::Failed("no input link".into()));
        }
        let mut sessions = self.sessions.lock().unwrap();
        let keys = sessions
            .get_mut(session_handle.as_str())
            .and_then(SessionData::remote_mut)
            .and_then(|s| s.keys.as_ref());
        match keys {
            Some(k) => {
                f(k);
                Ok(())
            }
            None => Err(zbus::fdo::Error::Failed(
                "no session granted the keyboard".into(),
            )),
        }
    }
}

fn unsupported() -> zbus::fdo::Error {
    zbus::fdo::Error::NotSupported("use ConnectToEIS".into())
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

fn our_grant(v: &Variant<'_>, app_id: &str) -> Option<u32> {
    let data = <(String, u32, OwnedVariant)>::try_from(v.try_to_owned().ok()?).ok()?;
    parse_grant(crate::restored_grant(&data, app_id)?).map(|(mask, _)| mask)
}

pub(crate) fn display_name(app_id: &str) -> String {
    if app_id.is_empty() {
        return String::new();
    }
    let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
    apps.iter()
        .find(|a| a.id == app_id || a.id.strip_suffix(".desktop") == Some(app_id))
        .map(|a| a.name.clone())
        .unwrap_or_else(|| app_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputlink::tests::{fake_compositor, link_on, request, send};
    use crate::{RESTORE_VENDOR, restore_data};
    use std::os::unix::net::UnixStream;
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

    fn token(app_id: &str, grant: &str) -> Variant<'static> {
        Variant::from(restore_data(app_id, grant))
    }

    #[test]
    fn only_our_own_restore_data_skips_the_dialog() {
        let ours = token(APP, &grant(DEVICE_POINTER, None));
        assert_eq!(our_grant(&ours, APP), Some(DEVICE_POINTER));
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
        assert_eq!(our_grant(&ours, APP), Some(DEVICE_KEYBOARD));
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
