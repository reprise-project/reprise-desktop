//! `org.freedesktop.impl.portal.RemoteDesktop` v2: input injection granted through the permit
//! dialog (remembered by restore token only when the user ticks Remember), served over libei via
//! ConnectToEIS. Keycode and keysym Notify methods go through the session's `reprise_keys_v1`;
//! pointer and touch are left to libei.

use crate::inputlink::{Link, live};
use crate::inputproto::reprise_input_v1::Device;
use crate::wayland::Target;
use crate::{
    CastState, RESPONSE_CANCELLED, RESPONSE_OK, RESPONSE_OTHER, Request, SessionData, SessionKind,
    Sessions,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::zvariant::{ObjectPath, OwnedValue, SerializeDict, Type, Value};

pub const DEVICE_KEYBOARD: u32 = Device::Keyboard as u32;
pub const DEVICE_POINTER: u32 = Device::Pointer as u32;
const RESTORE_TAG: &str = "remote-desktop";

#[derive(Default)]
pub(crate) struct RemoteSession {
    devices: u32,
    persist: u32,
    restored: bool,
    granted: bool,
    eis: Option<crate::inputlink::EisHandle>,
    keys: Option<crate::inputlink::KeysHandle>,
    pub(crate) screencast: CastState,
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
    restore_data: Option<(String, u32, OwnedValue)>,
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
        _options: HashMap<String, Value<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        self.frontend.check(&hdr)?;
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
        options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        self.frontend.check(&hdr)?;
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
        options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u32, StartResults)> {
        self.frontend.check(&hdr)?;
        if live(&self.link).is_none() {
            return Ok((RESPONSE_OTHER, empty()));
        }
        let key = session_handle.to_string();
        let parent = crate::dialog::parent_handle(&parent_window);
        let (devices, restored, mut requested) = {
            let mut sessions = self.sessions.lock().unwrap();
            let Some(s) = sessions.get_mut(&key).and_then(SessionData::remote_mut) else {
                return Ok((RESPONSE_OTHER, empty()));
            };
            (s.devices, s.restored, s.persist)
        };
        if let Some(p) = options
            .get("persist_mode")
            .and_then(|v| u32::try_from(v).ok())
        {
            requested = p;
        }
        let mut remember = restored;
        if !restored {
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
        match types {
            None => return Ok((RESPONSE_CANCELLED, empty())),
            Some(0) => {}
            Some(_) => {
                match crate::pick_and_cast(
                    &self.sessions,
                    &self.frontend,
                    server,
                    conn,
                    &handle,
                    &key,
                    &app_id,
                    parent,
                )
                .await
                {
                    Ok(st) => {
                        streams.push(st.stream);
                        kept = (st.persist_mode > 0).then_some(st.target);
                    }
                    Err(code) => return Ok((code, empty())),
                }
            }
        }
        log::info!(
            "RemoteDesktop.Start for {app_id:?}: devices {devices}, persist {persist}, streams {}",
            streams.len()
        );
        match (self.sessions.lock().unwrap().get_mut(&key)).and_then(SessionData::remote_mut) {
            Some(s) => {
                s.granted = true;
                if devices & DEVICE_KEYBOARD != 0 {
                    s.keys = live(&self.link).and_then(|l| l.create_keys());
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
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        self.frontend.check(&hdr)?;
        let Some(link) = live(&self.link) else {
            return Err(zbus::fdo::Error::Failed("no input link".into()));
        };
        let mut sessions = self.sessions.lock().unwrap();
        let Some(s) = sessions
            .get_mut(session_handle.as_str())
            .and_then(SessionData::remote_mut)
            .filter(|s| s.granted)
        else {
            return Err(zbus::fdo::Error::Failed("no such session".into()));
        };
        let (ours, theirs) = crate::inputlink::socketpair()?;
        s.eis = Some(link.connect_sender(ours, s.devices));
        log::info!("RemoteDesktop.ConnectToEIS for {app_id:?}");
        Ok(theirs.into())
    }

    async fn notify_pointer_motion(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        _dx: f64,
        _dy: f64,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_pointer_motion_absolute(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        _stream: u32,
        _x: f64,
        _y: f64,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_pointer_button(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        _button: i32,
        _state: u32,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_pointer_axis(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        _dx: f64,
        _dy: f64,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_pointer_axis_discrete(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        _axis: u32,
        _steps: i32,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
    async fn notify_keyboard_keycode(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        keycode: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr)?;
        self.with_keys(&session_handle, |k| k.key(keycode as u32, state != 0))
    }
    async fn notify_keyboard_keysym(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        keysym: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr)?;
        self.with_keys(&session_handle, |k| k.keysym(keysym as u32, state != 0))
    }
    async fn notify_touch_down(
        &self,
        _s: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
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
        _o: HashMap<String, Value<'_>>,
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
        _o: HashMap<String, Value<'_>>,
        _slot: u32,
    ) -> zbus::fdo::Result<()> {
        Err(unsupported())
    }
}

impl RemoteDesktop {
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

fn our_grant(v: &Value<'_>, app_id: &str) -> Option<u32> {
    let data = <(String, u32, OwnedValue)>::try_from(v.try_to_owned().ok()?).ok()?;
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
    use crate::{RESTORE_VENDOR, restore_data};

    const APP: &str = "org.example.Remote";

    fn token(app_id: &str, grant: &str) -> Value<'static> {
        Value::from(restore_data(app_id, grant))
    }

    #[test]
    fn only_our_own_restore_data_skips_the_dialog() {
        let ours = token(APP, &grant(DEVICE_POINTER, None));
        assert_eq!(our_grant(&ours, APP), Some(DEVICE_POINTER));
        let unasked = Value::from((
            RESTORE_VENDOR.to_string(),
            2u32,
            Value::from(format!("{RESTORE_TAG}:{DEVICE_POINTER}"))
                .try_to_owned()
                .unwrap(),
        ));
        assert_eq!(our_grant(&unasked, APP), None);
        let theirs = Value::from((
            "someone-else".to_string(),
            crate::RESTORE_VERSION,
            Value::from(RESTORE_TAG).try_to_owned().unwrap(),
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
