//! xdg-foreign v1 (zxdg_*_v1, still used by GTK 3 and older Qt), after Smithay's
//! `src/wayland/xdg_foreign/handlers.rs` (MIT); Smithay serves v2.

use crate::mutex::lock;
use std::collections::HashMap;

use smithay::{
    reexports::{
        wayland_protocols::xdg::foreign::zv1::server::{
            zxdg_exported_v1::{self, ZxdgExportedV1},
            zxdg_exporter_v1::{self, ZxdgExporterV1},
            zxdg_imported_v1::{self, ZxdgImportedV1},
            zxdg_importer_v1::{self, ZxdgImporterV1},
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, backend::ClientId,
            protocol::wl_surface::WlSurface,
        },
    },
    wayland::{
        Dispatch2, GlobalDispatch2, compositor,
        shell::{
            is_valid_parent,
            xdg::{XDG_TOPLEVEL_ROLE, XdgToplevelSurfaceData},
        },
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};

use crate::compositor::State;

pub struct Foreign {
    v2: XdgForeignState,
    v1: HashMap<String, Exported>,
}

struct Exported {
    surface: WlSurface,
    child: Option<(WlSurface, ZxdgImportedV1)>,
}

impl Foreign {
    pub fn new(dh: &DisplayHandle) -> Foreign {
        dh.create_global::<State, ZxdgExporterV1, _>(1, V1);
        dh.create_global::<State, ZxdgImporterV1, _>(1, V1);
        Foreign {
            v2: XdgForeignState::new::<State>(dh),
            v1: HashMap::new(),
        }
    }
}

pub struct V1;

pub struct V1Handle(String);

fn new_handle() -> String {
    let mut bytes = [0u8; 16];
    // SAFETY: `bytes` is a live buffer of the length passed.
    let n = unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    if n != bytes.len() as isize {
        // getrandom on a 16-byte buffer does not come up short; fall back
        // to the clock rather than a fixed handle.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        bytes = t.to_ne_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl State {
    fn set_foreign_parent(
        &mut self,
        child: &WlSurface,
        parent: Option<&WlSurface>,
        of: &WlSurface,
    ) {
        let mut changed = false;
        compositor::with_states(child, |states| {
            if let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>() {
                let mut role = lock(data);
                let now = match parent {
                    Some(p) => Some(p.clone()),
                    None if role.parent.as_ref() == Some(of) => None,
                    None => return,
                };
                changed = role.parent != now;
                role.parent = now;
            }
        });
        if changed && let Some(w) = self.window_for_surface(child) {
            self.on_parent_changed(w);
        }
    }

    fn unparent_v1(&mut self, handle: &str, by: Option<&ZxdgImportedV1>) {
        let Some(e) = self.foreign.v1.get_mut(handle) else {
            return;
        };
        if by.is_some_and(|by| e.child.as_ref().is_none_or(|(_, i)| i != by)) {
            return;
        }
        let Some((child, _)) = e.child.take() else {
            return;
        };
        let surface = e.surface.clone();
        self.set_foreign_parent(&child, None, &surface);
    }
}

impl GlobalDispatch2<ZxdgExporterV1, State> for V1 {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZxdgExporterV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, V1);
    }
}

impl GlobalDispatch2<ZxdgImporterV1, State> for V1 {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZxdgImporterV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, V1);
    }
}

impl Dispatch2<ZxdgExporterV1, State> for V1 {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZxdgExporterV1,
        request: zxdg_exporter_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        if let zxdg_exporter_v1::Request::Export { id, surface } = request {
            let handle = new_handle();
            let exported = data_init.init(id, V1Handle(handle.clone()));
            // v1 has no error for it: a surface that is not a toplevel is
            // never given a handle.
            if compositor::get_role(&surface) != Some(XDG_TOPLEVEL_ROLE) {
                return;
            }
            exported.handle(handle.clone());
            state.foreign.v1.insert(
                handle,
                Exported {
                    surface,
                    child: None,
                },
            );
        }
    }
}

impl Dispatch2<ZxdgExportedV1, State> for V1Handle {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ZxdgExportedV1,
        _request: zxdg_exported_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, _resource: &ZxdgExportedV1) {
        state.unparent_v1(&self.0, None);
        state.foreign.v1.remove(&self.0);
    }
}

impl Dispatch2<ZxdgImporterV1, State> for V1 {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZxdgImporterV1,
        request: zxdg_importer_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        if let zxdg_importer_v1::Request::Import { id, handle } = request {
            let known = state.foreign.v1.contains_key(&handle);
            let imported = data_init.init(id, V1Handle(handle));
            if !known {
                imported.destroyed();
            }
        }
    }
}

impl Dispatch2<ZxdgImportedV1, State> for V1Handle {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZxdgImportedV1,
        request: zxdg_imported_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let zxdg_imported_v1::Request::SetParentOf { surface: child } = request else {
            return;
        };
        let Some(parent) = state.foreign.v1.get(&self.0).map(|e| e.surface.clone()) else {
            return;
        };
        // v1 has no error for it: a child that is not a toplevel, or a
        // parent that would make a cycle, is left alone.
        if compositor::get_role(&child) != Some(XDG_TOPLEVEL_ROLE)
            || !is_valid_parent(&child, &parent)
        {
            return;
        }
        if let Some(e) = state.foreign.v1.get_mut(&self.0) {
            e.child = Some((child.clone(), resource.clone()));
        }
        state.set_foreign_parent(&child, Some(&parent), &parent);
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, resource: &ZxdgImportedV1) {
        state.unparent_v1(&self.0, Some(resource));
    }
}

impl XdgForeignHandler for State {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.foreign.v2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_are_32_hex_digits_and_differ() {
        let (a, b) = (new_handle(), new_handle());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
