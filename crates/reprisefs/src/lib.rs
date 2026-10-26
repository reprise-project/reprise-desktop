//! The file-system side of the shell, shared by the compositor (desktop icons, Send To, Properties, the
//! launcher's file search) and Explorer. `std` plus `libc` for system calls `std` lacks, zbus for
//! UDisks2, png and md5 for the thumbnail cache, and toml_edit for the Recycle Bin's settings in
//! `config.toml`; nothing here knows about a display, a seat or the compositor.
pub mod apps;
pub mod autostart;
pub mod bincap;
pub mod deadline;
pub mod dirchange;
pub mod drawncache;
pub mod dropdata;
pub mod fileclip;
pub mod folders;
pub mod gvfsmeta;
pub mod hidden;
pub mod history;
pub mod icons;
pub mod imagesize;
pub mod jobs;
pub mod launcher;
pub mod listing;
pub mod locale;
pub mod meta;
pub mod mimetree;
pub mod multicall;
pub mod names;
pub mod ops;
pub mod pictures;
pub mod pipe;
pub mod places;
pub mod program;
pub mod progress;
pub mod props;
pub mod quicklaunch;
pub mod recent;
pub mod removal;
pub mod replace;
pub mod scope;
pub mod search;
pub mod sendto;
pub mod sysinfo;
pub mod thumbnails;
pub mod thumbpipe;
pub mod trash;
pub mod udisks;
pub mod undo;
pub mod xkb;

#[cfg(test)]
pub(crate) mod testfs {
    use std::path::Path;

    pub fn mkfifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c` is a NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    /// `f`'s answer, run on a thread of its own; panics when it takes over five seconds, as a read blocked on a FIFO does.
    pub fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("returns without blocking")
    }
}
