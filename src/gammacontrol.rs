//! wlr-gamma-control-unstable-v1, the protocol side: one client at a time holds an output's gamma
//! ramp; `nightlight` implements the handler and steps aside while it does.
//!
//! Follows niri's `src/protocols/gamma_control.rs` (GPL-3.0-or-later).

use std::fs::File;
use std::io::Read;

use smithay::{
    output::Output,
    reexports::{
        wayland_protocols_wlr::gamma_control::v1::server::{
            zwlr_gamma_control_manager_v1::{self, ZwlrGammaControlManagerV1},
            zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, backend::ClientId,
        },
    },
    wayland::{Dispatch2, GlobalDispatch2},
};

const VERSION: u32 = 1;

/// The compositor's side of gamma control.
pub trait GammaControlHandler {
    fn gamma_control_state(&mut self) -> &mut GammaControlState;
    /// Entries per channel `output` takes, None when it takes no ramp.
    fn gamma_size(&mut self, output: &Output) -> Option<u32>;
    /// A client's ramp (red, green, blue, `gamma_size` each), or None when the client let go.
    /// Returns whether it took.
    fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> bool;
}

/// The live controls, one per output at most. Failed ones are dropped.
pub struct GammaControlState {
    controls: Vec<(Output, ZwlrGammaControlV1)>,
}

pub struct ManagerGlobal {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub struct ManagerData;

pub struct ControlData {
    gamma_size: u32,
}

impl GammaControlState {
    pub fn new<D, F>(dh: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrGammaControlManagerV1, ManagerGlobal> + 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        dh.create_global::<D, ZwlrGammaControlManagerV1, _>(
            VERSION,
            ManagerGlobal {
                filter: Box::new(filter),
            },
        );
        GammaControlState {
            controls: Vec::new(),
        }
    }

    /// Whether a client holds `output`'s gamma.
    pub fn held(&self, output: &Output) -> bool {
        self.controls.iter().any(|(o, _)| o == output)
    }

    /// Outputs went away: their controls fail.
    pub fn retain_outputs(&mut self, live: &[Output]) {
        self.controls.retain(|(o, c)| {
            let keep = live.contains(o);
            if !keep {
                c.failed();
            }
            keep
        });
    }
}

impl<D> GlobalDispatch2<ZwlrGammaControlManagerV1, D> for ManagerGlobal
where
    D: Dispatch<ZwlrGammaControlManagerV1, ManagerData> + 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrGammaControlManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ManagerData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<ZwlrGammaControlManagerV1, D> for ManagerData
where
    D: Dispatch<ZwlrGammaControlV1, ControlData> + GammaControlHandler + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrGammaControlManagerV1,
        request: zwlr_gamma_control_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } => {
                if let Some(output) = Output::from_resource(&output)
                    && !state.gamma_control_state().held(&output)
                    && let Some(gamma_size) = state.gamma_size(&output)
                {
                    let control = data_init.init(id, ControlData { gamma_size });
                    control.gamma_size(gamma_size);
                    state.gamma_control_state().controls.push((output, control));
                    return;
                }
                data_init.init(id, ControlData { gamma_size: 0 }).failed();
            }
            zwlr_gamma_control_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

/// The ramp in `file`: exactly `gamma_size` entries per channel, native endian, there already. The
/// file is made non-blocking first, so a pipe the client never fills fails instead of stalling the
/// loop.
fn read_ramp(file: &mut File, gamma_size: u32) -> Result<Vec<u16>, String> {
    nonblocking(file).map_err(|e| e.to_string())?;
    let mut bytes = vec![0u8; gamma_size as usize * 3 * 2];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    match file.read(&mut [0]) {
        Ok(0) => {}
        Ok(_) => return Err("the ramp is too long".into()),
        Err(e) => return Err(e.to_string()),
    }
    Ok(bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| u16::from_ne_bytes(b))
        .collect())
}

fn nonblocking(file: &File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    // SAFETY: `fd` stays open while `file` lives; F_GETFL and F_SETFL
    // touch only its status flags.
    let ok = unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        flags >= 0 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) >= 0
    };
    if ok {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

impl<D> Dispatch2<ZwlrGammaControlV1, D> for ControlData
where
    D: GammaControlHandler + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        resource: &ZwlrGammaControlV1,
        request: zwlr_gamma_control_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_gamma_control_v1::Request::SetGamma { fd } => {
                let controls = &state.gamma_control_state().controls;
                let Some(output) = controls
                    .iter()
                    .find(|(_, c)| c == resource)
                    .map(|(o, _)| o.clone())
                else {
                    return;
                };
                let took = match read_ramp(&mut File::from(fd), self.gamma_size) {
                    Ok(ramp) => state.set_gamma(&output, Some(ramp)),
                    Err(e) => {
                        log::warn!("gamma control on {}: {e}", output.name());
                        false
                    }
                };
                if !took {
                    resource.failed();
                    state
                        .gamma_control_state()
                        .controls
                        .retain(|(_, c)| c != resource);
                    state.set_gamma(&output, None);
                }
            }
            zwlr_gamma_control_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, resource: &ZwlrGammaControlV1) {
        let controls = &mut state.gamma_control_state().controls;
        let Some(i) = controls.iter().position(|(_, c)| c == resource) else {
            return;
        };
        let (output, _) = controls.remove(i);
        state.set_gamma(&output, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    fn file_with(bytes: &[u8]) -> File {
        let mut f = tempfile_in_tmp();
        f.write_all(bytes).unwrap();
        f.rewind().unwrap();
        f
    }

    fn tempfile_in_tmp() -> File {
        let path = std::env::temp_dir().join(format!(
            "reprise-gamma-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        f
    }

    #[test]
    fn a_ramp_from_an_empty_pipe_fails_at_once() {
        let (read, _write) = std::io::pipe().unwrap();
        let mut file = File::from(std::os::fd::OwnedFd::from(read));
        assert!(read_ramp(&mut file, 2).is_err(), "no ramp, no wait");
    }

    #[test]
    fn a_ramp_reads_three_channels_of_the_advertised_size() {
        let ramp: Vec<u16> = (0..6).map(|i| i * 1000).collect();
        let bytes: Vec<u8> = ramp.iter().flat_map(|v| v.to_ne_bytes()).collect();
        assert_eq!(read_ramp(&mut file_with(&bytes), 2).unwrap(), ramp);
    }

    #[test]
    fn a_ramp_of_the_wrong_length_is_refused() {
        assert!(read_ramp(&mut file_with(&[0; 10]), 2).is_err(), "short");
        assert!(read_ramp(&mut file_with(&[0; 14]), 2).is_err(), "long");
    }
}
