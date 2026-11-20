//! A global allocator that holds one thread's heap to a byte budget while [`within`] runs, for
//! decoders that take no allocator of their own.
//!
//! While armed, the thread's live bytes are counted (allocated minus freed, never below zero, so
//! freeing older memory cannot underflow). A request of [`LARGE`] bytes or more that would pass the
//! budget gets null, which Rust code meets with a `try_reserve` error or `handle_alloc_error`;
//! smaller requests always succeed, so the many infallible small allocations never abort. Other
//! threads, and this one when not armed, go straight to the system allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// The allocator a binary installs as its `#[global_allocator]` to hold a budget.
pub struct Capped;

/// The smallest request refused past the budget.
pub const LARGE: usize = 64 << 10;

/// What [`within`] saw its closure allocate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spent {
    /// The most the closure held at once.
    pub peak: usize,
    /// Whether a request was refused.
    pub refused: bool,
    /// Requests of [`LARGE`] bytes or more.
    pub large: usize,
}

/// [`Capped`] is not this binary's global allocator, so no budget can be held.
#[derive(Debug, PartialEq, Eq)]
pub struct NotInstalled;

/// Runs `f` with this thread's allocations held to `limit` bytes. A nested call is held to the
/// room the outer one has left too, and what it spends counts toward the outer.
pub fn within<R>(limit: usize, f: impl FnOnce() -> R) -> Result<(R, Spent), NotInstalled> {
    within_refusing(limit, usize::MAX, f)
}

/// [`within`], also refusing the request numbered `nth` (from 0) among the large ones, for tests
/// that check a decoder survives each refusal.
pub fn within_refusing<R>(
    limit: usize,
    nth: usize,
    f: impl FnOnce() -> R,
) -> Result<(R, Spent), NotInstalled> {
    let outer = ARM.with(|a| a.get());
    let (room, outer_nth) = if outer.armed {
        let nth = outer.refuse_at.checked_sub(outer.large);
        (
            outer.limit.saturating_sub(outer.live),
            nth.unwrap_or(usize::MAX),
        )
    } else {
        (usize::MAX, usize::MAX)
    };
    let guard = Disarm(outer);
    ARM.with(|a| {
        a.set(Arm {
            armed: true,
            limit: limit.min(room),
            refuse_at: nth.min(outer_nth),
            ..Arm::OFF
        })
    });
    let probe = std::hint::black_box(Box::new(0u8));
    let seen = ARM.with(|a| a.get().live) != 0;
    drop(probe);
    if !seen {
        return Err(NotInstalled);
    }
    let out = f();
    let inner = ARM.with(|a| a.get());
    drop(guard);
    if outer.armed {
        ARM.with(|a| {
            let mut o = a.get();
            o.peak = o.peak.max(o.live.saturating_add(inner.peak));
            o.live = o.live.saturating_add(inner.live);
            o.refused |= inner.refused;
            o.large = o.large.saturating_add(inner.large);
            a.set(o);
        });
    }
    let spent = Spent {
        peak: inner.peak,
        refused: inner.refused,
        large: inner.large,
    };
    Ok((out, spent))
}

#[derive(Clone, Copy)]
struct Arm {
    armed: bool,
    live: usize,
    peak: usize,
    limit: usize,
    refused: bool,
    large: usize,
    refuse_at: usize,
}

impl Arm {
    const OFF: Arm = Arm {
        armed: false,
        live: 0,
        peak: 0,
        limit: 0,
        refused: false,
        large: 0,
        refuse_at: usize::MAX,
    };
}

thread_local! {
    // Const-initialised and without a destructor, so reading it never allocates.
    static ARM: Cell<Arm> = const { Cell::new(Arm::OFF) };
}

/// Puts back the arming a [`within`] found, also when its closure panics.
struct Disarm(Arm);

impl Drop for Disarm {
    fn drop(&mut self) {
        ARM.with(|a| a.set(self.0));
    }
}

/// Whether a request for a block of `size` bytes, growing the thread's heap by `grow`, may go
/// ahead, counting it when it may. True when not armed; a shrinking `realloc` is never refused.
fn admit(size: usize, grow: usize) -> bool {
    ARM.try_with(|a| {
        let mut s = a.get();
        if !s.armed {
            return true;
        }
        let live = s.live.saturating_add(grow);
        if size >= LARGE && grow > 0 {
            let n = s.large;
            s.large += 1;
            if live > s.limit || n == s.refuse_at {
                s.refused = true;
                a.set(s);
                return false;
            }
        }
        s.live = live;
        s.peak = s.peak.max(live);
        a.set(s);
        true
    })
    .unwrap_or(true)
}

/// Uncounts `size` bytes freed, or given back by a failed request.
fn release(size: usize) {
    let _ = ARM.try_with(|a| {
        let mut s = a.get();
        if s.armed {
            s.live = s.live.saturating_sub(size);
            a.set(s);
        }
    });
}

// SAFETY: every block comes from and goes back to `System` with the caller's layout; refusing
// returns null, which `GlobalAlloc` allows.
unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !admit(layout.size(), layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's `alloc` contract, passed on.
        let p = unsafe { System.alloc(layout) };
        if p.is_null() {
            release(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !admit(layout.size(), layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's `alloc_zeroed` contract, passed on.
        let p = unsafe { System.alloc_zeroed(layout) };
        if p.is_null() {
            release(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        release(layout.size());
        // SAFETY: the caller's `dealloc` contract, passed on.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old = layout.size();
        let grow = new_size.saturating_sub(old);
        if !admit(new_size, grow) {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's `realloc` contract, passed on.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if p.is_null() {
            release(grow);
        } else {
            release(old.saturating_sub(new_size));
        }
        p
    }
}
