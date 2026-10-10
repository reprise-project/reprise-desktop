//! `org.freedesktop.impl.portal.RemoteDesktop` v2: input injection granted through the permit
//! dialog (remembered by restore token only when the user ticks Remember), served over libei via
//! ConnectToEIS. Keycode and keysym Notify methods go through the session's `reprise_keys_v1`;
//! pointer and touch are left to libei.

use crate::inputlink::{Link, live};
use crate::inputproto::reprise_input_v1::Device;
use crate::{
    CastState, RESPONSE_CANCELLED, RESPONSE_OK, RESPONSE_OTHER, RESTORE_VENDOR, RESTORE_VERSION,
    Request, SessionData, SessionKind, Sessions,
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
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
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
        let restored = options.get("restore_data").and_then(our_grant);
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
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u32, StartResults)> {
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
            crate::export_request(server, &handle, cancel.clone()).await?;
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
        match types {
            None => return Ok((RESPONSE_CANCELLED, empty())),
            Some(0) => {}
            Some(_) => {
                match crate::pick_and_cast(
                    &self.sessions,
                    server,
                    conn,
                    &handle,
                    &key,
                    &app_id,
                    parent,
                )
                .await
                {
                    Ok(st) => streams.push(st.stream),
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
        let restore_data = (persist > 0).then(|| {
            (
                RESTORE_VENDOR.to_string(),
                RESTORE_VERSION,
                Value::from(restore_payload(devices))
                    .try_to_owned()
                    .expect("string"),
            )
        });
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
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
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
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        keycode: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.with_keys(&session_handle, |k| k.key(keycode as u32, state != 0))
    }
    async fn notify_keyboard_keysym(
        &self,
        session_handle: ObjectPath<'_>,
        _o: HashMap<String, Value<'_>>,
        keysym: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
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

fn restore_payload(devices: u32) -> String {
    format!("{RESTORE_TAG}:{devices}")
}

fn our_grant(v: &Value<'_>) -> Option<u32> {
    let owned = v.try_to_owned().ok()?;
    let (vendor, version, payload) = <(String, u32, OwnedValue)>::try_from(owned).ok()?;
    if vendor != RESTORE_VENDOR || version != RESTORE_VERSION {
        return None;
    }
    let payload = as_str(&payload)?;
    let mask = payload.strip_prefix(RESTORE_TAG)?.strip_prefix(':')?;
    mask.parse::<u32>()
        .ok()
        .map(|m| m & (DEVICE_KEYBOARD | DEVICE_POINTER))
}

fn as_str<'a>(v: &'a Value<'a>) -> Option<&'a str> {
    match v {
        Value::Str(s) => Some(s.as_str()),
        Value::Value(inner) => as_str(inner),
        _ => None,
    }
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

    #[test]
    fn only_our_own_restore_data_skips_the_dialog() {
        let ours = Value::from((
            RESTORE_VENDOR.to_string(),
            RESTORE_VERSION,
            Value::from(restore_payload(DEVICE_POINTER))
                .try_to_owned()
                .unwrap(),
        ));
        assert_eq!(our_grant(&ours), Some(DEVICE_POINTER));
        let unasked = Value::from((
            RESTORE_VENDOR.to_string(),
            1u32,
            Value::from(restore_payload(DEVICE_POINTER))
                .try_to_owned()
                .unwrap(),
        ));
        assert_eq!(our_grant(&unasked), None);
        let theirs = Value::from((
            "someone-else".to_string(),
            RESTORE_VERSION,
            Value::from(RESTORE_TAG).try_to_owned().unwrap(),
        ));
        assert_eq!(our_grant(&theirs), None);
        let cast = Value::from((
            RESTORE_VENDOR.to_string(),
            RESTORE_VERSION,
            Value::from("output:Virtual-1").try_to_owned().unwrap(),
        ));
        assert_eq!(our_grant(&cast), None);
    }
}
