//! A dma-buf mapped for reading on the CPU.

use super::EncodeError;
use std::os::fd::{AsRawFd, OwnedFd};

/// A dma-buf mapped for reading, inside a CPU access window the driver keeps coherent.
pub struct DmabufRead<'a> {
    fd: &'a OwnedFd,
    ptr: *mut libc::c_void,
    len: usize,
}

/// `struct dma_buf_sync` and `DMA_BUF_IOCTL_SYNC` from linux/dma-buf.h.
#[repr(C)]
struct DmaBufSync {
    flags: u64,
}
const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x4008_6200;
const DMA_BUF_SYNC_READ: u64 = 1;
const DMA_BUF_SYNC_END: u64 = 4;

impl<'a> DmabufRead<'a> {
    pub fn new(fd: &'a OwnedFd, len: usize) -> Result<DmabufRead<'a>, EncodeError> {
        // SAFETY: a fresh read-only shared mapping of the dma-buf; the result is checked.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(EncodeError(format!(
                "dma-buf map: {}",
                std::io::Error::last_os_error()
            )));
        }
        if let Err(e) = sync(fd, DMA_BUF_SYNC_READ) {
            // SAFETY: the mapping made above, which nothing else holds.
            unsafe { libc::munmap(ptr, len) };
            return Err(EncodeError(format!("dma-buf sync: {e}")));
        }
        Ok(DmabufRead { fd, ptr, len })
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` maps `len` readable bytes until drop.
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

fn sync(fd: &OwnedFd, flags: u64) -> std::io::Result<()> {
    let arg = DmaBufSync { flags };
    loop {
        // SAFETY: DMA_BUF_IOCTL_SYNC reads one dma_buf_sync from `arg`, which outlives the call.
        if unsafe { libc::ioctl(fd.as_raw_fd(), DMA_BUF_IOCTL_SYNC, &arg) } == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if !matches!(e.raw_os_error(), Some(libc::EINTR | libc::EAGAIN)) {
            return Err(e);
        }
    }
}

impl Drop for DmabufRead<'_> {
    fn drop(&mut self) {
        if let Err(e) = sync(self.fd, DMA_BUF_SYNC_READ | DMA_BUF_SYNC_END) {
            log::warn!("dma-buf sync end: {e}");
        }
        // SAFETY: this value's own mapping, unmapped once.
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;

    #[test]
    fn a_buffer_without_a_cpu_access_window_is_not_read() {
        // SAFETY: a fresh memfd, owned from here on.
        let fd = unsafe { OwnedFd::from_raw_fd(libc::memfd_create(c"frame".as_ptr(), 0)) };
        std::fs::File::from(fd.try_clone().unwrap())
            .set_len(4096)
            .unwrap();
        let Err(EncodeError(e)) = DmabufRead::new(&fd, 4096) else {
            panic!("a memfd has no DMA_BUF_IOCTL_SYNC, so it is not mapped");
        };
        assert!(e.starts_with("dma-buf sync:"), "{e}");
    }
}
