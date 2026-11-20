//! A C library's allocator capped at a byte budget, for libraries that take an allocator of
//! `opaque`, `alloc(opaque, size)` and `free(opaque, address)` (libjxl's memory manager).

use std::alloc::Layout;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// What a library holds through [`alloc`]; a request past `limit`, or one the system allocator
/// cannot meet, gets null and sets `refused`.
/// Hand the library a pointer to it as `opaque`, alive until the library is done.
pub struct Budget {
    used: AtomicUsize,
    limit: usize,
    refused: AtomicBool,
}

impl Budget {
    pub fn new(limit: usize) -> Budget {
        Budget {
            used: AtomicUsize::new(0),
            limit,
            refused: AtomicBool::new(false),
        }
    }

    /// Whether a request was refused for the budget.
    pub fn refused(&self) -> bool {
        self.refused.load(Ordering::Relaxed)
    }
}

/// Each block starts with its total size, in a header that keeps malloc's 16-byte alignment.
const HEADER: usize = 16;

/// The allocator. It never panics: C calls it across FFI and treats null as failure.
///
/// # Safety
/// `opaque` points to a live [`Budget`].
pub unsafe extern "C-unwind" fn alloc(opaque: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: the caller's.
    let budget = unsafe { &*opaque.cast::<Budget>() };
    let refuse = || {
        budget.refused.store(true, Ordering::Relaxed);
        std::ptr::null_mut()
    };
    let Some(total) = size.checked_add(HEADER) else {
        return refuse();
    };
    let Ok(layout) = Layout::from_size_align(total, HEADER) else {
        return refuse();
    };
    let room = budget
        .used
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |u| {
            u.checked_add(total).filter(|&n| n <= budget.limit)
        });
    if room.is_err() {
        return refuse();
    }
    // SAFETY: `layout` is non-zero sized.
    let block = unsafe { std::alloc::alloc(layout) };
    if block.is_null() {
        budget.used.fetch_sub(total, Ordering::Relaxed);
        return refuse();
    }
    // SAFETY: `block` holds `total >= HEADER` bytes, aligned for usize.
    unsafe {
        block.cast::<usize>().write(total);
        block.add(HEADER).cast()
    }
}

/// The allocator's free.
///
/// # Safety
/// `address` is null or came from [`alloc`] with this `opaque`, and is freed once.
pub unsafe extern "C-unwind" fn free(opaque: *mut c_void, address: *mut c_void) {
    if address.is_null() {
        return;
    }
    // SAFETY: `address` came from `alloc` with this `opaque`, so its header precedes it.
    unsafe {
        let budget = &*opaque.cast::<Budget>();
        let block = address.cast::<u8>().sub(HEADER);
        let total = block.cast::<usize>().read();
        std::alloc::dealloc(block, Layout::from_size_align_unchecked(total, HEADER));
        budget.used.fetch_sub(total, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_the_system_allocator_cannot_meet_counts_as_refused() {
        let mut budget = Budget::new(usize::MAX);
        let opaque = (&raw mut budget).cast::<c_void>();
        // SAFETY: `opaque` points to `budget`, alive for both calls.
        let block = unsafe { alloc(opaque, 1 << 62) };
        assert!(block.is_null());
        assert!(budget.refused(), "a null from malloc sets refused");
        assert_eq!(budget.used.load(Ordering::Relaxed), 0);
    }
}
