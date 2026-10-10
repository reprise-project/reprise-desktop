//! The X11 display held for Xwayland from login: lock file and listening
//! sockets, handed to Xwayland on the first connection. Smithay's `XWayland::spawn`
//! takes the lock and path socket itself, so `hand_over` gives them up and
//! `take_back` regrabs them; the abstract socket stays here.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{SocketAddr, UnixListener};

const DISPLAYS: std::ops::Range<u32> = 0..33;

pub struct Sockets {
    display: u32,
    abstract_socket: UnixListener,
    path: Option<UnixListener>,
    locked: bool,
}

/// The fds an Xwayland spawn listens on, without `CLOEXEC` so the server inherits them.
pub struct HandOver {
    fds: Vec<OwnedFd>,
    _path: Option<UnixListener>,
}

impl HandOver {
    pub fn args(&self) -> Vec<String> {
        self.fds
            .iter()
            .flat_map(|fd| ["-listenfd".to_string(), fd.as_raw_fd().to_string()])
            .collect()
    }
}

impl Sockets {
    pub fn reserve(prefer: Option<u32>) -> io::Result<Sockets> {
        let Some(d) = prefer else {
            return DISPLAYS
                .clone()
                .find_map(|d| Sockets::reserve(Some(d)).ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::AddrInUse, "no free X11 display"));
        };
        lock(d)?;
        let bound = bind_abstract(d).and_then(|a| Ok((a, bind_path(d)?)));
        match bound {
            Ok((abstract_socket, path)) => Ok(Sockets {
                display: d,
                abstract_socket,
                path: Some(path),
                locked: true,
            }),
            Err(e) => {
                let _ = fs::remove_file(lock_path(d));
                Err(e)
            }
        }
    }

    pub fn display(&self) -> u32 {
        self.display
    }

    pub fn watch_fds(&self) -> io::Result<Vec<OwnedFd>> {
        let mut fds = vec![self.abstract_socket.as_fd().try_clone_to_owned()?];
        if let Some(p) = &self.path {
            fds.push(p.as_fd().try_clone_to_owned()?);
        }
        Ok(fds)
    }

    /// Before a spawn: release the lock and path socket for Smithay.
    pub fn hand_over(&mut self) -> io::Result<HandOver> {
        let mut fds = vec![inheritable(&self.abstract_socket)?];
        if let Some(p) = &self.path {
            fds.push(inheritable(p)?);
        }
        if self.locked {
            fs::remove_file(lock_path(self.display))?;
            self.locked = false;
        }
        Ok(HandOver {
            fds,
            _path: self.path.take(),
        })
    }

    /// After the server went: hold both again.
    pub fn take_back(&mut self) -> io::Result<()> {
        if !self.locked {
            lock(self.display)?;
            self.locked = true;
        }
        if self.path.is_none() {
            self.path = Some(bind_path(self.display)?);
        }
        Ok(())
    }
}

impl Drop for Sockets {
    fn drop(&mut self) {
        if self.path.take().is_some() {
            let _ = fs::remove_file(socket_path(self.display));
        }
        if self.locked {
            let _ = fs::remove_file(lock_path(self.display));
        }
    }
}

fn lock_path(d: u32) -> String {
    format!("/tmp/.X{d}-lock")
}

fn socket_path(d: u32) -> String {
    format!("/tmp/.X11-unix/X{d}")
}

fn lock_contents(pid: u32) -> String {
    format!("{pid:>10}\n")
}

fn lock_pid(contents: &str) -> Option<i32> {
    contents.trim().parse().ok().filter(|&p| p > 0)
}

/// Take display `d`'s lock file; one left by a dead process is taken over.
fn lock(d: u32) -> io::Result<()> {
    let name = lock_path(d);
    let open = || {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o444)
            .open(&name)
    };
    let mut file = match open() {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let mut held = String::new();
            fs::File::open(&name)?.read_to_string(&mut held)?;
            match lock_pid(&held) {
                Some(pid) if !alive(pid) => {
                    fs::remove_file(&name)?;
                    open()?
                }
                _ => return Err(e),
            }
        }
        Err(e) => return Err(e),
    };
    let written = file.write_all(lock_contents(std::process::id()).as_bytes());
    if let Err(e) = written {
        let _ = fs::remove_file(&name);
        return Err(e);
    }
    Ok(())
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

fn bind_abstract(d: u32) -> io::Result<UnixListener> {
    UnixListener::bind_addr(&SocketAddr::from_abstract_name(socket_path(d))?)
}

fn bind_path(d: u32) -> io::Result<UnixListener> {
    let path = socket_path(d);
    let _ = fs::remove_file(&path);
    UnixListener::bind(path)
}

fn inheritable(l: &UnixListener) -> io::Result<OwnedFd> {
    Ok(rustix::io::dup(l)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_file_names_its_holder_as_x_servers_write_it() {
        assert_eq!(lock_contents(4242), "      4242\n");
        assert_eq!(lock_pid(&lock_contents(4242)), Some(4242));
        assert_eq!(lock_pid("garbage"), None);
        assert_eq!(lock_pid("         0\n"), None, "no process holds pid 0");
    }

    #[test]
    fn a_lock_held_by_a_gone_process_is_stale() {
        assert!(alive(std::process::id() as i32));
        assert!(!alive(i32::MAX), "no process has the largest pid");
    }

    #[test]
    fn a_hand_over_passes_each_socket_without_cloexec() {
        let name = format!("reprise-test-listenfd-{}", std::process::id());
        let l = UnixListener::bind_addr(&SocketAddr::from_abstract_name(name).unwrap()).unwrap();
        let fd = inheritable(&l).unwrap();
        let flags = rustix::io::fcntl_getfd(&fd).unwrap();
        assert!(!flags.contains(rustix::io::FdFlags::CLOEXEC));
        let h = HandOver {
            fds: vec![fd],
            _path: None,
        };
        let n = h.fds[0].as_raw_fd().to_string();
        assert_eq!(h.args(), ["-listenfd".to_string(), n]);
    }
}
