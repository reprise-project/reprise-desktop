use crate::{Actions, DragAction, Dropped, FileClip, Offered, heads};
use reprisekit::textedit::EditKey;
use std::path::PathBuf;
use zeroize::Zeroize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    /// The X keysym (see `reprisekit::textedit::keysym`); 0 (NoSymbol)
    /// for an input method's commit (its text in `utf8` too) or delete.
    pub sym: u32,
    pub utf8: Option<String>,
    pub mods: Mods,
    pub edit: Option<EditKey>,
}

/// A key may be a password's character, so its text is zeroized once the
/// program is done with it.
impl Drop for KeyInput {
    fn drop(&mut self) {
        self.utf8.zeroize();
        if let Some(EditKey::Insert(s)) = &mut self.edit {
            s.zeroize();
        }
    }
}

pub const BTN_LEFT: u32 = 0x110;

pub const BTN_RIGHT: u32 = 0x111;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerInput {
    Motion {
        x: i32,
        y: i32,
    },
    /// `button` is the evdev code (0x110 left, 0x111 right, 0x112 middle).
    Press {
        x: i32,
        y: i32,
        button: u32,
        time: u32,
        mods: Mods,
    },
    Release {
        x: i32,
        y: i32,
        button: u32,
        mods: Mods,
    },
    /// Vertical scroll in rows (positive is down), `reprisekit::pointer::WHEEL_ROWS`
    /// per wheel notch.
    Scroll {
        x: i32,
        y: i32,
        rows: i32,
    },
    Leave,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Key(KeyInput),
    /// A key came back up. A menu bar activates on Alt's release,
    /// not on the key after it.
    KeyUp(KeyInput),
    Pointer(PointerInput),
    Resized,
    /// The main window is on an output this big, in logical px: sent
    /// each time it enters one (with several, the last it entered).
    Output((i32, i32)),
    /// The caret's blink flipped (`ctx.caret_on`). It goes to the window
    /// with the keyboard, else the one under the pointer, else nowhere, and
    /// only while that window's last frame drew a focused field or one of its
    /// forms waits to show a label tip.
    Blink,
    Tick,
    Pasted(String),
    /// The primary selection's text, asked for with `Reply::paste_primary`.
    PastedPrimary(String),
    PastedFiles(FileClip),
    Focus(bool),
    /// The compositor took its shortcuts back from the window that held
    /// them ([`App::shortcuts_inhibited`](crate::App::shortcuts_inhibited)), on the user's
    /// Super+Shift+Escape. The modifiers held for that chord are no
    /// longer a hot-key field's to record.
    ShortcutsInactive,
    /// The frame's X, or the window menu's Close. On `MAIN` the program
    /// answers `Reply::quit`; on a child it drops that `Child`. On a
    /// flyout, the compositor took it away; the program stops returning
    /// it from `App::flyout`.
    CloseRequest,
    /// Something off the event loop's thread has work for the program (see
    /// `Waker::wake`). Delivered on `MAIN` even when the main window is
    /// hidden, so `win` means nothing.
    Wake,
    /// A drag from any program came over this window, at `(x, y)`. The program answers this and
    /// every `DragMotion` with [`Reply::drag_accept`](crate::Reply::drag_accept); without an answer
    /// the last one stands, and a new drag starts refused. `source` is what the dragging program
    /// allows; `action` is what a drop would do as the compositor has it now (None before the first
    /// answer). `own` is a drag this program started and has not dropped yet, whose files it knows
    /// already.
    DragEnter {
        x: i32,
        y: i32,
        offered: Offered,
        source: Actions,
        action: Option<DragAction>,
        own: bool,
    },
    DragMotion {
        x: i32,
        y: i32,
        action: Option<DragAction>,
    },
    /// The files of another program's drag, read ahead of the drop and sent
    /// once after `DragEnter` (an `own` drag's are known), so the program can
    /// pick move or copy, or refuse a folder dropped into itself. Empty for
    /// web addresses.
    DragFiles {
        paths: Vec<PathBuf>,
    },
    DragLeave,
    /// The drag was dropped here; this ends it the way `DragLeave` would.
    /// With `DragAction::Ask` the program asks the user, then answers
    /// with [`Reply::answer_drop`](crate::Reply::answer_drop) (now or later), picking among
    /// `source`, what the dragging program allows (a drop menu greys the
    /// rest); any other action is already settled when this arrives.
    DragDrop {
        x: i32,
        y: i32,
        data: Dropped,
        action: DragAction,
        source: Actions,
    },
    /// A drag dropped on a field of the window's `App::drop_form` changed it (`Changed`
    /// with the field's id), as a key or paste in it would; `App::drop_file_dialog`'s File
    /// name reports `fielddrop::FILE_NAME`.
    FieldDropped(reprisekit::controls::form::Event),
    /// The `Ask` drop this window was answering is gone: the dragging
    /// program or the compositor dropped it, or the answer was an action
    /// the source does not allow. Nothing was finished.
    DropCancelled,
    /// The drag this window started with [`Reply::start_drag`](crate::Reply::start_drag) is over.
    /// `dropped`: released over a target that took it. `action`: what the
    /// target said it did, None when it never said (refusal, cancel, or a
    /// target that drops the offer unfinished, as some GTK programs do). The
    /// target does a move itself; the source deletes nothing.
    DragEnd {
        dropped: bool,
        action: Option<DragAction>,
    },
    /// The outputs as the compositor has them, on `MAIN`, after every change, for a program that
    /// asked ([`App::output_management`](crate::App::output_management)). Empty when the compositor
    /// offers no output management.
    Heads(Vec<heads::Head>),
    HeadsApplied {
        serial: u32,
        ok: bool,
    },
}
