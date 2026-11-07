use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

pub const GREETER_VT: u16 = 1;
pub const TEXT_LOGIN_VT: u16 = 2;
pub const FIRST_SESSION_VT: u16 = 7;
pub const LAST_VT: u16 = 63;

/// The first free session VT. `kernel_open` is `VT_GETSTATE`'s `v_state`, which covers VTs 1-15.
pub fn pick(taken: &BTreeSet<u16>, kernel_open: u16) -> Option<u16> {
    (FIRST_SESSION_VT..=LAST_VT)
        .find(|&vt| !taken.contains(&vt) && (vt > 15 || kernel_open & (1 << vt) == 0))
}

const VT_GETSTATE: libc::c_ulong = 0x5603;
const VT_ACTIVATE: libc::c_ulong = 0x5606;
const VT_WAITACTIVE: libc::c_ulong = 0x5607;
const VT_DISALLOCATE: libc::c_ulong = 0x5608;

/// `struct vt_stat` from `<linux/vt.h>`.
#[repr(C)]
#[derive(Default)]
struct VtStat {
    v_active: libc::c_ushort,
    v_signal: libc::c_ushort,
    v_state: libc::c_ushort,
}

fn tty0() -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open("/dev/tty0")
}

/// Runs a VT ioctl whose argument is a VT number, retrying on `EINTR`.
fn vt_ioctl(tty: &File, request: libc::c_ulong, vt: u16) -> io::Result<()> {
    loop {
        // SAFETY: the fd is open for the call, and these requests take the VT number by value.
        let r = unsafe { libc::ioctl(tty.as_raw_fd(), request as _, libc::c_int::from(vt)) };
        if r == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// The kernel's bitmask of open VTs 1-15, bit n for VT n.
pub fn open_mask() -> io::Result<u16> {
    let tty = tty0()?;
    let mut stat = VtStat::default();
    // SAFETY: the fd is open, and `stat` is a live `struct vt_stat` the kernel fills.
    let r = unsafe { libc::ioctl(tty.as_raw_fd(), VT_GETSTATE as _, &mut stat as *mut VtStat) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(stat.v_state)
}

/// Switches the console to `vt` and returns once it is the active VT.
pub fn activate_and_wait(vt: u16) -> io::Result<()> {
    let tty = tty0()?;
    vt_ioctl(&tty, VT_ACTIVATE, vt)?;
    vt_ioctl(&tty, VT_WAITACTIVE, vt)
}

/// Frees `vt` once nothing holds it open; a failure is logged.
pub fn disallocate(vt: u16) {
    if let Err(e) = tty0().and_then(|tty| vt_ioctl(&tty, VT_DISALLOCATE, vt)) {
        log::warn!("VT {vt}: disallocate: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_pick_is_7() {
        assert_eq!(pick(&BTreeSet::new(), 0), Some(7));
    }

    #[test]
    fn taken_and_kernel_open_vts_are_skipped() {
        assert_eq!(pick(&BTreeSet::from([7]), 1 << 8), Some(9));
    }

    #[test]
    fn past_15_only_the_table_counts() {
        let taken: BTreeSet<u16> = (7..=15).collect();
        assert_eq!(pick(&taken, u16::MAX), Some(16));
        let taken: BTreeSet<u16> = (7..=16).collect();
        assert_eq!(pick(&taken, 0), Some(17));
    }

    #[test]
    fn none_is_left_past_63() {
        let taken: BTreeSet<u16> = (FIRST_SESSION_VT..=LAST_VT).collect();
        assert_eq!(pick(&taken, 0), None);
    }
}
