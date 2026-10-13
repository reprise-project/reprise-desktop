//! The devices joined to a hotspot: a station dump over nl80211 (generic netlink), which any
//! user may read, and the `mlme` multicast group's station events to recount on.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const NLMSG_HDR: usize = 16;
const GENL_HDR: usize = 4;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_DUMP: u16 = 0x300;
const NLA_TYPE_MASK: u16 = 0x3fff;

const GENL_ID_CTRL: u16 = 0x10;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const CTRL_ATTR_MCAST_GROUPS: u16 = 7;
const CTRL_ATTR_MCAST_GRP_NAME: u16 = 1;
const CTRL_ATTR_MCAST_GRP_ID: u16 = 2;

const NL80211_CMD_GET_STATION: u8 = 17;
const NL80211_CMD_NEW_STATION: u8 = 19;
const NL80211_CMD_DEL_STATION: u8 = 20;
const NL80211_ATTR_IFINDEX: u16 = 3;

const SOL_NETLINK: libc::c_int = 270;
const NETLINK_ADD_MEMBERSHIP: libc::c_int = 1;

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// One netlink message: the header, a generic netlink header and `attrs` (type, payload).
fn message(
    kind: u16,
    flags: u16,
    seq: u32,
    cmd: u8,
    version: u8,
    attrs: &[(u16, &[u8])],
) -> Vec<u8> {
    let mut m = vec![0u8; NLMSG_HDR];
    m.extend_from_slice(&[cmd, version, 0, 0]);
    for (t, payload) in attrs {
        let len = 4 + payload.len();
        m.extend_from_slice(&(len as u16).to_ne_bytes());
        m.extend_from_slice(&t.to_ne_bytes());
        m.extend_from_slice(payload);
        m.resize(align4(m.len()), 0);
    }
    let len = m.len() as u32;
    m[0..4].copy_from_slice(&len.to_ne_bytes());
    m[4..6].copy_from_slice(&kind.to_ne_bytes());
    m[6..8].copy_from_slice(&flags.to_ne_bytes());
    m[8..12].copy_from_slice(&seq.to_ne_bytes());
    m
}

/// The netlink messages in a datagram: (type, body after the netlink header).
fn messages(buf: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let hdr = buf.get(at..at + NLMSG_HDR)?;
        let len = u32::from_ne_bytes(hdr[0..4].try_into().ok()?) as usize;
        let kind = u16::from_ne_bytes(hdr[4..6].try_into().ok()?);
        let body = buf.get(at + NLMSG_HDR..at + len.max(NLMSG_HDR))?;
        at += align4(len.max(NLMSG_HDR));
        Some((kind, body))
    })
}

/// The attributes in `buf`: (type without its flags, payload).
fn attrs(buf: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let hdr = buf.get(at..at + 4)?;
        let len = u16::from_ne_bytes([hdr[0], hdr[1]]) as usize;
        let kind = u16::from_ne_bytes([hdr[2], hdr[3]]) & NLA_TYPE_MASK;
        let payload = buf.get(at + 4..at + len.max(4))?;
        at += align4(len.max(4));
        Some((kind, payload))
    })
}

fn u16_of(b: &[u8]) -> Option<u16> {
    Some(u16::from_ne_bytes(b.get(..2)?.try_into().ok()?))
}

fn u32_of(b: &[u8]) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(..4)?.try_into().ok()?))
}

/// The errno of an `NLMSG_ERROR` body; 0 is an acknowledgement.
fn error_of(body: &[u8]) -> i32 {
    body.get(..4)
        .map_or(0, |b| i32::from_ne_bytes(b.try_into().unwrap_or_default()))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Family {
    id: u16,
    mlme: Option<u32>,
}

/// nl80211's family id and its `mlme` group from a `CTRL_CMD_GETFAMILY` reply's generic body.
fn parse_family(genl: &[u8]) -> Option<Family> {
    let mut f = Family::default();
    for (t, v) in attrs(genl.get(GENL_HDR..)?) {
        match t {
            CTRL_ATTR_FAMILY_ID => f.id = u16_of(v)?,
            CTRL_ATTR_MCAST_GROUPS => {
                for (_, group) in attrs(v) {
                    let (mut name, mut id) = (None, None);
                    for (t, v) in attrs(group) {
                        match t {
                            CTRL_ATTR_MCAST_GRP_NAME => name = Some(v),
                            CTRL_ATTR_MCAST_GRP_ID => id = u32_of(v),
                            _ => {}
                        }
                    }
                    if name == Some(b"mlme\0".as_slice()) {
                        f.mlme = id;
                    }
                }
            }
            _ => {}
        }
    }
    (f.id != 0).then_some(f)
}

/// The stations in a dump's datagram for `family`, and whether it ended there.
fn tally(buf: &[u8], family: u16) -> io::Result<(usize, bool)> {
    let mut n = 0;
    for (kind, body) in messages(buf) {
        match kind {
            NLMSG_DONE => return Ok((n, true)),
            NLMSG_ERROR => match error_of(body) {
                0 => return Ok((n, true)),
                e => return Err(io::Error::from_raw_os_error(-e)),
            },
            k if k == family => n += 1,
            _ => {}
        }
    }
    Ok((n, false))
}

fn socket() -> io::Result<OwnedFd> {
    // SAFETY: plain socket(2); the descriptor is owned from here.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_GENERIC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd was just returned by socket(2) and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn send(fd: &OwnedFd, msg: &[u8]) -> io::Result<()> {
    // SAFETY: msg is a live buffer of msg.len() bytes.
    let n = unsafe { libc::send(fd.as_raw_fd(), msg.as_ptr().cast(), msg.len(), 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn recv<'a>(fd: &OwnedFd, buf: &'a mut [u8]) -> io::Result<&'a [u8]> {
    loop {
        // SAFETY: buf is a live, writable buffer of buf.len() bytes.
        let n = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n >= 0 {
            return Ok(&buf[..n as usize]);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

fn family(fd: &OwnedFd) -> io::Result<Family> {
    let req = message(
        GENL_ID_CTRL,
        NLM_F_REQUEST,
        1,
        CTRL_CMD_GETFAMILY,
        1,
        &[(CTRL_ATTR_FAMILY_NAME, b"nl80211\0")],
    );
    send(fd, &req)?;
    let mut buf = vec![0u8; 16384];
    let reply = recv(fd, &mut buf)?;
    for (kind, body) in messages(reply) {
        match kind {
            NLMSG_ERROR if error_of(body) != 0 => {
                return Err(io::Error::from_raw_os_error(-error_of(body)));
            }
            GENL_ID_CTRL => {
                if let Some(f) = parse_family(body) {
                    return Ok(f);
                }
            }
            _ => {}
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "no nl80211 family"))
}

fn dump(iface: &str) -> io::Result<usize> {
    let name = CString::new(iface).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: name is a NUL-terminated string that outlives the call.
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if index == 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = socket()?;
    let fam = family(&fd)?;
    let req = message(
        fam.id,
        NLM_F_REQUEST | NLM_F_DUMP,
        2,
        NL80211_CMD_GET_STATION,
        0,
        &[(NL80211_ATTR_IFINDEX, &index.to_ne_bytes())],
    );
    send(&fd, &req)?;
    let mut buf = vec![0u8; 32768];
    let mut total = 0;
    loop {
        let (n, done) = tally(recv(&fd, &mut buf)?, fam.id)?;
        total += n;
        if done {
            return Ok(total);
        }
    }
}

/// The stations joined to the access point on `iface`; None when nl80211 will not say.
pub fn count(iface: &str) -> Option<usize> {
    dump(iface)
        .inspect_err(|e| log::debug!("no station dump for {iface}: {e}"))
        .ok()
}

fn listen(wake: impl Fn() + Send + 'static) -> io::Result<()> {
    let fd = socket()?;
    let fam = family(&fd)?;
    let group = fam
        .mlme
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no mlme group"))?;
    // SAFETY: group is a live u32 and its size is passed with it.
    let r = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            SOL_NETLINK,
            NETLINK_ADD_MEMBERSHIP,
            (&raw const group).cast(),
            size_of::<u32>() as libc::socklen_t,
        )
    };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    std::thread::Builder::new()
        .name("stations".into())
        .spawn(move || {
            let mut buf = vec![0u8; 16384];
            loop {
                let got = match recv(&fd, &mut buf) {
                    Ok(got) => got,
                    // A burst overran the socket; whatever it held, recount.
                    Err(e) if e.raw_os_error() == Some(libc::ENOBUFS) => {
                        wake();
                        continue;
                    }
                    Err(e) => {
                        log::warn!("station events: {e}");
                        return;
                    }
                };
                if messages(got).any(|(k, body)| k == fam.id && joins_or_leaves(body)) {
                    wake();
                }
            }
        })?;
    Ok(())
}

fn joins_or_leaves(genl: &[u8]) -> bool {
    matches!(
        genl.first(),
        Some(&NL80211_CMD_NEW_STATION | &NL80211_CMD_DEL_STATION)
    )
}

/// Call `wake` from a thread of its own whenever a station joins or leaves any access point.
/// Without the events the count follows NetworkManager's own changes only.
pub fn watch(wake: impl Fn() + Send + 'static) {
    if let Err(e) = listen(wake) {
        log::info!("station events are not watched: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut a = ((4 + payload.len()) as u16).to_ne_bytes().to_vec();
        a.extend_from_slice(&kind.to_ne_bytes());
        a.extend_from_slice(payload);
        a.resize(align4(a.len()), 0);
        a
    }

    const NESTED: u16 = 0x8000;

    #[test]
    fn the_family_reply_gives_nl80211s_id_and_mlme_group() {
        let group = |n: u16, name: &[u8], id: u32| {
            let inner = [
                attr(CTRL_ATTR_MCAST_GRP_NAME, name),
                attr(CTRL_ATTR_MCAST_GRP_ID, &id.to_ne_bytes()),
            ]
            .concat();
            attr(n | NESTED, &inner)
        };
        let groups = [group(1, b"config\0", 5), group(2, b"mlme\0", 8)].concat();
        let reply = message(
            GENL_ID_CTRL,
            0,
            1,
            1,
            2,
            &[
                (CTRL_ATTR_FAMILY_NAME, b"nl80211\0"),
                (CTRL_ATTR_FAMILY_ID, &0x1cu16.to_ne_bytes()),
                (CTRL_ATTR_MCAST_GROUPS | NESTED, &groups),
            ],
        );
        let (kind, body) = messages(&reply).next().unwrap();
        assert_eq!(kind, GENL_ID_CTRL);
        assert_eq!(
            parse_family(body),
            Some(Family {
                id: 0x1c,
                mlme: Some(8)
            })
        );
    }

    #[test]
    fn a_dump_counts_its_stations_up_to_done() {
        let station = |seq| message(0x1c, 0x2, seq, NL80211_CMD_NEW_STATION, 1, &[(1, b"x")]);
        let mut first = station(2);
        first.extend(station(2));
        assert_eq!(tally(&first, 0x1c).unwrap(), (2, false), "more to come");
        let mut last = station(2);
        last.extend(message(NLMSG_DONE, 0x2, 2, 0, 0, &[]));
        assert_eq!(tally(&last, 0x1c).unwrap(), (1, true));
        let none = message(NLMSG_DONE, 0x2, 2, 0, 0, &[]);
        assert_eq!(tally(&none, 0x1c).unwrap(), (0, true), "no stations");
    }

    #[test]
    fn a_refused_dump_is_an_error() {
        let mut err = message(NLMSG_ERROR, 0, 2, 0, 0, &[]);
        err.truncate(NLMSG_HDR);
        err.extend_from_slice(&(-libc::EOPNOTSUPP).to_ne_bytes());
        let len = err.len() as u32;
        err[0..4].copy_from_slice(&len.to_ne_bytes());
        let e = tally(&err, 0x1c).unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::EOPNOTSUPP));
    }

    #[test]
    fn only_station_events_wake() {
        assert!(joins_or_leaves(&[NL80211_CMD_NEW_STATION, 1, 0, 0]));
        assert!(joins_or_leaves(&[NL80211_CMD_DEL_STATION, 1, 0, 0]));
        assert!(!joins_or_leaves(&[NL80211_CMD_GET_STATION, 1, 0, 0]));
        assert!(!joins_or_leaves(&[]));
    }

    #[test]
    fn a_missing_interface_counts_nothing() {
        assert_eq!(count("no-such-if0"), None);
    }
}
