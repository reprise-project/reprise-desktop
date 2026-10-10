//! A key typed into a password field leaves no copy of its text in freed
//! memory once the host's `KeyInput` is dropped. Its own binary, because
//! the check is a global allocator that scans every block as it is freed.

use repriseapp::KeyInput;
use repriseapp::keys::dialog_key;
use reprisekit::controls::form::{self, Form};
use reprisekit::raster::Rect;
use reprisekit::textedit::{TextEdit, key_for};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

/// What one key press types (a compose result can be several chars).
const NEEDLE: &[u8] = b"qZ7";

static LEAKED: AtomicBool = AtomicBool::new(false);

struct Scan;

// SAFETY: forwards to `System`; `dealloc` only reads the block it is handed,
// which is live until the forwarded call.
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

#[test]
fn a_key_typed_into_a_password_field_leaves_no_copy_in_freed_memory() {
    {
        let mut form = Form::new(vec![form::Item::new(
            "pw",
            Rect::new(0, 0, 100, 20),
            form::Control::Edit {
                edit: TextEdit::secret(),
                first: Cell::new(0),
                enabled: true,
            },
        )]);
        form.set_focus(Some(0));
        let utf8 = Some(String::from_utf8(NEEDLE.to_vec()).unwrap());
        let edit = key_for(0x71, false, false, utf8.as_deref());
        let key = KeyInput {
            sym: 0x71,
            utf8,
            mods: repriseapp::Mods::default(),
            edit,
        };
        form.key(dialog_key(&key).expect("an insert"));
        drop(key);
        let form::Control::Edit { edit, .. } = &form.items[0].control else {
            unreachable!()
        };
        assert_eq!(edit.text, "qZ7", "the key reached the field");
    }
    assert!(
        !LEAKED.load(Ordering::SeqCst),
        "no freed block holds the typed text"
    );
}
