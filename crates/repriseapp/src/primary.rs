use std::sync::Arc;

use smithay_client_toolkit::{
    data_device_manager::{ReadPipe, WritePipe},
    primary_selection::{
        PrimarySelectionManagerState,
        device::{PrimarySelectionDevice, PrimarySelectionDeviceHandler},
        selection::{PrimarySelectionSource, PrimarySelectionSourceHandler},
    },
    reexports::client::{Connection, QueueHandle, globals::GlobalList, protocol::wl_seat::WlSeat},
    reexports::protocols::wp::primary_selection::zv1::client::{
        zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
    },
};

use super::Host;

const MIMES: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

/// The seat's primary selection. Without the global every request is a no-op.
pub(crate) struct Primary {
    manager: Option<PrimarySelectionManagerState>,
    device: Option<PrimarySelectionDevice>,
    source: Option<PrimarySelectionSource>,
    text: Arc<[u8]>,
    warned: bool,
}

impl Primary {
    pub(crate) fn bind(globals: &GlobalList, qh: &QueueHandle<Host>) -> Primary {
        Primary {
            manager: PrimarySelectionManagerState::bind(globals, qh).ok(),
            device: None,
            source: None,
            text: Arc::default(),
            warned: false,
        }
    }

    pub(crate) fn attach(&mut self, qh: &QueueHandle<Host>, seat: &WlSeat) {
        if let Some(manager) = &self.manager
            && self.device.is_none()
        {
            self.device = Some(manager.get_selection_device(qh, seat));
        }
    }

    fn missing(&mut self) -> bool {
        let missing = self.manager.is_none() || self.device.is_none();
        if missing && !std::mem::replace(&mut self.warned, true) {
            log::info!("primary selection: not offered by the compositor");
        }
        missing
    }

    pub(crate) fn copy(&mut self, qh: &QueueHandle<Host>, text: String, serial: u32) {
        if self.missing() {
            return;
        }
        let (Some(manager), Some(device)) = (&self.manager, &self.device) else {
            return;
        };
        let source = manager.create_selection_source(qh, MIMES);
        source.set_selection(device, serial);
        self.text = text.into_bytes().into();
        self.source = Some(source);
    }

    pub(crate) fn receive(&mut self) -> Option<ReadPipe> {
        if self.missing() {
            return None;
        }
        let offer = self.device.as_ref()?.data().selection_offer()?;
        let mime = MIMES
            .iter()
            .find(|m| offer.with_mime_types(|types| types.iter().any(|t| t == *m)))?;
        offer.receive(mime.to_string()).ok()
    }
}

impl PrimarySelectionDeviceHandler for Host {
    fn selection(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ZwpPrimarySelectionDeviceV1,
    ) {
    }
}

impl PrimarySelectionSourceHandler for Host {
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
        mime: String,
        fd: WritePipe,
    ) {
        let ours = self
            .primary
            .source
            .as_ref()
            .is_some_and(|s| s.inner() == source);
        if ours && MIMES.contains(&mime.as_str()) {
            reprisefs::pipe::write_detached(fd.into(), self.primary.text.clone());
        }
    }

    fn cancelled(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
    ) {
        if self
            .primary
            .source
            .as_ref()
            .is_some_and(|s| s.inner() == source)
        {
            self.primary.source = None;
        }
    }
}
