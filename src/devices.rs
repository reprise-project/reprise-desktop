//! The touchscreens and drawing tablets: the state each kind
//! keeps (`touch`, `tablet`, `tabletpad`) and what a device coming or going
//! changes. Keyboards and pointers are the keyboard and pointer code's.

use smithay::{
    backend::input::{Device, DeviceCapability},
    input::tablet::{TabletDescriptor, TabletSeatTrait},
    reexports::wayland_server::DisplayHandle,
};

use crate::{compositor::State, tablet::TabletState, touch::TouchState};

pub struct Devices {
    pub touch: TouchState,
    pub tablet: TabletState,
    pub pads: crate::tabletpad::Pads,
}

impl Devices {
    pub fn new(dh: &DisplayHandle) -> Devices {
        Devices {
            touch: TouchState::default(),
            tablet: TabletState::new(dh),
            pads: crate::tabletpad::Pads::default(),
        }
    }
}

impl State {
    /// A new input device: a touchscreen gives the seat wl_touch, a tablet
    /// shows up in tablet-v2. A switch needs nothing until it toggles.
    pub fn device_added(&mut self, device: &impl Device) {
        let name = device.name();
        if device.has_capability(DeviceCapability::TabletTool) {
            log::info!("new tablet {name}");
            let dh = self.dh.clone();
            self.seat
                .tablet_seat()
                .add_wp_tablet(&dh, &TabletDescriptor::from(device));
        }
        if device.has_capability(DeviceCapability::Touch) {
            log::info!("new touchscreen {name}");
            self.touch_added(device);
        }
        if device.has_capability(DeviceCapability::Switch) {
            log::info!("new switch {name}");
        }
    }

    pub fn device_removed(&mut self, device: &impl Device) {
        if device.has_capability(DeviceCapability::TabletTool) {
            self.tablet_removed(&TabletDescriptor::from(device));
        }
        if device.has_capability(DeviceCapability::Touch) {
            self.touch_removed(device);
        }
    }
}
