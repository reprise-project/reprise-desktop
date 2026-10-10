//! A `PasteBuf` never hands the allocator a block that still holds the pasted bytes.
//! Its own binary: the check is a global allocator that scans every block as
//! it is freed.

use reprisefs::pipe::PasteBuf;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, Ordering};
use zeroize::Zeroize;

const NEEDLE: &[u8] = b"qZ7";

static LEAKED: AtomicBool = AtomicBool::new(false);

struct Scan;

// SAFETY: forwards to `System`; `dealloc` only reads the block it is handed,
// which is live until the forwarded call. `realloc` keeps the default
// alloc-copy-dealloc, so an outgrown buffer passes through `dealloc` too.
unsafe impl GlobalAlloc for Scan {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's `alloc` contract, passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` is a live block of `layout.size()` bytes that this
        // allocator handed out.
        let block = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
        if block.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
            LEAKED.store(true, Ordering::SeqCst);
        }
        // SAFETY: the caller's `dealloc` contract, passed on unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Scan = Scan;

fn filled(tail: &[u8]) -> PasteBuf {
    let mut buf = PasteBuf::default();
    assert!(buf.push(NEEDLE));
    for _ in 0..8 {
        assert!(buf.push(&[b'x'; 4096]));
    }
    assert!(buf.push(tail));
    buf
}

#[test]
fn a_paste_buffer_leaves_no_copy_of_its_bytes_in_freed_memory() {
    let mut text = filled(b"end").into_text();
    assert!(text.starts_with("qZ7") && text.ends_with("end"));
    text.zeroize();
    let mut text = filled(b"\xff").into_text();
    assert!(text.starts_with("qZ7") && text.ends_with('\u{fffd}'));
    text.zeroize();
    let buf = filled(b"");
    assert!(buf.bytes().starts_with(NEEDLE));
    drop(buf);
    assert!(
        !LEAKED.load(Ordering::SeqCst),
        "no freed block holds the pasted bytes"
    );
}
