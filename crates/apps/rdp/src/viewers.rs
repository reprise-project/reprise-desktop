//! The connected viewers, each with a way to end it from this side.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::oneshot;

/// Connections from one address still logging in at once; xfreerdp3 opens two about 5 ms apart.
pub const LOGGING_IN_PER_IP: usize = 2;

/// Why a connection was ended from this side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kick {
    Asked,
    PortChanged,
    /// The server is stopping.
    Stopped,
    /// Another connection logged in with `second_client = "replace"`.
    Replaced,
    /// The session is ending.
    LoggedOff,
}

/// Why [`Viewers::join`] turned a connection away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// `max_viewers` connections are already served.
    Full,
    /// [`LOGGING_IN_PER_IP`] connections from this address are still logging in.
    LoggingIn(IpAddr),
}

/// One connection's place in [`Viewers`]; never given out twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewerId(u64);

struct Entry {
    peer: SocketAddr,
    logged_in: bool,
    kick: Option<oneshot::Sender<Kick>>,
}

#[derive(Default)]
pub struct Viewers {
    next: u64,
    live: HashMap<ViewerId, Entry>,
}

impl Viewers {
    /// A place for `peer` and the receiver its kick arrives on, unless it is [`Refused`].
    pub fn join(
        &mut self,
        peer: SocketAddr,
        cap: usize,
    ) -> Result<(ViewerId, oneshot::Receiver<Kick>), Refused> {
        if self.live.len() >= cap {
            return Err(Refused::Full);
        }
        let ip = peer.ip().to_canonical();
        let logging_in = (self.live.values())
            .filter(|e| !e.logged_in && e.peer.ip().to_canonical() == ip)
            .count();
        if logging_in >= LOGGING_IN_PER_IP {
            return Err(Refused::LoggingIn(ip));
        }
        let id = ViewerId(self.next);
        self.next += 1;
        let (kick, kicked) = oneshot::channel();
        let entry = Entry {
            peer,
            logged_in: false,
            kick: Some(kick),
        };
        self.live.insert(id, entry);
        Ok((id, kicked))
    }

    /// Marks `id` logged in, so it no longer counts towards [`LOGGING_IN_PER_IP`].
    pub fn logged_in(&mut self, id: ViewerId) {
        if let Some(e) = self.live.get_mut(&id) {
            e.logged_in = true;
        }
    }

    /// Frees `id`'s place; whether it still had one.
    pub fn leave(&mut self, id: ViewerId) -> bool {
        self.live.remove(&id).is_some()
    }

    /// Ends the viewer whose peer reads as `address`; whether there was one.
    pub fn kick(&mut self, address: &str, why: Kick) -> bool {
        let found = self
            .live
            .values_mut()
            .find(|e| e.peer.to_string() == address);
        found.map(|e| e.end(why)).is_some()
    }

    pub fn kick_all(&mut self, why: Kick) {
        self.live.values_mut().for_each(|e| e.end(why));
    }

    /// Ends every connection but `keep`'s, each told `why` once logged in.
    pub fn kick_others(&mut self, keep: ViewerId, why: Kick) {
        (self.live.iter_mut())
            .filter(|(id, _)| **id != keep)
            .for_each(|(_, e)| e.end(why));
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }
}

impl Entry {
    fn end(&mut self, why: Kick) {
        if let Some(kick) = self.kick.take() {
            let _ = kick.send(why);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot::error::TryRecvError;

    /// `address` as the intake gives it, an IPv4-mapped address as IPv4.
    fn peer(address: &str) -> SocketAddr {
        let a: SocketAddr = address.parse().unwrap();
        SocketAddr::new(a.ip().to_canonical(), a.port())
    }

    #[test]
    fn join_refuses_at_the_cap() {
        let mut v = Viewers::default();
        let (first, _k1) = v.join(peer("10.0.0.1:1"), 2).expect("the first joins");
        let _second = v.join(peer("10.0.0.2:2"), 2).expect("the second joins");
        assert_eq!(
            v.join(peer("10.0.0.3:3"), 2).err(),
            Some(Refused::Full),
            "the third is refused"
        );
        assert_eq!(v.len(), 2);
        v.leave(first);
        assert!(
            v.join(peer("10.0.0.3:3"), 2).is_ok(),
            "a freed place is taken"
        );
    }

    #[test]
    fn a_third_connection_logging_in_from_one_ip_is_refused() {
        let mut v = Viewers::default();
        v.join(peer("10.0.0.1:1"), 16).unwrap();
        let mapped: SocketAddr = "[::ffff:10.0.0.1]:2".parse().unwrap();
        v.join(mapped, 16).unwrap();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(
            v.join(peer("10.0.0.1:3"), 16).err(),
            Some(Refused::LoggingIn(ip)),
            "a third from 10.0.0.1 is refused"
        );
        assert!(
            v.join(peer("10.0.0.2:1"), 16).is_ok(),
            "another address is let in"
        );
    }

    #[test]
    fn a_logged_in_connection_does_not_count_as_logging_in() {
        let mut v = Viewers::default();
        let (first, _) = v.join(peer("10.0.0.1:1"), 16).unwrap();
        v.join(peer("10.0.0.1:2"), 16).unwrap();
        v.logged_in(first);
        assert!(
            v.join(peer("10.0.0.1:3"), 16).is_ok(),
            "one of the two has logged in"
        );
    }

    #[test]
    fn kick_ends_only_the_matching_address() {
        let mut v = Viewers::default();
        let (_, mut mapped) = v.join(peer("[::ffff:10.0.0.5]:5000"), 4).unwrap();
        let (_, mut plain) = v.join(peer("10.0.0.5:5001"), 4).unwrap();
        assert!(v.kick("10.0.0.5:5001", Kick::Asked));
        assert_eq!(plain.try_recv(), Ok(Kick::Asked));
        assert_eq!(mapped.try_recv(), Err(TryRecvError::Empty));
        assert!(
            !v.kick("10.0.0.5:5002", Kick::Asked),
            "no viewer at that port"
        );
    }

    #[test]
    fn kick_all_fires_every_receiver() {
        let mut v = Viewers::default();
        let (_, mut a) = v.join(peer("10.0.0.1:1"), 4).unwrap();
        let (_, mut b) = v.join(peer("10.0.0.1:2"), 4).unwrap();
        v.kick_all(Kick::PortChanged);
        assert_eq!(a.try_recv(), Ok(Kick::PortChanged));
        assert_eq!(b.try_recv(), Ok(Kick::PortChanged));
    }

    #[test]
    fn a_viewer_kicked_once_is_not_kicked_again_by_the_stop() {
        let mut v = Viewers::default();
        let (_, mut a) = v.join(peer("10.0.0.1:1"), 4).unwrap();
        let (_, mut b) = v.join(peer("10.0.0.2:1"), 4).unwrap();
        v.kick_all(Kick::LoggedOff);
        v.kick_all(Kick::Stopped);
        for kicked in [&mut a, &mut b] {
            assert_eq!(kicked.try_recv(), Ok(Kick::LoggedOff));
            assert_eq!(kicked.try_recv(), Err(TryRecvError::Closed));
        }
    }

    #[test]
    fn kick_others_kicks_every_viewer_but_the_one_given() {
        let mut v = Viewers::default();
        let (_, mut a) = v.join(peer("10.0.0.1:1"), 4).unwrap();
        let (b, mut kept) = v.join(peer("10.0.0.2:1"), 4).unwrap();
        let (_, mut c) = v.join(peer("10.0.0.3:1"), 4).unwrap();
        v.kick_others(b, Kick::Replaced);
        assert_eq!(a.try_recv(), Ok(Kick::Replaced));
        assert_eq!(c.try_recv(), Ok(Kick::Replaced));
        assert_eq!(kept.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn leave_twice_reports_only_the_first() {
        let mut v = Viewers::default();
        let (id, _) = v.join(peer("10.0.0.1:1"), 4).unwrap();
        assert!(v.leave(id), "the first leave frees the place");
        assert!(!v.leave(id), "the second finds no place");
    }

    #[test]
    fn ids_are_not_reused() {
        let mut v = Viewers::default();
        let (first, _) = v.join(peer("10.0.0.1:1"), 4).unwrap();
        v.leave(first);
        let (second, _) = v.join(peer("10.0.0.1:1"), 4).unwrap();
        assert_ne!(first, second);
    }
}
