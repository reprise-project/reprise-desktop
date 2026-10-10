//! wlr-output-power-management-v1: `wlopm` and idle daemons switch an
//! output's scanout off and on via the backend's DPMS.
//!
//! Follows cosmic-comp's `src/wayland/protocols/output_power.rs`
//! (GPL-3.0-only), on Smithay's `Dispatch2` so `delegate_dispatch2!`
//! covers it.

use std::collections::HashMap;

use smithay::{
    output::{Output, WeakOutput},
    reexports::{
        wayland_protocols_wlr::output_power_management::v1::server::{
            zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
            zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
            backend::ClientId,
        },
    },
    wayland::{Dispatch2, GlobalDispatch2},
};

const VERSION: u32 = 1;

/// Whether an output's scanout is on, and switching it.
pub trait OutputPowerHandler {
    fn output_power_state(&mut self) -> &mut OutputPowerState;
    fn output_power(&mut self, output: &Output) -> Option<bool>;
    fn set_output_power(&mut self, output: &Output, on: bool);
}

pub struct OutputPowerState {
    powers: HashMap<ZwlrOutputPowerV1, zwlr_output_power_v1::Mode>,
}

pub struct ManagerGlobal {
    filter: Box<dyn for<'a> Fn(&'a Client) -> bool + Send + Sync>,
}

pub struct ManagerData;

pub struct PowerData {
    output: WeakOutput,
}

impl OutputPowerState {
    pub fn new<D, F>(dh: &DisplayHandle, filter: F) -> OutputPowerState
    where
        D: GlobalDispatch<ZwlrOutputPowerManagerV1, ManagerGlobal> + 'static,
        F: for<'a> Fn(&'a Client) -> bool + Send + Sync + 'static,
    {
        dh.create_global::<D, ZwlrOutputPowerManagerV1, _>(
            VERSION,
            ManagerGlobal {
                filter: Box::new(filter),
            },
        );
        OutputPowerState {
            powers: HashMap::new(),
        }
    }

    /// Send the mode to every object whose output changed power since it last heard.
    pub fn refresh<D: OutputPowerHandler>(state: &mut D) {
        let mut powers = std::mem::take(&mut state.output_power_state().powers);
        for (power, old) in powers.iter_mut() {
            let Some(output) = power.data::<PowerData>().and_then(|d| d.output.upgrade()) else {
                continue;
            };
            if let Some(on) = state.output_power(&output) {
                let mode = mode_of(on);
                if mode != *old {
                    power.mode(mode);
                    *old = mode;
                }
            }
        }
        // A request handled meanwhile cannot have added any: refresh runs
        // outside dispatch.
        state.output_power_state().powers.extend(powers);
    }
}

fn mode_of(on: bool) -> zwlr_output_power_v1::Mode {
    if on {
        zwlr_output_power_v1::Mode::On
    } else {
        zwlr_output_power_v1::Mode::Off
    }
}

impl<D> GlobalDispatch2<ZwlrOutputPowerManagerV1, D> for ManagerGlobal
where
    D: Dispatch<ZwlrOutputPowerManagerV1, ManagerData> + 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ManagerData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<ZwlrOutputPowerManagerV1, D> for ManagerData
where
    D: Dispatch<ZwlrOutputPowerV1, PowerData> + OutputPowerHandler + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrOutputPowerManagerV1,
        request: zwlr_output_power_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } => {
                let output = Output::from_resource(&output);
                let power = data_init.init(
                    id,
                    PowerData {
                        output: output.as_ref().map(Output::downgrade).unwrap_or_default(),
                    },
                );
                match output.as_ref().and_then(|o| state.output_power(o)) {
                    Some(on) => {
                        power.mode(mode_of(on));
                        state.output_power_state().powers.insert(power, mode_of(on));
                    }
                    None => power.failed(),
                }
            }
            zwlr_output_power_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

impl<D> Dispatch2<ZwlrOutputPowerV1, D> for PowerData
where
    D: OutputPowerHandler + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        resource: &ZwlrOutputPowerV1,
        request: zwlr_output_power_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_v1::Request::SetMode { mode } => {
                let on = match mode {
                    WEnum::Value(zwlr_output_power_v1::Mode::On) => true,
                    WEnum::Value(zwlr_output_power_v1::Mode::Off) => false,
                    _ => return,
                };
                let Some(output) = self.output.upgrade() else {
                    resource.failed();
                    state.output_power_state().powers.remove(resource);
                    return;
                };
                state.set_output_power(&output, on);
                match state.output_power(&output) {
                    Some(on) => {
                        let mode = mode_of(on);
                        for (power, old) in state.output_power_state().powers.iter_mut() {
                            let same = power
                                .data::<PowerData>()
                                .is_some_and(|d| d.output == output);
                            if same && mode != *old {
                                power.mode(mode);
                                *old = mode;
                            }
                        }
                    }
                    None => {
                        resource.failed();
                        state.output_power_state().powers.remove(resource);
                    }
                }
            }
            zwlr_output_power_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, resource: &ZwlrOutputPowerV1) {
        state.output_power_state().powers.remove(resource);
    }
}
