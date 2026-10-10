//! A secret `TextEdit` never hands the allocator a block that still holds
//! the typed text. Its own binary, because the check is a global allocator
//! that scans every block as it is freed.

use reprisekit::controls::form::{self, Form};
use reprisekit::raster::Rect;
use reprisekit::textedit::{EditKey, TextEdit};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

/// The first bytes of the typed secret. Every copy the edit could leave
/// behind (a clone, an undo snapshot, a buffer outgrown) starts with them.
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

#[test]
fn a_secret_edit_leaves_no_copy_of_its_text_in_freed_memory() {
    {
        let mut edit = TextEdit::secret();
        for c in "qZ7".chars().chain(std::iter::repeat_n('x', 600)) {
            edit.key(EditKey::Insert(c.to_string()));
            edit.place(1, true);
            edit.place(edit.text.len(), false);
        }
        edit.key(EditKey::Backspace { word: false });
        edit.key(EditKey::Home { select: false });
        edit.key(EditKey::Delete { word: false });
        edit.key(EditKey::Undo);
        edit.paste("qZ7 pasted", &[]);
        assert!(edit.text.contains("Z7"), "the edits reached the field");
    }
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
        form.paste(String::from("qZ7 pasted\nnext"));
    }
    assert!(
        !LEAKED.load(Ordering::SeqCst),
        "no freed block holds the secret"
    );
}
