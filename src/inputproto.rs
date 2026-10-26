//! `reprise_input_v1`: the private global reprise-portal binds to hand in EIS
//! sockets, drive pointer barriers, receive global shortcuts and press
//! RemoteDesktop keys without libei. Glue only: each request calls into
//! `eis`, `inputcapture`, `globalshortcuts` or `remotekeys`.
//!
//! Protocol objects carry ids, not the objects themselves: a resource whose
//! object the compositor already dropped finds nothing behind its id.

use smithay::reexports::wayland_server::{
    Client, DataInit, DisplayHandle, New, Resource, WEnum, backend::ClientId,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::compositor::State;
use crate::eis::EisId;
use crate::globalshortcuts::ShortcutsId;
use crate::inputcapture::{Added, Barrier, CaptureId};
use crate::remotekeys::KeysId;

use proto::reprise_eis_client_v1::{self, RepriseEisClientV1};
use proto::reprise_input_capture_v1::{self, RepriseInputCaptureV1};
use proto::reprise_input_v1::{self, RepriseInputV1};
use proto::reprise_keys_v1::{self, RepriseKeysV1};
use proto::reprise_shortcuts_v1::{self, RepriseShortcutsV1};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    clippy::all
)]
pub mod proto {
    // The scanner macros open the XML with File::open, which leaves cargo no
    // dependency edge; this keeps an edit to the protocol rebuilding us.
    const _: &[u8] = include_bytes!("../protocols/reprise-input-v1.xml");

    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;

    pub mod __interfaces {
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/reprise-input-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/reprise-input-v1.xml");
}

const VERSION: u32 = 4;

pub const DEVICE_KEYBOARD: u32 = 1;
pub const DEVICE_POINTER: u32 = 2;

pub struct InputProto {
    pub eis: crate::eis::EisState,
    pub capture: crate::inputcapture::CaptureState,
    pub shortcuts: crate::globalshortcuts::ShortcutsState,
    pub keys: crate::remotekeys::KeysState,
    /// Set while `eis` feeds a RemoteDesktop sender's request through the input
    /// paths: the InputCapture gate lets it past (`gated`).
    pub injecting: bool,
}

impl InputProto {
    pub fn new(dh: &DisplayHandle) -> InputProto {
        dh.create_global::<State, RepriseInputV1, _>(VERSION, InputGlobal);
        InputProto {
            eis: Default::default(),
            capture: Default::default(),
            shortcuts: Default::default(),
            keys: Default::default(),
            injecting: false,
        }
    }

    pub fn captured(&self) -> bool {
        self.capture.active.is_some()
    }

    /// Whether the input being handled goes to the capture's receiver instead of
    /// this desktop. A RemoteDesktop sender's injected input drives this desktop
    /// even while a capture holds the physical devices (a machine in a Deskflow
    /// chain is both a client and a server).
    pub fn gated(&self) -> bool {
        self.captured() && !self.injecting
    }
}

/// Where installed Reprise programs live; both are root-owned, so a match
/// means root put the binary there.
const INSTALL_DIRS: &[&str] = &["/usr/bin/", "/usr/local/bin/"];

/// Whether `exe`, a `/proc/<pid>/exe` target, is `program` in `INSTALL_DIRS`.
/// The kernel appends " (deleted)" once an upgrade replaces the file.
pub fn is_installed_exe(exe: &[u8], program: &str) -> bool {
    let exe = exe.strip_suffix(b" (deleted)").unwrap_or(exe);
    INSTALL_DIRS.iter().any(|d| {
        exe.strip_prefix(d.as_bytes())
            .is_some_and(|name| name == program.as_bytes())
    })
}

/// Whether process `pid` runs the installed `program`: its own binary, or the installed
/// multi-call binary started as `program`.
pub fn runs_installed(pid: i32, program: &str) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
        return false;
    };
    let exe = exe.as_os_str().as_bytes();
    is_installed_exe(exe, program)
        || (is_installed_exe(exe, reprisefs::multicall::BINARY)
            && reprisefs::multicall::program_of(pid) == Some(program))
}

pub struct InputGlobal;

pub struct InputData;

impl GlobalDispatch2<RepriseInputV1, State> for InputGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<RepriseInputV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, InputData);
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "reprise_input_v1")
    }
}

impl Dispatch2<RepriseInputV1, State> for InputData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &RepriseInputV1,
        request: reprise_input_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        use reprise_input_v1::{Error, Kind, Request};
        match request {
            Request::ConnectEis {
                id,
                fd,
                kind,
                capture,
                devices,
            } => {
                let kind = match kind {
                    WEnum::Value(Kind::Sender) => crate::eis::Kind::Sender,
                    WEnum::Value(Kind::Receiver) => crate::eis::Kind::Receiver,
                    _ => {
                        resource.post_error(Error::InvalidKind, "kind");
                        return;
                    }
                };
                let cap = capture
                    .as_ref()
                    .and_then(|c| c.data::<CaptureData>())
                    .map(|d| d.0)
                    .filter(|&c| state.inputproto.capture.get(c).is_some());
                if kind == crate::eis::Kind::Receiver && cap.is_none() {
                    resource.post_error(Error::CaptureRequired, "receiver needs a capture");
                    return;
                }
                if kind == crate::eis::Kind::Sender && cap.is_some() {
                    resource.post_error(Error::CaptureForbidden, "sender with a capture");
                    return;
                }
                match state.eis_connect(fd, kind, cap, devices) {
                    Ok(ec) => {
                        if let Some(c) = cap {
                            // A capture has one receiver; a second replaces the first, and disconnecting
                            // the old one releases the capture if active.
                            let old = state.inputproto.capture.get(c).and_then(|c| c.receiver);
                            if let Some(old) = old {
                                state.eis_disconnect(old);
                            }
                            if let Some(c) = state.inputproto.capture.get_mut(c) {
                                c.receiver = Some(ec);
                            }
                        }
                        data_init.init(id, EisClientData(Some(ec)));
                    }
                    Err(e) => {
                        log::warn!("connect_eis: {e}");
                        // A live resource with no client behind it: the portal still owns it and
                        // destroys it when it gives up.
                        data_init.init(id, EisClientData(None));
                    }
                }
            }
            Request::CreateCapture { id } => {
                let held = state.inputproto.capture.count_for_client(resource);
                if held >= crate::inputcapture::MAX_CAPTURES_PER_CLIENT {
                    resource.post_error(Error::TooMany, "too many captures");
                    return;
                }
                let cid = state.inputproto.capture.next_id();
                let r = data_init.init(id, CaptureData(cid));
                state.inputproto.capture.create(cid, r);
            }
            Request::CreateShortcuts { id, app_id } => {
                let held = state.inputproto.shortcuts.count_for_client(resource);
                if held >= crate::globalshortcuts::MAX_PER_CLIENT {
                    resource.post_error(Error::TooMany, "too many shortcut objects");
                    return;
                }
                let sid = state.inputproto.shortcuts.next_id();
                let r = data_init.init(id, ShortcutsData(sid));
                state.inputproto.shortcuts.create(sid, r, app_id);
            }
            Request::CreateKeys { id } => {
                let kid = state.inputproto.keys.create();
                data_init.init(id, KeysData(kid));
            }
            Request::Destroy => {}
        }
    }
}

pub struct EisClientData(Option<EisId>);

impl Dispatch2<RepriseEisClientV1, State> for EisClientData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &RepriseEisClientV1,
        _request: reprise_eis_client_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, _resource: &RepriseEisClientV1) {
        if let Some(id) = self.0 {
            state.eis_disconnect(id);
        }
    }
}

pub struct CaptureData(CaptureId);

impl Dispatch2<RepriseInputCaptureV1, State> for CaptureData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &RepriseInputCaptureV1,
        request: reprise_input_capture_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        use reprise_input_capture_v1::Request;
        let cap = self.0;
        match request {
            Request::ClearBarriers => state.capture_clear_barriers(cap),
            Request::AddBarrier { id, x1, y1, x2, y2 } => {
                let Some(c) = state.inputproto.capture.get_mut(cap) else {
                    return;
                };
                match crate::inputcapture::add_barrier(
                    &mut c.barriers,
                    Barrier { id, x1, y1, x2, y2 },
                ) {
                    Added::Ok => {}
                    Added::Invalid => {
                        log::info!("input capture: barrier {id} ({x1},{y1})-({x2},{y2}) dropped");
                    }
                    Added::TooMany => {
                        resource.post_error(reprise_input_v1::Error::TooMany, "too many barriers")
                    }
                }
            }
            Request::Enable => {
                if let Some(c) = state.inputproto.capture.get_mut(cap) {
                    c.enabled = true;
                }
            }
            Request::Disable => state.capture_disable(cap),
            Request::Release {
                activation_id,
                has_position,
                x,
                y,
            } => {
                let pos = (has_position != 0).then_some((x, y));
                // 0 is not a real activation id: it means "whatever is active".
                let activation = (activation_id != 0).then_some(activation_id);
                state.capture_release(cap, activation, pos);
            }
            Request::Destroy => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, _resource: &RepriseInputCaptureV1) {
        state.capture_destroy(self.0);
    }
}

pub struct ShortcutsData(ShortcutsId);

impl Dispatch2<RepriseShortcutsV1, State> for ShortcutsData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &RepriseShortcutsV1,
        _request: reprise_shortcuts_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, _resource: &RepriseShortcutsV1) {
        state.inputproto.shortcuts.destroy(self.0);
    }
}

pub struct KeysData(KeysId);

impl Dispatch2<RepriseKeysV1, State> for KeysData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &RepriseKeysV1,
        request: reprise_keys_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        use reprise_keys_v1::{KeyState, Request};
        let down = |s: WEnum<KeyState>| s == WEnum::Value(KeyState::Pressed);
        match request {
            Request::Key { key, state: s } => state.keys_key(self.0, key, down(s)),
            Request::Keysym { keysym, state: s } => state.keys_keysym(self.0, keysym, down(s)),
            Request::Destroy => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, _resource: &RepriseKeysV1) {
        state.keys_destroy(self.0);
    }
}
