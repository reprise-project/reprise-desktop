//! ext-workspace-v1: the desktops as workspaces for external pagers and bars. One group spans
//! every output, since a desktop switch changes every output at once; there is no global with a
//! single desktop.
//!
//! The binding's shape follows niri's `src/protocols/ext_workspace.rs`
//! (GPL-3.0-or-later).

use smithay::reexports::{
    wayland_protocols::ext::workspace::v1::server::{
        ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
        ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
        ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
    },
    wayland_server::{
        Client, DataInit, DisplayHandle, New, Resource,
        backend::{ClientId, GlobalId},
        protocol::wl_output::WlOutput,
    },
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::compositor::State;

const VERSION: u32 = 1;

/// The global while there is more than one desktop, and every bound manager with what it was last
/// told.
#[derive(Default)]
pub struct Workspaces {
    global: Option<GlobalId>,
    bound: Vec<Bound>,
}

/// One client's manager: its group, a handle per desktop, the wl_outputs the group entered, the
/// states last sent and the desktop an activate asked for since the last commit.
struct Bound {
    manager: ExtWorkspaceManagerV1,
    group: ExtWorkspaceGroupHandleV1,
    handles: Vec<ExtWorkspaceHandleV1>,
    outputs: Vec<WlOutput>,
    states: Vec<ext_workspace_handle_v1::State>,
    activate: Option<usize>,
}

/// The manager global's data; also each manager's and group's.
pub struct WorkspaceGlobal;

/// A workspace handle's data: its desktop.
pub struct WorkspaceData(usize);

impl Workspaces {
    /// The global, when there is more than one desktop.
    pub fn new(dh: &DisplayHandle, desktops: crate::desktops::Desktops) -> Workspaces {
        Workspaces {
            global: desktops.enabled().then(|| {
                dh.create_global::<State, ExtWorkspaceManagerV1, _>(VERSION, WorkspaceGlobal)
            }),
            bound: Vec::new(),
        }
    }
}

fn new_handle(
    dh: &DisplayHandle,
    client: &Client,
    manager: &ExtWorkspaceManagerV1,
    group: &ExtWorkspaceGroupHandleV1,
    i: usize,
    state: ext_workspace_handle_v1::State,
) -> Option<ExtWorkspaceHandleV1> {
    let h = client
        .create_resource::<ExtWorkspaceHandleV1, _, State>(dh, manager.version(), WorkspaceData(i))
        .ok()?;
    manager.workspace(&h);
    let name = (i + 1).to_string();
    h.id(name.clone());
    h.name(name);
    h.coordinates((i as u32).to_ne_bytes().to_vec());
    h.state(state);
    h.capabilities(ext_workspace_handle_v1::WorkspaceCapabilities::Activate);
    group.workspace_enter(&h);
    Some(h)
}

fn remove_handle(group: &ExtWorkspaceGroupHandleV1, h: &ExtWorkspaceHandleV1) {
    group.workspace_leave(h);
    h.removed();
}

fn states(count: usize, current: usize, attention: &[bool]) -> Vec<ext_workspace_handle_v1::State> {
    use ext_workspace_handle_v1::State as S;
    (0..count)
        .map(|i| {
            let mut s = S::empty();
            if i == current {
                s |= S::Active;
            }
            if attention.get(i).copied().unwrap_or(false) {
                s |= S::Urgent;
            }
            s
        })
        .collect()
}

impl State {
    fn workspace_states(&self) -> Vec<ext_workspace_handle_v1::State> {
        let d = self.desktops;
        let asks = self
            .windows
            .iter()
            .filter(|w| w.mapped && w.attention)
            .map(|w| (self.desktop_of(w.id), true));
        states(
            d.count,
            d.current,
            &crate::desktops::attention(d.count, d.current, asks),
        )
    }

    /// Follow a new `desktops.count`: each manager gets handles for the new desktops or loses the
    /// gone ones' (an activate of one is dropped). Down to one desktop the global goes; up from one
    /// it comes back. The new states go out with the next `refresh_workspaces`.
    pub fn workspaces_resized(&mut self) {
        let d = self.desktops;
        if !d.enabled() {
            for b in self.workspaces.bound.drain(..) {
                for h in &b.handles {
                    remove_handle(&b.group, h);
                }
                b.group.removed();
                b.manager.done();
                b.manager.finished();
            }
            if let Some(g) = self.workspaces.global.take() {
                self.dh.remove_global::<State>(g);
            }
            return;
        }
        if self.workspaces.global.is_none() {
            self.workspaces.global = Some(
                self.dh
                    .create_global::<State, ExtWorkspaceManagerV1, _>(VERSION, WorkspaceGlobal),
            );
        }
        let now = self.workspace_states();
        for b in &mut self.workspaces.bound {
            for h in b.handles.drain(d.count.min(b.handles.len())..) {
                remove_handle(&b.group, &h);
            }
            b.states.truncate(d.count);
            b.activate = b.activate.filter(|&n| n < d.count);
            if let Some(client) = b.manager.client() {
                for (i, s) in now.iter().enumerate().skip(b.handles.len()) {
                    let Some(h) = new_handle(&self.dh, &client, &b.manager, &b.group, i, *s) else {
                        break;
                    };
                    b.handles.push(h);
                    b.states.push(*s);
                }
            }
            b.manager.done();
        }
    }

    /// Send each manager the states that changed and the outputs its client bound since; once per
    /// loop iteration.
    pub fn refresh_workspaces(&mut self) {
        if self.workspaces.bound.is_empty() {
            return;
        }
        let now = self.workspace_states();
        let outputs = self.scene.outputs().to_vec();
        for b in &mut self.workspaces.bound {
            let mut changed = false;
            for (i, (h, s)) in b.handles.iter().zip(&now).enumerate() {
                if b.states.get(i) != Some(s) {
                    h.state(*s);
                    changed = true;
                }
            }
            b.states.clone_from(&now);
            b.outputs.retain(|o| o.is_alive());
            if let Some(client) = b.manager.client() {
                for o in outputs.iter().flat_map(|o| o.client_outputs(&client)) {
                    if !b.outputs.contains(&o) {
                        b.group.output_enter(&o);
                        b.outputs.push(o);
                        changed = true;
                    }
                }
            }
            if changed {
                b.manager.done();
            }
        }
    }
}

impl GlobalDispatch2<ExtWorkspaceManagerV1, State> for WorkspaceGlobal {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, WorkspaceGlobal);
        let Ok(group) = client.create_resource::<ExtWorkspaceGroupHandleV1, _, State>(
            dh,
            manager.version(),
            WorkspaceGlobal,
        ) else {
            return;
        };
        manager.workspace_group(&group);
        group.capabilities(ext_workspace_group_handle_v1::GroupCapabilities::empty());
        let mut outputs = Vec::new();
        for o in state.scene.outputs() {
            for wl in o.client_outputs(client) {
                group.output_enter(&wl);
                outputs.push(wl);
            }
        }
        let now = state.workspace_states();
        let mut handles = Vec::new();
        for (i, s) in now.iter().enumerate() {
            let Some(h) = new_handle(dh, client, &manager, &group, i, *s) else {
                return;
            };
            handles.push(h);
        }
        manager.done();
        state.workspaces.bound.push(Bound {
            manager,
            group,
            handles,
            outputs,
            states: now,
            activate: None,
        });
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "ext_workspace_manager_v1")
    }
}

impl Dispatch2<ExtWorkspaceManagerV1, State> for WorkspaceGlobal {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ExtWorkspaceManagerV1,
        request: ext_workspace_manager_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            // An activate takes effect on the commit after it; the last one wins.
            ext_workspace_manager_v1::Request::Commit => {
                let n = state
                    .workspaces
                    .bound
                    .iter_mut()
                    .find(|b| b.manager == *resource)
                    .and_then(|b| b.activate.take());
                if let Some(n) = n {
                    state.switch_desktop(n);
                }
            }
            ext_workspace_manager_v1::Request::Stop => {
                resource.finished();
                state.workspaces.bound.retain(|b| b.manager != *resource);
            }
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, resource: &ExtWorkspaceManagerV1) {
        state.workspaces.bound.retain(|b| b.manager != *resource);
    }
}

impl Dispatch2<ExtWorkspaceGroupHandleV1, State> for WorkspaceGlobal {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ExtWorkspaceGroupHandleV1,
        _request: ext_workspace_group_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }
}

impl Dispatch2<ExtWorkspaceHandleV1, State> for WorkspaceData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ExtWorkspaceHandleV1,
        request: ext_workspace_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let ext_workspace_handle_v1::Request::Activate = request
            && let Some(b) = state
                .workspaces
                .bound
                .iter_mut()
                .find(|b| b.handles.contains(resource))
        {
            b.activate = Some(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ext_workspace_handle_v1::State as S;

    #[test]
    fn the_current_desktop_is_active_and_others_asking_are_urgent() {
        assert_eq!(
            states(3, 1, &[false, false, true]),
            vec![S::empty(), S::Active, S::Urgent]
        );
        assert_eq!(states(2, 0, &[]), vec![S::Active, S::empty()]);
    }
}
