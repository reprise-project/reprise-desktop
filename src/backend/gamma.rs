use smithay::{
    backend::drm::{DrmDevice, DrmDeviceFd},
    output::Output,
    reexports::drm::control::crtc,
};

use crate::compositor::State;

use super::render::send_job;
use super::steps::watch;
use super::{CrtcNow, Head, OutputHead, Surface, Work, commit, surface_of};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Lut {
    Identity,
    Ramp(Vec<u16>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GammaStep {
    Skip,
    Wait,
    Commit,
}

/// A gamma set is a blocking commit: it waits while the session is away, the output is off, a flip is in
/// flight, or no frame was queued since power-on. A LUT another master left (`held` None) is replaced before
/// that first frame, which would show it.
pub(super) fn gamma_step(wanted: &Lut, held: Option<&Lut>, crtc: CrtcNow) -> GammaStep {
    if held == Some(wanted) {
        return GammaStep::Skip;
    }
    let crtc = CrtcNow {
        unlit: crtc.unlit && held.is_some(),
        ..crtc
    };
    if crtc.ready() {
        GammaStep::Commit
    } else {
        GammaStep::Wait
    }
}

/// What a CRTC whose GAMMA_LUT reads `value` holds, given the blob we last put there (`ours`, holding `held`).
pub(super) fn lut_from_blob(value: u64, ours: Option<u64>, held: Option<Lut>) -> Option<Lut> {
    if value == 0 {
        Some(Lut::Identity)
    } else if ours == Some(value) {
        held
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Gamma {
    Lut {
        prop: smithay::reexports::drm::control::property::Handle,
        size: u32,
        blob: Option<u64>,
    },
    Legacy {
        size: u32,
    },
}

impl Gamma {
    fn size(&self) -> u32 {
        match *self {
            Gamma::Lut { size, .. } | Gamma::Legacy { size } => size,
        }
    }
}

pub(super) fn find_gamma(dev: &DrmDevice, crtc: crtc::Handle) -> Option<Gamma> {
    use smithay::reexports::drm::control::{Device as _, property};
    let mut lut = None;
    let mut lut_size = None;
    if let Ok(props) = dev.get_properties(crtc) {
        for (prop, value) in props {
            let Ok(info) = dev.get_property(prop) else {
                continue;
            };
            match info.name().to_str() {
                Ok("GAMMA_LUT") if matches!(info.value_type(), property::ValueType::Blob) => {
                    lut = Some(prop)
                }
                Ok("GAMMA_LUT_SIZE") => lut_size = u32::try_from(value).ok(),
                _ => {}
            }
        }
    }
    if let (Some(prop), Some(size)) = (lut, lut_size.filter(|&s| s > 1)) {
        return Some(Gamma::Lut {
            prop,
            size,
            blob: None,
        });
    }
    let size = dev.get_crtc(crtc).ok()?.gamma_length();
    (size > 1).then_some(Gamma::Legacy { size })
}

/// What the CRTC's gamma holds now: GAMMA_LUT is read back, a legacy table is unknown.
pub(super) fn read_lut(
    dev: &DrmDevice,
    crtc: crtc::Handle,
    gamma: &Gamma,
    held: Option<Lut>,
) -> Option<Lut> {
    use smithay::reexports::drm::control::Device as _;
    let Gamma::Lut { prop, blob, .. } = *gamma else {
        return None;
    };
    let props = dev.get_properties(crtc).ok()?;
    let (handles, values) = props.as_props_and_values();
    let i = handles.iter().position(|&h| h == prop)?;
    lut_from_blob(values[i], blob, held)
}

fn create_blob(dev: &impl std::os::fd::AsFd, data: &[u8]) -> std::io::Result<u64> {
    use std::os::fd::AsRawFd;
    #[repr(C)]
    struct CreateBlob {
        data: u64,
        length: u32,
        blob_id: u32,
    }
    const DRM_IOCTL_MODE_CREATEPROPBLOB: libc::c_ulong = 0xC010_64BD;
    let mut arg = CreateBlob {
        data: data.as_ptr() as u64,
        length: data.len() as u32,
        blob_id: 0,
    };
    // SAFETY: `arg` is the kernel's `drm_mode_create_blob` layout and points
    // at `data`, which outlives the call; the kernel copies it.
    let r = unsafe {
        libc::ioctl(
            dev.as_fd().as_raw_fd(),
            DRM_IOCTL_MODE_CREATEPROPBLOB,
            &mut arg,
        )
    };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(u64::from(arg.blob_id))
}

fn lut_bytes(ramp: &[u16], size: usize) -> Vec<u8> {
    let (r, rest) = ramp.split_at(size);
    let (g, b) = rest.split_at(size);
    (0..size)
        .flat_map(|i| [r[i], g[i], b[i], 0])
        .flat_map(u16::to_ne_bytes)
        .collect()
}

pub fn gamma_size(state: &State, output: &Output) -> Option<usize> {
    let head = output.user_data().get::<OutputHead>()?.0;
    let b = state.backend.as_ref()?;
    let s = b.surfaces.get(&head)?;
    s.hw.gamma.map(|g| g.size() as usize)
}

/// Put `ramp` (None: identity) on `output`'s CRTC, now or once it is ready; nothing when it holds it already.
/// False when the output has no gamma table; a ramp the kernel refuses later goes to
/// `nightlight::gamma_failed`.
pub fn set_gamma(state: &mut State, output: &Output, ramp: Option<&[u16]>) -> bool {
    let Some((b, head)) = surface_of(state, output) else {
        return false;
    };
    let active = b.settled();
    let Some(s) = b.surfaces.get_mut(&head) else {
        return false;
    };
    let Some(g) = s.hw.gamma else {
        return false;
    };
    if ramp.is_some_and(|r| r.len() != g.size() as usize * 3) {
        log::warn!("{}: gamma ramp of the wrong length", output.name());
        return false;
    }
    let wanted = ramp.map_or(Lut::Identity, |r| Lut::Ramp(r.to_vec()));
    match gamma_step(&wanted, s.hw.lut.as_ref(), CrtcNow::of(active, s)) {
        GammaStep::Skip => {
            s.hw.wanted_lut = None;
            true
        }
        GammaStep::Wait => {
            s.hw.wanted_lut = Some(wanted);
            true
        }
        GammaStep::Commit => {
            s.hw.wanted_lut = None;
            let sent = commit_gamma(s, wanted);
            watch(state);
            sent
        }
    }
}

/// Put a ramp that waited on the CRTC if it can take it now (`gamma_step`).
pub(super) fn flush_gamma(state: &mut State, head: Head) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let active = b.settled();
    let Some(s) = b.surfaces.get_mut(&head) else {
        return;
    };
    let Some(wanted) = s.hw.wanted_lut.take() else {
        return;
    };
    match gamma_step(&wanted, s.hw.lut.as_ref(), CrtcNow::of(active, s)) {
        GammaStep::Skip => {}
        GammaStep::Wait => s.hw.wanted_lut = Some(wanted),
        GammaStep::Commit => {
            commit_gamma(s, wanted);
        }
    }
}

/// Hand `wanted` to the output's commit thread (`write_gamma`); false when it is gone.
fn commit_gamma(s: &mut Surface, wanted: Lut) -> bool {
    let Some(gamma) = s.hw.gamma else {
        return false;
    };
    send_job(s, commit::Kind::Gamma, Work::Gamma { gamma, wanted })
}

/// Put `wanted` on `crtc`'s gamma, on the commit thread. Returns the GAMMA_LUT blob now
/// holding it; the one `gamma` held before is destroyed.
pub(super) fn write_gamma(
    fd: &DrmDeviceFd,
    crtc: crtc::Handle,
    gamma: Gamma,
    wanted: &Lut,
) -> Result<Option<u64>, String> {
    use smithay::reexports::drm::control::{Device as _, property};
    let size = gamma.size() as usize;
    let ramp = match wanted {
        Lut::Ramp(r) => Some(r.as_slice()),
        Lut::Identity => None,
    };
    match gamma {
        Gamma::Lut { prop, blob, .. } => {
            let new = match ramp {
                Some(r) => Some(
                    create_blob(fd, &lut_bytes(r, size)).map_err(|e| format!("gamma blob: {e}"))?,
                ),
                None => None,
            };
            let value = property::Value::Blob(new.unwrap_or(0)).into();
            match fd.set_property(crtc, prop, value) {
                Ok(()) => {
                    if let Some(old) = blob {
                        let _ = fd.destroy_property_blob(old);
                    }
                    Ok(new)
                }
                Err(e) => {
                    if let Some(id) = new {
                        let _ = fd.destroy_property_blob(id);
                    }
                    Err(e.to_string())
                }
            }
        }
        Gamma::Legacy { .. } => {
            let identity: Vec<u16>;
            let r = match ramp {
                Some(r) => r,
                None => {
                    identity = (0..3)
                        .flat_map(|_| {
                            (0..size).map(move |i| (i * 0xffff / (size - 1).max(1)) as u16)
                        })
                        .collect();
                    &identity
                }
            };
            let (red, rest) = r.split_at(size);
            let (green, blue) = rest.split_at(size);
            fd.set_gamma(crtc, red, green, blue)
                .map(|()| None)
                .map_err(|e| e.to_string())
        }
    }
}
