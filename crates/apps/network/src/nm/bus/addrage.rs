//! When an interface took its oldest global address, from the kernel's per-address creation
//! stamp (rtnetlink `IFA_CACHEINFO`). The status sheet's Duration counts from it, so a link that
//! came up before the tray started shows its real age.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

const NLMSG_HDR: usize = 16;
const IFADDRMSG: usize = 8;

/// When `iface` took its oldest address of global scope; None when it has none or the kernel
/// does not answer.
pub fn since(iface: &str) -> Option<Instant> {
    let name = std::ffi::CString::new(iface).ok()?;
    // SAFETY: a NUL-terminated name; the call only reads it.
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if index == 0 {
        return None;
    }
    let stamp = oldest_stamp(index).ok()??;
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: fills the one timespec passed.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } != 0 {
        return None;
    }
    let now_cs = now.tv_sec as u64 * 100 + now.tv_nsec as u64 / 10_000_000;
    let age = now_cs.checked_sub(u64::from(stamp))?;
    Instant::now().checked_sub(Duration::from_millis(age * 10))
}

/// The oldest `cstamp` (hundredths of a second since boot, on the monotonic clock) of
/// interface `index`'s global addresses, from one `RTM_GETADDR` dump.
fn oldest_stamp(index: u32) -> io::Result<Option<u32>> {
    // SAFETY: a plain socket call; the descriptor is owned below.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: just opened and owned only here.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut req = [0u8; NLMSG_HDR + IFADDRMSG];
    let len = req.len() as u32;
    req[0..4].copy_from_slice(&len.to_ne_bytes());
    req[4..6].copy_from_slice(&libc::RTM_GETADDR.to_ne_bytes());
    let flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
    req[6..8].copy_from_slice(&flags.to_ne_bytes());
    req[8..12].copy_from_slice(&1u32.to_ne_bytes());
    // SAFETY: sends the request buffer, which outlives the call.
    if unsafe { libc::send(fd.as_raw_fd(), req.as_ptr().cast(), req.len(), 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u8; 32 * 1024];
    let mut oldest = None;
    loop {
        // SAFETY: receives into `buf`, at most its length.
        let n = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 || scan(&buf[..n as usize], index, &mut oldest) {
            return Ok(oldest);
        }
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Folds one received chunk of the dump into `oldest`; true once the dump is over.
fn scan(chunk: &[u8], index: u32, oldest: &mut Option<u32>) -> bool {
    let mut at = 0;
    while let (Some(len), Some(kind)) = (u32_at(chunk, at), u16_at(chunk, at + 4)) {
        let len = len as usize;
        if len < NLMSG_HDR || at + len > chunk.len() {
            return true;
        }
        match kind {
            k if k == libc::NLMSG_DONE as u16 || k == libc::NLMSG_ERROR as u16 => return true,
            libc::RTM_NEWADDR => {
                if let Some(stamp) = stamp(&chunk[at + NLMSG_HDR..at + len], index) {
                    *oldest = Some(oldest.map_or(stamp, |o: u32| o.min(stamp)));
                }
            }
            _ => {}
        }
        at += len.next_multiple_of(4);
    }
    false
}

/// The creation stamp in one `ifaddrmsg` and its attributes, when it is a global address of
/// interface `index`.
fn stamp(msg: &[u8], index: u32) -> Option<u32> {
    let scope = *msg.get(3)?;
    if scope != libc::RT_SCOPE_UNIVERSE || u32_at(msg, 4)? != index {
        return None;
    }
    let mut at = IFADDRMSG;
    while let (Some(len), Some(kind)) = (u16_at(msg, at), u16_at(msg, at + 2)) {
        let len = usize::from(len);
        if len < 4 {
            return None;
        }
        if kind == libc::IFA_CACHEINFO {
            // `ifa_cacheinfo`: preferred, valid, cstamp, tstamp.
            return u32_at(msg, at + 4 + 8);
        }
        at += len.next_multiple_of(4);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `RTM_NEWADDR` for interface `index` at `scope`, with an address and its cache info.
    fn newaddr(index: u32, scope: u8, cstamp: u32) -> Vec<u8> {
        let mut m = vec![libc::AF_INET as u8, 24, 0, scope];
        m.extend(index.to_ne_bytes());
        m.extend(8u16.to_ne_bytes());
        m.extend(libc::IFA_ADDRESS.to_ne_bytes());
        m.extend([192, 168, 1, 2]);
        m.extend(20u16.to_ne_bytes());
        m.extend(libc::IFA_CACHEINFO.to_ne_bytes());
        for v in [u32::MAX, u32::MAX, cstamp, cstamp + 500] {
            m.extend(v.to_ne_bytes());
        }
        message(libc::RTM_NEWADDR, &m)
    }

    fn message(kind: u16, body: &[u8]) -> Vec<u8> {
        let mut out = ((NLMSG_HDR + body.len()) as u32).to_ne_bytes().to_vec();
        out.extend(kind.to_ne_bytes());
        out.extend([0; 10]);
        out.extend(body);
        out
    }

    #[test]
    fn the_oldest_global_address_of_the_interface_dates_it() {
        let mut chunk = Vec::new();
        chunk.extend(newaddr(2, libc::RT_SCOPE_UNIVERSE, 9_000));
        chunk.extend(newaddr(2, libc::RT_SCOPE_UNIVERSE, 4_000));
        chunk.extend(newaddr(2, libc::RT_SCOPE_LINK, 100));
        chunk.extend(newaddr(3, libc::RT_SCOPE_UNIVERSE, 50));
        let mut oldest = None;
        assert!(!scan(&chunk, 2, &mut oldest), "the dump goes on");
        assert_eq!(oldest, Some(4_000));
        assert!(scan(
            &message(libc::NLMSG_DONE as u16, &[0; 4]),
            2,
            &mut oldest
        ));
        assert_eq!(oldest, Some(4_000));
    }

    #[test]
    fn an_interface_with_no_global_address_has_no_date() {
        let mut oldest = None;
        scan(&newaddr(2, libc::RT_SCOPE_LINK, 100), 2, &mut oldest);
        assert_eq!(oldest, None);
        assert_eq!(since("lo"), None, "loopback's addresses are host scope");
    }
}
