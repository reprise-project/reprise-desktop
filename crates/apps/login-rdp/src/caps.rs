//! The capabilities the unit grants for binding the port, dropped once it is bound.

use std::io;

/// `_LINUX_CAPABILITY_VERSION_3`: two data words of 32 capabilities each.
const VERSION_3: u32 = 0x2008_0522;

#[repr(C)]
struct Header {
    version: u32,
    pid: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Data {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// Clears the calling thread's ambient, effective, permitted and inheritable sets; threads it
/// starts later inherit the empty sets.
pub fn drop_all() -> io::Result<()> {
    // SAFETY: PR_CAP_AMBIENT_CLEAR_ALL takes no pointers; the unused arguments are zero.
    let cleared = unsafe {
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        )
    };
    if cleared != 0 {
        return Err(io::Error::last_os_error());
    }
    let header = Header {
        version: VERSION_3,
        pid: 0,
    };
    let data = [Data::default(); 2];
    // SAFETY: capset reads one header and, for version 3, two data structs, both alive here.
    let set = unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) };
    if set != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(status: &str, field: &str) -> u64 {
        let line = status.lines().find(|l| l.starts_with(field)).unwrap();
        u64::from_str_radix(line[field.len()..].trim(), 16).unwrap()
    }

    #[test]
    fn dropping_leaves_the_thread_no_capability() {
        let status = std::thread::spawn(|| {
            drop_all().unwrap();
            std::fs::read_to_string("/proc/thread-self/status").unwrap()
        })
        .join()
        .unwrap();
        for field in ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"] {
            assert_eq!(caps(&status, field), 0, "{field}");
        }
    }
}
