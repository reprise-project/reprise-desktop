//! wlr-output-power-management-v1: `wlopm` and idle daemons switch an
//! output's scanout off and on via the backend's DPMS.

use smithay::{
    output::{Output, WeakOutput},
    reexports::{
        wayland_protocols_wlr::output_power_management::v1::server::{
            zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
            zwlr_output_power_v1::{self, Mode, ZwlrOutputPowerV1},
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

/// The live `zwlr_output_power_v1` objects, each with the mode it was last sent.
pub struct OutputPowerState {
    controls: Vec<(ZwlrOutputPowerV1, bool)>,
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
        let filter = Box::new(filter);
        dh.create_global::<D, ZwlrOutputPowerManagerV1, _>(VERSION, ManagerGlobal { filter });
        OutputPowerState {
            controls: Vec::new(),
        }
    }

    /// Send the mode to every object whose output changed power since it last heard.
    pub fn refresh<D: OutputPowerHandler>(state: &mut D) {
        // Taken out so `output_power` can borrow `state`; refresh runs outside
        // dispatch, so no object is added meanwhile.
        let mut controls = std::mem::take(&mut state.output_power_state().controls);
        for (control, sent) in &mut controls {
            let Some(output) = output_of(control) else {
                continue;
            };
            if let Some(on) = state.output_power(&output) {
                announce(control, sent, on);
            }
        }
        state.output_power_state().controls = controls;
    }

    fn forget(&mut self, control: &ZwlrOutputPowerV1) {
        self.controls.retain(|(c, _)| c != control);
    }
}

fn output_of(control: &ZwlrOutputPowerV1) -> Option<Output> {
    control.data::<PowerData>()?.output.upgrade()
}

fn mode(on: bool) -> Mode {
    if on { Mode::On } else { Mode::Off }
}

fn announce(control: &ZwlrOutputPowerV1, sent: &mut bool, on: bool) {
    if *sent != on {
        control.mode(mode(on));
        *sent = on;
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
        let zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } = request else {
            return;
        };
        let output = Output::from_resource(&output);
        let weak = output.as_ref().map(Output::downgrade).unwrap_or_default();
        let control = data_init.init(id, PowerData { output: weak });
        match output.and_then(|o| state.output_power(&o)) {
            Some(on) => {
                control.mode(mode(on));
                state.output_power_state().controls.push((control, on));
            }
            None => control.failed(),
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
        let zwlr_output_power_v1::Request::SetMode { mode } = request else {
            return;
        };
        let on = match mode {
            WEnum::Value(Mode::On) => true,
            WEnum::Value(Mode::Off) => false,
            _ => return,
        };
        let Some(output) = self.output.upgrade() else {
            resource.failed();
            state.output_power_state().forget(resource);
            return;
        };
        state.set_output_power(&output, on);
        let Some(on) = state.output_power(&output) else {
            resource.failed();
            state.output_power_state().forget(resource);
            return;
        };
        for (control, sent) in &mut state.output_power_state().controls {
            if output_of(control).is_some_and(|o| o == output) {
                announce(control, sent, on);
            }
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, resource: &ZwlrOutputPowerV1) {
        state.output_power_state().forget(resource);
    }
}
