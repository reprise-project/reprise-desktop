use reprisebus::bounded::{Dict, Value, decode};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use zbus::{Message, Variant};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

// SAFETY: every call goes to `System` unchanged; only the byte counts are kept.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc`, passed through.
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            grew(l.size());
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract for `dealloc`, passed through.
        unsafe { System.dealloc(p, l) }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, size: usize) -> *mut u8 {
        // SAFETY: the caller's contract for `realloc`, passed through.
        let q = unsafe { System.realloc(p, l, size) };
        if !q.is_null() {
            LIVE.fetch_sub(l.size(), Ordering::Relaxed);
            grew(size);
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// A 2 MiB `ay` next to a number decodes in under 64 KiB; one `Variant` per byte would be ~80 MiB.
#[test]
fn a_large_byte_array_is_skipped_without_allocating_for_it() {
    let bytes = vec![7u8; 2 << 20];
    let body = (
        "application://mail.desktop",
        HashMap::from([
            ("junk", Variant::from(&bytes[..])),
            ("progress", Variant::from(0.5f64)),
        ]),
    );
    let msg = Message::signal("/a", "org.example.A", "Update")
        .build(&body)
        .unwrap();
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let (_, props) = decode::<(String, Dict)>(&msg).unwrap();
    let peak = PEAK.load(Ordering::Relaxed) - base;
    assert!(peak < 64 << 10, "decoding allocated {peak} bytes");
    assert_eq!(props.get("progress").and_then(Value::float), Some(0.5));
    assert_eq!(props.get("junk"), Some(&Value::Other));
}
