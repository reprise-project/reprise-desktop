//! wlr-output-management-v1: publish the outputs and apply or test
//! client-requested configurations (wlr-randr, kanshi, the Settings Display page).
//!
//! Follows niri's `src/protocols/output_management.rs` (GPL-3.0-or-later). Unlike
//! niri's, a configuration is checked against the outputs (`resolve`) before
//! anything is applied, so a test answers what an apply would do and a refused
//! configuration changes nothing; an apply runs head by head through
//! `State::apply_head`.

use std::collections::HashMap;

use smithay::{
    reexports::{
        wayland_protocols_wlr::output_management::v1::server::{
            zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
            zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
            zwlr_output_head_v1::{self, AdaptiveSyncState, ZwlrOutputHeadV1},
            zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
            zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource, WEnum, backend::ClientId,
            protocol::wl_output::Transform as WlTransform,
        },
    },
    utils::Transform,
    wayland::{Dispatch2, GlobalDispatch2},
};

use crate::{
    compositor::State,
    output::{HeadRequest, ModeInfo},
};

const VERSION: u32 = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct HeadState {
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub physical_size: (i32, i32),
    pub modes: Vec<ModeInfo>,
    pub current: Option<usize>,
    pub enabled: bool,
    pub position: (i32, i32),
    pub transform: Transform,
    pub scale: f64,
    pub adaptive_sync: bool,
    /// Not published: whether turning adaptive sync on can take.
    pub vrr_capable: bool,
}

/// Which mode a configuration head asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModeAsk {
    Keep,
    Index(usize),
    Custom(i32, i32, i32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeadAsk {
    pub enabled: bool,
    pub mode: ModeAsk,
    pub position: Option<(i32, i32)>,
    pub transform: Transform,
    pub scale: f64,
    pub adaptive_sync: bool,
}

impl HeadAsk {
    /// Enabling `head` with nothing set yet: what it runs, at its place.
    /// A disabled head asks no position (`State::apply_head` picks it).
    fn from_head(head: &HeadState) -> HeadAsk {
        HeadAsk {
            enabled: true,
            mode: ModeAsk::Keep,
            position: head.enabled.then_some(head.position),
            transform: head.transform,
            scale: head.scale,
            adaptive_sync: head.adaptive_sync,
        }
    }

    fn disabled(head: &HeadState) -> HeadAsk {
        HeadAsk {
            enabled: false,
            ..HeadAsk::from_head(head)
        }
    }
}

pub struct OutputMgmt {
    dh: DisplayHandle,
    serial: u32,
    clients: HashMap<ClientId, ClientData>,
    heads: Vec<HeadState>,
}

struct ClientData {
    manager: ZwlrOutputManagerV1,
    heads: HashMap<String, (ZwlrOutputHeadV1, Vec<ZwlrOutputModeV1>)>,
    confs: HashMap<ZwlrOutputConfigurationV1, Conf>,
}

enum Conf {
    Ongoing(Vec<(String, HeadAsk)>),
    Used,
    Cancelled,
}

impl OutputMgmt {
    pub fn new(dh: &DisplayHandle) -> OutputMgmt {
        dh.create_global::<State, ZwlrOutputManagerV1, _>(VERSION, ManagerGlobal);
        OutputMgmt {
            dh: dh.clone(),
            serial: 0,
            clients: HashMap::new(),
            heads: Vec::new(),
        }
    }

    /// Tell every client what changed from the heads it was last told of to `new`;
    /// configurations still being built are cancelled. Nothing if nothing changed.
    fn notify(&mut self, new: Vec<HeadState>) {
        if new == self.heads {
            return;
        }
        let OutputMgmt {
            dh, clients, heads, ..
        } = self;
        for data in clients.values_mut() {
            let Some(client) = data.manager.client() else {
                continue;
            };
            for h in &new {
                match heads.iter().find(|o| o.name == h.name) {
                    Some(old) if old.modes == h.modes => send_changes(data, old, h),
                    Some(_) => {
                        finish_head(data, &h.name);
                        send_new_head(dh, &client, data, h);
                    }
                    None => send_new_head(dh, &client, data, h),
                }
            }
            for old in heads.iter() {
                if !new.iter().any(|h| h.name == old.name) {
                    finish_head(data, &old.name);
                }
            }
        }
        self.heads = new;
        self.serial = self.serial.wrapping_add(1);
        for data in self.clients.values_mut() {
            data.manager.done(self.serial);
            for (conf, state) in data.confs.iter_mut() {
                if matches!(state, Conf::Ongoing(_)) {
                    conf.cancelled();
                    *state = Conf::Cancelled;
                }
            }
        }
    }
}

/// Tell output management clients about the outputs; runs on every layout change.
pub fn publish(state: &mut State) {
    let heads = crate::backend::heads(state)
        .into_iter()
        .map(|h| {
            let o = &h.output;
            let props = o.physical_properties();
            HeadState {
                name: o.name(),
                description: o.description(),
                make: props.make,
                model: props.model,
                serial: props.serial_number,
                physical_size: (props.size.w, props.size.h),
                modes: h.modes,
                current: h.current,
                enabled: h.enabled,
                position: (o.current_location().x, o.current_location().y),
                transform: o.current_transform(),
                scale: o.current_scale().fractional_scale(),
                adaptive_sync: h.vrr_on,
                vrr_capable: h.vrr_capable,
            }
        })
        .collect();
    state.outputs_mgmt.notify(heads);
}

/// How far from the origin a head may sit: the layout's arithmetic is i32, and
/// 2^24 leaves room for size and neighbours.
pub const MAX_POSITION: i32 = 1 << 24;

/// Whether a configuration is one this compositor will try: at least one head
/// stays enabled and every enabled head's position is within `MAX_POSITION`.
/// Refused before any head is applied.
fn acceptable(heads: &[(bool, i32, i32)]) -> bool {
    let mut enabled = heads.iter().filter(|(on, _, _)| *on).peekable();
    enabled.peek().is_some()
        && enabled.all(|(_, x, y)| {
            (-MAX_POSITION..=MAX_POSITION).contains(x) && (-MAX_POSITION..=MAX_POSITION).contains(y)
        })
}

/// Check a configuration against the current heads and turn it into what
/// `State::apply_head` runs, enabled heads first so the layout is never empty.
/// None when it is not `acceptable`, names a gone head, asks for a mode the head
/// lacks, or adaptive sync on a head that cannot. Scale is rounded to a quarter
/// step in 1..=4 when applied.
pub fn resolve(
    heads: &[HeadState],
    asks: &[(String, HeadAsk)],
) -> Option<Vec<(String, HeadRequest)>> {
    let mut on: Vec<(bool, i32, i32)> = heads
        .iter()
        .filter(|h| h.enabled && !asks.iter().any(|(n, _)| *n == h.name))
        .map(|h| (true, h.position.0, h.position.1))
        .collect();
    on.extend(asks.iter().map(|(_, a)| {
        let (x, y) = a.position.unwrap_or_default();
        (a.enabled, x, y)
    }));
    if !acceptable(&on) {
        return None;
    }
    let mut out = Vec::with_capacity(asks.len());
    for (name, a) in asks {
        let head = heads.iter().find(|h| h.name == *name)?;
        let mode = if !a.enabled {
            None
        } else {
            match a.mode {
                ModeAsk::Keep if head.enabled => head.current,
                ModeAsk::Keep => None,
                ModeAsk::Index(i) => Some(i).filter(|&i| i < head.modes.len()),
                ModeAsk::Custom(w, h, mhz) => Some(
                    head.modes
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| m.width == w && m.height == h)
                        .filter(|(_, m)| mhz == 0 || (m.refresh_mhz - mhz).abs() <= 1)
                        .max_by_key(|(_, m)| m.refresh_mhz)?
                        .0,
                ),
            }
        };
        if a.enabled && mode.is_none() && !matches!(a.mode, ModeAsk::Keep) {
            return None;
        }
        if a.enabled && a.adaptive_sync && !head.vrr_capable && !head.adaptive_sync {
            return None;
        }
        out.push((
            name.clone(),
            HeadRequest {
                enabled: a.enabled,
                mode,
                position: a.position,
                transform: a.transform,
                scale: a.scale,
                adaptive_sync: a.adaptive_sync,
            },
        ));
    }
    out.sort_by_key(|(_, r)| !r.enabled);
    Some(out)
}

/// Run a resolved configuration head by head, stopping at the first failure
/// (earlier heads stay applied), then lay out once.
fn apply(state: &mut State, reqs: Vec<(String, HeadRequest)>) -> bool {
    if !crate::backend::quiet(state) {
        log::warn!("output management: display busy, configuration refused");
        return false;
    }
    let mut ok = true;
    for (name, req) in reqs {
        let output = crate::backend::heads(state)
            .into_iter()
            .find(|h| h.output.name() == name)
            .map(|h| h.output);
        let Some(output) = output else {
            ok = false;
            break;
        };
        if !state.apply_head(&output, &req) {
            log::warn!("output management: {name} refused its head");
            ok = false;
            break;
        }
    }
    state.update_layout();
    ok
}

fn transform_from_wl(t: WlTransform) -> Option<Transform> {
    Some(match t {
        WlTransform::Normal => Transform::Normal,
        WlTransform::_90 => Transform::_90,
        WlTransform::_180 => Transform::_180,
        WlTransform::_270 => Transform::_270,
        WlTransform::Flipped => Transform::Flipped,
        WlTransform::Flipped90 => Transform::Flipped90,
        WlTransform::Flipped180 => Transform::Flipped180,
        WlTransform::Flipped270 => Transform::Flipped270,
        _ => return None,
    })
}

pub struct ManagerGlobal;
pub struct ManagerData;
pub struct HeadData(String);
pub struct ModeData;
pub struct ConfData(u32);
/// The configuration and head name a configuration head belongs to; None
/// for one made on a configuration that cannot take it.
pub struct ConfHeadData(Option<(ZwlrOutputConfigurationV1, String)>);

impl GlobalDispatch2<ZwlrOutputManagerV1, State> for ManagerGlobal {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        manager: New<ZwlrOutputManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(manager, ManagerData);
        let g = &mut state.outputs_mgmt;
        let mut data = ClientData {
            manager: manager.clone(),
            heads: HashMap::new(),
            confs: HashMap::new(),
        };
        for h in &g.heads {
            send_new_head(dh, client, &mut data, h);
        }
        g.clients.insert(client.id(), data);
        manager.done(g.serial);
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "zwlr_output_manager_v1")
    }
}

impl Dispatch2<ZwlrOutputManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        _manager: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let g = &mut state.outputs_mgmt;
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                let conf = data_init.init(id, ConfData(serial));
                let Some(data) = g.clients.get_mut(&client.id()) else {
                    return;
                };
                let st = if serial == g.serial {
                    Conf::Ongoing(Vec::new())
                } else {
                    conf.cancelled();
                    Conf::Cancelled
                };
                data.confs.insert(conf, st);
            }
            zwlr_output_manager_v1::Request::Stop => {
                if let Some(data) = g.clients.remove(&client.id()) {
                    data.manager.finished();
                }
            }
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, client: ClientId, _manager: &ZwlrOutputManagerV1) {
        state.outputs_mgmt.clients.remove(&client);
    }
}

impl Dispatch2<ZwlrOutputConfigurationV1, State> for ConfData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        conf: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let g = &mut state.outputs_mgmt;
        let current = g.serial;
        let Some(data) = g.clients.get_mut(&client.id()) else {
            if let zwlr_output_configuration_v1::Request::EnableHead { id, .. } = request {
                data_init.init(id, ConfHeadData(None));
            }
            return;
        };
        let used = || {
            conf.post_error(
                zwlr_output_configuration_v1::Error::AlreadyUsed,
                "configuration had already been used",
            )
        };
        let configured = || {
            conf.post_error(
                zwlr_output_configuration_v1::Error::AlreadyConfiguredHead,
                "head has been already configured",
            )
        };
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head } => {
                let name = head.data::<HeadData>().map(|d| d.0.clone());
                let found = name
                    .as_ref()
                    .and_then(|n| g.heads.iter().find(|h| h.name == *n));
                match (data.confs.get_mut(conf), found) {
                    (Some(Conf::Ongoing(asks)), Some(h)) => {
                        if asks.iter().any(|(n, _)| *n == h.name) {
                            data_init.init(id, ConfHeadData(None));
                            return configured();
                        }
                        asks.push((h.name.clone(), HeadAsk::from_head(h)));
                        data_init.init(id, ConfHeadData(Some((conf.clone(), h.name.clone()))));
                    }
                    (Some(Conf::Used), _) => {
                        data_init.init(id, ConfHeadData(None));
                        used();
                    }
                    _ => {
                        data_init.init(id, ConfHeadData(None));
                    }
                }
            }
            zwlr_output_configuration_v1::Request::DisableHead { head } => {
                let name = head.data::<HeadData>().map(|d| d.0.clone());
                let found = name
                    .as_ref()
                    .and_then(|n| g.heads.iter().find(|h| h.name == *n));
                match (data.confs.get_mut(conf), found) {
                    (Some(Conf::Ongoing(asks)), Some(h)) => {
                        if asks.iter().any(|(n, _)| *n == h.name) {
                            return configured();
                        }
                        asks.push((h.name.clone(), HeadAsk::disabled(h)));
                    }
                    (Some(Conf::Used), _) => used(),
                    _ => {}
                }
            }
            zwlr_output_configuration_v1::Request::Apply
            | zwlr_output_configuration_v1::Request::Test => {
                let test = matches!(request, zwlr_output_configuration_v1::Request::Test);
                let asks = match data.confs.get_mut(conf) {
                    Some(st @ Conf::Ongoing(_)) => match std::mem::replace(st, Conf::Used) {
                        Conf::Ongoing(asks) => asks,
                        _ => unreachable!("matched above"),
                    },
                    Some(Conf::Used) => return used(),
                    Some(Conf::Cancelled) | None => return,
                };
                if self.0 != current {
                    conf.cancelled();
                    return;
                }
                let Some(reqs) = resolve(&g.heads, &asks) else {
                    // A failed test is an answer; a failed apply is worth a line.
                    let level = if test {
                        log::Level::Debug
                    } else {
                        log::Level::Warn
                    };
                    log::log!(
                        level,
                        "output management: configuration refused (every head off, a position past {MAX_POSITION}, an unknown mode or adaptive sync a head cannot do)"
                    );
                    conf.failed();
                    return;
                };
                if test {
                    conf.succeeded();
                    return;
                }
                if apply(state, reqs) {
                    conf.succeeded();
                } else {
                    conf.failed();
                }
            }
            zwlr_output_configuration_v1::Request::Destroy => {
                data.confs.remove(conf);
            }
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, client: ClientId, conf: &ZwlrOutputConfigurationV1) {
        if let Some(data) = state.outputs_mgmt.clients.get_mut(&client) {
            data.confs.remove(conf);
        }
    }
}

impl Dispatch2<ZwlrOutputConfigurationHeadV1, State> for ConfHeadData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        conf_head: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let Some((conf, name)) = &self.0 else {
            return;
        };
        let g = &mut state.outputs_mgmt;
        let Some(data) = g.clients.get_mut(&client.id()) else {
            return;
        };
        let modes = data.heads.get(name).map(|(_, m)| m.clone());
        let Some(Conf::Ongoing(asks)) = data.confs.get_mut(conf) else {
            return;
        };
        let Some((_, ask)) = asks.iter_mut().find(|(n, _)| n == name) else {
            return;
        };
        use zwlr_output_configuration_head_v1::{Error, Request};
        match request {
            Request::SetMode { mode } => {
                match modes.and_then(|m| m.iter().position(|x| *x == mode)) {
                    Some(i) => ask.mode = ModeAsk::Index(i),
                    None => conf_head.post_error(Error::InvalidMode, "not a mode of this head"),
                }
            }
            Request::SetCustomMode {
                width,
                height,
                refresh,
            } => {
                if width <= 0 || height <= 0 || refresh < 0 {
                    conf_head.post_error(Error::InvalidCustomMode, "invalid custom mode");
                } else {
                    ask.mode = ModeAsk::Custom(width, height, refresh);
                }
            }
            Request::SetPosition { x, y } => ask.position = Some((x, y)),
            Request::SetTransform { transform } => {
                match transform.into_result().ok().and_then(transform_from_wl) {
                    Some(t) => ask.transform = t,
                    None => conf_head.post_error(Error::InvalidTransform, "unknown transform"),
                }
            }
            Request::SetScale { scale } => {
                if scale.is_finite() && scale > 0.0 {
                    ask.scale = scale;
                } else {
                    conf_head.post_error(Error::InvalidScale, "scale is not positive");
                }
            }
            Request::SetAdaptiveSync { state } => match state {
                WEnum::Value(AdaptiveSyncState::Enabled) => ask.adaptive_sync = true,
                WEnum::Value(AdaptiveSyncState::Disabled) => ask.adaptive_sync = false,
                _ => conf_head.post_error(
                    Error::InvalidAdaptiveSyncState,
                    "unknown adaptive sync state",
                ),
            },
            _ => {}
        }
    }
}

impl Dispatch2<ZwlrOutputHeadV1, State> for HeadData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _head: &ZwlrOutputHeadV1,
        _request: zwlr_output_head_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // Release is a destructor: `destroyed` forgets the head.
    }

    fn destroyed(&self, state: &mut State, client: ClientId, head: &ZwlrOutputHeadV1) {
        if let Some(data) = state.outputs_mgmt.clients.get_mut(&client)
            && data.heads.get(&self.0).is_some_and(|(h, _)| h == head)
        {
            data.heads.remove(&self.0);
        }
    }
}

impl Dispatch2<ZwlrOutputModeV1, State> for ModeData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _mode: &ZwlrOutputModeV1,
        _request: zwlr_output_mode_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }
}

fn finish_head(data: &mut ClientData, name: &str) {
    if let Some((head, modes)) = data.heads.remove(name) {
        for m in modes {
            m.finished();
        }
        head.finished();
    }
}

/// What changed on a head between `old` and `new`, same modes. A head
/// that turns on gets its whole state again.
fn send_changes(data: &ClientData, old: &HeadState, new: &HeadState) {
    let Some((head, modes)) = data.heads.get(&new.name) else {
        return;
    };
    if old.enabled != new.enabled {
        head.enabled(i32::from(new.enabled));
    }
    if new.enabled {
        let fresh = !old.enabled;
        if (fresh || old.current != new.current)
            && let Some(m) = new.current.and_then(|i| modes.get(i))
        {
            head.current_mode(m);
        }
        if fresh || old.position != new.position {
            head.position(new.position.0, new.position.1);
        }
        if fresh || old.transform != new.transform {
            head.transform(new.transform.into());
        }
        if fresh || old.scale != new.scale {
            head.scale(new.scale);
        }
    }
    if old.adaptive_sync != new.adaptive_sync
        && head.version() >= zwlr_output_head_v1::EVT_ADAPTIVE_SYNC_SINCE
    {
        head.adaptive_sync(if new.adaptive_sync {
            AdaptiveSyncState::Enabled
        } else {
            AdaptiveSyncState::Disabled
        });
    }
}

/// Announce `h` with its modes and state to the client of `data`.
fn send_new_head(dh: &DisplayHandle, client: &Client, data: &mut ClientData, h: &HeadState) {
    let version = data.manager.version();
    let Ok(head) =
        client.create_resource::<ZwlrOutputHeadV1, _, State>(dh, version, HeadData(h.name.clone()))
    else {
        return;
    };
    data.manager.head(&head);
    head.name(h.name.clone());
    head.description(h.description.clone());
    if h.physical_size.0 > 0 && h.physical_size.1 > 0 {
        head.physical_size(h.physical_size.0, h.physical_size.1);
    }
    let mut modes = Vec::with_capacity(h.modes.len());
    for (i, m) in h.modes.iter().enumerate() {
        let Ok(mode) = client.create_resource::<ZwlrOutputModeV1, _, State>(dh, version, ModeData)
        else {
            continue;
        };
        head.mode(&mode);
        mode.size(m.width, m.height);
        if m.refresh_mhz > 0 {
            mode.refresh(m.refresh_mhz);
        }
        if m.preferred {
            mode.preferred();
        }
        if h.enabled && Some(i) == h.current {
            head.current_mode(&mode);
        }
        modes.push(mode);
    }
    head.enabled(i32::from(h.enabled));
    if h.enabled {
        head.position(h.position.0, h.position.1);
        head.transform(h.transform.into());
        head.scale(h.scale);
    }
    if version >= zwlr_output_head_v1::EVT_MAKE_SINCE {
        head.make(h.make.clone());
        head.model(h.model.clone());
        head.serial_number(h.serial.clone());
    }
    if version >= zwlr_output_head_v1::EVT_ADAPTIVE_SYNC_SINCE {
        head.adaptive_sync(if h.adaptive_sync {
            AdaptiveSyncState::Enabled
        } else {
            AdaptiveSyncState::Disabled
        });
    }
    data.heads.insert(h.name.clone(), (head, modes));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configuration_keeps_a_head_on_and_in_range() {
        assert!(acceptable(&[(true, 0, 0), (false, 0, 0)]));
        assert!(acceptable(&[(true, -MAX_POSITION, MAX_POSITION)]));
        assert!(
            !acceptable(&[(false, 0, 0), (false, 0, 0)]),
            "every head off"
        );
        assert!(!acceptable(&[]), "no heads at all");
        assert!(
            !acceptable(&[(true, MAX_POSITION + 1, 0)]),
            "past the extent"
        );
        assert!(!acceptable(&[(true, 0, i32::MIN)]), "the far end of i32");
        assert!(
            acceptable(&[(true, 0, 0), (false, i32::MAX, i32::MAX)]),
            "a disabled head's position is not looked at"
        );
    }

    fn head(name: &str, enabled: bool) -> HeadState {
        let mode = |width, height, hz, preferred| ModeInfo {
            width,
            height,
            refresh_mhz: hz,
            preferred,
        };
        HeadState {
            name: name.into(),
            description: String::new(),
            make: String::new(),
            model: String::new(),
            serial: String::new(),
            physical_size: (0, 0),
            modes: vec![
                mode(1920, 1080, 60_000, true),
                mode(1280, 720, 60_000, false),
                mode(1280, 720, 50_000, false),
            ],
            current: Some(0),
            enabled,
            position: (0, 0),
            transform: Transform::Normal,
            scale: 1.0,
            adaptive_sync: false,
            vrr_capable: false,
        }
    }

    fn ask(h: &HeadState) -> (String, HeadAsk) {
        (h.name.clone(), HeadAsk::from_head(h))
    }

    #[test]
    fn turning_off_the_only_output_is_refused() {
        let heads = [head("Virtual-1", true)];
        assert!(
            resolve(
                &heads,
                &[("Virtual-1".into(), HeadAsk::disabled(&heads[0]))]
            )
            .is_none()
        );
        // A head left out of the configuration stays on.
        let two = [head("A", true), head("B", true)];
        assert!(resolve(&two, &[("B".into(), HeadAsk::disabled(&two[1]))]).is_some());
    }

    #[test]
    fn a_mode_must_be_one_the_head_has() {
        let heads = [head("A", true)];
        let (n, mut a) = ask(&heads[0]);
        a.mode = ModeAsk::Index(2);
        let r = resolve(&heads, &[(n.clone(), a.clone())]).expect("a listed mode");
        assert_eq!(r[0].1.mode, Some(2));
        a.mode = ModeAsk::Index(3);
        assert!(resolve(&heads, &[(n.clone(), a.clone())]).is_none());
        a.mode = ModeAsk::Custom(1280, 720, 50_000);
        assert_eq!(
            resolve(&heads, &[(n.clone(), a.clone())]).unwrap()[0]
                .1
                .mode,
            Some(2),
            "a custom mode the head lists"
        );
        a.mode = ModeAsk::Custom(1280, 720, 0);
        assert_eq!(
            resolve(&heads, &[(n.clone(), a.clone())]).unwrap()[0]
                .1
                .mode,
            Some(1),
            "no refresh takes the highest"
        );
        a.mode = ModeAsk::Custom(1024, 768, 60_000);
        assert!(resolve(&heads, &[(n, a)]).is_none(), "a size it lacks");
    }

    #[test]
    fn nothing_set_keeps_what_runs_and_where() {
        let mut heads = [head("A", true)];
        heads[0].current = Some(1);
        heads[0].position = (100, 0);
        let r = resolve(&heads, &[ask(&heads[0])]).unwrap();
        assert_eq!(r[0].1.mode, Some(1));
        assert_eq!(r[0].1.position, Some((100, 0)));
        // A disabled head coming back takes its configured mode and asks no
        // position.
        let heads = [head("A", true), head("B", false)];
        let r = resolve(&heads, &[ask(&heads[1])]).unwrap();
        assert_eq!((r[0].1.mode, r[0].1.position), (None, None));
    }

    #[test]
    fn adaptive_sync_only_on_a_head_that_can() {
        let mut heads = [head("A", true)];
        let (n, mut a) = ask(&heads[0]);
        a.adaptive_sync = true;
        assert!(resolve(&heads, &[(n.clone(), a.clone())]).is_none());
        heads[0].vrr_capable = true;
        assert!(resolve(&heads, &[(n, a)]).is_some());
    }

    #[test]
    fn enabled_heads_go_first_and_unknown_heads_fail() {
        let heads = [head("A", true), head("B", false)];
        let r = resolve(
            &heads,
            &[("A".into(), HeadAsk::disabled(&heads[0])), ask(&heads[1])],
        )
        .unwrap();
        assert_eq!(
            r.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["B", "A"]
        );
        assert!(resolve(&heads, &[ask(&head("C", true))]).is_none());
    }
}
