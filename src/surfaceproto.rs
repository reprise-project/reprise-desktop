//! Protocols that only shape how a surface is drawn: viewporter,
//! fractional-scale, single-pixel-buffer, content-type, alpha-modifier,
//! ext-background-effect (`crate::clientglass`) and linux-drm-syncobj.
//! Smithay keeps their state on the surface; Reprise adds the globals, a
//! first preferred scale and the commit blockers.
//!
//! A commit with a dmabuf waits for the buffer to be ready before it
//! applies: on its explicit-sync acquire point when it set one, else on
//! the dmabuf's implicit fence. The blocker follows Smithay's
//! `anvil/src/shell/mod.rs`.

use smithay::{
    backend::drm::DrmDeviceFd,
    reexports::{
        calloop::Interest,
        wayland_server::{DisplayHandle, Resource, protocol::wl_surface::WlSurface},
    },
    wayland::{
        alpha_modifier::AlphaModifierState,
        background_effect::BackgroundEffectState,
        compositor::{
            BufferAssignment, CompositorHandler, SurfaceAttributes, add_blocker,
            add_pre_commit_hook, with_states,
        },
        content_type::ContentTypeState,
        dmabuf::get_dmabuf,
        drm_syncobj::{
            DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd,
        },
        fractional_scale::{
            FractionalScaleHandler, FractionalScaleManagerState, with_fractional_scale,
        },
        single_pixel_buffer::SinglePixelBufferState,
        viewporter::ViewporterState,
    },
};

use crate::compositor::State;

pub struct SurfaceProtos {
    #[allow(dead_code)]
    viewporter: ViewporterState,
    #[allow(dead_code)]
    fractional_scale: FractionalScaleManagerState,
    #[allow(dead_code)]
    single_pixel_buffer: SinglePixelBufferState,
    #[allow(dead_code)]
    content_type: ContentTypeState,
    #[allow(dead_code)]
    alpha_modifier: AlphaModifierState,
    #[allow(dead_code)]
    background_effect: BackgroundEffectState,
    syncobj: Option<DrmSyncobjState>,
}

impl SurfaceProtos {
    pub fn new(dh: &DisplayHandle) -> SurfaceProtos {
        SurfaceProtos {
            viewporter: ViewporterState::new::<State>(dh),
            fractional_scale: FractionalScaleManagerState::new::<State>(dh),
            single_pixel_buffer: SinglePixelBufferState::new::<State>(dh),
            content_type: ContentTypeState::new::<State>(dh),
            alpha_modifier: AlphaModifierState::new::<State>(dh),
            background_effect: BackgroundEffectState::new::<State>(dh),
            syncobj: None,
        }
    }

    /// Offer linux-drm-syncobj when `device` can wait on a timeline point
    /// through an eventfd; otherwise clients keep implicit sync.
    pub fn init_syncobj(&mut self, dh: &DisplayHandle, device: DrmDeviceFd) {
        if self.syncobj.is_some() {
            return;
        }
        if supports_syncobj_eventfd(&device) {
            self.syncobj = Some(DrmSyncobjState::new::<State>(dh, device));
            log::info!("explicit sync: linux-drm-syncobj offered");
        } else {
            log::info!("explicit sync: the GPU has no syncobj eventfd; not offered");
        }
    }
}

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.surface_protos.syncobj.as_mut()
    }
}

pub fn on_new_surface(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _dh, surface| {
        let mut acquire = None;
        let dmabuf = with_states(surface, |data| {
            acquire.clone_from(
                &data
                    .cached_state
                    .get::<DrmSyncobjCachedState>()
                    .pending()
                    .acquire_point,
            );
            data.cached_state
                .get::<SurfaceAttributes>()
                .pending()
                .buffer
                .as_ref()
                .and_then(|a| match a {
                    BufferAssignment::NewBuffer(b) => get_dmabuf(b).cloned().ok(),
                    _ => None,
                })
        });
        let Some(dmabuf) = dmabuf else {
            return;
        };
        let Some(client) = surface.client() else {
            return;
        };
        if let Some(point) = acquire
            && let Ok((blocker, source)) = point.generate_blocker()
        {
            let client = client.clone();
            let inserted = state.handle.insert_source(source, move |_, _, state| {
                let dh = state.dh.clone();
                state
                    .client_compositor_state(&client)
                    .blocker_cleared(state, &dh);
                Ok(())
            });
            if inserted.is_ok() {
                add_blocker(surface, blocker);
                return;
            }
        }
        if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
            let inserted = state.handle.insert_source(source, move |_, _, state| {
                let dh = state.dh.clone();
                state
                    .client_compositor_state(&client)
                    .blocker_cleared(state, &dh);
                Ok(())
            });
            if inserted.is_ok() {
                add_blocker(surface, blocker);
            }
        }
    });
}

/// Preferred scale: the output under the pointer until the surface shows
/// somewhere, then its primary output's.
impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let pointer = self.pointer_location();
        let outputs = self.scene.outputs();
        let output = outputs
            .iter()
            .find(|o| reprisescene::output_rect(o).is_some_and(|r| r.to_f64().contains(pointer)))
            .or(outputs.first());
        let Some(scale) = output.map(|o| o.current_scale().fractional_scale()) else {
            return;
        };
        with_states(&surface, |data| {
            with_fractional_scale(data, |fs| fs.set_preferred_scale(scale));
        });
    }
}
