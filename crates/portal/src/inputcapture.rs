//! `org.freedesktop.impl.portal.InputCapture` version 1. Barriers are checked against the zones here
//! (the frontend wants failed ids back at once). Sessions start at once, so CreateSession asks the
//! user through the permit prompt first.

use crate::inputlink::{Barrier, CaptureEvent, CaptureHandle, Link, Zone, live};
use crate::{RESPONSE_OK, RESPONSE_OTHER, SessionData, SessionKind, Sessions};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

pub const CAP_KEYBOARD: u32 = 1;
pub const CAP_POINTER: u32 = 2;

/// The barriers one capture holds. The compositor ends the whole `reprise_input_v1` with `too_many`
/// past its `inputcapture::MAX_BARRIERS`, so the two must stay equal (or this lower).
pub const MAX_BARRIERS: usize = 64;

pub(crate) struct CaptureSession {
    capture: CaptureHandle,
    eis: Option<crate::inputlink::EisHandle>,
    _relay: zbus::Task<()>,
}

pub struct InputCapture {
    pub sessions: Sessions,
    /// None when the compositor offered no `reprise_input_v1`. Every
    /// method then fails rather than pretending to capture anything, as it
    /// does once the link's connection is lost.
    pub link: Option<Link>,
}

pub fn validate(barriers: &[Barrier], zones: &[Zone]) -> Vec<u32> {
    barriers
        .iter()
        .filter(|b| !barrier_ok(b, zones))
        .map(|b| b.id)
        .collect()
}

/// The barriers to forward and the ids that failed: the invalid ones and any past `MAX_BARRIERS`.
pub fn accept(barriers: &[Barrier], zones: &[Zone]) -> (Vec<Barrier>, Vec<u32>) {
    let mut failed = validate(barriers, zones);
    let mut good: Vec<Barrier> = barriers
        .iter()
        .filter(|b| !failed.contains(&b.id))
        .copied()
        .collect();
    if good.len() > MAX_BARRIERS {
        failed.extend(good.split_off(MAX_BARRIERS).iter().map(|b| b.id));
    }
    (good, failed)
}

fn barrier_ok(b: &Barrier, zones: &[Zone]) -> bool {
    let vertical = b.x1 == b.x2;
    let horizontal = b.y1 == b.y2;
    if !(vertical || horizontal) {
        return false;
    }
    // The zone the barrier lies within (inclusive extents, on its edge).
    let Some(z) = zones.iter().find(|z| within(b, z)) else {
        return false;
    };
    let (zx2, zy2) = (z.x + z.width as i32, z.y + z.height as i32);
    if vertical {
        let x = b.x1;
        let (y1, y2) = (b.y1.min(b.y2), b.y1.max(b.y2));
        let on_edge = x == z.x || x == zx2;
        // Outer means no zone sits on the far side of that edge along this span.
        let far = if x == z.x { x - 1 } else { x };
        on_edge
            && y1 >= z.y
            && y2 < zy2
            && !zones
                .iter()
                .any(|o| covers(o, far, y1) || covers(o, far, y2))
    } else {
        let y = b.y1;
        let (x1, x2) = (b.x1.min(b.x2), b.x1.max(b.x2));
        let on_edge = y == z.y || y == zy2;
        let far = if y == z.y { y - 1 } else { y };
        on_edge
            && x1 >= z.x
            && x2 < zx2
            && !zones
                .iter()
                .any(|o| covers(o, x1, far) || covers(o, x2, far))
    }
}

fn within(b: &Barrier, z: &Zone) -> bool {
    let (zx2, zy2) = (z.x + z.width as i32, z.y + z.height as i32);
    let xs = [b.x1, b.x2];
    let ys = [b.y1, b.y2];
    xs.iter().all(|&x| x >= z.x && x <= zx2) && ys.iter().all(|&y| y >= z.y && y <= zy2)
}

fn covers(z: &Zone, x: i32, y: i32) -> bool {
    x >= z.x && x < z.x + z.width as i32 && y >= z.y && y < z.y + z.height as i32
}

#[zbus::interface(name = "org.freedesktop.impl.portal.InputCapture")]
impl InputCapture {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    #[zbus(property)]
    fn supported_capabilities(&self) -> u32 {
        CAP_KEYBOARD | CAP_POINTER
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_session(
        &self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: HashMap<String, Value<'_>>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        crate::check_session(&session_handle)?;
        let Some(link) = live(&self.link) else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        let wanted = options
            .get("capabilities")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let caps = wanted & (CAP_KEYBOARD | CAP_POINTER);
        if caps == 0 {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        }
        log::info!(
            "InputCapture.CreateSession {} for {app_id:?}: capabilities {caps}",
            session_handle.as_str()
        );
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone()).await?;
        let id = app_id.clone();
        let parent = crate::dialog::parent_handle(&parent_window);
        let ok = blocking::unblock(move || {
            let kind = crate::permit::Kind::InputCapture(crate::permit::Devices::from_mask(caps));
            crate::permit::ask(kind, &id, Vec::new(), false, parent, cancel).allowed()
        })
        .await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        if !ok {
            log::info!("InputCapture.CreateSession for {app_id:?} denied");
            return Ok((crate::RESPONSE_CANCELLED, HashMap::new()));
        }
        let Some(capture) = link.create_capture() else {
            log::warn!(
                "InputCapture.CreateSession for {app_id:?}: {} captures are live already",
                crate::inputlink::MAX_CAPTURES
            );
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        let events = capture.events();
        let path: OwnedObjectPath = session_handle.clone().into();
        // Relay the compositor's events as portal signals for as long as
        // the session lives; the task dies with the session's data.
        let task = conn.executor().spawn(
            relay(conn.clone(), path.clone(), events),
            "input-capture-relay",
        );
        crate::replace_session(
            &self.sessions,
            session_handle.to_string(),
            SessionData::new(
                crate::sender(&hdr),
                SessionKind::InputCapture(CaptureSession {
                    capture,
                    eis: None,
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
        let mut r = HashMap::new();
        r.insert(
            "capabilities".into(),
            Value::from(caps).try_to_owned().expect("u32"),
        );
        Ok((RESPONSE_OK, r))
    }

    async fn get_zones(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let Some(link) = live(&self.link) else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        // Only a capture session has zones to report: a RemoteDesktop
        // session on the same map is not one.
        if !self
            .sessions
            .lock()
            .unwrap()
            .get(session_handle.as_str())
            .is_some_and(|s| s.capture().is_some())
        {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        }
        let (zones, set) = link.zones();
        let tuples: Vec<(u32, u32, i32, i32)> = zones
            .iter()
            .map(|z| (z.width, z.height, z.x, z.y))
            .collect();
        log::info!(
            "InputCapture.GetZones {}: {tuples:?} set {set}",
            session_handle.as_str()
        );
        let mut r = HashMap::new();
        r.insert(
            "zones".into(),
            Value::from(tuples).try_to_owned().expect("zones"),
        );
        r.insert(
            "zone_set".into(),
            Value::from(set).try_to_owned().expect("u32"),
        );
        Ok((RESPONSE_OK, r))
    }

    async fn set_pointer_barriers(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: HashMap<String, Value<'_>>,
        barriers: Vec<HashMap<String, Value<'_>>>,
        zone_set: u32,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let Some(link) = live(&self.link) else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        let (zones, current) = link.zones();
        let parsed: Vec<Barrier> = barriers
            .iter()
            .filter_map(|b| {
                let id = u32::try_from(b.get("barrier_id")?).ok()?;
                let (x1, y1, x2, y2) =
                    <(i32, i32, i32, i32)>::try_from(b.get("position")?.try_to_owned().ok()?)
                        .ok()?;
                Some(Barrier { id, x1, y1, x2, y2 })
            })
            .collect();
        // A stale zone set means the client drew its barriers against a
        // layout that is gone: every one of them fails.
        let (good, failed) = if zone_set != current {
            (Vec::new(), parsed.iter().map(|b| b.id).collect())
        } else {
            accept(&parsed, &zones)
        };
        let sessions = self.sessions.lock().unwrap();
        let Some(c) = sessions
            .get(session_handle.as_str())
            .and_then(SessionData::capture)
            .map(|s| &s.capture)
        else {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        };
        c.set_barriers(&good);
        log::info!(
            "InputCapture.SetPointerBarriers {}: {} accepted, {:?} failed",
            session_handle.as_str(),
            good.len(),
            failed
        );
        let mut r = HashMap::new();
        r.insert(
            "failed_barriers".into(),
            Value::from(failed).try_to_owned().expect("au"),
        );
        Ok((RESPONSE_OK, r))
    }

    async fn enable(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        log::info!("InputCapture.Enable {}", session_handle.as_str());
        self.with_capture(&session_handle, |c| c.enable())
    }

    async fn disable(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        log::info!("InputCapture.Disable {}", session_handle.as_str());
        self.with_capture(&session_handle, |c| c.disable())
    }

    async fn release(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let id = options
            .get("activation_id")
            .and_then(|v| u32::try_from(v).ok());
        let pos = options
            .get("cursor_position")
            .and_then(|v| <(f64, f64)>::try_from(v.try_to_owned().ok()?).ok());
        log::info!(
            "InputCapture.Release {} activation {id:?} at {pos:?}",
            session_handle.as_str()
        );
        self.with_capture(&session_handle, |c| c.release(id, pos))
    }

    // zbus would name this ConnectToEis; the interface spells it EIS.
    #[zbus(name = "ConnectToEIS")]
    async fn connect_to_eis(
        &self,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        if live(&self.link).is_none() {
            return Err(zbus::fdo::Error::Failed("no input link".into()));
        }
        let mut sessions = self.sessions.lock().unwrap();
        let Some(s) = sessions.get_mut(session_handle.as_str()) else {
            return Err(zbus::fdo::Error::Failed("no such session".into()));
        };
        let Some(s) = s.capture_mut() else {
            return Err(zbus::fdo::Error::Failed("not a capture session".into()));
        };
        let (ours, theirs) = crate::inputlink::socketpair()?;
        s.eis = Some(s.capture.connect_receiver(ours));
        log::info!("InputCapture.ConnectToEIS for {app_id:?}");
        Ok(theirs.into())
    }

    #[zbus(signal)]
    async fn disabled(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn activated(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn deactivated(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn zones_changed(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;
}

impl InputCapture {
    fn with_capture(
        &self,
        session: &ObjectPath<'_>,
        f: impl FnOnce(&CaptureHandle),
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        if live(&self.link).is_none() {
            return Ok((RESPONSE_OTHER, HashMap::new()));
        }
        let sessions = self.sessions.lock().unwrap();
        match sessions
            .get(session.as_str())
            .and_then(SessionData::capture)
            .map(|s| &s.capture)
        {
            Some(c) => {
                f(c);
                Ok((RESPONSE_OK, HashMap::new()))
            }
            None => Ok((RESPONSE_OTHER, HashMap::new())),
        }
    }
}

async fn relay(
    conn: zbus::Connection,
    session: OwnedObjectPath,
    events: async_channel::Receiver<CaptureEvent>,
) {
    let emitter = match SignalEmitter::new(&conn, crate::PATH) {
        Ok(e) => e,
        Err(_) => return,
    };
    while let Ok(ev) = events.recv().await {
        let mut opts: HashMap<String, OwnedValue> = HashMap::new();
        let r = match ev {
            CaptureEvent::Activated {
                activation_id,
                barrier_id,
                x,
                y,
            } => {
                opts.insert(
                    "activation_id".into(),
                    Value::from(activation_id).try_to_owned().unwrap(),
                );
                opts.insert(
                    "barrier_id".into(),
                    Value::from(barrier_id).try_to_owned().unwrap(),
                );
                opts.insert(
                    "cursor_position".into(),
                    Value::from((x, y)).try_to_owned().unwrap(),
                );
                InputCapture::activated(&emitter, session.clone(), opts).await
            }
            CaptureEvent::Deactivated { activation_id } => {
                opts.insert(
                    "activation_id".into(),
                    Value::from(activation_id).try_to_owned().unwrap(),
                );
                InputCapture::deactivated(&emitter, session.clone(), opts).await
            }
            CaptureEvent::ZonesChanged => {
                // The compositor cleared the barriers and disabled the
                // capture with the layout: say both, in the spec's order.
                let _ =
                    InputCapture::zones_changed(&emitter, session.clone(), HashMap::new()).await;
                InputCapture::disabled(&emitter, session.clone(), HashMap::new()).await
            }
        };
        if let Err(e) = r {
            log::warn!("InputCapture signal: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputlink::{Barrier, Zone};

    fn two_screens() -> Vec<Zone> {
        vec![
            Zone {
                width: 1920,
                height: 1080,
                x: 0,
                y: 0,
            },
            Zone {
                width: 1920,
                height: 1080,
                x: 1920,
                y: 0,
            },
        ]
    }
    fn b(id: u32, x1: i32, y1: i32, x2: i32, y2: i32) -> Barrier {
        Barrier { id, x1, y1, x2, y2 }
    }

    #[test]
    fn the_portal_specs_examples_pass() {
        let z = two_screens();
        let ok = [
            b(1, 0, 0, 1919, 0),       // top of left
            b(2, 0, 1080, 1919, 1080), // bottom of left
            b(3, 1920, 0, 3839, 0),    // top of right
            b(4, 1920, 1080, 3839, 1080),
            b(5, 0, 0, 0, 1079),       // left of left
            b(6, 3840, 0, 3840, 1079), // right of right
        ];
        assert_eq!(validate(&ok, &z), Vec::<u32>::new());
    }

    #[test]
    fn an_inner_edge_a_diagonal_and_a_span_across_two_zones_fail() {
        let z = two_screens();
        let bad = [
            b(7, 1920, 0, 1920, 1079), // seam between the screens
            b(8, 0, 0, 100, 100),      // diagonal
            b(9, 0, 0, 3839, 0),       // top edge across both zones
            b(10, 0, 500, 1919, 500),  // inside a zone
        ];
        assert_eq!(validate(&bad, &z), vec![7, 8, 9, 10]);
    }

    #[test]
    fn barriers_past_the_compositors_limit_fail() {
        let z = two_screens();
        let many: Vec<Barrier> = (0..70)
            .map(|i| b(i, i as i32 * 10, 0, i as i32 * 10 + 5, 0))
            .collect();
        let (good, failed) = accept(&many, &z);
        assert_eq!(good.len(), MAX_BARRIERS, "64 barriers are forwarded");
        assert_eq!(
            failed,
            (64..70).collect::<Vec<u32>>(),
            "the last 6 ids fail"
        );
    }

    #[test]
    fn nothing_is_valid_without_zones() {
        assert_eq!(validate(&[b(1, 0, 0, 10, 0)], &[]), vec![1]);
    }
}
