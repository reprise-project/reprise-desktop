//! libinput's device files, opened and released through the session.

use std::{cell::RefCell, os::fd::OwnedFd, path::Path, rc::Rc};

use smithay::{
    backend::session::{AsErrno, Session},
    reexports::{input::LibinputInterface, rustix::fs::OFlags},
};

/// Device files libinput has closed but the session has not released yet.
pub type Held = Rc<RefCell<Vec<OwnedFd>>>;

/// Holds each close until `release`: logind answers ReleaseDevice only once a VT switch is
/// done, so releasing the devices the switch revoked would block the main loop through it.
pub struct InputFds<S: Session> {
    session: S,
    held: Held,
}

impl<S: Session> InputFds<S> {
    pub fn new(session: S, held: Held) -> Self {
        Self { session, held }
    }
}

impl<S: Session> LibinputInterface for InputFds<S> {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        // A device still taken from an earlier open would refuse this one.
        release(&mut self.session, &self.held);
        self.session
            .open(path, OFlags::from_bits_truncate(flags as u32))
            .map_err(|e| e.as_errno().unwrap_or(1))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        self.held.borrow_mut().push(fd);
    }
}

/// Releases the held device files, only while the session is active.
pub fn release<S: Session>(session: &mut S, held: &Held) {
    if !session.is_active() {
        return;
    }
    for fd in held.take() {
        if let Err(e) = session.close(fd) {
            log::warn!("input device release: {e:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, fs::File};

    #[derive(Clone, Default)]
    struct FakeSession {
        active: Rc<Cell<bool>>,
        opened: Rc<Cell<usize>>,
        closed: Rc<Cell<usize>>,
    }

    #[derive(Debug)]
    struct NoErr;
    impl AsErrno for NoErr {
        fn as_errno(&self) -> Option<i32> {
            None
        }
    }

    impl Session for FakeSession {
        type Error = NoErr;
        fn open(&mut self, _: &Path, _: OFlags) -> Result<OwnedFd, NoErr> {
            self.opened.set(self.opened.get() + 1);
            Ok(File::open("/dev/null").unwrap().into())
        }
        fn close(&mut self, _: OwnedFd) -> Result<(), NoErr> {
            self.closed.set(self.closed.get() + 1);
            Ok(())
        }
        fn change_vt(&mut self, _: i32) -> Result<(), NoErr> {
            Ok(())
        }
        fn is_active(&self) -> bool {
            self.active.get()
        }
        fn seat(&self) -> String {
            "seat0".into()
        }
    }

    fn null() -> OwnedFd {
        File::open("/dev/null").unwrap().into()
    }

    #[test]
    fn a_close_during_a_pause_is_released_only_once_the_session_is_back() {
        let mut s = FakeSession::default();
        let held = Held::default();
        let mut fds = InputFds::new(s.clone(), held.clone());
        fds.close_restricted(null());
        fds.close_restricted(null());
        assert_eq!(s.closed.get(), 0, "a close is held, not released at once");
        release(&mut s, &held);
        assert_eq!(s.closed.get(), 0, "nothing is released while paused");
        s.active.set(true);
        release(&mut s, &held);
        assert_eq!(s.closed.get(), 2, "both devices are released on return");
        assert!(held.borrow().is_empty());
    }

    #[test]
    fn an_open_releases_the_held_devices_first() {
        let s = FakeSession::default();
        s.active.set(true);
        let held = Held::default();
        let mut fds = InputFds::new(s.clone(), held.clone());
        fds.close_restricted(null());
        fds.open_restricted(Path::new("/dev/input/event0"), 0)
            .unwrap();
        assert_eq!(
            s.closed.get(),
            1,
            "the held device is released before the open"
        );
        assert_eq!(s.opened.get(), 1);
    }
}
